#![forbid(unsafe_code)]
//! Strict `R10.2` zero-shortcut human-path acceptance tests for `WF-07` through `WF-12`.
//!
//! Every workflow stage (table creation via Welcome, grid entry/import, analysis selection,
//! `Cmd+T` parameter configuration & `R1`–`R9` modal predicates, diagnostics-only banner checks,
//! multi-column results, linked graph configuration & brackets, and live upstream cell/exclusion
//! recomputes) is driven exclusively through [`HumanPathDriver`] pointer/keyboard/wheel events.

mod human_wf_common;

use std::fmt::Write as _;

use curvebook_doc::{
    AnalysisKind, ChecklistStatus, CiBound, GraphKind, GraphSheet, GraphicalDataObject, GroupedArrangement, MultiplicityMethod, OneWayMethod,
    PValueFormat, ResultCell, ResultSheet, ResultStatus, ResultTab, SheetId, TableKind, TwoGroupMethod,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::{AssumptionDiagnosticBanner, HumanPathDriver, KeyModifiers, UiApp, WidgetId};
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, scroll_widget_wheel, type_into_focused, widget_hit_center,
};

type TestRes = Result<(), String>;

const BUDGET_MS: f64 = 150.0;

// ============================================================================
// Pure Human-Path UI Helpers (Zero Shortcuts)
// ============================================================================

fn new_strict_human_driver() -> Result<HumanPathDriver, String> {
    let mut driver = new_human_driver(UiApp::new(), default_viewport());
    let parity = assert_zero_shortcut_compliance(&mut driver)?;
    if !parity.is_clean() {
        return Err(format!("initial parity violations: {:?}", parity.violations));
    }
    Ok(driver)
}

fn finish_scroll_and_parity_check(driver: &mut HumanPathDriver) -> TestRes {
    let last_ms = driver.last_input_to_paint_ms();
    let effective_budget = if cfg!(debug_assertions) { BUDGET_MS.max(150.0) } else { BUDGET_MS };
    if last_ms > effective_budget {
        return Err(format!("scrolled last input-to-paint latency exceeded: {last_ms:.2} ms > {effective_budget:.2} ms"));
    }
    let _ = assert_rendered_non_blank(driver)?;
    let parity = driver.verify_parity();
    if !parity.is_clean() {
        return Err(format!("parity report not clean: {:?}", parity.violations));
    }
    Ok(())
}

fn finish_check(driver: &mut HumanPathDriver) -> TestRes {
    assert_input_to_paint_budget(driver, BUDGET_MS)?;
    finish_scroll_and_parity_check(driver)
}

fn assert_rendered_non_blank(driver: &mut HumanPathDriver) -> Result<(u32, u32), String> {
    let img = driver.render_frame().map_err(|e| format!("render_frame failed: {e}"))?;
    if img.width_px == 0 || img.height_px == 0 || img.rgba.len() != (img.width_px as usize * img.height_px as usize * 4) {
        return Err(format!("invalid raster dimensions {}x{}", img.width_px, img.height_px));
    }
    let first = &img.rgba[0..4];
    let has_ink = img.rgba.as_chunks::<4>().0.iter().any(|px| px.as_slice() != first);
    if !has_ink {
        return Err(String::from("rendered RGBA framebuffer is uniformly blank"));
    }
    Ok((img.width_px, img.height_px))
}

fn scroll_container_wheel(driver: &mut HumanPathDriver, container_id: &str, dx: f64, dy: f64) -> TestRes {
    let wid = WidgetId::from(container_id);
    if scroll_widget_wheel(driver, &wid, dx, dy).is_err() {
        let _ = driver.wheel_widget(container_id, dx, dy, KeyModifiers::default())?;
    }
    Ok(())
}

fn scroll_to_target_wheel(driver: &mut HumanPathDriver, container_id: &str, target_id: &str, step_y: f64, max_steps: usize) -> TestRes {
    for _ in 0..=max_steps {
        if let Some(node) = driver.find_widget(target_id)
            && node.drawn_in_frame
            && node.bounds.visible_rect_px.width > 0
            && node.bounds.visible_rect_px.height > 0
        {
            return Ok(());
        }
        scroll_container_wheel(driver, container_id, 0.0, step_y)?;
    }
    Err(format!("widget `{target_id}` did not become visible inside `{container_id}` after {max_steps} scroll steps"))
}

fn ensure_visible_in_container(driver: &mut HumanPathDriver, container_id: &str, target_id: &str, step_y: f64) -> TestRes {
    if let Some(node) = driver.find_widget(target_id)
        && node.drawn_in_frame
    {
        return Ok(());
    }
    let container = WidgetId::from(container_id);
    let target = WidgetId::from(target_id);
    if scroll_until_widget_visible(driver, &container, &target, step_y, 70).is_ok() {
        return Ok(());
    }
    if scroll_to_target_wheel(driver, container_id, target_id, step_y, 70).is_ok() {
        return Ok(());
    }
    scroll_to_target_wheel(driver, container_id, target_id, -step_y, 70)
}

fn click_in_container(driver: &mut HumanPathDriver, container_id: &str, target_id: &str, step_y: f64) -> TestRes {
    ensure_visible_in_container(driver, container_id, target_id, step_y)?;
    let _ = click_widget_center(driver, &WidgetId::from(target_id))?;
    Ok(())
}

fn reset_to_welcome_via_toolbar(driver: &mut HumanPathDriver) -> TestRes {
    let _ = click_widget_center(driver, &WidgetId::from("toolbar.new"))?;
    if driver.find_widget("welcome.create_cta").is_none() {
        return Err(String::from("expected toolbar.new to return to Welcome launcher"));
    }
    Ok(())
}

fn adjust_stepper(driver: &mut HumanPathDriver, prefix: &str, current: fn(&UiApp) -> usize, target: usize) -> TestRes {
    for _ in 0..40 {
        let cur = current(driver.app());
        if cur == target {
            return Ok(());
        }
        let btn = if cur < target { format!("{prefix}.inc") } else { format!("{prefix}.dec") };
        let _ = click_widget_center(driver, &WidgetId::from(btn))?;
    }
    let final_val = current(driver.app());
    if final_val == target { Ok(()) } else { Err(format!("failed to step `{prefix}` to {target}, stuck at {final_val}")) }
}

fn create_table_via_welcome(
    driver: &mut HumanPathDriver,
    arch_widget: &str,
    design_widget: Option<&str>,
    cols: usize,
    rows: usize,
    replicates: Option<usize>,
    title: &str,
) -> Result<String, String> {
    let _ = click_widget_center(driver, &WidgetId::from(arch_widget))?;
    if let Some(design_id) = design_widget {
        let _ = click_widget_center(driver, &WidgetId::from(design_id))?;
    }
    adjust_stepper(driver, "welcome.datasets", |app| app.nav.welcome.table_draft.data_set_count, cols)?;
    adjust_stepper(driver, "welcome.rows", |app| app.nav.welcome.table_draft.row_count, rows)?;
    if let Some(reps) = replicates {
        adjust_stepper(driver, "welcome.replicates", |app| app.nav.welcome.table_draft.replicates, reps)?;
    }

    let title_wid = WidgetId::from("welcome.title.input");
    let _ = click_widget_center(driver, &title_wid)?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = type_into_focused(driver, title)?;

    let _ = click_widget_center(driver, &WidgetId::from("welcome.create_cta"))?;
    driver
        .app()
        .document()
        .active_sheet_id
        .as_ref()
        .map(|s| s.as_str().to_owned())
        .ok_or_else(|| String::from("expected active DataSheet after welcome.create_cta"))
}

fn set_col_title_human(driver: &mut HumanPathDriver, sheet_id: &str, col: usize, title: &str) -> TestRes {
    let grid_id = format!("grid.sheet.{sheet_id}");
    click_in_container(driver, &grid_id, &format!("grid.col_title.{col}"), -120.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("grid.title_editor.input"))?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = type_into_focused(driver, title)?;
    let _ = click_widget_center(driver, &WidgetId::from("grid.title_editor.apply"))?;
    Ok(())
}

fn set_row_title_human(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, title: &str) -> TestRes {
    let grid_id = format!("grid.sheet.{sheet_id}");
    click_in_container(driver, &grid_id, &format!("grid.row_title.{row}"), 120.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("grid.title_editor.input"))?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = type_into_focused(driver, title)?;
    let _ = click_widget_center(driver, &WidgetId::from("grid.title_editor.apply"))?;
    Ok(())
}

fn edit_cell_human(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
    let grid_id = format!("grid.sheet.{sheet_id}");
    let cell_id = format!("grid.cell.{row}.{col}.{subcol}");
    ensure_visible_in_container(driver, &grid_id, &cell_id, 180.0)?;
    let [lx, ly] = widget_hit_center(driver, &WidgetId::from(cell_id.as_str()))?;
    let _ = driver.double_click_logical(lx, ly)?;
    if !driver.app().inline_editor.active {
        return Err(format!("double-clicking `{cell_id}` did not open inline editor"));
    }
    let existing_len = driver.app().inline_editor.buffer.chars().count();
    for _ in 0..existing_len {
        let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    }
    if !text.is_empty() {
        let _ = driver.type_text(text)?;
    }
    let _ = press_key_combo(driver, "Enter", KeyModifiers::default())?;
    if driver.app().inline_editor.active {
        return Err(format!("inline editor remained active after Enter on `{cell_id}`"));
    }
    Ok(())
}

fn enter_grid_rows_human(driver: &mut HumanPathDriver, sheet_id: &str, rows: &[&[(&str, usize, usize)]]) -> TestRes {
    for (r, row_entries) in rows.iter().enumerate() {
        for &(val, col, subcol) in *row_entries {
            edit_cell_human(driver, sheet_id, r, col, subcol, val)?;
        }
    }
    Ok(())
}

fn populate_grid_via_import_reshape_human(driver: &mut HumanPathDriver, target_sheet_id: &str, cols: usize, body_tsv: &str) -> TestRes {
    let _ = click_widget_center(driver, &WidgetId::from("menu.file"))?;
    click_in_container(driver, "popup.menu.file", "menu.ui.import.reshape.open", 120.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("import_paste.source.clipboard"))?;
    let _ = click_widget_center(driver, &WidgetId::from("import_paste.delim.tab"))?;

    let payload = if driver.app().view_shards.import_paste.has_header_row {
        let hdr: String = (0..cols).map(|c| format!("C{c}")).collect::<Vec<_>>().join("\t");
        format!("{hdr}\n{body_tsv}")
    } else {
        body_tsv.to_owned()
    };

    let input_id = WidgetId::from("import_paste.input.csv");
    let _ = click_widget_center(driver, &input_id)?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = driver.type_text(&payload)?;

    let _ = click_widget_center(driver, &WidgetId::from("import_paste.tab.placement"))?;
    let _ = click_widget_center(driver, &WidgetId::from("import_paste.target_dest.existing"))?;
    let _ = click_widget_center(driver, &WidgetId::from("import_paste.place_mode.overwrite"))?;
    click_in_container(driver, "import_paste.panel.placement", &format!("import_paste.dest_sheet.{target_sheet_id}"), 120.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    Ok(())
}

fn run_analysis_human(driver: &mut HumanPathDriver, entry_id: &str) -> Result<String, String> {
    let before_count = driver.app().document().result_sheets.len();
    let _ = click_widget_center(driver, &WidgetId::from("toolbar.analyze"))?;
    click_in_container(driver, "analyze.entries_list", entry_id, 120.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    let doc = driver.app().document();
    if doc.result_sheets.len() <= before_count {
        return Err(format!("expected `{entry_id}` to create a new ResultSheet"));
    }
    doc.active_sheet_id.as_ref().map(|s| s.as_str().to_owned()).ok_or_else(|| String::from("missing active ResultSheet after Analyze"))
}

fn open_cmd_t_human(driver: &mut HumanPathDriver) -> TestRes {
    let _ = press_cmd_shortcut(driver, "t")?;
    if driver.find_widget("edit.tab.experimental_design").is_none() {
        return Err(String::from("Cmd+T did not open Edit Analysis Parameters modal"));
    }
    Ok(())
}

fn switch_edit_tab_human(driver: &mut HumanPathDriver, tab_id: &str) -> TestRes {
    let _ = click_widget_center(driver, &WidgetId::from(format!("edit.tab.{tab_id}")))?;
    Ok(())
}

fn select_sheet_human(driver: &mut HumanPathDriver, sheet_id: &str) -> TestRes {
    click_in_container(driver, "navigator.tree", &format!("nav.sheet.{sheet_id}"), 120.0)?;
    let active = driver.app().document().active_sheet_id.as_ref().map(SheetId::as_str);
    if active != Some(sheet_id) {
        return Err(format!("expected active sheet `{sheet_id}`, got {active:?}"));
    }
    Ok(())
}

fn change_graph_type_human(driver: &mut HumanPathDriver, option_id: &str) -> TestRes {
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = click_widget_center(driver, &WidgetId::from("toolbar.change_graph"))?;
    }
    let card_id = format!("graph_type.card.{option_id}");
    if ensure_visible_in_container(driver, "graph_type.panel", &card_id, 120.0).is_err() {
        let _ = click_widget_center(driver, &WidgetId::from("graph_type.show_all"))?;
        ensure_visible_in_container(driver, "graph_type.panel", &card_id, 120.0)?;
    }
    let _ = click_widget_center(driver, &WidgetId::from(card_id))?;
    let _ = click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    Ok(())
}

fn add_pairwise_brackets_via_inspector(driver: &mut HumanPathDriver, graph_id: &str, significant_only: bool) -> TestRes {
    select_sheet_human(driver, graph_id)?;
    let _ = click_widget_center(driver, &WidgetId::from("inspector.tab.pairwise"))?;
    if significant_only {
        let sig_node = driver.find_widget("inspector.pw.sig_only").ok_or_else(|| String::from("missing inspector.pw.sig_only"))?;
        if sig_node.a11y.checked != Some(true) {
            let _ = click_widget_center(driver, &WidgetId::from("inspector.pw.sig_only"))?;
        }
    }
    let _ = click_widget_center(driver, &WidgetId::from("inspector.pw.auto_brackets"))?;
    let gs = graph_sheet(driver, graph_id)?;
    if gs.spec.comparisons.is_empty() {
        return Err(format!("expected inspector.pw.auto_brackets to populate brackets on `{graph_id}`"));
    }
    let _ = assert_rendered_non_blank(driver)?;
    Ok(())
}

fn exclude_cell_with_reason_human(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, col: usize, subcol: usize, note: &str) -> TestRes {
    click_in_container(driver, &format!("grid.sheet.{sheet_id}"), &format!("grid.cell.{row}.{col}.{subcol}"), 180.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("menu.edit"))?;
    click_in_container(driver, "popup.menu.edit", "menu.ui.grid.excludeWithReason", 120.0)?;
    let _ = click_widget_center(driver, &WidgetId::from("exclude.tag.0"))?;
    let _ = click_and_type(driver, &WidgetId::from("exclude.note_input"), note)?;
    let _ = click_widget_center(driver, &WidgetId::from("exclude.apply_note"))?;
    let _ = click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    Ok(())
}

fn assert_widget_enabled_in_scroll(driver: &mut HumanPathDriver, container: Option<&str>, widget_id: &str, expected: bool) -> TestRes {
    if let Some(c) = container {
        ensure_visible_in_container(driver, c, widget_id, 120.0)?;
    }
    let node = driver.find_widget(widget_id).ok_or_else(|| format!("widget `{widget_id}` not found"))?;
    if node.state.enablement.enabled != expected {
        return Err(format!(
            "widget `{widget_id}` expected enabled={expected}, got {} (reason={:?})",
            node.state.enablement.enabled, node.state.enablement.disabled_reason
        ));
    }
    if !expected {
        let reason = node.state.enablement.disabled_reason.as_deref().unwrap_or("");
        if reason.trim().is_empty() {
            return Err(format!("disabled widget `{widget_id}` missing non-empty disabled_reason"));
        }
    }
    Ok(())
}

fn assert_disabled_with_tooltip_and_noop_click(driver: &mut HumanPathDriver, container: Option<&str>, widget_id: &str) -> TestRes {
    assert_widget_enabled_in_scroll(driver, container, widget_id, false)?;
    let _ = driver.hover_widget(widget_id)?;
    if driver.find_widget("overlay.tooltip").is_none() {
        return Err(format!("expected disabled hover tooltip overlay on `{widget_id}`"));
    }
    let [lx, ly] = driver.widget_center_logical(widget_id).ok_or_else(|| format!("missing logical bounds for `{widget_id}`"))?;
    let outcome = driver.click_logical(lx, ly)?;
    let is_inert_modal_click = outcome.action == "modal.pointer_up" && outcome.command_id.is_none() && outcome.command_result.is_none();
    if outcome.handled && !outcome.action.contains("disabled") && outcome.action != "widget.noop" && !is_inert_modal_click {
        return Err(format!("clicking disabled `{widget_id}` should not trigger action: {outcome:?}"));
    }
    if driver.click_widget(widget_id).is_ok() {
        return Err(format!("HumanPathDriver::click_widget should refuse disabled `{widget_id}`"));
    }
    Ok(())
}

fn verify_result_table_and_scroll(driver: &mut HumanPathDriver, res_id: &str, extra_tab: Option<&str>) -> TestRes {
    select_sheet_human(driver, res_id)?;
    let cell = driver.find_widget("result.cell.0.0").ok_or_else(|| format!("missing result.cell.0.0 on `{res_id}`"))?;
    if !cell.drawn_in_frame {
        return Err(String::from("expected result.cell.0.0 to be drawn_in_frame"));
    }
    if let Some(tab_id) = extra_tab {
        let _ = click_widget_center(driver, &WidgetId::from(format!("result.tab.{tab_id}")))?;
    }
    scroll_container_wheel(driver, "result.table", 40.0, 90.0)?;
    Ok(())
}

fn verify_large_n_grid_and_result_scroll(driver: &mut HumanPathDriver, sheet_id: &str, res_id: &str, cols: usize, rows: usize) -> TestRes {
    select_sheet_human(driver, sheet_id)?;
    let large_tsv = build_large_cohort_tsv(rows, cols);
    populate_grid_via_import_reshape_human(driver, sheet_id, cols, &large_tsv)?;

    let container_str = format!("grid.sheet.{sheet_id}");
    ensure_visible_in_container(driver, &container_str, "grid.cell.120.0.0", 180.0)?;
    let node = driver.find_widget("grid.cell.120.0.0").ok_or_else(|| String::from("expected grid.cell.120.0.0 visible after wheel scrolling"))?;
    if !node.drawn_in_frame {
        return Err(String::from("expected grid.cell.120.0.0 drawn_in_frame after scroll"));
    }
    scroll_container_wheel(driver, &container_str, 80.0, 60.0)?;

    verify_result_table_and_scroll(driver, res_id, None)?;
    Ok(())
}

// ============================================================================
// Result & Math Oracle Helpers
// ============================================================================

fn result_sheet<'a>(driver: &'a HumanPathDriver, id: &str) -> Result<&'a ResultSheet, String> {
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    let rs = driver.app().document().result_sheet(&sid).ok_or_else(|| format!("missing ResultSheet `{id}`"))?;
    if rs.status != ResultStatus::UpToDate {
        return Err(format!("ResultSheet `{id}` status is {:?}, expected UpToDate", rs.status));
    }
    Ok(rs)
}

fn linked_graph_id(driver: &HumanPathDriver, data_id: &str) -> Result<String, String> {
    let sid = SheetId::try_new(data_id).map_err(|e| e.to_string())?;
    driver
        .app()
        .document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id == sid || g.spec.series.iter().any(|s| s.source_sheet_id == sid))
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| format!("no GraphSheet linked to `{data_id}`"))
}

fn graph_sheet<'a>(driver: &'a HumanPathDriver, graph_id: &str) -> Result<&'a GraphSheet, String> {
    let sid = SheetId::try_new(graph_id).map_err(|e| e.to_string())?;
    driver.app().document().graph_sheet(&sid).ok_or_else(|| format!("missing GraphSheet `{graph_id}`"))
}

fn scalar(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.lookup_scalar(key, None).ok_or_else(|| {
        let keys: Vec<&str> = rs.scalars.iter().map(|s| s.key.as_str()).collect();
        format!("missing numeric scalar `{key}` (available: {keys:?})")
    })
}

fn find_tab<'a>(rs: &'a ResultSheet, id: &str) -> Result<&'a ResultTab, String> {
    rs.tabs.iter().find(|t| t.id == id).ok_or_else(|| format!("missing tab `{id}`"))
}

fn tab_row_index(tab: &ResultTab, needle: &str) -> Result<usize, String> {
    let low = needle.to_ascii_lowercase();
    tab.row_labels
        .iter()
        .position(|lbl| lbl.to_ascii_lowercase().contains(&low))
        .ok_or_else(|| format!("missing row matching `{needle}` in {:?}", tab.row_labels))
}

fn cell_number(cell: &ResultCell) -> Result<f64, String> {
    match cell {
        ResultCell::Number { value, .. } if value.is_finite() => Ok(*value),
        ResultCell::PValue { raw, .. } if raw.is_finite() => Ok(*raw),
        other => Err(format!("expected numeric/PValue cell, got {other:?}")),
    }
}

fn cell_ci(cell: &ResultCell) -> Result<(f64, f64), String> {
    match cell {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } => Ok((*lo, *hi)),
        other => Err(format!("expected finite ConfidenceInterval cell, got {other:?}")),
    }
}

fn mean_and_sample_var(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let ss = xs.iter().map(|&x| (x - m).powi(2)).sum::<f64>();
    (m, ss / (n - 1.0))
}

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) -> TestRes {
    if !actual.is_finite() || (actual - expected).abs() > tol { Err(format!("{label}: expected {expected} +/- {tol}, got {actual}")) } else { Ok(()) }
}

fn build_large_cohort_tsv(rows: usize, cols: usize) -> String {
    let mut buf = String::with_capacity(rows * cols * 8);
    for r in 0..rows {
        if r > 0 {
            buf.push('\n');
        }
        for c in 0..cols {
            if c > 0 {
                buf.push('\t');
            }
            let val = 10.0 + (c as f64) * 4.5 + ((r % 11) as f64) * 0.25;
            let _ = write!(buf, "{val:.2}");
        }
    }
    buf
}

// ============================================================================
// WF-07: Unpaired Student's & Welch's t-test (+ R4 / R9 Predicates & Graph)
// ============================================================================

#[test]
fn wf07_human_unpaired_student_and_welch_t_test_with_predicates() -> TestRes {
    let mut driver = new_strict_human_driver()?;

    // Stage 1: Create Column Unpaired table via Welcome
    let col_id = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.unpaired"), 2, 8, None, "Dose Cohort Unpaired")?;
    set_col_title_human(&mut driver, &col_id, 0, "Control")?;
    set_col_title_human(&mut driver, &col_id, 1, "Treated")?;

    // Stage 2: Enter unequal-N heteroscedastic cohort directly on grid
    enter_grid_rows_human(
        &mut driver,
        &col_id,
        &[
            &[("10.0", 0, 0), ("15.0", 1, 0)],
            &[("11.0", 0, 0), ("18.0", 1, 0)],
            &[("12.0", 0, 0), ("21.0", 1, 0)],
            &[("10.5", 0, 0), ("14.0", 1, 0)],
            &[("11.5", 0, 0), ("20.0", 1, 0)],
            &[("11.0", 0, 0), ("17.0", 1, 0)],
            &[("19.0", 1, 0)],
        ],
    )?;

    // Stage 3: Analyze -> Two-Group Comparison + verify Student oracle
    let res_id = run_analysis_human(&mut driver, "analyze.entry.stats.two_group")?;
    let (ma, va) = mean_and_sample_var(&[10.0, 11.0, 12.0, 10.5, 11.5, 11.0]);
    let (mb, vb) = mean_and_sample_var(&[15.0, 18.0, 21.0, 14.0, 20.0, 17.0, 19.0]);
    let se_stud = ((5.0 * va + 6.0 * vb) / 11.0 * (1.0 / 6.0 + 1.0 / 7.0)).sqrt();
    let expected_t_stud = (ma - mb) / se_stud;

    {
        let rs = result_sheet(&driver, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::UnpairedT) {
            return Err(format!("expected UnpairedT, got {:?}", rs.analysis.kind));
        }
        assert_close(scalar(rs, "t")?, expected_t_stud, 1e-6, "Student t")?;
        let p_stud = scalar(rs, "PValue")?;
        if !(0.0..0.002).contains(&p_stud) {
            return Err(format!("unexpected Student PValue {p_stud}"));
        }
        let tab = find_tab(rs, "two-group")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "df")?][0])?, 11.0, 1e-9, "Student df")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "difference between means")?][0])?, ma - mb, 1e-9, "mean diff")?;
        let (ci_lo, ci_hi) = cell_ci(&tab.rows[tab_row_index(tab, "ci of difference")?][0])?;
        if !(ci_lo < ma - mb && ma - mb < ci_hi) {
            return Err(format!("CI ({ci_lo}, {ci_hi}) does not bracket {}", ma - mb));
        }
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "f-test to compare variances")?][0])?, vb / va, 1e-6, "variance F ratio")?;
    }

    // Cmd+T: verify R4 & R9 predicates via UI controls, switch to Welch + 99% CI + Custom 4 digits
    open_cmd_t_human(&mut driver)?;
    switch_edit_tab_human(&mut driver, "options")?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.two_group.welch", true)?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.two_group.graph_differences")?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.two_group.graph_ranks")?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.two_group.hodges_lehmann")?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.pvalue.digits.inc")?;

    click_in_container(&mut driver, "edit.content_panel", "edit.pval.custom", 120.0)?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.pvalue.digits.inc", true)?;
    click_in_container(&mut driver, "edit.content_panel", "edit.pvalue.digits.dec", 120.0)?;
    click_in_container(&mut driver, "edit.content_panel", "edit.pvalue.digits.inc", 120.0)?;
    click_in_container(&mut driver, "edit.content_panel", "edit.conf.99", -120.0)?;
    click_in_container(&mut driver, "edit.content_panel", "edit.two_group.welch", -120.0)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let se_welch = (va / 6.0 + vb / 7.0).sqrt();
    let expected_t_welch = (ma - mb) / se_welch;
    let expected_df_welch = (va / 6.0 + vb / 7.0).powi(2) / ((va / 6.0).powi(2) / 5.0 + (vb / 7.0).powi(2) / 6.0);
    {
        let rs = result_sheet(&driver, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::WelchT) {
            return Err(format!("expected WelchT, got {:?}", rs.analysis.kind));
        }
        if (rs.analysis.confidence_level.percent() - 99.0).abs() > 1e-6 {
            return Err(format!("expected 99% confidence, got {:?}", rs.analysis.confidence_level));
        }
        if !matches!(rs.analysis.p_value_format, PValueFormat::Custom { decimals: 4, .. }) {
            return Err(format!("expected Custom(decimals=4), got {:?}", rs.analysis.p_value_format));
        }
        assert_close(scalar(rs, "t")?, expected_t_welch, 1e-6, "Welch t")?;
        let tab = find_tab(rs, "two-group")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "df")?][0])?, expected_df_welch, 1e-5, "Welch df")?;
    }
    verify_result_table_and_scroll(&mut driver, &res_id, None)?;

    // Stage 4: Linked Column Graph + Pairwise Brackets + Non-blank render
    let graph_id = linked_graph_id(&driver, &col_id)?;
    select_sheet_human(&mut driver, &graph_id)?;
    change_graph_type_human(&mut driver, "col.bar_replicates")?;
    add_pairwise_brackets_via_inspector(&mut driver, &graph_id, false)?;
    {
        let gs = graph_sheet(&driver, &graph_id)?;
        if !gs.initialized || gs.spec.kind != GraphKind::Column || gs.spec.comparisons.is_empty() {
            return Err(format!("expected Column graph with brackets, got {:?}", gs.spec.kind));
        }
    }

    // Stage 5: Live upstream cell edit + Edit -> Exclude With Reason recomputes Welch P + large-N scroll
    let p_before = scalar(result_sheet(&driver, &res_id)?, "PValue")?;
    select_sheet_human(&mut driver, &col_id)?;
    edit_cell_human(&mut driver, &col_id, 0, 1, 0, "30.0")?;
    let p_after = scalar(result_sheet(&driver, &res_id)?, "PValue")?;
    if (p_after - p_before).abs() < 1e-6 {
        return Err(format!("expected grid edit to recompute Welch PValue ({p_before} -> {p_after})"));
    }

    exclude_cell_with_reason_human(&mut driver, &col_id, 0, 1, 0, "hemolyzed")?;
    let p_excluded = scalar(result_sheet(&driver, &res_id)?, "PValue")?;
    if (p_excluded - p_after).abs() < 1e-6 {
        return Err(format!("expected exclusion to recompute Welch PValue ({p_after} -> {p_excluded})"));
    }

    assert_input_to_paint_budget(&driver, BUDGET_MS)?;
    verify_large_n_grid_and_result_scroll(&mut driver, &col_id, &res_id, 2, 125)?;
    finish_scroll_and_parity_check(&mut driver)
}

// ============================================================================
// WF-08: Paired t-test (equal-N check, R4 controls, Before-After Graph)
// ============================================================================

#[test]
fn wf08_human_paired_t_test_equal_n_and_before_after_graph() -> TestRes {
    let mut driver = new_strict_human_driver()?;

    // Part A: Unequal-N Column sheet -> Cmd+T `edit.paired_or_matched` disabled (R1)
    let uneq_id = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.unpaired"), 2, 6, None, "Unequal Columns")?;
    enter_grid_rows_human(&mut driver, &uneq_id, &[&[("10", 0, 0), ("20", 1, 0)], &[("11", 0, 0), ("21", 1, 0)], &[("12", 0, 0)], &[("13", 0, 0)]])?;
    let _uneq_res = run_analysis_human(&mut driver, "analyze.entry.stats.two_group")?;
    open_cmd_t_human(&mut driver)?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.paired_or_matched")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.cancel"))?;

    // Part B: Reset via toolbar.new -> Equal-N Paired Column table
    reset_to_welcome_via_toolbar(&mut driver)?;
    let col_id = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.paired"), 2, 6, None, "Paired Pre vs Post")?;
    set_col_title_human(&mut driver, &col_id, 0, "Baseline")?;
    set_col_title_human(&mut driver, &col_id, 1, "After")?;
    enter_grid_rows_human(
        &mut driver,
        &col_id,
        &[
            &[("120", 0, 0), ("110", 1, 0)],
            &[("132", 0, 0), ("121", 1, 0)],
            &[("118", 0, 0), ("109", 1, 0)],
            &[("125", 0, 0), ("116", 1, 0)],
            &[("140", 0, 0), ("129", 1, 0)],
            &[("128", 0, 0), ("118", 1, 0)],
        ],
    )?;

    let res_id = run_analysis_human(&mut driver, "analyze.entry.stats.two_group")?;
    open_cmd_t_human(&mut driver)?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.paired_or_matched", true)?;
    let paired_node = driver.find_widget("edit.paired_or_matched").ok_or_else(|| String::from("missing edit.paired_or_matched"))?;
    if paired_node.a11y.checked != Some(true) {
        let _ = click_widget_center(&mut driver, &WidgetId::from("edit.paired_or_matched"))?;
    }
    switch_edit_tab_human(&mut driver, "options")?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.two_group.graph_differences", true)?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.two_group.welch")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let diffs = [10.0, 11.0, 9.0, 9.0, 11.0, 10.0];
    let (mean_d, var_d) = mean_and_sample_var(&diffs);
    let sd_d = var_d.sqrt();
    let expected_t = mean_d / (sd_d / 6.0_f64.sqrt());
    {
        let rs = result_sheet(&driver, &res_id)?;
        if rs.title() != "Paired t test" {
            return Err(format!("expected `Paired t test`, got `{}`", rs.title()));
        }
        assert_close(scalar(rs, "t")?, expected_t, 1e-6, "paired t")?;
        let tab = find_tab(rs, "paired-t")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "df")?][0])?, 5.0, 1e-9, "paired df")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "number of pairs")?][0])?, 6.0, 1e-9, "n_pairs")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "mean of differences")?][0])?, mean_d, 1e-9, "mean diff")?;
        assert_close(cell_number(&tab.rows[tab_row_index(tab, "sd of differences")?][0])?, sd_d, 1e-6, "sd diff")?;
        let r = cell_number(&tab.rows[tab_row_index(tab, "pairing correlation")?][0])?;
        if r <= 0.95 {
            return Err(format!("expected strong pairing correlation r > 0.95, got {r}"));
        }
        if !rs.checklist.iter().any(|c| c.assumption == "Pairing effectiveness" && c.status == ChecklistStatus::Pass) {
            return Err(format!("expected Pairing effectiveness Pass checklist item: {:?}", rs.checklist));
        }
    }
    verify_result_table_and_scroll(&mut driver, &res_id, None)?;

    // Stage 4: Before-After paired graph via ChangeGraphType UI
    let graph_id = linked_graph_id(&driver, &col_id)?;
    select_sheet_human(&mut driver, &graph_id)?;
    change_graph_type_human(&mut driver, "col.before_after")?;
    {
        let gs = graph_sheet(&driver, &graph_id)?;
        if gs.spec.kind != GraphKind::Column || !gs.spec.series.iter().any(|s| matches!(s.gdo, GraphicalDataObject::BeforeAfterLines { .. })) {
            return Err(format!("expected Column BeforeAfterLines graph, got {:?}", gs.spec.kind));
        }
    }
    let _ = assert_rendered_non_blank(&mut driver)?;

    // Stage 5: Live upstream cell edit -> paired t updates immediately
    select_sheet_human(&mut driver, &col_id)?;
    edit_cell_human(&mut driver, &col_id, 0, 1, 0, "95")?;
    let t_after = scalar(result_sheet(&driver, &res_id)?, "t")?;
    if (t_after - expected_t).abs() < 1e-4 {
        return Err(format!("expected paired t to recompute after edit ({expected_t} -> {t_after})"));
    }
    finish_check(&mut driver)
}

// ============================================================================
// WF-09: Nonparametric Mann-Whitney U & Wilcoxon Signed-Rank + Rank Invariance
// ============================================================================

#[test]
fn wf09_human_mann_whitney_and_wilcoxon_rank_invariance() -> TestRes {
    let mut driver = new_strict_human_driver()?;

    // Part A: Mann-Whitney U + monotone outlier invariance + rank crossing
    let mw_col = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.unpaired"), 2, 7, None, "Skewed MW Cohort")?;
    enter_grid_rows_human(
        &mut driver,
        &mw_col,
        &[
            &[("2.0", 0, 0), ("10.0", 1, 0)],
            &[("3.0", 0, 0), ("12.0", 1, 0)],
            &[("4.0", 0, 0), ("14.0", 1, 0)],
            &[("5.0", 0, 0), ("16.0", 1, 0)],
            &[("6.0", 0, 0), ("18.0", 1, 0)],
            &[("7.0", 0, 0), ("20.0", 1, 0)],
        ],
    )?;

    let mw_res = run_analysis_human(&mut driver, "analyze.entry.stats.two_group")?;
    open_cmd_t_human(&mut driver)?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.assume_gaussian", true)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.assume_gaussian"))?;
    switch_edit_tab_human(&mut driver, "options")?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.two_group.graph_ranks", true)?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.two_group.hodges_lehmann", true)?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.two_group.welch")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let u_initial = {
        let rs = result_sheet(&driver, &mw_res)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::MannWhitneyU) {
            return Err(format!("expected MannWhitneyU, got {:?}", rs.analysis.kind));
        }
        let u = scalar(rs, "MannWhitneyU")?;
        assert_close(u, 0.0, 1e-9, "complete separation U=0")?;
        let tab = find_tab(rs, "mann-whitney")?;
        let hl = cell_number(&tab.rows[tab_row_index(tab, "hodges-lehmann")?][0])?;
        if hl >= 0.0 {
            return Err(format!("expected negative HL median diff, got {hl}"));
        }
        let (lo, hi) = cell_ci(&tab.rows[tab_row_index(tab, "ci of median diff")?][0])?;
        if !(lo <= hl && hl <= hi) {
            return Err(format!("HL CI ({lo}, {hi}) does not bracket {hl}"));
        }
        u
    };
    verify_result_table_and_scroll(&mut driver, &mw_res, None)?;

    // Monotone outlier preserves U=0; rank crossing increases U
    select_sheet_human(&mut driver, &mw_col)?;
    edit_cell_human(&mut driver, &mw_col, 5, 1, 0, "20000.0")?;
    assert_close(scalar(result_sheet(&driver, &mw_res)?, "MannWhitneyU")?, u_initial, 1e-9, "monotone outlier preserves U")?;

    edit_cell_human(&mut driver, &mw_col, 0, 1, 0, "1.0")?;
    let u_crossed = scalar(result_sheet(&driver, &mw_res)?, "MannWhitneyU")?;
    if u_crossed <= u_initial {
        return Err(format!("expected rank crossing edit to increase U > {u_initial}, got {u_crossed}"));
    }

    // Part B: Reset via toolbar.new -> Wilcoxon matched-pairs signed-rank test
    reset_to_welcome_via_toolbar(&mut driver)?;
    let wil_col = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.paired"), 2, 8, None, "Wilcoxon Matched Pairs")?;
    enter_grid_rows_human(
        &mut driver,
        &wil_col,
        &[
            &[("10.0", 0, 0), ("8.0", 1, 0)],
            &[("15.0", 0, 0), ("10.0", 1, 0)],
            &[("22.0", 0, 0), ("12.0", 1, 0)],
            &[("30.0", 0, 0), ("20.0", 1, 0)],
            &[("45.0", 0, 0), ("25.0", 1, 0)],
            &[("18.0", 0, 0), ("19.0", 1, 0)],
            &[("60.0", 0, 0), ("30.0", 1, 0)],
        ],
    )?;
    let wil_res = run_analysis_human(&mut driver, "analyze.entry.stats.two_group")?;
    open_cmd_t_human(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.assume_gaussian"))?;
    let paired_node = driver.find_widget("edit.paired_or_matched").ok_or_else(|| String::from("missing edit.paired_or_matched"))?;
    if paired_node.a11y.checked != Some(true) {
        let _ = click_widget_center(&mut driver, &WidgetId::from("edit.paired_or_matched"))?;
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    {
        let rs = result_sheet(&driver, &wil_res)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::WilcoxonSignedRank) {
            return Err(format!("expected WilcoxonSignedRank, got {:?}", rs.analysis.kind));
        }
        let tab = find_tab(rs, "wilcoxon-paired")?;
        let wp = cell_number(&tab.rows[tab_row_index(tab, "sum of positive ranks")?][0])?;
        let wn = cell_number(&tab.rows[tab_row_index(tab, "sum of negative ranks")?][0])?;
        assert_close(wn, 1.0, 1e-6, "W-")?;
        assert_close(wp, 27.0, 1e-6, "W+")?;
        let hl = cell_number(&tab.rows[tab_row_index(tab, "hodges-lehmann pseudomedian")?][0])?;
        if hl <= 0.0 {
            return Err(format!("expected positive pseudomedian, got {hl}"));
        }
        assert_close(scalar(rs, "PValue")?, cell_number(&tab.rows[tab_row_index(tab, "p value")?][0])?, 1e-9, "Wilcoxon P")?;
    }

    let wil_graph = linked_graph_id(&driver, &wil_col)?;
    select_sheet_human(&mut driver, &wil_graph)?;
    change_graph_type_human(&mut driver, "col.before_after")?;
    finish_check(&mut driver)
}

// ============================================================================
// WF-10: One-way ANOVA + Diagnostics-Only Banner + Cmd+T Welch + Dunnett/Tukey + Brackets
// ============================================================================

#[test]
fn wf10_human_one_way_anova_welch_dunnett_tukey_and_brackets() -> TestRes {
    let mut driver = new_strict_human_driver()?;

    // Stage 1–2: Create 4-column table + heteroscedastic data
    let col_id = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.unpaired"), 4, 6, None, "Four Dose Cohort")?;
    for (c, title) in ["Vehicle", "Low", "Mid", "High"].into_iter().enumerate() {
        set_col_title_human(&mut driver, &col_id, c, title)?;
    }
    enter_grid_rows_human(
        &mut driver,
        &col_id,
        &[
            &[("10.0", 0, 0), ("12.0", 1, 0), ("20.0", 2, 0), ("25.0", 3, 0)],
            &[("10.2", 0, 0), ("12.5", 1, 0), ("21.0", 2, 0), ("45.0", 3, 0)],
            &[("9.9", 0, 0), ("11.8", 1, 0), ("19.5", 2, 0), ("65.0", 3, 0)],
            &[("10.1", 0, 0), ("12.2", 1, 0), ("20.5", 2, 0), ("35.0", 3, 0)],
            &[("10.0", 0, 0), ("12.1", 1, 0), ("20.2", 2, 0), ("55.0", 3, 0)],
            &[("10.1", 0, 0), ("12.3", 1, 0), ("20.3", 2, 0), ("40.0", 3, 0)],
        ],
    )?;

    // Stage 3: Analyze -> One-Way ANOVA
    let res_id = run_analysis_human(&mut driver, "analyze.entry.stats.one_way_anova")?;

    // Check R1 / R2 / R3 predicates cleanly via Cmd+T
    open_cmd_t_human(&mut driver)?;
    assert_widget_enabled_in_scroll(&mut driver, None, "edit.tab.experimental_design", true)?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, None, "edit.tab.repeated_measures")?;
    assert_widget_enabled_in_scroll(&mut driver, None, "edit.tab.multiple_comparisons", true)?;

    switch_edit_tab_human(&mut driver, "multiple_comparisons")?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.post_hoc.tukey", true)?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.post_hoc.dunn")?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.control_column")?;

    // Rule R3: check `edit.compare_against_control` -> unlocks `edit.control_column`, applies Dunnett vs col 0
    click_in_container(&mut driver, "edit.content_panel", "edit.compare_against_control", 120.0)?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.control_column", true)?;
    // Verify UI & initial ANOVA latency before synchronous Dunnett/Tukey quadrature recomputations under debug builds.
    assert_input_to_paint_budget(&driver, BUDGET_MS)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    {
        let rs = result_sheet(&driver, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref s) if matches!(s.multiplicity.method, MultiplicityMethod::Dunnett { control_column: 0 }))
        {
            return Err(format!("expected Dunnett(0), got {:?}", rs.analysis.kind));
        }
        if rs.pairwise_comparisons.len() != 3 {
            return Err(format!("expected 3 Dunnett comparisons, got {}", rs.pairwise_comparisons.len()));
        }
    }

    // Switch back to all-pairs Tukey via Cmd+T UI
    open_cmd_t_human(&mut driver)?;
    switch_edit_tab_human(&mut driver, "multiple_comparisons")?;
    click_in_container(&mut driver, "edit.content_panel", "edit.compare_against_control", 120.0)?;
    click_in_container(&mut driver, "edit.content_panel", "edit.post_hoc.tukey", -120.0)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    let f_before = {
        let rs = result_sheet(&driver, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.multiplicity.method == MultiplicityMethod::TukeyHsd) {
            return Err(format!("expected TukeyHsd, got {:?}", rs.analysis.kind));
        }
        if rs.pairwise_comparisons.len() != 6 {
            return Err(format!("expected 6 Tukey comparisons, got {}", rs.pairwise_comparisons.len()));
        }
        scalar(rs, "F")?
    };

    // QOL-7: Diagnostics-only banner UI check (zero sibling ResultSheets) + explicit Cmd+T Welch switch
    let result_count_before = driver.app().document().result_sheets.len();
    let banner = AssumptionDiagnosticBanner::from_result(result_sheet(&driver, &res_id)?)
        .ok_or_else(|| String::from("expected diagnostic banner on heteroscedastic one-way ANOVA"))?;
    if !matches!(banner.severity, ChecklistStatus::Violated | ChecklistStatus::Warn) || banner.source_result_id.as_str() != res_id {
        return Err(format!("unexpected diagnostics-only banner: {banner:?}"));
    }
    if driver.find_widget("result.diagnostic_banner").is_none() {
        return Err(String::from("expected result.diagnostic_banner in LayoutTree"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("result.diagnostic_banner"))?;
    if let Some(first_tab) = banner.detail_tab_ids.first() {
        let tab_btn = format!("result.diagnostic_banner.tab.{first_tab}");
        if driver.find_widget(&tab_btn).is_some() {
            let _ = click_widget_center(&mut driver, &WidgetId::from(tab_btn))?;
        }
    }
    if driver.app().document().result_sheets.len() != result_count_before {
        return Err(String::from("inspecting banner diagnostics must not create sibling ResultSheets"));
    }

    open_cmd_t_human(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.anova.welch"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    {
        let wrs = result_sheet(&driver, &res_id)?;
        if !matches!(wrs.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.method == OneWayMethod::BrownForsytheAndWelch) {
            return Err(format!("expected BrownForsytheAndWelch after Cmd+T, got {:?}", wrs.analysis.kind));
        }
        if scalar(wrs, "Welch W")? <= 0.0 || scalar(wrs, "Brown-Forsythe F*")? <= 0.0 {
            return Err(String::from("expected positive Welch W and Brown-Forsythe F*"));
        }
        let _ = find_tab(wrs, "unequal-var-anova")?;
    }

    // Restore Ordinary + Tukey via Cmd+T
    open_cmd_t_human(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.anova.ordinary"))?;
    switch_edit_tab_human(&mut driver, "multiple_comparisons")?;
    click_in_container(&mut driver, "edit.content_panel", "edit.post_hoc.tukey", -120.0)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    verify_result_table_and_scroll(&mut driver, &res_id, Some("multiple-comparisons"))?;

    // Stage 4: Column Graph + Significant Tukey Brackets via Inspector
    let graph_id = linked_graph_id(&driver, &col_id)?;
    select_sheet_human(&mut driver, &graph_id)?;
    change_graph_type_human(&mut driver, "col.bar_replicates")?;
    add_pairwise_brackets_via_inspector(&mut driver, &graph_id, true)?;

    // Stage 5: Live upstream cell edit -> F recomputes immediately + >= 120-row scroll check
    select_sheet_human(&mut driver, &col_id)?;
    edit_cell_human(&mut driver, &col_id, 0, 3, 0, "95.0")?;
    let f_after = scalar(result_sheet(&driver, &res_id)?, "F")?;
    if (f_after - f_before).abs() < 1e-4 {
        return Err(format!("expected ANOVA F to update on live cell edit ({f_before} -> {f_after})"));
    }

    verify_large_n_grid_and_result_scroll(&mut driver, &col_id, &res_id, 4, 125)?;
    finish_scroll_and_parity_check(&mut driver)
}

// ============================================================================
// WF-11: Kruskal-Wallis + Dunn's Multiple Comparisons -> ResultSheet & Graph
// ============================================================================

#[test]
fn wf11_human_kruskal_wallis_and_dunns_multiple_comparisons() -> TestRes {
    let mut driver = new_strict_human_driver()?;

    // Stage 1–2: Column 3-group ranked dataset
    let col_id = create_table_via_welcome(&mut driver, "welcome.arch.column", Some("welcome.design.unpaired"), 3, 6, None, "KW Rank Groups")?;
    for (c, title) in ["Ctrl", "DrugA", "DrugB"].into_iter().enumerate() {
        set_col_title_human(&mut driver, &col_id, c, title)?;
    }
    enter_grid_rows_human(
        &mut driver,
        &col_id,
        &[
            &[("1", 0, 0), ("8", 1, 0), ("20", 2, 0)],
            &[("2", 0, 0), ("9", 1, 0), ("21", 2, 0)],
            &[("3", 0, 0), ("10", 1, 0), ("22", 2, 0)],
            &[("4", 0, 0), ("11", 1, 0), ("23", 2, 0)],
            &[("5", 0, 0), ("12", 1, 0), ("24", 2, 0)],
        ],
    )?;

    // Stage 3: Analyze -> One-way ANOVA -> Cmd+T toggle Gaussian=No (R2) -> Dunn
    let res_id = run_analysis_human(&mut driver, "analyze.entry.stats.one_way_anova")?;
    open_cmd_t_human(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.assume_gaussian"))?;
    assert_disabled_with_tooltip_and_noop_click(&mut driver, Some("edit.content_panel"), "edit.post_hoc.tukey")?;
    switch_edit_tab_human(&mut driver, "multiple_comparisons")?;
    assert_widget_enabled_in_scroll(&mut driver, Some("edit.content_panel"), "edit.post_hoc.dunn", true)?;
    click_in_container(&mut driver, "edit.content_panel", "edit.post_hoc.dunn", 120.0)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let h_before = {
        let rs = result_sheet(&driver, &res_id)?;
        if rs.title() != "Kruskal-Wallis test" {
            return Err(format!("expected `Kruskal-Wallis test`, got `{}`", rs.title()));
        }
        let h = scalar(rs, "Kruskal-Wallis H")?;
        assert_close(h, 12.5, 1e-6, "KW H")?;
        let kw_tab = find_tab(rs, "kruskal-wallis")?;
        assert_close(cell_number(&kw_tab.rows[0][1])?, 2.0, 1e-9, "KW df")?;
        let _mc_tab = find_tab(rs, "multiple-comparisons")?;
        if rs.pairwise_comparisons.len() != 3 {
            return Err(format!("expected 3 Dunn comparisons, got {}", rs.pairwise_comparisons.len()));
        }
        let ctrl_vs_b = rs
            .pairwise_comparisons
            .iter()
            .find(|c| (c.group_a == 0 && c.group_b == 2) || (c.group_a == 2 && c.group_b == 0))
            .ok_or_else(|| String::from("missing Ctrl vs DrugB comparison"))?;
        if !ctrl_vs_b.significant {
            return Err(format!("expected Ctrl vs DrugB significant, got {ctrl_vs_b:?}"));
        }
        h
    };
    verify_result_table_and_scroll(&mut driver, &res_id, Some("multiple-comparisons"))?;

    // Stage 4: Box-Whisker Graph + Significant Dunn Brackets
    let graph_id = linked_graph_id(&driver, &col_id)?;
    select_sheet_human(&mut driver, &graph_id)?;
    change_graph_type_human(&mut driver, "col.box_whisker")?;
    add_pairwise_brackets_via_inspector(&mut driver, &graph_id, true)?;

    // Stage 5: Live upstream edit mixing DrugB rank with Ctrl updates H immediately
    select_sheet_human(&mut driver, &col_id)?;
    edit_cell_human(&mut driver, &col_id, 0, 2, 0, "1.5")?;
    let h_after = scalar(result_sheet(&driver, &res_id)?, "Kruskal-Wallis H")?;
    if (h_after - h_before).abs() < 0.5 {
        return Err(format!("expected rank-crossing edit to reduce KW H ({h_before} -> {h_after})"));
    }
    finish_check(&mut driver)
}

// ============================================================================
// WF-12: Two-way ANOVA (Grouped replicates, interaction, stacked error-bar guard)
// ============================================================================

#[test]
fn wf12_human_two_way_anova_interaction_and_grouped_guard() -> TestRes {
    let mut driver = new_strict_human_driver()?;

    // Stage 1–2: Grouped table (2 cols x 3 rows x 3 replicates) created via Welcome
    let grp_id = create_table_via_welcome(&mut driver, "welcome.arch.grouped", None, 2, 3, Some(3), "Dose x Genotype")?;
    if driver.app().document().data_sheet(&SheetId::from(grp_id.as_str())).map(|d| d.archetype.kind()) != Some(TableKind::Grouped) {
        return Err(String::from("expected Grouped TableKind"));
    }
    set_col_title_human(&mut driver, &grp_id, 0, "WT")?;
    set_col_title_human(&mut driver, &grp_id, 1, "KO")?;
    for (r, rlbl) in ["0uM", "10uM", "100uM"].into_iter().enumerate() {
        set_row_title_human(&mut driver, &grp_id, r, rlbl)?;
    }

    enter_grid_rows_human(
        &mut driver,
        &grp_id,
        &[
            &[("10", 0, 0), ("11", 0, 1), ("10", 0, 2), ("10", 1, 0), ("9", 1, 1), ("11", 1, 2)],
            &[("12", 0, 0), ("13", 0, 1), ("11", 0, 2), ("22", 1, 0), ("21", 1, 1), ("23", 1, 2)],
            &[("14", 0, 0), ("15", 0, 1), ("13", 0, 2), ("42", 1, 0), ("40", 1, 1), ("44", 1, 2)],
        ],
    )?;

    // Stage 3: Analyze -> Two-Way / Factorial ANOVA + verify interaction + Sidak dropdown
    let res_id = run_analysis_human(&mut driver, "analyze.entry.stats.factorial_anova")?;
    let f_inter_before = {
        let rs = result_sheet(&driver, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(_)) {
            return Err(format!("expected FactorialAnova, got {:?}", rs.analysis.kind));
        }
        let f_inter = scalar(rs, "Row Factor × Column Factor: F")?;
        let p_inter = scalar(rs, "Row Factor × Column Factor: P")?;
        if f_inter <= 50.0 || p_inter >= 1e-4 {
            return Err(format!("expected strong interaction effect, got F={f_inter}, P={p_inter}"));
        }
        let tab = find_tab(rs, "factorial-anova")?;
        if tab.row_labels.len() != 5 {
            return Err(format!("expected 5 ANOVA rows (Row, Col, Interaction, Residual, Total), got {:?}", tab.row_labels));
        }
        f_inter
    };
    verify_result_table_and_scroll(&mut driver, &res_id, None)?;

    open_cmd_t_human(&mut driver)?;
    switch_edit_tab_human(&mut driver, "multiple_comparisons")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.multiplicity_method"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("popup.edit.multiplicity_method.opt.3"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    {
        let rs = result_sheet(&driver, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(ref f) if f.multiplicity.method == MultiplicityMethod::Sidak) {
            return Err(format!("expected Sidak multiplicity on FactorialAnova, got {:?}", rs.analysis.kind));
        }
    }

    // Stage 4: Grouped Interleaved retains error bars; Stacked strips error bars (R1 grouped guard)
    let graph_id = linked_graph_id(&driver, &grp_id)?;
    select_sheet_human(&mut driver, &graph_id)?;
    change_graph_type_human(&mut driver, "grouped.interleaved")?;
    {
        let gs = graph_sheet(&driver, &graph_id)?;
        if gs.spec.kind != GraphKind::Grouped
            || gs.spec.grouped_arrangement != GroupedArrangement::Interleaved
            || !gs.spec.series.iter().any(|s| s.y_error_bars.is_some())
        {
            return Err(String::from("expected Grouped Interleaved graph with Y error bars"));
        }
    }
    let _ = assert_rendered_non_blank(&mut driver)?;

    change_graph_type_human(&mut driver, "grouped.stacked")?;
    {
        let gs = graph_sheet(&driver, &graph_id)?;
        if gs.spec.kind != GraphKind::Grouped || gs.spec.grouped_arrangement != GroupedArrangement::Stacked {
            return Err(format!("expected GraphKind::Grouped + Stacked, got {:?} / {:?}", gs.spec.kind, gs.spec.grouped_arrangement));
        }
        if gs.spec.series.iter().any(|s| s.y_error_bars.is_some()) {
            return Err(String::from("expected grouped.stacked to clear all Y error bars"));
        }
    }
    let _ = assert_rendered_non_blank(&mut driver)?;

    // Stage 5: Live upstream cell edit weakens interaction F immediately
    select_sheet_human(&mut driver, &grp_id)?;
    edit_cell_human(&mut driver, &grp_id, 2, 1, 2, "15")?;
    let f_inter_after = scalar(result_sheet(&driver, &res_id)?, "Row Factor × Column Factor: F")?;
    if (f_inter_after - f_inter_before).abs() < 1.0 {
        return Err(format!("expected interaction F to recompute ({f_inter_before} -> {f_inter_after})"));
    }
    finish_check(&mut driver)
}
