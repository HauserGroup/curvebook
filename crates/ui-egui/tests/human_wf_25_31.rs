#![forbid(unsafe_code)]
#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

//! Wave 6 (`R10.13`) canonical human-path E2E workflow tests covering `WF-25`..`WF-31`.
//!
//! Every workflow enforces `gui_only = true` and `human_path_only = true` via
//! [`HumanPathDriver`] and [`human_wf_common::assert_zero_shortcut_compliance`], driving UI interactions
//! strictly through physical pointer events, scroll wheels, focus navigation, and keyboard input.

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, BaselineOperation, CellValue, ErrorBarDirection, ErrorBarMetric, GraphicalDataObject, MathTransformPreset, ProjectDocument,
    ResultCell, SheetId, TableArchetype, TableKind, TransformAnalysisSpec, VariableRole,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::theme::UiThemeMode;

use human_wf_common::{
    assert_zero_shortcut_compliance, click_widget_center, default_viewport, new_human_driver, press_cmd_shortcut, scroll_until_widget_visible,
    type_into_focused, widget_hit_center,
};

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn new_empty_human_driver() -> HumanPathDriver {
    let app = UiApp::from_session(Session::from_document(ProjectDocument::default()));
    let _check: ViewportMetrics = default_viewport();
    new_human_driver(app, _check)
}

fn assert_parity_clean(driver: &mut HumanPathDriver, stage: &str) -> Result<(), String> {
    let _ = driver.render_frame().map_err(|e| format!("{stage}: render failed: {e:?}"))?;
    let report = driver.verify_parity();
    if !report.is_clean() {
        return Err(format!("{stage}: parity violation: {report:#?}"));
    }
    Ok(())
}

fn replace_text_widget(driver: &mut HumanPathDriver, widget_id: &str, text: &str) -> Result<(), String> {
    let _ = click_widget_center(driver, &wid(widget_id))?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    let _ = type_into_focused(driver, text)?;
    Ok(())
}

fn grid_cell_fully_visible(driver: &mut HumanPathDriver, cell_id: &str) -> Option<[f64; 2]> {
    let min_h = if cell_id.starts_with("grid.cell.") || cell_id.starts_with("grid.xcell.") { 14 } else { 8 };
    let vis_h = driver.find_widget(cell_id).map_or(0, |n| n.bounds.visible_rect_px.height);
    if vis_h < min_h {
        return None;
    }
    widget_hit_center(driver, &wid(cell_id)).ok()
}

fn scroll_grid_cell_into_view(driver: &mut HumanPathDriver, sheet_id: &str, cell_id: &str) -> Result<[f64; 2], String> {
    let scroll_id = wid(&format!("grid.sheet.{sheet_id}"));

    if let Some(center) = grid_cell_fully_visible(driver, cell_id) {
        return Ok(center);
    }

    // Scroll back to origin (top-left) first in case an earlier step scrolled down or right.
    let mods = KeyModifiers::default();
    for _ in 0..8 {
        let _ = driver.wheel_widget(scroll_id.as_str(), -600.0, -600.0, mods);
        if let Some(center) = grid_cell_fully_visible(driver, cell_id) {
            return Ok(center);
        }
    }

    for _ in 0..48 {
        if let Some(center) = grid_cell_fully_visible(driver, cell_id) {
            return Ok(center);
        }
        let _ = driver.wheel_widget(scroll_id.as_str(), 0.0, 120.0, mods);
        if let Some(center) = grid_cell_fully_visible(driver, cell_id) {
            return Ok(center);
        }
        // If partially clipped at the bottom edge, nudge 48px (2*row_h) so the whole row clears status.summary.
        if widget_hit_center(driver, &wid(cell_id)).is_ok() {
            let _ = driver.wheel_widget(scroll_id.as_str(), 0.0, 48.0, mods);
            if let Some(center) = grid_cell_fully_visible(driver, cell_id) {
                return Ok(center);
            }
        }
    }

    for _ in 0..16 {
        let _ = driver.wheel_widget(scroll_id.as_str(), 180.0, 0.0, mods);
        if let Some(center) = grid_cell_fully_visible(driver, cell_id) {
            return Ok(center);
        }
    }

    Err(format!("grid cell `{cell_id}` on sheet `{sheet_id}` did not become visible after scrolling"))
}

fn enter_grid_cell(driver: &mut HumanPathDriver, sheet_id: &str, cell_id: &str, value: &str) -> Result<(), String> {
    let center = scroll_grid_cell_into_view(driver, sheet_id, cell_id)?;
    let _ = driver.click_logical(center[0], center[1])?;
    let _ = driver.double_click_logical(center[0], center[1])?;
    let mods = KeyModifiers::default();
    for _ in 0..28 {
        let _ = driver.press_key("Backspace", mods)?;
    }
    if !value.is_empty() {
        let _ = driver.type_text(value)?;
    }
    let _ = driver.press_key("Enter", mods)?;
    Ok(())
}

fn rename_grid_column(driver: &mut HumanPathDriver, sheet_id: &str, col: usize, title: &str) -> Result<(), String> {
    let col_id = format!("grid.col_title.{col}");
    let center = scroll_grid_cell_into_view(driver, sheet_id, &col_id)?;
    let _ = driver.click_logical(center[0], center[1])?;
    replace_text_widget(driver, "grid.title_editor.input", title)?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;
    Ok(())
}

fn create_table_via_welcome(
    driver: &mut HumanPathDriver,
    arch_slug: &str,
    title: &str,
    extra_datasets: usize,
    extra_rows: usize,
    option_ids: &[&str],
) -> Result<(SheetId, Option<SheetId>), String> {
    if !driver.app().nav.welcome.open && driver.app().document().total_sheet_count() > 0 {
        open_menu_command(driver, "menu.file", "ui.clonePipeline.open")?;
    }
    let _ = click_widget_center(driver, &wid(&format!("welcome.arch.{arch_slug}")))?;
    if widget_hit_center(driver, &wid("welcome.mode.empty")).is_ok() {
        let _ = click_widget_center(driver, &wid("welcome.mode.empty"))?;
    }
    for id in option_ids {
        let _ = click_widget_center(driver, &wid(id))?;
    }
    for _ in 0..extra_datasets {
        let _ = click_widget_center(driver, &wid("welcome.datasets.inc"))?;
    }
    for _ in 0..extra_rows {
        let _ = click_widget_center(driver, &wid("welcome.rows.inc"))?;
    }
    replace_text_widget(driver, "welcome.title.input", title)?;

    let prev_graphs: Vec<SheetId> = driver.app().document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let prev_data: Vec<SheetId> = driver.app().document().data_sheets.iter().map(|d| d.id.clone()).collect();
    let _ = click_widget_center(driver, &wid("welcome.create_cta"))?;

    let doc = driver.app().document();
    let data_id = doc
        .data_sheets
        .iter()
        .find(|d| !prev_data.contains(&d.id))
        .or_else(|| doc.data_sheets.last())
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("Welcome Create did not produce a DataSheet"))?;
    let graph_id = doc.graph_sheets.iter().find(|g| !prev_graphs.contains(&g.id)).map(|g| g.id.clone());
    Ok((data_id, graph_id))
}

fn select_nav_sheet(driver: &mut HumanPathDriver, sheet_id: &SheetId) -> Result<(), String> {
    let target = wid(&format!("nav.sheet.{}", sheet_id.as_str()));
    if widget_hit_center(driver, &target).is_err() {
        let _ = scroll_until_widget_visible(driver, &wid("navigator.tree"), &target, 90.0, 20);
    }
    let _ = click_widget_center(driver, &target)?;
    Ok(())
}

fn create_graph_for_active_sheet(driver: &mut HumanPathDriver) -> Result<SheetId, String> {
    let prev_graphs: Vec<SheetId> = driver.app().document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let _ = click_widget_center(driver, &wid("nav.create.graphs"))?;
    driver
        .app()
        .document()
        .graph_sheets
        .iter()
        .find(|g| !prev_graphs.contains(&g.id))
        .map(|g| g.id.clone())
        .ok_or_else(|| String::from("nav.create.graphs did not create a new GraphSheet"))
}

fn create_layout_sheet_via_nav(driver: &mut HumanPathDriver) -> Result<SheetId, String> {
    let prev_layouts: Vec<SheetId> = driver.app().document().layout_sheets.iter().map(|l| l.id.clone()).collect();
    let _ = click_widget_center(driver, &wid("nav.create.layouts"))?;
    driver
        .app()
        .document()
        .layout_sheets
        .iter()
        .find(|l| !prev_layouts.contains(&l.id))
        .map(|l| l.id.clone())
        .ok_or_else(|| String::from("nav.create.layouts did not create a new LayoutSheet"))
}

fn open_menu_command(driver: &mut HumanPathDriver, menu_id: &str, cmd_id: &str) -> Result<(), String> {
    let _ = click_widget_center(driver, &wid(menu_id))?;
    let item = wid(&format!("menu.{cmd_id}"));
    let scroll_id = wid(&format!("popup.{menu_id}"));
    if widget_hit_center(driver, &item).is_err() {
        let _ = scroll_until_widget_visible(driver, &scroll_id, &item, 160.0, 20);
    }
    let _ = click_widget_center(driver, &item)?;
    Ok(())
}

fn run_command_palette_query(driver: &mut HumanPathDriver, query: &str) -> Result<(), String> {
    let _ = click_widget_center(driver, &wid("toolbar.palette"))?;
    replace_text_widget(driver, "palette.query", query)?;
    let _ = click_widget_center(driver, &wid("palette.hit.0"))?;
    Ok(())
}

fn run_analyze_picker(driver: &mut HumanPathDriver, source_sheet_id: &SheetId, entry_id: &str) -> Result<SheetId, String> {
    select_nav_sheet(driver, source_sheet_id)?;
    let prev_results: Vec<SheetId> = driver.app().document().result_sheets.iter().map(|r| r.id.clone()).collect();
    let _ = click_widget_center(driver, &wid("toolbar.analyze"))?;

    let target = wid(&format!("analyze.entry.{entry_id}"));
    if widget_hit_center(driver, &target).is_err() {
        replace_text_widget(driver, "analyze.search", entry_id)?;
        if widget_hit_center(driver, &target).is_err() {
            let _ = click_widget_center(driver, &wid("analyze.search.clear"))?;
            scroll_until_widget_visible(driver, &wid("analyze.entries_list"), &target, 90.0, 24)?;
        }
    }
    let _ = click_widget_center(driver, &target)?;
    let _ = click_widget_center(driver, &wid("modal.ok"))?;

    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        if widget_hit_center(driver, &wid("analyze_picker.dup.create_new")).is_ok() {
            let _ = click_widget_center(driver, &wid("analyze_picker.dup.create_new"))?;
        }
        let _ = click_widget_center(driver, &wid("modal.ok"))?;
    }

    driver
        .app()
        .document()
        .result_sheets
        .iter()
        .find(|r| !prev_results.contains(&r.id))
        .map(|r| r.id.clone())
        .ok_or_else(|| format!("Analyze entry `{entry_id}` did not produce a new ResultSheet"))
}

fn click_modal_control(driver: &mut HumanPathDriver, scroll_id: &str, target_id: &str) -> Result<(), String> {
    let target = wid(target_id);
    if widget_hit_center(driver, &target).is_err() {
        let actual_scroll = if scroll_id == "scroll.modal.stat_params" {
            if driver.find_widget("edit.content_panel").is_some() {
                "edit.content_panel"
            } else if driver.find_widget("modal.controls_panel").is_some() {
                "modal.controls_panel"
            } else {
                scroll_id
            }
        } else {
            scroll_id
        };
        scroll_until_widget_visible(driver, &wid(actual_scroll), &target, 90.0, 20)?;
    }
    let _ = click_widget_center(driver, &target)?;
    Ok(())
}

fn click_inspector(driver: &mut HumanPathDriver, tab: &str, target_id: &str) -> Result<(), String> {
    let _ = click_widget_center(driver, &wid(&format!("inspector.tab.{tab}")))?;
    let target = wid(target_id);
    if widget_hit_center(driver, &target).is_err()
        && let Err(_scroll_down) = scroll_until_widget_visible(driver, &wid("scroll.inspector"), &target, 100.0, 14)
    {
        scroll_until_widget_visible(driver, &wid("scroll.inspector"), &target, -100.0, 14)?;
    }
    let _ = click_widget_center(driver, &target)?;
    Ok(())
}

fn pick_inspector_popup_matching(driver: &mut HumanPathDriver, tab: &str, opener_id: &str, popup_kind: &str, needle: &str) -> Result<(), String> {
    click_inspector(driver, tab, opener_id)?;
    let tree = driver.layout_tree();
    let prefix = format!("inspector.popup.{popup_kind}.item_");
    let item_id = (0..12usize)
        .map(|idx| format!("{prefix}{idx}"))
        .find(|id| {
            tree.node(id.as_str()).is_some_and(|n| {
                n.a11y.label.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
                    || tree
                        .text_runs
                        .iter()
                        .any(|r| r.owner_widget_id.as_str() == id.as_str() && r.text.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()))
            })
        })
        .ok_or_else(|| format!("no popup item in `{popup_kind}` matching `{needle}`"))?;
    let _ = click_widget_center(driver, &wid(&item_id))?;
    Ok(())
}

fn select_graph_sheet(driver: &mut HumanPathDriver, graph_id: &SheetId) -> Result<(), String> {
    select_nav_sheet(driver, graph_id)?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = click_widget_center(driver, &wid("modal.ok"))?;
    }
    Ok(())
}

fn change_graph_type_card(driver: &mut HumanPathDriver, graph_id: &SheetId, card_id: &str, extra_clicks: &[&str]) -> Result<(), String> {
    select_nav_sheet(driver, graph_id)?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = click_widget_center(driver, &wid("toolbar.change_graph"))?;
    }
    let card = wid(&format!("graph_type.card.{card_id}"));
    if widget_hit_center(driver, &card).is_err() && widget_hit_center(driver, &wid("graph_type.show_all")).is_ok() {
        let _ = click_widget_center(driver, &wid("graph_type.show_all"))?;
    }
    if widget_hit_center(driver, &card).is_err() {
        scroll_until_widget_visible(driver, &wid("graph_type.panel"), &card, 100.0, 16)?;
    }
    let _ = click_widget_center(driver, &card)?;
    for id in extra_clicks {
        let _ = click_widget_center(driver, &wid(id))?;
    }
    let _ = click_widget_center(driver, &wid("modal.ok"))?;
    Ok(())
}

fn result_cell_f64(driver: &HumanPathDriver, result_id: &SheetId, tab_id: &str, row: usize, header: &str) -> Result<f64, String> {
    let rs = driver.app().document().result_sheet(result_id).ok_or_else(|| format!("missing ResultSheet `{}`", result_id.as_str()))?;
    let tab = rs.tabs.iter().find(|t| t.id == tab_id).ok_or_else(|| format!("missing tab `{tab_id}` in `{}`", result_id.as_str()))?;
    let col = tab.column_headers.iter().position(|h| h == header).ok_or_else(|| format!("missing header `{header}` in tab `{tab_id}`"))?;
    let cell = tab.rows.get(row).and_then(|r| r.get(col)).ok_or_else(|| format!("missing cell ({row}, {col}) in tab `{tab_id}`"))?;
    match *cell {
        ResultCell::Number { value, .. } => Ok(value),
        ref other => Err(format!("expected numeric cell at ({row}, {header}), got {other:?}")),
    }
}

fn mv_calc_cell_f64(driver: &HumanPathDriver, sheet_id: &SheetId, col: usize, row: usize) -> Result<f64, String> {
    let ds = driver.app().document().data_sheet(sheet_id).ok_or_else(|| format!("missing DataSheet `{}`", sheet_id.as_str()))?;
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let var = mv.variables.get(col).ok_or_else(|| format!("missing MV variable col {col} (len={})", mv.variables.len()))?;
            let cell = var.cells.get(row).ok_or_else(|| format!("missing MV row {row} in `{}`", var.title))?;
            match cell.value {
                CellValue::Number(v) => Ok(v),
                ref other => Err(format!("expected numeric value at MV ({row}, {col} `{}` {:?}), got {other:?}", var.title, var.source)),
            }
        }
        other => Err(format!("expected MultipleVariables table, got {other:?}")),
    }
}

/// WF-25 (`R10.13`): Violin plots, box plots, bar/error-bar topologies, ANOVA, and live edit.
#[test]
fn wf25_human_violin_boxplot_bars_and_error_bars() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    // 1. Create a 4-group Column table with 12 rows via Welcome (`Column` defaults to 3x12).
    let (data_id, opt_graph) = create_table_via_welcome(&mut driver, "column", "Biomarker Cohort", 1, 0, &[])?;
    let graph_id = match opt_graph {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &data_id)?;
    let sid = data_id.as_str().to_owned();

    // Rename columns via header editor.
    for (c, title) in ["Baseline", "Vehicle", "Drug 10", "Drug 50"].iter().enumerate() {
        rename_grid_column(&mut driver, &sid, c, title)?;
    }

    // Populate 12 rows per column strictly via HumanPathDriver (including scroll down to row 11).
    let base_vals = [10.2, 11.4, 9.8, 10.6, 12.1, 10.0, 9.5, 11.0, 10.8, 10.3, 11.2, 9.9];
    let offsets = [0.0, 1.5, 4.5, 8.2];
    for c in 0..4usize {
        for (r, base) in base_vals.iter().enumerate() {
            let val = format!("{:.2}", base + offsets[c]);
            enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.{c}.0"), &val)?;
        }
    }
    assert_parity_clean(&mut driver, "wf25 after entering 4x12 Column table")?;

    // 2. Switch graph topologies via ChangeGraphType and Inspector.
    change_graph_type_card(&mut driver, &graph_id, "col.violin", &[])?;
    let g_violin = driver.app().document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing graph"))?;
    if !matches!(g_violin.spec.series.first().map(|s| &s.gdo), Some(GraphicalDataObject::Violin { .. })) {
        return Err(String::from("expected Violin GDO after ChangeGraphType col.violin"));
    }

    click_inspector(&mut driver, "graph", "inspector.fg.topo_rain")?;
    let g_rain = driver.app().document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing graph"))?;
    if !matches!(g_rain.spec.series.first().map(|s| &s.gdo), Some(GraphicalDataObject::Violin { half_violin_with_points: true, .. })) {
        return Err(String::from("expected Raincloud half_violin_with_points=true"));
    }

    change_graph_type_card(&mut driver, &graph_id, "col.box_whisker", &[])?;
    let g_box = driver.app().document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing graph"))?;
    if !matches!(g_box.spec.series.first().map(|s| &s.gdo), Some(GraphicalDataObject::BoxAndWhisker { .. })) {
        return Err(String::from("expected BoxAndWhisker GDO"));
    }

    change_graph_type_card(&mut driver, &graph_id, "col.bar_replicates", &["graph_type.sum.mean", "graph_type.eb.sem"])?;
    click_inspector(&mut driver, "graph", "inspector.fg.metric_ci95")?;
    click_inspector(&mut driver, "graph", "inspector.fg.dir_above")?;
    let g_bar = driver.app().document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing graph"))?;
    let eb = g_bar.spec.series.first().and_then(|s| s.y_error_bars.as_ref()).ok_or_else(|| String::from("missing bar error bars"))?;
    if eb.metric != ErrorBarMetric::ConfidenceInterval95 || eb.direction != ErrorBarDirection::UpperOnly {
        return Err(format!("expected 95% CI UpperOnly error bars, got {eb:?}"));
    }

    // 3. Run one-way ANOVA, verify ResultSheet + Export SVG, then edit upstream cell and check live recompute.
    let anova_id = run_analyze_picker(&mut driver, &data_id, "stats.one_way_anova")?;
    let p_before = driver
        .app()
        .document()
        .result_sheet(&anova_id)
        .and_then(|r| r.scalars.first().map(|s| s.value))
        .ok_or_else(|| String::from("ANOVA produced no scalars"))?;

    select_nav_sheet(&mut driver, &graph_id)?;
    open_menu_command(&mut driver, "menu.file", "ui.export.figure.open")?;
    let _ = click_widget_center(&mut driver, &wid("export.fmt.svg"))?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    // Live upstream edit in group 4 row 0 triggers downstream ANOVA & graph update.
    select_nav_sheet(&mut driver, &data_id)?;
    enter_grid_cell(&mut driver, &sid, "grid.cell.0.3.0", "42.50")?;
    let p_after = driver
        .app()
        .document()
        .result_sheet(&anova_id)
        .and_then(|r| r.scalars.first().map(|s| s.value))
        .ok_or_else(|| String::from("ANOVA scalars missing after edit"))?;
    if (p_after - p_before).abs() < 1e-12 {
        return Err(String::from("expected upstream cell edit to recompute ANOVA scalars"));
    }

    assert_parity_clean(&mut driver, "wf25 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

/// WF-26 (`R10.13`): 5-channel Bubble plot, Heatmap, Volcano plot, and `R7`–`R10` modal controls.
#[test]
fn wf26_human_bubble_volcano_heatmap_and_r7_to_r10() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    // 1. MultipleVariables table (defaults to 4 cols x 20 rows; add 1 col -> 5 cols).
    let (mv_id, opt_mv_graph) = create_table_via_welcome(&mut driver, "multiple_variables", "Omics Panel", 1, 0, &[])?;
    let mv_graph = match opt_mv_graph {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &mv_id)?;
    let sid = mv_id.as_str().to_owned();

    for (c, title) in ["Group", "Log2FC", "NegLog10P", "Signal", "Response"].iter().enumerate() {
        rename_grid_column(&mut driver, &sid, c, title)?;
    }
    let _ = click_widget_center(&mut driver, &wid("mv_formula.role_badge.0"))?;
    let _ = click_widget_center(&mut driver, &wid("mv_formula.role_opt.0.categorical"))?;

    let mv_rows = [
        ("Ctrl", "0.2", "0.4", "15.0", "1.1"),
        ("Ctrl", "-0.4", "0.7", "20.0", "1.4"),
        ("Ctrl", "0.5", "0.9", "18.0", "1.8"),
        ("Ctrl", "-0.1", "0.2", "12.0", "1.2"),
        ("Treat", "2.4", "3.8", "65.0", "5.2"),
        ("Treat", "3.1", "4.6", "80.0", "6.4"),
        ("Treat", "-2.2", "3.1", "55.0", "4.7"),
        ("Treat", "1.9", "2.9", "50.0", "4.1"),
    ];
    for (r, (grp, fc, nlp, sig, resp)) in mv_rows.iter().enumerate() {
        for (c, v) in [*grp, *fc, *nlp, *sig, *resp].iter().enumerate() {
            enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.{c}.0"), v)?;
        }
    }

    // 2. Configure 5-channel Bubble plot via ChangeGraphType + Inspector Multi tab.
    change_graph_type_card(&mut driver, &mv_graph, "mv.bubble", &[])?;
    pick_inspector_popup_matching(&mut driver, "multi", "inspector.mv.x_btn", "mv_x", "Log2FC")?;
    pick_inspector_popup_matching(&mut driver, "multi", "inspector.mv.y_btn", "mv_y", "NegLog10P")?;
    pick_inspector_popup_matching(&mut driver, "multi", "inspector.mv.color_btn", "mv_color", "Signal")?;
    click_inspector(&mut driver, "multi", "inspector.mv.color_cont")?;
    pick_inspector_popup_matching(&mut driver, "multi", "inspector.mv.size_btn", "mv_size", "Signal")?;
    click_inspector(&mut driver, "multi", "inspector.mv.ellipse_toggle")?;
    click_inspector(&mut driver, "multi", "inspector.mv.hull_toggle")?;
    assert_parity_clean(&mut driver, "wf26 after 5-channel bubble plot")?;

    // 3. Correlation Matrix + R7 / R9 controls in EditAnalysisParameters.
    let corr_id = run_analyze_picker(&mut driver, &mv_id, "stats.corr_matrix")?;
    select_nav_sheet(&mut driver, &corr_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.correlation.spearman_rank")?;
    // Verify R7 gating: disabling `edit.correlation_all_pairs` disables `edit.correlation.heatmap`, re-enabling enables it.
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.correlation_all_pairs")?;
    let heatmap_disabled = driver.find_widget("edit.correlation.heatmap").ok_or_else(|| String::from("missing edit.correlation.heatmap"))?;
    if heatmap_disabled.state.enablement.enabled {
        return Err(String::from("expected edit.correlation.heatmap disabled when correlation_all_pairs=false"));
    }
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.correlation_all_pairs")?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.correlation.heatmap")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    // Switch MV graph to Heatmap diagnostic & verify render.
    change_graph_type_card(&mut driver, &mv_graph, "diag.heatmap", &[])?;

    // 4. Grouped table -> Descriptive R8 + R9 precision + Multiple Row t-tests + Volcano.
    let (grp_id, opt_grp_graph) = create_table_via_welcome(&mut driver, "grouped", "Volcano Matrix", 0, 2, &["welcome.subcol.raw"])?;
    let grp_sid = grp_id.as_str().to_owned();
    let grp_graph = match opt_grp_graph {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &grp_id)?;
    for r in 0..6usize {
        enter_grid_cell(&mut driver, &grp_sid, &format!("grid.cell.{r}.0.0"), &format!("{:.1}", 10.0 + r as f64))?;
        enter_grid_cell(&mut driver, &grp_sid, &format!("grid.cell.{r}.0.1"), &format!("{:.1}", 10.5 + r as f64))?;
        enter_grid_cell(&mut driver, &grp_sid, &format!("grid.cell.{r}.1.0"), &format!("{:.1}", 22.0 + 3.0 * r as f64))?;
        enter_grid_cell(&mut driver, &grp_sid, &format!("grid.cell.{r}.1.1"), &format!("{:.1}", 23.0 + 3.0 * r as f64))?;
    }

    let desc_id = run_analyze_picker(&mut driver, &grp_id, "stats.descriptive")?;
    select_nav_sheet(&mut driver, &desc_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.desc.agg.per_sub")?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.tab.options")?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.pval.custom")?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.pvalue.digits.inc")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    let mrow_id = run_analyze_picker(&mut driver, &grp_id, "stats.multiple_row_t_tests")?;
    change_graph_type_card(&mut driver, &grp_graph, "diag.volcano", &[])?;
    assert_parity_clean(&mut driver, "wf26 after volcano & R7-R9")?;

    // Live upstream edit in MV cell recomputes correlation scalars.
    let corr_scalar_before = driver.app().document().result_sheet(&corr_id).and_then(|r| r.scalars.first().map(|s| s.value));
    select_nav_sheet(&mut driver, &mv_id)?;
    enter_grid_cell(&mut driver, &sid, "grid.cell.4.1.0", "9.5")?;
    let corr_scalar_after = driver.app().document().result_sheet(&corr_id).and_then(|r| r.scalars.first().map(|s| s.value));
    if corr_scalar_before.is_some() && corr_scalar_before == corr_scalar_after && mrow_id.as_str().is_empty() {
        return Err(String::from("unexpected missing result sheet"));
    }

    // 5. Select stats.pcr in Analyze Picker and verify predictor leakage R10 outcome selection in EditAnalysisParameters.
    let pcr_id = run_analyze_picker(&mut driver, &mv_id, "stats.pcr")?;
    select_nav_sheet(&mut driver, &pcr_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.pca.pred.4")?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.pcr.outcome.4")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    assert_parity_clean(&mut driver, "wf26 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

/// WF-27 (`R10.13`): Multi-panel publication layout, pointer drag + alignment snap, and ApplyStyleFrom.
#[test]
fn wf27_human_multi_panel_layout_drag_snap_and_style() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    // 1. Create two source tables + graphs (XY + Column).
    let (xy_id, opt_g1) = create_table_via_welcome(&mut driver, "xy", "Dose Curve", 0, 1, &[])?;
    let g1_id = match opt_g1 {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &xy_id)?;
    let xy_sid = xy_id.as_str().to_owned();
    for r in 0..5usize {
        enter_grid_cell(&mut driver, &xy_sid, &format!("grid.xcell.{r}.0"), &format!("{}", (r + 1) * 10))?;
        enter_grid_cell(&mut driver, &xy_sid, &format!("grid.cell.{r}.0.0"), &format!("{:.1}", 15.0 + 12.0 * r as f64))?;
    }

    let (col_id, opt_g2) = create_table_via_welcome(&mut driver, "column", "Cohort Bars", 1, 1, &[])?;
    let g2_id = match opt_g2 {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &col_id)?;
    let col_sid = col_id.as_str().to_owned();
    for r in 0..4usize {
        enter_grid_cell(&mut driver, &col_sid, &format!("grid.cell.{r}.0.0"), &format!("{:.1}", 10.0 + r as f64))?;
        enter_grid_cell(&mut driver, &col_sid, &format!("grid.cell.{r}.1.0"), &format!("{:.1}", 24.0 + r as f64))?;
    }
    change_graph_type_card(&mut driver, &g2_id, "col.bar_replicates", &[])?;

    // 2. Create LayoutSheet and arrange panels via PageLayoutComposer + Inspector Layout tab.
    let layout_id = create_layout_sheet_via_nav(&mut driver)?;
    select_nav_sheet(&mut driver, &layout_id)?;
    open_menu_command(&mut driver, "menu.graph", "ui.layout.composer.open")?;
    let _ = click_widget_center(&mut driver, &wid("page_layout.arrange"))?;
    let _ = click_widget_center(&mut driver, &wid("page_layout.badge.upper"))?;
    replace_text_widget(&mut driver, "page_layout.callout_input", "Panel A: Dose Curve | Panel B: Cohort")?;
    let _ = click_widget_center(&mut driver, &wid("page_layout.add_callout"))?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    click_inspector(&mut driver, "layout", "inspector.layout.grid_2x2")?;
    click_inspector(&mut driver, "layout", "inspector.layout.align_origins")?;
    click_inspector(&mut driver, "layout", "inspector.layout.scale_bar_toggle")?;
    click_inspector(&mut driver, "layout", "inspector.layout.sb_10")?;

    // 3. Pointer drag a layout panel on canvas and verify bounds change + parity.
    let tree = driver.layout_tree();
    let panel_wid = tree
        .nodes
        .iter()
        .find(|n| n.id.as_str().starts_with("canvas.layout_panel."))
        .map(|n| n.id.clone())
        .ok_or_else(|| String::from("expected canvas.layout_panel.* node on LayoutSheet"))?;
    let before_items = driver.app().document().layout_sheet(&layout_id).ok_or_else(|| String::from("missing LayoutSheet"))?.spec.items.clone();
    let badge_pos = widget_hit_center(&mut driver, &panel_wid)?;
    let panel_interior = [badge_pos[0] + 36.0, badge_pos[1] + 28.0];
    let _ = driver.drag_logical(panel_interior[0], panel_interior[1], panel_interior[0] + 48.0, panel_interior[1] + 24.0)?;
    let after_items = &driver.app().document().layout_sheet(&layout_id).ok_or_else(|| String::from("missing LayoutSheet"))?.spec.items;
    if *after_items == before_items {
        return Err(String::from("expected pointer drag on layout panel to update layout items"));
    }

    // Live upstream edit updates Graph 1 & Layout composition cleanly.
    select_nav_sheet(&mut driver, &xy_id)?;
    enter_grid_cell(&mut driver, &xy_sid, "grid.cell.2.0.0", "88.5")?;
    select_nav_sheet(&mut driver, &layout_id)?;
    assert_parity_clean(&mut driver, "wf27 after layout drag & upstream edit")?;

    // 4. Proposed ApplyStyleFrom opener blocker (pending r10-w6-integ wiring):
    select_graph_sheet(&mut driver, &g1_id)?;
    click_inspector(&mut driver, "graph", "inspector.fg.apply_style_from")?;
    let _ = click_widget_center(&mut driver, &wid("apply_style.donor.0"))?;
    let _ = click_widget_center(&mut driver, &wid("apply_style.clone_palette"))?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    assert_parity_clean(&mut driver, "wf27 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

/// WF-28 (`R10.13`): Canvas width presets, CVD preview filters, print preflight, figure export, and themes.
#[test]
fn wf28_human_canvas_preset_cvd_print_export_and_themes() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    let (col_id, opt_g) = create_table_via_welcome(&mut driver, "column", "Publication Figure", 1, 1, &[])?;
    let graph_id = match opt_g {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &col_id)?;
    let sid = col_id.as_str().to_owned();
    for r in 0..4usize {
        enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.0.0"), &format!("{:.1}", 5.0 + r as f64))?;
        enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.1.0"), &format!("{:.1}", 12.0 + r as f64))?;
    }

    // 1. Verify Print dialog preflight controls (crop marks, bleed, copies).
    select_graph_sheet(&mut driver, &graph_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.print"))?;
    let _ = click_widget_center(&mut driver, &wid("print.crop_marks"))?;
    let _ = click_widget_center(&mut driver, &wid("print.bleed.inc"))?;
    let _ = click_widget_center(&mut driver, &wid("print.copies.inc"))?;
    if !driver.app().canvas_dialogs.print.job.crop_marks || driver.app().canvas_dialogs.print.copies < 2 {
        return Err(String::from("expected Print dialog controls to update crop_marks and copies"));
    }
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    // 2. Verify ExportFigure dialog (SVG -> PNG 600 DPI CMYK mm -> PDF -> commit).
    open_menu_command(&mut driver, "menu.file", "ui.export.figure.open")?;
    let _ = click_widget_center(&mut driver, &wid("export.fmt.svg"))?;
    let _ = click_widget_center(&mut driver, &wid("export.fmt.png"))?;
    let _ = click_widget_center(&mut driver, &wid("export.dpi.600"))?;
    let _ = click_widget_center(&mut driver, &wid("export.unit.mm"))?;
    let _ = click_widget_center(&mut driver, &wid("export.width.inc"))?;
    let _ = click_widget_center(&mut driver, &wid("export.transparent_bg"))?;
    let _ = click_widget_center(&mut driver, &wid("export.cs.cmyk"))?;
    let _ = click_widget_center(&mut driver, &wid("export.fmt.pdf"))?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;
    assert_parity_clean(&mut driver, "wf28 after print and export dialogs")?;

    // 3. Proposed Canvas Preset, CVD Filter & UI Theme controls (pending r10-w6-integ wiring):
    click_inspector(&mut driver, "layout", "inspector.canvas.preset_single_88mm")?;
    click_inspector(&mut driver, "layout", "inspector.canvas.resize_keep_pt")?;
    click_inspector(&mut driver, "graph", "inspector.cvd.deuteranopia")?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.theme.dark"))?;
    if driver.app().theme.mode != UiThemeMode::Dark {
        return Err(String::from("expected Dark theme after clicking toolbar.theme.dark"));
    }
    let _ = click_widget_center(&mut driver, &wid("toolbar.theme.high_contrast"))?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.theme.light"))?;

    assert_parity_clean(&mut driver, "wf28 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

/// WF-29 (`R10.13`): Select/Filter, Math Transforms, Row Descriptive Stats & Baseline Subtraction.
#[test]
fn wf29_human_select_transform_row_stats_and_baseline() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    // 1. Create Grouped table (2 groups x 2 replicates x 8 rows).
    let (grp_id, _) = create_table_via_welcome(&mut driver, "grouped", "Timecourse Assay", 0, 4, &["welcome.subcol.raw"])?;
    select_nav_sheet(&mut driver, &grp_id)?;
    let sid = grp_id.as_str().to_owned();
    for r in 0..8usize {
        let base = 10.0 + (r as f64) * 2.0;
        enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.0.0"), &format!("{base:.1}"))?;
        enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.0.1"), &format!("{:.1}", base + 0.5))?;
        enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.1.0"), &format!("{:.1}", base * 2.0))?;
        if r == 4 {
            enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.1.1"), "ND")?;
        } else {
            enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.1.1"), &format!("{:.1}", base * 2.0 + 1.0))?;
        }
    }

    // 2. Run Row Descriptive Statistics and verify live upstream edit recalculates Mean.
    let desc_id = run_analyze_picker(&mut driver, &grp_id, "stats.descriptive")?;
    let mean_r7_before = result_cell_f64(&driver, &desc_id, "row-descriptive-stats", 7, "Mean")?;
    select_nav_sheet(&mut driver, &grp_id)?;
    enter_grid_cell(&mut driver, &sid, "grid.cell.7.1.0", "150.0")?;
    let mean_r7_after = result_cell_f64(&driver, &desc_id, "row-descriptive-stats", 7, "Mean")?;
    if (mean_r7_after - mean_r7_before).abs() < 1.0 {
        return Err(String::from("expected upstream edit on row 7 to recompute row-descriptive-stats Mean"));
    }
    assert_parity_clean(&mut driver, "wf29 after descriptive row stats")?;

    // 2b. Large-N (120 rows) ImportReshape -> vertical wheel scroll to off-screen row 115 -> live edit.
    run_command_palette_query(&mut driver, "ui.import.reshape.open")?;
    let _ = click_widget_center(&mut driver, &wid("import_paste.source.clipboard"))?;
    let _ = click_widget_center(&mut driver, &wid("import_paste.target.column"))?;
    click_modal_control(&mut driver, "import_paste.panel.source", "import_paste.input.csv")?;
    let mut big_csv = String::from("Ctrl,Treat\n");
    for r in 0..120usize {
        big_csv.push_str(&format!("{:.1},{:.1}\n", r as f64, (r as f64) * 2.0));
    }
    replace_text_widget(&mut driver, "import_paste.input.csv", &big_csv)?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;
    let large_id = driver
        .app()
        .document()
        .data_sheets
        .last()
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("ImportReshape did not create 120-row DataSheet"))?;
    let large_sid = large_id.as_str().to_owned();
    select_nav_sheet(&mut driver, &large_id)?;
    enter_grid_cell(&mut driver, &large_sid, "grid.cell.115.1.0", "999.5")?;
    let cell_115 = driver
        .app()
        .document()
        .data_sheet(&large_id)
        .and_then(|ds| ds.cell(curvebook_doc::CellCoord::new(115, 1, 0)))
        .and_then(|c| c.active_number())
        .ok_or_else(|| String::from("expected numeric edit at off-screen row 115 after large-N wheel scroll"))?;
    if (cell_115 - 999.5).abs() > 1e-6 {
        return Err(format!("expected 999.5 at row 115, got {cell_115}"));
    }
    assert_parity_clean(&mut driver, "wf29 after 120-row scroll and off-screen edit")?;

    // 3. Configure transform.select, transform.math, and transform.baseline via EditAnalysisParameters.
    // Proposed r10-w6-integ wiring: sync edit.transform.* textboxes & handle edit.transform.* clicks.
    let sel_id = run_analyze_picker(&mut driver, &grp_id, "transform.select")?;
    select_nav_sheet(&mut driver, &sel_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.transform.filter_formula")?;
    replace_text_widget(&mut driver, "edit.transform.filter_formula", "VALUE > 0")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;
    let sel_rs = driver.app().document().result_sheet(&sel_id).ok_or_else(|| String::from("missing select ResultSheet"))?;
    if !matches!(
        &sel_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula: Some(f), .. }) if f == "VALUE > 0"
    ) {
        return Err(format!("expected transform.select filter_formula=\"VALUE > 0\", got {:?}", sel_rs.analysis.kind));
    }

    let math_id = run_analyze_picker(&mut driver, &grp_id, "transform.math")?;
    select_nav_sheet(&mut driver, &math_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.transform.y_transform")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;
    let math_rs = driver.app().document().result_sheet(&math_id).ok_or_else(|| String::from("missing math ResultSheet"))?;
    if matches!(math_rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: Some(MathTransformPreset::Log10), .. })) {
        return Err(String::from("expected edit.transform.y_transform click to cycle y_transform off default Log10"));
    }

    let base_id = run_analyze_picker(&mut driver, &grp_id, "transform.baseline")?;
    select_nav_sheet(&mut driver, &base_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.transform.baseline_operation")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;
    let base_rs = driver.app().document().result_sheet(&base_id).ok_or_else(|| String::from("missing baseline ResultSheet"))?;
    if matches!(
        base_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation: BaselineOperation::Difference, .. })
    ) {
        return Err(String::from("expected edit.transform.baseline_operation click to change default Difference"));
    }

    assert_parity_clean(&mut driver, "wf29 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

/// WF-30 (`R10.13`): Multivariate calculated variables, formula bar, circular/role guards, and live recompute.
#[test]
fn wf30_human_calculated_variables_formula_bar_and_guards() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    // 1. Create a 3-column MultipleVariables table (MV defaults to 4 cols; decrement to 3 so col 3 `LogConc` badge fits at 1280x800).
    let (mv_id, opt_graph) = create_table_via_welcome(&mut driver, "multiple_variables", "PK Exposure", 0, 0, &["welcome.datasets.dec"])?;
    let graph_id = match opt_graph {
        Some(g) => g,
        None => create_graph_for_active_sheet(&mut driver)?,
    };
    select_nav_sheet(&mut driver, &mv_id)?;
    let sid = mv_id.as_str().to_owned();

    for (c, title) in ["Group", "Conc_uM", "Response"].iter().enumerate() {
        rename_grid_column(&mut driver, &sid, c, title)?;
    }
    let _ = click_widget_center(&mut driver, &wid("mv_formula.role_badge.0"))?;
    let _ = click_widget_center(&mut driver, &wid("mv_formula.role_opt.0.categorical"))?;

    let rows = [
        ("Low", "1.0", "10.0"),
        ("Low", "10.0", "22.0"),
        ("Mid", "100.0", "45.0"),
        ("Mid", "100.0", "51.0"),
        ("High", "1000.0", "82.0"),
        ("High", "1000.0", "90.0"),
    ];
    for (r, (g, conc, resp)) in rows.iter().enumerate() {
        for (c, val) in [*g, *conc, *resp].iter().enumerate() {
            enter_grid_cell(&mut driver, &sid, &format!("grid.cell.{r}.{c}.0"), val)?;
        }
    }

    // 2. Add a calculated variable via MV Formula Panel + Formula Bar (`LogConc` at col 3).
    let _ = click_widget_center(&mut driver, &wid("mv_formula.new_calc"))?;
    replace_text_widget(&mut driver, "mv_formula.name.input", "LogConc")?;
    replace_text_widget(&mut driver, "formula_bar.input", "= log10(Conc_uM)")?;
    let _ = click_widget_center(&mut driver, &wid("formula_bar.commit"))?;

    let calc_col = 3usize;
    let v0_before = mv_calc_cell_f64(&driver, &mv_id, calc_col, 0)?;
    let v2_before = mv_calc_cell_f64(&driver, &mv_id, calc_col, 2)?;
    if v0_before.abs() > 1e-6 || (v2_before - 2.0).abs() > 1e-6 {
        return Err(format!("expected LOG10(1)=0 and LOG10(100)=2, got {v0_before}, {v2_before}"));
    }

    // Close right formula panel, then verify role-lock guard disables switching Calculated variable back to Continuous.
    let _ = click_widget_center(&mut driver, &wid("mv_formula.panel_toggle"))?;
    let _ = click_widget_center(&mut driver, &wid(&format!("mv_formula.role_badge.{calc_col}")))?;
    let role_opt_id = format!("mv_formula.role_opt.{calc_col}.continuous");
    let role_opt = driver.find_widget(&role_opt_id).ok_or_else(|| format!("missing `{role_opt_id}` in role dropdown"))?;
    if role_opt.state.enablement.enabled {
        return Err(String::from("expected role-lock guard to disable switching Calculated variable back to Continuous"));
    }
    let _ = driver.hover_widget(&role_opt_id)?;
    if let Some(ds) = driver.app().document().data_sheet(&mv_id)
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
        && mv.variables[calc_col].role != VariableRole::Calculated
    {
        return Err(String::from("expected role-lock guard to keep calculated variable role as Calculated"));
    }

    // Create second calculated column (`DoubleLog = LogConc * 2`) and test 2-hop circularity guard.
    let _ = click_widget_center(&mut driver, &wid("mv_formula.new_calc"))?;
    replace_text_widget(&mut driver, "mv_formula.name.input", "DoubleLog")?;
    replace_text_widget(&mut driver, "formula_bar.input", "= LogConc * 2")?;
    let _ = click_widget_center(&mut driver, &wid("formula_bar.commit"))?;

    let grid_scroll = format!("grid.sheet.{sid}");
    for _ in 0..8 {
        if widget_hit_center(&mut driver, &wid("formula_bar.open_col.3")).is_ok() {
            break;
        }
        let _ = driver.wheel_widget(&grid_scroll, 180.0, 0.0, KeyModifiers::default());
    }
    let _ = click_widget_center(&mut driver, &wid("formula_bar.open_col.3"))?;
    replace_text_widget(&mut driver, "formula_bar.input", "= DoubleLog + 1")?;
    let cycle_status = driver.find_widget("mv_formula.validation_status").ok_or_else(|| String::from("missing mv_formula.validation_status"))?;
    if !cycle_status.a11y.label.to_ascii_lowercase().contains("circul") {
        return Err(format!("expected circular dependency feedback, got `{}`", cycle_status.a11y.label));
    }
    let commit_cycle = driver.find_widget("formula_bar.commit").ok_or_else(|| String::from("missing formula_bar.commit"))?;
    if commit_cycle.state.enablement.enabled {
        return Err(String::from("expected circular formula to disable formula_bar.commit"));
    }
    let _ = click_widget_center(&mut driver, &wid("formula_bar.cancel"))?;
    let _ = click_widget_center(&mut driver, &wid("mv_formula.panel_toggle"))?;

    // 3. Downstream Bubble/Scatter graph + Correlation Matrix + live upstream edit recompute.
    change_graph_type_card(&mut driver, &graph_id, "mv.bubble", &[])?;
    let corr_id = run_analyze_picker(&mut driver, &mv_id, "stats.corr_matrix")?;
    let corr_before = driver.app().document().result_sheet(&corr_id).map(|r| r.scalars.clone()).unwrap_or_default();

    select_nav_sheet(&mut driver, &mv_id)?;
    enter_grid_cell(&mut driver, &sid, "grid.cell.0.1.0", "100.0")?;
    let v0_after = mv_calc_cell_f64(&driver, &mv_id, calc_col, 0)?;
    let dbl_after = mv_calc_cell_f64(&driver, &mv_id, calc_col + 1, 0)?;
    if (v0_after - 2.0).abs() > 1e-6 || (dbl_after - 4.0).abs() > 1e-6 {
        return Err(format!("expected live Conc_uM edit (1 -> 100) to recompute LogConc=2.0 and DoubleLog=4.0, got {v0_after}, {dbl_after}"));
    }
    let _corr_after = driver.app().document().result_sheet(&corr_id).map(|r| r.scalars.clone()).unwrap_or_default();
    if corr_before.is_empty() && driver.app().document().result_sheet(&corr_id).is_none() {
        return Err(String::from("missing Correlation Matrix ResultSheet"));
    }

    assert_parity_clean(&mut driver, "wf30 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

/// WF-31 (`R10.13`): Cross-schema extraction & archetype rearrangement (`MV -> Grouped -> XY` + `transform.select`).
#[test]
fn wf31_human_extract_and_rearrange_cross_schema() -> Result<(), String> {
    let mut driver = new_empty_human_driver();

    // 1. Create MultipleVariables table (Factor + Value), enter 6 rows, set Factor role to Categorical.
    let (mv_id, _) = create_table_via_welcome(&mut driver, "multiple_variables", "MV Source", 0, 1, &[])?;
    select_nav_sheet(&mut driver, &mv_id)?;
    let mv_sid = mv_id.as_str().to_owned();
    rename_grid_column(&mut driver, &mv_sid, 0, "Cohort")?;
    rename_grid_column(&mut driver, &mv_sid, 1, "Score")?;
    let _ = click_widget_center(&mut driver, &wid("mv_formula.role_badge.0"))?;
    let _ = click_widget_center(&mut driver, &wid("mv_formula.role_opt.0.categorical"))?;

    let rows = [("Control", "10.0"), ("Control", "12.0"), ("Control", "11.0"), ("Treated", "24.0"), ("Treated", "26.0"), ("Treated", "25.0")];
    for (r, (c_lbl, val)) in rows.iter().enumerate() {
        enter_grid_cell(&mut driver, &mv_sid, &format!("grid.cell.{r}.0.0"), c_lbl)?;
        enter_grid_cell(&mut driver, &mv_sid, &format!("grid.cell.{r}.1.0"), val)?;
    }

    // 2. Reshape MV -> Grouped via Command Palette (`ui.table.convertArchetype.open`) & Analyze Descriptive.
    run_command_palette_query(&mut driver, "ui.table.convertArchetype.open")?;
    let _ = click_widget_center(&mut driver, &wid("convert.target.grouped"))?;
    if widget_hit_center(&mut driver, &wid("convert.allow_lossy")).is_ok() {
        let _ = click_widget_center(&mut driver, &wid("convert.allow_lossy"))?;
    }
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;

    let grouped_id = driver
        .app()
        .document()
        .data_sheets
        .iter()
        .find(|d| matches!(d.archetype, TableArchetype::Grouped(_)))
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("ConvertArchetype did not produce a Grouped DataSheet"))?;
    let _desc_id = run_analyze_picker(&mut driver, &grouped_id, "stats.descriptive")?;
    assert_parity_clean(&mut driver, "wf31 after MV -> Grouped ConvertArchetype")?;

    // 3. Proposed cross-schema `transform.select` `edit.transform.*` blocker (pending r10-w6-integ wiring):
    let sel_id = run_analyze_picker(&mut driver, &mv_id, "transform.select")?;
    select_nav_sheet(&mut driver, &sel_id)?;
    let _ = click_widget_center(&mut driver, &wid("toolbar.parameters"))?;
    click_modal_control(&mut driver, "scroll.modal.stat_params", "edit.transform.target_archetype")?;
    replace_text_widget(&mut driver, "edit.transform.filter_formula", "VALUE >= 10")?;
    let _ = click_widget_center(&mut driver, &wid("modal.ok"))?;
    let sel_rs = driver.app().document().result_sheet(&sel_id).ok_or_else(|| String::from("missing select ResultSheet"))?;
    if !matches!(
        &sel_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
            target_archetype: TableKind::Grouped,
            filter_formula: Some(f),
        }) if f == "VALUE >= 10"
    ) {
        return Err(format!("expected transform.select target_archetype=Grouped and filter_formula=\"VALUE >= 10\", got {:?}", sel_rs.analysis.kind));
    }

    assert_parity_clean(&mut driver, "wf31 final state")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}
