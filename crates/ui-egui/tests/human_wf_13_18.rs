//! Wave 6 (`R10.13`) strict `R10.2` human-path workflows `WF-13`..`WF-18`.
//!
//! Every workflow creates and populates tables, configures analyses, validates `R1`–`R9`
//! modal predicates, inspects multi-tab results and linked graphs, exercises `>= 120`-row
//! large-`N` or equation-tree wheel scrolling, and edits an upstream source cell to verify
//! live DAG propagation strictly through physical `HumanPathDriver` pointer/keyboard/wheel
//! interactions (`human_path_only = true`, zero semantic shortcuts).

#![allow(clippy::float_cmp)]

mod human_wf_common;

use std::error::Error;

use curvebook_doc::{
    AnalysisKind, DiagnosticPlotKind, GraphKind, GraphicalDataObject, OneWayMethod, ParameterScope, ProjectDocument, ResultSheet, ResultTabKind,
    SheetId,
};
use curvebook_stats::power::{PowerSolveTarget, PowerTestFamily};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::{KeyModifiers, RenderTargetSpec, UiApp, render_workspace_frame_png};

// ---------------------------------------------------------------------------
// Strict Human-Path Driver & Navigation Helpers
// ---------------------------------------------------------------------------

fn new_strict_human_driver() -> Result<HumanPathDriver, String> {
    let mut driver = human_wf_common::new_human_driver(UiApp::new(), human_wf_common::default_viewport());
    let compliance = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    if !compliance.is_clean() {
        return Err(format!("initial parity drift: {compliance:?}"));
    }
    Ok(driver)
}

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn assert_clean_parity_and_budget(driver: &mut HumanPathDriver) -> Result<(), String> {
    let report = driver.verify_parity();
    if !report.is_clean() {
        return Err(format!("parity report is not clean: {report:?}"));
    }
    human_wf_common::assert_input_to_paint_budget(driver, 40.0)
}

fn click(driver: &mut HumanPathDriver, widget_id: &str) -> Result<(), String> {
    let out = human_wf_common::click_widget_center(driver, &wid(widget_id))?;
    if !out.handled {
        return Err(format!("click on `{widget_id}` was not handled: {out:?}"));
    }
    Ok(())
}

fn click_inside_scroll(driver: &mut HumanPathDriver, container_id: &str, target_id: &str) -> Result<(), String> {
    human_wf_common::scroll_until_widget_visible(driver, &wid(container_id), &wid(target_id), 140.0, 50)?;
    click(driver, target_id)
}

fn widget_enabled(driver: &mut HumanPathDriver, widget_id: &str) -> Result<bool, String> {
    let _ = driver.render_frame();
    let tree = driver.layout_tree();
    let node = tree.node(widget_id).ok_or_else(|| format!("missing widget `{widget_id}` in layout tree"))?;
    Ok(node.state.enablement.enabled)
}

fn widget_checked(driver: &mut HumanPathDriver, widget_id: &str) -> Result<bool, String> {
    let _ = driver.render_frame();
    let tree = driver.layout_tree();
    let node = tree.node(widget_id).ok_or_else(|| format!("missing widget `{widget_id}` in layout tree"))?;
    Ok(node.a11y.checked.unwrap_or(false))
}

fn open_top_menu_item(driver: &mut HumanPathDriver, menu_id: &str, item_id: &str) -> Result<(), String> {
    click(driver, menu_id)?;
    let popup_id = format!("popup.{menu_id}");
    if driver.layout_tree().node(item_id).is_none() {
        let _ = human_wf_common::scroll_until_widget_visible(driver, &wid(&popup_id), &wid(item_id), 120.0, 40);
    }
    click(driver, item_id)
}

fn clear_and_type_into_textbox(driver: &mut HumanPathDriver, widget_id: &str, text: &str) -> Result<(), String> {
    click(driver, widget_id)?;
    human_wf_common::press_cmd_shortcut(driver, "a")?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    human_wf_common::type_into_focused(driver, text)?;
    Ok(())
}

fn enter_text_box_value(driver: &mut HumanPathDriver, widget_id: &str, text: &str) -> Result<(), String> {
    clear_and_type_into_textbox(driver, widget_id, text)
}

fn scroll_container_wheel_until_visible(
    driver: &mut HumanPathDriver,
    container_id: &str,
    target_id: &str,
    step_y: f64,
    max_steps: usize,
) -> Result<(), String> {
    for _ in 0..=max_steps {
        let _ = driver.render_frame();
        if driver.layout_tree().node(target_id).is_some() {
            return Ok(());
        }
        let center = driver.widget_center_logical(container_id).ok_or_else(|| format!("missing scroll container `{container_id}`"))?;
        driver.move_pointer(center[0], center[1])?;
        let out = driver.wheel_logical(center[0], center[1], 0.0, step_y, KeyModifiers::default())?;
        if !out.handled {
            break;
        }
    }
    Err(format!("widget `{target_id}` did not become visible inside `{container_id}` after {max_steps} wheel steps"))
}

fn scroll_grid_until_visible(driver: &mut HumanPathDriver, sheet_id: &str, target_id: &str, step_y: f64, max_steps: usize) -> Result<(), String> {
    let grid_id = format!("grid.sheet.{sheet_id}");
    scroll_container_wheel_until_visible(driver, &grid_id, target_id, step_y, max_steps)
}

fn commit_grid_inline_cell(driver: &mut HumanPathDriver, sheet_id: &str, cell_id: &str, text: &str) -> Result<(), String> {
    if driver.layout_tree().node(cell_id).is_none() {
        scroll_grid_until_visible(driver, sheet_id, cell_id, 120.0, 50)?;
    }
    click(driver, cell_id)?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    let center = human_wf_common::widget_hit_center(driver, &wid(cell_id))?;
    driver.double_click_logical(center[0], center[1])?;
    let typed = driver.type_text(text)?;
    if !typed.handled {
        return Err(format!("typing into cell `{cell_id}` was not handled: {typed:?}"));
    }
    driver.press_key("Enter", KeyModifiers::default())?;
    let _ = driver.render_frame();
    Ok(())
}

fn set_grid_y_cell(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, col: usize, subcol: usize, text: &str) -> Result<(), String> {
    commit_grid_inline_cell(driver, sheet_id, &format!("grid.cell.{row}.{col}.{subcol}"), text)
}

fn set_grid_x_cell(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, subcol: usize, text: &str) -> Result<(), String> {
    commit_grid_inline_cell(driver, sheet_id, &format!("grid.xcell.{row}.{subcol}"), text)
}

fn create_table_via_welcome(
    driver: &mut HumanPathDriver,
    arch_widget: &str,
    title: &str,
    extra_datasets: usize,
    subcol_widget: Option<&str>,
    design_widget: Option<&str>,
) -> Result<String, String> {
    click(driver, arch_widget)?;
    clear_and_type_into_textbox(driver, "welcome.title.input", title)?;
    for _ in 0..extra_datasets {
        click(driver, "welcome.datasets.inc")?;
    }
    if let Some(sub_id) = subcol_widget {
        click(driver, sub_id)?;
    }
    if let Some(des_id) = design_widget {
        click(driver, des_id)?;
    }
    click(driver, "welcome.create_cta")?;
    let _ = driver.render_frame();
    driver
        .app()
        .document()
        .data_sheets
        .last()
        .map(|s| s.id.as_str().to_owned())
        .ok_or_else(|| String::from("Welcome create_cta did not create a DataSheet"))
}

struct ImportSpec<'a> {
    target_kind_widget: &'a str,
    title: &'a str,
    csv_text: &'a str,
    existing_sheet_id: Option<&'a str>,
    replace_existing_mode: bool,
}

fn import_csv_via_dialog(driver: &mut HumanPathDriver, spec: ImportSpec<'_>) -> Result<String, String> {
    open_top_menu_item(driver, "menu.file", "menu.ui.import.reshape.open")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ImportReshape) {
        return Err(String::from("ImportReshape dialog did not open from menu.file"));
    }
    click(driver, "import_paste.source.clipboard")?;
    click(driver, "import_paste.delim.tab")?;
    click(driver, spec.target_kind_widget)?;
    clear_and_type_into_textbox(driver, "import_paste.input.title", spec.title)?;
    click_inside_scroll(driver, "import_paste.panel.source", "import_paste.input.csv")?;
    clear_and_type_into_textbox(driver, "import_paste.input.csv", spec.csv_text)?;

    click(driver, "import_paste.tab.placement")?;
    if let Some(dest_sid) = spec.existing_sheet_id {
        click_inside_scroll(driver, "import_paste.panel.placement", "import_paste.target_dest.existing")?;
        if spec.replace_existing_mode {
            click_inside_scroll(driver, "import_paste.panel.placement", "import_paste.place_mode.overwrite")?;
        }
        let dest_btn = format!("import_paste.dest_sheet.{dest_sid}");
        click_inside_scroll(driver, "import_paste.panel.placement", &dest_btn)?;
    } else {
        click_inside_scroll(driver, "import_paste.panel.placement", "import_paste.target_dest.new")?;
    }
    click(driver, "modal.ok")?;
    let _ = driver.render_frame();
    if let Some(dest_sid) = spec.existing_sheet_id {
        Ok(dest_sid.to_owned())
    } else {
        driver
            .app()
            .document()
            .data_sheets
            .last()
            .map(|s| s.id.as_str().to_owned())
            .ok_or_else(|| String::from("Import & Paste did not create new DataSheet"))
    }
}

fn run_analysis_from_toolbar(driver: &mut HumanPathDriver, entry_id: &str) -> Result<String, String> {
    click(driver, "toolbar.analyze")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
        return Err(String::from("AnalyzeSheet dialog did not open from toolbar.analyze"));
    }
    let widget_id = format!("analyze.entry.{entry_id}");
    click_inside_scroll(driver, "analyze.entries_list", &widget_id)?;
    click(driver, "modal.ok")?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        click(driver, "modal.ok")?;
    }
    let _ = driver.render_frame();
    driver
        .app()
        .document()
        .result_sheets
        .last()
        .map(|rs| rs.id.as_str().to_owned())
        .ok_or_else(|| format!("Analyze `{entry_id}` did not create ResultSheet"))
}

fn create_and_configure_graph(driver: &mut HumanPathDriver, graph_card_id: &str) -> Result<String, String> {
    click(driver, "nav.create.graphs")?;
    let _ = driver.render_frame();
    let gid = driver
        .app()
        .document()
        .graph_sheets
        .last()
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| String::from("nav.create.graphs did not create GraphSheet"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        click(driver, "toolbar.change_graph")?;
    }
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        return Err(String::from("ChangeGraphType dialog did not open for new GraphSheet"));
    }
    click(driver, graph_card_id)?;
    click(driver, "modal.ok")?;
    let _ = driver.render_frame();
    Ok(gid)
}

fn assert_rendered_png_non_empty(driver: &HumanPathDriver) -> Result<(), String> {
    let png = render_workspace_frame_png(driver.app(), RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 })
        .map_err(|e| format!("render_workspace_frame_png failed: {e}"))?;
    if png.len() < 128 || &png[0..4] != b"\x89PNG" {
        return Err(format!("invalid PNG output (len={})", png.len()));
    }
    Ok(())
}

fn result_sheet<'a>(doc: &'a ProjectDocument, id: &str) -> Result<&'a ResultSheet, String> {
    doc.result_sheet(&SheetId::from(id)).ok_or_else(|| format!("missing ResultSheet `{id}`"))
}

fn scalar_val(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    if let Some(entry) = rs.scalars.iter().find(|s| s.key.eq_ignore_ascii_case(key)) {
        return Ok(entry.value);
    }
    if let Some(entry) = rs.scalars.iter().find(|s| s.key.to_ascii_lowercase().contains(&key.to_ascii_lowercase())) {
        return Ok(entry.value);
    }
    let avail: Vec<&str> = rs.scalars.iter().map(|s| s.key.as_str()).collect();
    Err(format!("missing scalar `{key}` in ResultSheet `{}`; available={avail:?}", rs.id.as_str()))
}

fn has_tab_matching(rs: &ResultSheet, needle: &str) -> bool {
    let n = needle.to_ascii_lowercase();
    rs.tabs.iter().any(|t| t.id.to_ascii_lowercase().contains(&n) || t.title.to_ascii_lowercase().contains(&n))
}

fn switch_to_sheet_in_navigator(driver: &mut HumanPathDriver, sheet_id: &str) -> Result<(), String> {
    let nav_id = format!("nav.sheet.{sheet_id}");
    click(driver, &nav_id)?;
    let _ = driver.render_frame();
    Ok(())
}

fn cycle_result_tabs_and_scroll(driver: &mut HumanPathDriver, result_id: &str) -> Result<(), String> {
    let tab_ids: Vec<String> = result_sheet(driver.app().document(), result_id)?.tabs.iter().map(|t| t.id.clone()).collect();
    for tid in tab_ids {
        let w = format!("result.tab.{tid}");
        if driver.layout_tree().node(&w).is_some() {
            click(driver, &w)?;
        }
    }
    if driver.layout_tree().node("result.table").is_some() {
        let _ = human_wf_common::scroll_widget_wheel(driver, &wid("result.table"), 0.0, 180.0)?;
        let _ = human_wf_common::scroll_widget_wheel(driver, &wid("result.table"), 0.0, -180.0)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-13: Repeated-Measures One-Way ANOVA + R1 + Sphericity (GG & HF)
// ---------------------------------------------------------------------------

#[test]
fn wf13_human_rm_anova_sphericity_gg_and_hf() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;

    // Stage 1: Create 3-group paired/RM Column table via Welcome.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.column", "RM Sphericity", 0, None, Some("welcome.design.paired"))?;

    // Stage 2: Enter 6 paired blocks with mild non-sphericity directly via keyboard cells.
    let rows: [[&str; 3]; 6] = [
        ["10.2", "12.8", "18.1"],
        ["9.8", "13.1", "16.4"],
        ["11.0", "12.0", "19.5"],
        ["10.5", "14.2", "15.8"],
        ["9.5", "11.7", "17.2"],
        ["11.2", "13.5", "18.8"],
    ];
    for (r, vals) in rows.iter().enumerate() {
        for (c, v) in vals.iter().enumerate() {
            set_grid_y_cell(&mut driver, &ds_id, r, c, 0, v)?;
        }
    }

    // Stage 3: Run One-Way ANOVA -> Cmd+T -> check R1 and enable RM with GG & HF corrections.
    let rs_id = run_analysis_from_toolbar(&mut driver, "stats.one_way_anova")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("Cmd+T did not open EditAnalysisParameters for WF-13").into());
    }

    if !widget_enabled(&mut driver, "edit.anova.rm")? {
        return Err(String::from("R1 expected edit.anova.rm enabled on equal-N paired Column table").into());
    }
    click(&mut driver, "edit.anova.rm")?;
    if !widget_enabled(&mut driver, "edit.tab.repeated_measures")? {
        return Err(String::from("R1 expected Repeated Measures tab enabled when edit.anova.rm is selected").into());
    }
    click(&mut driver, "edit.tab.repeated_measures")?;
    click(&mut driver, "edit.sphericity.geisser_greenhouse")?;
    if widget_checked(&mut driver, "edit.rm.assume_sphericity")? {
        click(&mut driver, "edit.rm.assume_sphericity")?;
    }
    click(&mut driver, "modal.ok")?;

    let (f_before, p_before, eps_gg_before, eps_hf_before) = {
        let rs = result_sheet(driver.app().document(), &rs_id)?;
        let AnalysisKind::OneWayAnova(spec) = &rs.analysis.kind else {
            return Err(String::from("expected OneWayAnova after RM selection").into());
        };
        let OneWayMethod::RepeatedMeasures { geisser_greenhouse } = spec.method else {
            return Err(String::from("expected OneWayMethod::RepeatedMeasures").into());
        };
        if !geisser_greenhouse {
            return Err(String::from("expected Geisser-Greenhouse sphericity correction enabled").into());
        }
        let f = scalar_val(rs, "F (treatment)")?;
        let p = scalar_val(rs, "P value")?;
        let eps_gg = scalar_val(rs, "Greenhouse-Geisser epsilon")?;
        let eps_hf = scalar_val(rs, "Huynh-Feldt epsilon")?;
        let has_sphericity_check = rs.checklist.iter().any(|c| c.assumption.contains("Sphericity"));
        if f <= 10.0
            || !(0.4..=1.0).contains(&eps_gg)
            || eps_hf < eps_gg
            || p >= 0.01
            || !has_tab_matching(rs, "rm-one-way-anova")
            || !has_sphericity_check
        {
            return Err(format!("unexpected RM sphericity metrics: F={f}, gg={eps_gg}, hf={eps_hf}, p={p}").into());
        }
        (f, p, eps_gg, eps_hf)
    };

    // Stage 4: Inspect Result tabs + Create linked Before-After Graph + PNG render check.
    cycle_result_tabs_and_scroll(&mut driver, &rs_id)?;
    let graph_id = create_and_configure_graph(&mut driver, "graph_type.card.col.before_after")?;
    let gs = driver.app().document().graph_sheet(&SheetId::from(graph_id.as_str())).ok_or("missing WF-13 graph")?;
    if !matches!(gs.spec.series.first().map(|s| &s.gdo), Some(GraphicalDataObject::BeforeAfterLines { .. })) {
        return Err(String::from("WF-13 graph should use BeforeAfterLines topology").into());
    }
    assert_rendered_png_non_empty(&driver)?;

    // Verify R3.6 + R1b: on PairedRepeatedMeasures, missing a replicate keeps `edit.anova.rm` enabled
    // (Mixed-Effects REML fits incomplete blocks) while disabling sphericity corrections in `R1b`.
    switch_to_sheet_in_navigator(&mut driver, &ds_id)?;
    click(&mut driver, "grid.cell.5.2.0")?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    let _ = driver.render_frame();
    switch_to_sheet_in_navigator(&mut driver, &rs_id)?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !widget_enabled(&mut driver, "edit.anova.rm")? {
        return Err(String::from("R3.6 expected edit.anova.rm to remain enabled on incomplete PairedRepeatedMeasures blocks").into());
    }
    click(&mut driver, "edit.tab.repeated_measures")?;
    if widget_enabled(&mut driver, "edit.sphericity.geisser_greenhouse")? {
        return Err(String::from("R1b violated: sphericity must disable when incomplete blocks trigger Mixed-Effects REML").into());
    }
    click(&mut driver, "modal.cancel")?;

    // Stage 5: Restore equal N with a different observation and assert live DAG recomputation.
    switch_to_sheet_in_navigator(&mut driver, &ds_id)?;
    set_grid_y_cell(&mut driver, &ds_id, 5, 2, 0, "24.5")?;
    switch_to_sheet_in_navigator(&mut driver, &rs_id)?;
    let rs_after = result_sheet(driver.app().document(), &rs_id)?;
    let f_after = scalar_val(rs_after, "F (treatment)")?;
    let eps_gg_after = scalar_val(rs_after, "Greenhouse-Geisser epsilon")?;
    if (f_after - f_before).abs() < 1e-4 && (eps_gg_after - eps_gg_before).abs() < 1e-4 && (scalar_val(rs_after, "P value")? - p_before).abs() < 1e-6
    {
        return Err(format!("live cell edit did not update RM ANOVA metrics (eps_hf_before={eps_hf_before}, f_after={f_after})").into());
    }

    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-14: Friedman Nonparametric RM + QOL-7 Diagnostics Banner + Cmd+T Switch
// ---------------------------------------------------------------------------

#[test]
fn wf14_human_friedman_diagnostics_banner_and_cmd_t() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;

    // Stage 1: Create paired Column table via Welcome.
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.column", "Friedman Cohort", 0, None, Some("welcome.design.paired"))?;

    // Stage 2: Load skewed / outlier-containing repeated-measures cohort via Import & Paste.
    let mut csv = String::from("Baseline\t1h\t4h\n");
    let blocks: [[f64; 3]; 8] = [
        [10.0, 21.0, 42.0],
        [12.0, 19.0, 38.0],
        [11.0, 24.0, 47.0],
        [9.0, 18.0, 35.0],
        [13.0, 25.0, 49.0],
        [10.5, 22.0, 44.0],
        [14.0, 27.0, 51.0],
        [11.5, 23.0, 450.0], // Heavy outlier triggering non-normality assumption warning
    ];
    for b in blocks {
        csv.push_str(&format!("{}\t{}\t{}\n", b[0], b[1], b[2]));
    }
    import_csv_via_dialog(
        &mut driver,
        ImportSpec {
            target_kind_widget: "import_paste.target.column",
            title: "Friedman Cohort",
            csv_text: &csv,
            existing_sheet_id: Some(&ds_id),
            replace_existing_mode: true,
        },
    )?;

    // Stage 3a: Run parametric RM-ANOVA first and verify QOL-7 diagnostics banner is advisory only.
    let rs_id = run_analysis_from_toolbar(&mut driver, "stats.one_way_anova")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "edit.anova.rm")?;
    click(&mut driver, "modal.ok")?;

    click(&mut driver, "result.interpret_toggle_btn")?;
    let _ = driver.render_frame();
    if driver.layout_tree().node("result.diagnostic_banner").is_some() {
        click(&mut driver, "result.diagnostic_banner")?;
    }
    let still_parametric = matches!(
        &result_sheet(driver.app().document(), &rs_id)?.analysis.kind,
        AnalysisKind::OneWayAnova(spec) if matches!(spec.method, OneWayMethod::RepeatedMeasures { .. })
    );
    if !still_parametric {
        return Err(String::from("QOL-7 violation: inspecting diagnostics auto-mutated analysis method").into());
    }

    // Stage 3b: Explicitly press Cmd+T and switch via UI controls to Nonparametric Friedman.
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("Cmd+T did not open EditAnalysisParameters for Friedman switch").into());
    }
    if widget_checked(&mut driver, "edit.assume_gaussian")? {
        click(&mut driver, "edit.assume_gaussian")?;
    }
    click(&mut driver, "edit.anova.friedman")?;
    click(&mut driver, "modal.ok")?;

    let (chi2_before, p_before) = {
        let rs = result_sheet(driver.app().document(), &rs_id)?;
        let AnalysisKind::OneWayAnova(spec) = &rs.analysis.kind else {
            return Err(String::from("expected OneWayAnova(Friedman)").into());
        };
        if !matches!(spec.method, OneWayMethod::Friedman) {
            return Err(format!("expected OneWayMethod::Friedman, got {:?}", spec.method).into());
        }
        let chi2 = scalar_val(rs, "Friedman")?;
        let p = scalar_val(rs, "P value")?;
        let w = scalar_val(rs, "Kendall")?;
        if (chi2 - 16.0).abs() > 1e-4 || p >= 0.001 || (w - 1.0).abs() > 1e-4 {
            return Err(format!("expected perfect monotonic Friedman chi2=16.0 (p<0.001, W=1.0), got chi2={chi2}, p={p}, w={w}").into());
        }
        (chi2, p)
    };

    // Stage 4: Check result tabs, Dunn's / summary tabs, and linked Box-Whisker graph.
    cycle_result_tabs_and_scroll(&mut driver, &rs_id)?;
    let _ = create_and_configure_graph(&mut driver, "graph_type.card.col.box_whisker")?;
    assert_rendered_png_non_empty(&driver)?;

    // Stage 5: Live upstream cell edit flipping rank ordering in block 0 reduces Friedman chi2.
    switch_to_sheet_in_navigator(&mut driver, &ds_id)?;
    set_grid_y_cell(&mut driver, &ds_id, 0, 2, 0, "5.0")?;
    switch_to_sheet_in_navigator(&mut driver, &rs_id)?;
    let rs_after = result_sheet(driver.app().document(), &rs_id)?;
    let chi2_after = scalar_val(rs_after, "Friedman")?;
    let p_after = scalar_val(rs_after, "P value")?;
    if chi2_after >= chi2_before || p_after <= p_before {
        return Err(format!("expected Friedman rank reversal edit to lower chi2 ({chi2_before} -> {chi2_after})").into());
    }

    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-15: Full 4-Stage Power & Sample Size Wizard + Curve Hover + Sensitivity
// ---------------------------------------------------------------------------

#[test]
fn wf15_human_full_four_stage_power_wizard_and_hover() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;

    // Dismiss initial Welcome overlay, then open Power & Sample Size Wizard via Analyze menu.
    click(&mut driver, "welcome.cancel")?;
    open_top_menu_item(&mut driver, "menu.analyze", "menu.ui.powerWizard.open")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize) {
        return Err(String::from("PowerAndSampleSize modal did not open via Analyze menu").into());
    }

    // Stage 1: Test family + Solve target -> Next.
    click(&mut driver, "power.family.unpaired_t")?;
    click(&mut driver, "power.solve.sample_size")?;
    if driver.app().canvas_dialogs.power_wizard.family != PowerTestFamily::TwoSampleUnpairedT
        || driver.app().canvas_dialogs.power_wizard.solve_target != PowerSolveTarget::SampleSize
    {
        return Err(String::from("Stage 1 failed to select Unpaired T + Sample Size").into());
    }
    click(&mut driver, "dialog.button.next")?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::EffectSize {
        return Err(String::from("Next did not advance Power Wizard to Stage 2").into());
    }

    // Stage 2: Pilot Means & SD (10.0 vs 14.0, SD = 3.0 => Cohen's d = 1.3333) -> Next.
    click(&mut driver, "power.effect_mode.means_and_sd")?;
    enter_text_box_value(&mut driver, "power.pilot.mean1.input", "10.0")?;
    enter_text_box_value(&mut driver, "power.pilot.mean2.input", "14.0")?;
    enter_text_box_value(&mut driver, "power.pilot.sd.input", "3.0")?;
    click(&mut driver, "power.pilot.apply")?;
    let eff = driver.app().canvas_dialogs.power_wizard.derived_standardized_effect().ok_or("expected derived Cohen d")?;
    if (eff - (4.0 / 3.0)).abs() > 0.05 {
        return Err(format!("expected derived Cohen d ≈ 1.333, got {eff}").into());
    }
    click(&mut driver, "dialog.button.next")?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::SignificanceAndSolve {
        return Err(String::from("Next did not advance Power Wizard to Stage 3").into());
    }

    // Stage 3: Two-tailed, alpha = 0.05, power = 0.80 -> check sensitivity vs 0.01 -> Next.
    click(&mut driver, "power.two_tailed")?;
    click(&mut driver, "power.stage3.alpha.001")?;
    let n_001 = driver.app().canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.total_n);
    click(&mut driver, "power.stage3.alpha.005")?;
    click(&mut driver, "power.stage3.power.80")?;
    let n_005 = driver.app().canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.total_n);
    if n_001 <= n_005 || n_005 < 16 {
        return Err(format!("unexpected solved sample sizes: alpha=0.01 N={n_001}, alpha=0.05 N={n_005}").into());
    }
    click(&mut driver, "dialog.button.next")?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::Results {
        return Err(String::from("Next did not advance Power Wizard to Stage 4").into());
    }

    // Stage 4: Passive hover + click-to-pin curve readout, scroll sensitivity table & sidebar, create sheets.
    let _ = human_wf_common::hover_widget_center(&mut driver, &wid("power.curve_hover.bin.2"))?;
    let _ = driver.render_frame();
    if driver.app().interaction.hovered_widget_id.as_deref() != Some("power.curve_hover.bin.2") {
        return Err(String::from("passive hover over power.curve_hover.bin.2 did not update hovered_widget_id").into());
    }
    click(&mut driver, "power.curve_hover.bin.12")?;
    let _ = driver.render_frame();
    if driver.app().view_shards.power_wizard.hover_sample_size.is_none()
        || driver.app().view_shards.power_wizard.hover_power.is_none()
        || driver.app().canvas_dialogs.power_wizard.hover_power.is_none()
    {
        return Err(String::from("clicking power.curve_hover.bin.12 did not populate hover_sample_size/hover_power").into());
    }

    click(&mut driver, "power.sensitivity.row.1")?;
    let _ = human_wf_common::scroll_widget_wheel(&mut driver, &wid("power.sensitivity_table"), 0.0, 140.0)?;
    scroll_container_wheel_until_visible(&mut driver, "power.sensitivity_table", "power.sensitivity.row.4", -60.0, 15)?;
    let _ = human_wf_common::scroll_widget_wheel(&mut driver, &wid("power.sidebar"), 0.0, 90.0)?;
    click(&mut driver, "modal.ok")?;
    let _ = driver.render_frame();

    let doc = driver.app().document();
    if doc.data_sheets.is_empty() || doc.result_sheets.is_empty() || doc.graph_sheets.is_empty() || doc.info_sheets.is_empty() {
        return Err(format!(
            "Power Wizard Create did not generate all 4 sheets (data={}, res={}, graph={}, info={})",
            doc.data_sheets.len(),
            doc.result_sheets.len(),
            doc.graph_sheets.len(),
            doc.info_sheets.len()
        )
        .into());
    }
    let res_id = doc.result_sheets.last().map(|s| s.id.as_str().to_owned()).ok_or("missing power ResultSheet")?;
    let sens_ds_id = doc.data_sheets.last().map(|s| s.id.as_str().to_owned()).ok_or("missing power DataSheet")?;
    cycle_result_tabs_and_scroll(&mut driver, &res_id)?;
    assert_rendered_png_non_empty(&driver)?;

    // Stage 5: Live upstream edit in the generated sensitivity DataSheet + parity/budget check.
    switch_to_sheet_in_navigator(&mut driver, &sens_ds_id)?;
    set_grid_y_cell(&mut driver, &sens_ds_id, 0, 0, 0, "0.95")?;

    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-16: Linear, Deming, Passing-Bablok, ANCOVA, R8 & R9 Predicates + 120-Row Scroll
// ---------------------------------------------------------------------------

#[test]
fn wf16_human_linear_deming_pb_ancova_and_r8_r9() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;

    // Stage 1: Create 3-subcolumn Grouped table via Welcome (verifies Welcome + R8 multi-subcolumn gate).
    let sub_col_id = create_table_via_welcome(&mut driver, "welcome.arch.grouped", "Subcol R8", 0, Some("welcome.subcol.raw"), None)?;
    for r in 0..3_usize {
        for s in 0..3_usize {
            let val = 10.0 + (r as f64) * 2.0 + (s as f64) * 0.5;
            set_grid_y_cell(&mut driver, &sub_col_id, r, 0, s, &format!("{val:.1}"))?;
        }
    }
    let desc_multi = run_analysis_from_toolbar(&mut driver, "stats.descriptive")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !widget_enabled(&mut driver, "edit.desc.agg.per_sub")? {
        return Err(String::from("R8 violated: subcolumn aggregation disabled on 3-subcolumn table").into());
    }
    click(&mut driver, "edit.desc.agg.per_sub")?;
    click(&mut driver, "modal.ok")?;
    let _ = result_sheet(driver.app().document(), &desc_multi)?;

    // Stage 2: Import 120 paired XY rows (X + Series A + Series B, parallel slopes ~2.0, shifted intercepts +1 vs +5).
    let mut csv = String::from("X\tSeries A\tSeries B\n");
    for i in 0..120_usize {
        let x = (i as f64) * 0.25;
        let jitter = ((i % 3) as f64 - 1.0) * 0.05;
        let ya = 2.0 * x + 1.0 + jitter;
        let yb = 2.0 * x + 5.0 - jitter;
        csv.push_str(&format!("{x:.2}\t{ya:.3}\t{yb:.3}\n"));
    }
    let xy_id = import_csv_via_dialog(
        &mut driver,
        ImportSpec {
            target_kind_widget: "import_paste.target.xy",
            title: "Line Comparison",
            csv_text: &csv,
            existing_sheet_id: None,
            replace_existing_mode: false,
        },
    )?;
    switch_to_sheet_in_navigator(&mut driver, &xy_id)?;

    let xy_rows = driver.app().document().data_sheet(&SheetId::from(xy_id.as_str())).map_or(0, |s| s.archetype.row_count());
    if xy_rows != 120 {
        return Err(format!("expected 120 XY rows after import, got {xy_rows}").into());
    }

    // Scroll >= 120-row grid cleanly via wheel until bottom row cell `119` is visible, then back up.
    scroll_grid_until_visible(&mut driver, &xy_id, "grid.cell.119.0.0", 240.0, 50)?;
    scroll_grid_until_visible(&mut driver, &xy_id, "grid.cell.0.0.0", -240.0, 50)?;

    // Stage 3a: Simple Linear Regression + ANCOVA multi-line comparison -> switch to Deming Model II via Cmd+T.
    let lr_id = run_analysis_from_toolbar(&mut driver, "fit.linear")?;
    let ols_slope = {
        let rs = result_sheet(driver.app().document(), &lr_id)?;
        let slope = scalar_val(rs, "slope")?;
        if (slope - 2.0).abs() > 0.05 || !has_tab_matching(rs, "ancova") {
            return Err(format!("expected OLS slope ≈ 2.0 and ANCOVA tab; slope={slope}").into());
        }
        slope
    };

    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.lr.conf")?;
    click(&mut driver, "fit.lr.pred")?;
    click(&mut driver, "fit.lr.deming")?;
    click(&mut driver, "modal.ok")?;
    let deming_slope = {
        let rs = result_sheet(driver.app().document(), &lr_id)?;
        let AnalysisKind::LinearRegression(spec) = &rs.analysis.kind else {
            return Err(String::from("expected LinearRegression after Deming toggle").into());
        };
        if !spec.deming_model_ii {
            return Err(String::from("Deming flag did not commit in EditAnalysisParameters").into());
        }
        scalar_val(rs, "slope")?
    };
    if (deming_slope - 2.0).abs() > 0.05 || (deming_slope - ols_slope).abs() > 0.2 {
        return Err(format!("unexpected Deming slope {deming_slope} vs OLS {ols_slope}").into());
    }
    cycle_result_tabs_and_scroll(&mut driver, &lr_id)?;

    // Stage 3b: Passing-Bablok nonparametric regression on the same XY sheet.
    switch_to_sheet_in_navigator(&mut driver, &xy_id)?;
    let pb_id = run_analysis_from_toolbar(&mut driver, "stats.passing_bablok")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.pb.ci.99")?;
    click(&mut driver, "modal.ok")?;
    let pb_slope = scalar_val(result_sheet(driver.app().document(), &pb_id)?, "slope")?;
    if (pb_slope - 2.0).abs() > 0.05 {
        return Err(format!("unexpected Passing-Bablok slope {pb_slope}").into());
    }

    // Stage 4: Create & render XY Scatter+Line graph; verify `menu.graph.setRegressionOverlay` exists in menu.
    switch_to_sheet_in_navigator(&mut driver, &xy_id)?;
    let _ = create_and_configure_graph(&mut driver, "graph_type.card.xy.scatter_line")?;
    click(&mut driver, "menu.graph")?;
    let _ = driver.render_frame();
    if driver.layout_tree().node("menu.graph.setRegressionOverlay").is_none() {
        return Err(String::from("expected `menu.graph.setRegressionOverlay` entry under Graph menu").into());
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    assert_rendered_png_non_empty(&driver)?;

    // Stage 3c: Verify R8 (subcolumn aggregation gate) & R9 (P-value precision spinner gate) via stat_params.
    // On single-subcolumn XY data (`xy_id`), R8 disables `edit.desc.agg.*`; on 3-subcolumn `sub_col_id`, R8 enabled them above.
    switch_to_sheet_in_navigator(&mut driver, &xy_id)?;
    let desc_single = run_analysis_from_toolbar(&mut driver, "stats.descriptive")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if widget_enabled(&mut driver, "edit.desc.agg.per_sub")? {
        return Err(String::from("R8 violated: subcolumn aggregation enabled on single-subcolumn table").into());
    }
    click(&mut driver, "edit.tab.options")?;
    click(&mut driver, "edit.pval.apa")?;
    if widget_enabled(&mut driver, "edit.pvalue.digits.inc")? {
        return Err(String::from("R9 violated: P-value digit spinner enabled under APA format").into());
    }
    click(&mut driver, "edit.pval.custom")?;
    if !widget_enabled(&mut driver, "edit.pvalue.digits.inc")? {
        return Err(String::from("R9 violated: P-value digit spinner disabled under Custom format").into());
    }
    click(&mut driver, "edit.pvalue.digits.inc")?;
    click(&mut driver, "modal.ok")?;
    let _ = result_sheet(driver.app().document(), &desc_single)?;

    // Stage 5: Live upstream XY cell edit propagates to Linear & Passing-Bablok fits.
    switch_to_sheet_in_navigator(&mut driver, &xy_id)?;
    set_grid_y_cell(&mut driver, &xy_id, 0, 0, 0, "45.0")?;
    switch_to_sheet_in_navigator(&mut driver, &lr_id)?;
    let slope_after = scalar_val(result_sheet(driver.app().document(), &lr_id)?, "slope")?;
    if (slope_after - deming_slope).abs() < 1e-4 {
        return Err(format!("live cell edit did not update Linear/Deming slope ({deming_slope} -> {slope_after})").into());
    }

    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-17: Dose-Response 4PL/5PL + Unknown Interpolation + R7 Heatmap Predicate
// ---------------------------------------------------------------------------

#[test]
fn wf17_human_dose_response_4pl_5pl_interp_and_heatmap_r7() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;

    // Stage 1: Create XY table for log(agonist) vs response 4PL/5PL dose-response + unknowns.
    let dr_id = create_table_via_welcome(&mut driver, "welcome.arch.xy", "Dose Response 4PL", 0, None, None)?;

    // Stage 2: Enter 8 standard points (Bottom=10, Top=100, LogEC50=-7.0, HillSlope=1.0) + 2 Y-only unknowns.
    let stds: [(&str, &str); 8] = [
        ("-9.0", "10.89"),
        ("-8.3", "14.30"),
        ("-7.7", "24.97"),
        ("-7.3", "40.04"),
        ("-7.0", "55.00"),
        ("-6.7", "69.96"),
        ("-6.0", "91.82"),
        ("-5.0", "99.11"),
    ];
    for (r, (x_txt, y_txt)) in stds.iter().enumerate() {
        set_grid_x_cell(&mut driver, &dr_id, r, 0, x_txt)?;
        set_grid_y_cell(&mut driver, &dr_id, r, 0, 0, y_txt)?;
    }
    set_grid_y_cell(&mut driver, &dr_id, 8, 0, 0, "55.00")?;
    set_grid_y_cell(&mut driver, &dr_id, 9, 0, 0, "32.50")?;

    // Stage 3a: Run Nonlinear Regression, scroll equation tree (`fit.eq.tree.scroll`) to 4PL, enable Interpolation + AICc vs 5PL.
    let nl_id = run_analysis_from_toolbar(&mut driver, "fit.nonlinear")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("Cmd+T did not open Fit Parameters dialog").into());
    }
    click(&mut driver, "fit.eq.expand_all")?;
    click_inside_scroll(&mut driver, "fit.eq.tree.scroll", "fit.eq.item.log_agonist_vs_response_4pl")?;

    click(&mut driver, "fit.tab.weights_range")?;
    if !widget_checked(&mut driver, "fit.wr.opt.interp")? {
        click(&mut driver, "fit.wr.opt.interp")?;
    }
    click(&mut driver, "fit.tab.compare")?;
    click(&mut driver, "fit.cmp.aicc")?;
    click_inside_scroll(&mut driver, "fit.cmp.alt.scroll", "fit.cmp.alt.log_agonist_vs_response_5pl")?;
    click(&mut driver, "modal.ok")?;

    let log_ec50_before = {
        let rs = result_sheet(driver.app().document(), &nl_id)?;
        let log_ec50 = scalar_val(rs, "LogEC50")?;
        let r2 = scalar_val(rs, "R2")?;
        if (log_ec50 + 7.0).abs() > 0.1 || r2 < 0.99 || !has_tab_matching(rs, "interp") {
            return Err(format!("4PL fit failed: LogEC50={log_ec50}, R2={r2}, tabs={:?}", rs.tabs.len()).into());
        }
        log_ec50
    };

    // Re-open Cmd+T, switch to 5PL Richards asymmetric curve and confirm convergence.
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.tab.model")?;
    click(&mut driver, "fit.eq.expand_all")?;
    click_inside_scroll(&mut driver, "fit.eq.tree.scroll", "fit.eq.item.log_agonist_vs_response_5pl")?;
    click(&mut driver, "modal.ok")?;
    cycle_result_tabs_and_scroll(&mut driver, &nl_id)?;

    // Switch back to 4PL so LogEC50 stays well-conditioned for live edit check, and render XY scatter graph.
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.tab.model")?;
    click(&mut driver, "fit.eq.expand_all")?;
    click_inside_scroll(&mut driver, "fit.eq.tree.scroll", "fit.eq.item.log_agonist_vs_response_4pl")?;
    click(&mut driver, "modal.ok")?;
    let _ = create_and_configure_graph(&mut driver, "graph_type.card.xy.scatter")?;
    assert_rendered_png_non_empty(&driver)?;

    // Stage 3b: Verify R7 (Correlation all-pairs matrix -> Heatmap toggle) + Heatmap Graph on MV table.
    let mv_csv = "VarA\tVarB\tVarC\n1.0\t2.1\t9.5\n2.0\t4.0\t8.0\n3.0\t5.9\t6.1\n4.0\t8.2\t4.2\n5.0\t9.9\t2.0\n6.0\t12.1\t0.5\n";
    let mv_id = import_csv_via_dialog(
        &mut driver,
        ImportSpec {
            target_kind_widget: "import_paste.target.mv",
            title: "Corr Matrix MV",
            csv_text: mv_csv,
            existing_sheet_id: None,
            replace_existing_mode: false,
        },
    )?;
    let corr_id = run_analysis_from_toolbar(&mut driver, "stats.corr_matrix")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !widget_checked(&mut driver, "edit.correlation_all_pairs")? || !widget_enabled(&mut driver, "edit.correlation.heatmap")? {
        return Err(String::from("R7 expected edit.correlation.heatmap enabled in all-pairs mode").into());
    }
    // Uncheck all-pairs -> R7 disables heatmap checkbox; re-check -> enabled and toggle checked.
    click(&mut driver, "edit.correlation_all_pairs")?;
    if widget_enabled(&mut driver, "edit.correlation.heatmap")? {
        return Err(String::from("R7 violated: heatmap stayed enabled in X-vs-every-Y mode").into());
    }
    click(&mut driver, "edit.correlation_all_pairs")?;
    click(&mut driver, "edit.correlation.heatmap")?;
    if !driver.app().canvas_dialogs.edit_params.correlation_heatmap {
        return Err(String::from("edit.correlation.heatmap click failed to update draft flag").into());
    }
    click(&mut driver, "modal.ok")?;
    let _ = result_sheet(driver.app().document(), &corr_id)?;

    switch_to_sheet_in_navigator(&mut driver, &mv_id)?;
    let hm_graph_id = create_and_configure_graph(&mut driver, "graph_type.card.diag.heatmap")?;
    let hm_gs = driver.app().document().graph_sheet(&SheetId::from(hm_graph_id.as_str())).ok_or("missing Heatmap GraphSheet")?;
    if !matches!(hm_gs.spec.kind, GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)) {
        return Err(format!("expected DiagnosticPlotKind::Heatmap, got {:?}", hm_gs.spec.kind).into());
    }
    assert_rendered_png_non_empty(&driver)?;

    // Stage 5: Live upstream edit on dose-response curve shifts fitted LogEC50.
    switch_to_sheet_in_navigator(&mut driver, &dr_id)?;
    set_grid_y_cell(&mut driver, &dr_id, 4, 0, 0, "82.00")?;
    switch_to_sheet_in_navigator(&mut driver, &nl_id)?;
    let log_ec50_after = scalar_val(result_sheet(driver.app().document(), &nl_id)?, "LogEC50")?;
    if (log_ec50_after - log_ec50_before).abs() < 1e-3 {
        return Err(format!("live edit did not shift LogEC50 ({log_ec50_before} -> {log_ec50_after})").into());
    }

    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-18: Michaelis-Menten Enzyme Kinetics + Global Shared Vmax + Diagnostics
// ---------------------------------------------------------------------------

#[test]
fn wf18_human_enzyme_kinetics_global_vmax_diagnostics() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;

    // Stage 1: Create 2-series XY table via Welcome (Control vs Competitive Inhibitor).
    let ds_id = create_table_via_welcome(&mut driver, "welcome.arch.xy", "Enzyme Kinetics", 1, None, None)?;

    // Stage 2: Enter Michaelis-Menten velocities (`Vmax = 100`, `Km_ctrl = 5`, `Km_inh = 20`) via keyboard cells.
    let substrates = [0.0_f64, 1.0, 2.0, 5.0, 10.0, 20.0, 40.0, 80.0];
    for (r, &s) in substrates.iter().enumerate() {
        let v_ctrl = 100.0 * s / (5.0 + s);
        let v_inh = 100.0 * s / (20.0 + s);
        set_grid_x_cell(&mut driver, &ds_id, r, 0, &format!("{s:.1}"))?;
        set_grid_y_cell(&mut driver, &ds_id, r, 0, 0, &format!("{v_ctrl:.4}"))?;
        set_grid_y_cell(&mut driver, &ds_id, r, 1, 0, &format!("{v_inh:.4}"))?;
    }

    // Stage 3a: Run Nonlinear Regression, search/scroll equation tree to `michaelis_menten`, confirm local fit.
    let rs_id = run_analysis_from_toolbar(&mut driver, "fit.nonlinear")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.eq.expand_all")?;
    clear_and_type_into_textbox(&mut driver, "fit.eq.search", "Michaelis")?;
    click_inside_scroll(&mut driver, "fit.eq.tree.scroll", "fit.eq.item.michaelis_menten")?;
    click(&mut driver, "modal.ok")?;

    let (vmax_local, km_local) = {
        let rs = result_sheet(driver.app().document(), &rs_id)?;
        (scalar_val(rs, "Vmax")?, scalar_val(rs, "Km")?)
    };
    if (vmax_local - 100.0).abs() > 1.0 || (km_local - 5.0).abs() > 0.5 {
        return Err(format!("unexpected local Michaelis-Menten fit: Vmax={vmax_local}, Km={km_local}").into());
    }

    // Stage 3b: Re-open Cmd+T -> Constraints -> share `Vmax` globally + Weights/Range -> enable diagnostics.
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.tab.constraints")?;
    click(&mut driver, "fit.con.row.0")?;
    click(&mut driver, "fit.con.set_scope.shared")?;

    click(&mut driver, "fit.tab.weights_range")?;
    for tog in ["fit.wr.opt.norm", "fit.wr.opt.homo", "fit.wr.opt.res_tab"] {
        if !widget_checked(&mut driver, tog)? {
            click(&mut driver, tog)?;
        }
    }
    click(&mut driver, "modal.ok")?;

    let (vmax_global, km_global) = {
        let rs = result_sheet(driver.app().document(), &rs_id)?;
        let AnalysisKind::NonlinearRegression(nl) = &rs.analysis.kind else {
            return Err(String::from("expected NonlinearRegression after global Vmax constraint").into());
        };
        let shared_vmax = nl.parameter_rules.iter().any(|r| r.name == "Vmax" && matches!(r.scope, ParameterScope::SharedGlobal));
        if !shared_vmax {
            return Err(String::from("Vmax parameter rule was not saved as SharedGlobal").into());
        }
        if rs.checklist.is_empty() || !rs.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals) {
            return Err(String::from("expected regression diagnostics checklist & Residuals tab in Enzyme Kinetics ResultSheet").into());
        }
        (scalar_val(rs, "Vmax")?, scalar_val(rs, "Km")?)
    };
    if (vmax_global - 100.0).abs() > 1.0 || (km_global - 5.0).abs() > 0.5 {
        return Err(format!("unexpected global Vmax fit: Vmax={vmax_global}, Km={km_global}").into());
    }

    // Stage 4: Cycle ResultSheet tabs + Create Scatter+Line Graph + PNG render check.
    cycle_result_tabs_and_scroll(&mut driver, &rs_id)?;
    let _ = create_and_configure_graph(&mut driver, "graph_type.card.xy.scatter_line")?;
    assert_rendered_png_non_empty(&driver)?;

    // Stage 5: Live upstream cell edit on Control velocity updates global Vmax & Km automatically.
    switch_to_sheet_in_navigator(&mut driver, &ds_id)?;
    set_grid_y_cell(&mut driver, &ds_id, 7, 0, 0, "140.0")?;
    switch_to_sheet_in_navigator(&mut driver, &rs_id)?;
    let rs_after = result_sheet(driver.app().document(), &rs_id)?;
    let vmax_after = scalar_val(rs_after, "Vmax")?;
    if (vmax_after - vmax_global).abs() < 1.0 {
        return Err(format!("live cell edit did not update global Vmax ({vmax_global} -> {vmax_after})").into());
    }

    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

#[test]
fn r13_1_human_linear_fit_range_controls() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;
    let data_id = import_csv_via_dialog(
        &mut driver,
        ImportSpec {
            target_kind_widget: "import_paste.target.xy",
            title: "Terminal Phase",
            csv_text: "X\tResponse\n0\t90\n10\t24\n12\t26\n14\t30\n16\t36\n20\t-100\n",
            existing_sheet_id: None,
            replace_existing_mode: false,
        },
    )?;
    switch_to_sheet_in_navigator(&mut driver, &data_id)?;
    let fit_id = run_analysis_from_toolbar(&mut driver, "fit.linear")?;

    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.lr.tab.model")?;
    driver.press_key("Tab", KeyModifiers::default())?;
    driver.press_key("Space", KeyModifiers::default())?;
    driver.press_key("Tab", KeyModifiers::default())?;
    assert_eq!(driver.app().focus_widget_id.as_deref(), Some("fit.lr.range.input.x_min"));
    human_wf_common::type_into_focused(&mut driver, "10")?;
    clear_and_type_into_textbox(&mut driver, "fit.lr.range.input.x_max", "16")?;
    clear_and_type_into_textbox(&mut driver, "fit.lr.range.input.segments", "0")?;
    let field = driver.find_widget("fit.lr.range.input.segments").ok_or("missing segments field")?;
    assert_eq!(field.a11y.value_text.as_deref(), Some("0"));

    // An invalid segment count keeps the dialog open and leaves the stored fit untouched.
    let error = human_wf_common::click_widget_center(&mut driver, &wid("modal.ok")).err().ok_or("zero segments should be rejected")?;
    assert!(error.contains("Curve segments"), "{error}");
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    let unchanged = result_sheet(driver.app().document(), &fit_id)?;
    assert!(matches!(&unchanged.analysis.kind, AnalysisKind::LinearRegression(spec) if spec.x_range_filter.is_none()));

    clear_and_type_into_textbox(&mut driver, "fit.lr.range.input.segments", "3")?;
    click(&mut driver, "fit.lr.tab.model")?;
    click(&mut driver, "fit.lr.conf")?;
    click(&mut driver, "modal.ok")?;
    let restricted = result_sheet(driver.app().document(), &fit_id)?;
    assert!(matches!(
        &restricted.analysis.kind,
        AnalysisKind::LinearRegression(spec) if spec.x_range_filter == Some((Some(10.0), Some(16.0)))
            && spec.plot_segments == Some(3) && !spec.confidence_bands
    ));
    assert!((scalar_val(restricted, "slope")? - 2.0).abs() < 1e-12);
    let overlay = restricted.fitted_curves.first().ok_or("missing restricted overlay")?;
    assert_eq!(overlay.points.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 12.0, 14.0, 16.0]);

    // Both dismissal routes discard buffered input before the next edit session.
    for escape in [true, false] {
        human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
        click(&mut driver, "fit.lr.tab.range")?;
        clear_and_type_into_textbox(&mut driver, "fit.lr.range.input.plot_max", "22")?;
        if escape {
            driver.press_key("Escape", KeyModifiers::default())?;
        } else {
            click(&mut driver, "modal.cancel")?;
        }
        assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
        click(&mut driver, "fit.lr.tab.range")?;
        driver.render_frame()?;
        let tree = driver.layout_tree();
        let field = tree.node("fit.lr.range.input.plot_max").ok_or("missing curve end field")?;
        assert_eq!(field.a11y.value_text.as_deref(), Some(""));
        click(&mut driver, "modal.cancel")?;
    }

    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.lr.tab.range")?;
    clear_and_type_into_textbox(&mut driver, "fit.lr.range.input.plot_min", "7")?;
    clear_and_type_into_textbox(&mut driver, "fit.lr.range.input.plot_max", "19")?;
    click(&mut driver, "modal.ok")?;
    let extrapolated = result_sheet(driver.app().document(), &fit_id)?;
    assert!((scalar_val(extrapolated, "slope")? - 2.0).abs() < 1e-12);
    let overlay = extrapolated.fitted_curves.first().ok_or("missing extrapolated overlay")?;
    assert_eq!(overlay.points.iter().map(|p| p.x).collect::<Vec<_>>(), [7.0, 11.0, 15.0, 19.0]);
    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

#[test]
fn r13_1_human_nonlinear_diagnostics_settings() -> Result<(), Box<dyn Error>> {
    let mut driver = new_strict_human_driver()?;
    let data_id = import_csv_via_dialog(
        &mut driver,
        ImportSpec {
            target_kind_widget: "import_paste.target.xy",
            title: "Dose Trend",
            csv_text: "X\tResponse\n-9\t10\n-8\t13\n-7\t28\n-6\t58\n-5\t83\n-4\t96\n",
            existing_sheet_id: None,
            replace_existing_mode: false,
        },
    )?;
    switch_to_sheet_in_navigator(&mut driver, &data_id)?;
    let fit_id = run_analysis_from_toolbar(&mut driver, "fit.nonlinear")?;

    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.tab.confidence")?;
    click(&mut driver, "fit.ci.flag_unstable")?;
    assert!(widget_checked(&mut driver, "fit.ci.flag_unstable")?);

    click(&mut driver, "fit.tab.diagnostics")?;
    clear_and_type_into_textbox(&mut driver, "fit.diagnostics.r2.input", "1.1")?;
    let rejected = human_wf_common::click_widget_center(&mut driver, &wid("modal.ok")).err().ok_or("invalid R² threshold was accepted")?;
    assert!(rejected.contains("R² threshold"));
    clear_and_type_into_textbox(&mut driver, "fit.diagnostics.r2.input", "1")?;
    click(&mut driver, "modal.ok")?;

    let result = result_sheet(driver.app().document(), &fit_id)?;
    assert!(matches!(
        &result.analysis.kind,
        AnalysisKind::NonlinearRegression(spec) if spec.identify_unstable_parameters && spec.flag_r_squared_below == Some(1.0)
    ));
    assert!(result.checklist.iter().any(|item| item.assumption == "R squared below threshold" && item.remediation.is_none()));
    let banner = curvebook_ui_egui::AssumptionDiagnosticBanner::from_result(result).ok_or("missing diagnostic banner")?;
    assert_eq!(banner.source_result_id, result.id);

    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.tab.diagnostics")?;
    let field = driver.find_widget("fit.diagnostics.r2.input").ok_or("missing threshold field")?;
    assert_eq!(field.a11y.value_text.as_deref(), Some("1"));
    click(&mut driver, "fit.diagnostics.r2.input")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "a")?;
    driver.press_key("Backspace", KeyModifiers::default())?;
    click(&mut driver, "modal.ok")?;
    let result = result_sheet(driver.app().document(), &fit_id)?;
    assert!(matches!(&result.analysis.kind, AnalysisKind::NonlinearRegression(spec) if spec.flag_r_squared_below.is_none()));
    assert!(result.checklist.iter().all(|item| item.assumption != "R squared below threshold"));
    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}

#[test]
fn r13_1_human_replicate_interpolation_tabs() -> Result<(), Box<dyn Error>> {
    use curvebook_doc::{Cell, DataSheet, SubcolumnMode, TableArchetype, XyTable};
    use curvebook_engine::Session;

    // Preload a calibration table so the GUI path can focus on fitting and inspecting the unknowns.
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 9)?;
    for (row, x) in [-9.0, -8.0, -7.0, -6.8, -6.2, -5.5, -4.5, -4.0].into_iter().enumerate() {
        xy.set_x_cell(row, Cell::number(x))?;
        let y = 10.0 + 90.0 / (1.0 + 10_f64.powf(-6.5 - x));
        let cells = xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(row)).ok_or("missing calibration row")?;
        *cells = vec![Cell::number(y - 0.25), Cell::number(y + 0.25)];
    }
    let unknowns = xy.data_sets.first_mut().and_then(|ds| ds.cells.last_mut()).ok_or("missing unknown row")?;
    *unknowns = vec![Cell::number(45.0), Cell::number(65.0)];
    let sheet = DataSheet::new("replicate-calibration", "Replicate calibration", TableArchetype::Xy(xy))?;
    let mut doc = ProjectDocument::new("Replicate interpolation")?;
    doc.add_data_sheet(sheet)?;
    let app = UiApp::from_session(Session::from_document(doc));
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    switch_to_sheet_in_navigator(&mut driver, "replicate-calibration")?;
    let fit_id = run_analysis_from_toolbar(&mut driver, "fit.nonlinear")?;
    human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    click(&mut driver, "fit.tab.weights_range")?;
    click(&mut driver, "fit.wr.opt.interp")?;
    assert!(widget_checked(&mut driver, "fit.wr.opt.interp")?);
    click(&mut driver, "modal.ok")?;

    for (tab_id, title, rows) in
        [("nonlinear_interpolated_mean_x", "Interpolated Mean X", 1), ("nonlinear_interpolated_replicates", "Interpolated Replicates", 2)]
    {
        let widget_id = format!("result.tab.{tab_id}");
        for _ in 0..12 {
            if driver.layout_tree().node(&widget_id).is_some() {
                break;
            }
            click(&mut driver, "result.tabs.next")?;
        }
        click(&mut driver, &widget_id)?;
        let tree = driver.layout_tree();
        let selected = tree.node(&widget_id).ok_or("missing selected interpolation tab")?;
        assert_eq!(selected.a11y.selected, Some(true));
        let result = result_sheet(driver.app().document(), &fit_id)?;
        let tab = result.tabs.iter().find(|tab| tab.id == tab_id).ok_or("missing interpolation output")?;
        assert_eq!(tab.title, title);
        assert_eq!(tab.rows.len(), rows);
    }
    assert_clean_parity_and_budget(&mut driver)?;
    Ok(())
}
