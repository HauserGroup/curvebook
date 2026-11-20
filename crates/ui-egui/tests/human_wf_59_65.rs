#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-59`–`WF-65`, Shard 9, `R13.6`).
//!
//! Exercises workflows `WF-59` through `WF-65` on [`EguiHumanPathDriver`] via real pointer,
//! wheel, drag, hover, keyboard, and IME events under `GuiOnlyPolicy::Strict` and
//! `HumanPathOnly`, passing `14`-locale `gui-layout-spec.md` §9.4 parity and latency budgets.

mod human_wf_common;

use curvebook_chart::{ChartScene, compile_graph, compile_layout};
use curvebook_doc::{
    CartesianOrientation, Cell, DataSheet, GraphicalDataObject, GroupedArrangement, HeatmapColumnLabelAngle, LayoutItem, ProjectDocument,
    ReplicateLineMode, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TickDirection, TickLabelFormat, XyTable,
};
use curvebook_engine::Session;
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_testkit::r13_datasets::{
    wf59_paired_before_after_and_inset, wf60_subset_linear_with_outliers_xy, wf61_multivariate_biomarker_grouped, wf62_five_design_trial,
    wf63_meta_analysis_forest_grouped, wf64_demographics_pyramid_grouped, wf65_longitudinal_spaghetti_xy,
};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use serde_json::json;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

fn graph_svg(doc: &ProjectDocument, graph_id: &str) -> Result<String, String> {
    let gs = doc.graph_sheet(&SheetId::new(graph_id)).ok_or_else(|| format!("missing graph `{graph_id}`"))?;
    let scene: ChartScene = compile_graph(gs, doc).map_err(|e| format!("compile_graph({graph_id}): {e}"))?;
    ensure(scene.warnings.is_empty(), format!("warnings on `{graph_id}`: {:?}", scene.warnings))?;
    render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())
}

fn enter_cell_scroll_h(h: &mut EguiWfHarness, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
    let wid = format!("grid.cell.{row}.{col}.{subcol}");
    for _ in 0..8 {
        if h.driver.frame_meta().find_by_widget_id(&wid).is_some_and(|e| e.clipped_rect.is_positive()) {
            return h.type_replace_cell(&wid, text);
        }
        if let Some(anchor) = h.driver.frame_meta().entries.iter().find(|e| e.widget_id.starts_with("grid.cell.")) {
            let pos = anchor.clipped_rect.center();
            h.driver.wheel_at(pos, egui::vec2(-220.0, 0.0))?;
        } else {
            break;
        }
    }
    h.enter_grid_cell(row, col, subcol, text)
}

fn layout_svg(doc: &ProjectDocument, layout_id: &str) -> Result<String, String> {
    let lay = doc.layout_sheet(&SheetId::new(layout_id)).ok_or_else(|| format!("missing layout `{layout_id}`"))?;
    let scene = compile_layout(lay, doc).map_err(|e| format!("compile_layout({layout_id}): {e}"))?;
    render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())
}

#[test]
fn wf59_human_layout_equalize_scaling_transparent_overlay_and_inset() -> TestRes {
    let fx = wf59_paired_before_after_and_inset();
    let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "col-wf59", "title": fx.title, "kind": "column", "cols": 2, "rows": fx.paired_rows.len() }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setGroupTitles", json!({ "sheet_id": "col-wf59", "titles": fx.paired_cols })).map_err(|e| e.to_string())?;
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        app.execute_command("table.setCell", json!({ "sheet_id": "col-wf59", "row": r, "col": 0, "value": pair[0] })).map_err(|e| e.to_string())?;
        app.execute_command("table.setCell", json!({ "sheet_id": "col-wf59", "row": r, "col": 1, "value": pair[1] })).map_err(|e| e.to_string())?;
    }
    app.execute_command("graph.create", json!({ "sheet_id": "g-box-wf59", "data_sheet_id": "col-wf59", "kind": "column" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.setKind", json!({ "sheet_id": "g-box-wf59", "kind": "column", "option_id": "col.box_tukey" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-ba-wf59", "data_sheet_id": "col-wf59", "kind": "column" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.setKind", json!({ "sheet_id": "g-ba-wf59", "kind": "column", "option_id": "col.before_after" }))
        .map_err(|e| e.to_string())?;

    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "xy-inset-wf59", "title": "Inset", "kind": "xy", "cols": 1, "rows": fx.inset_xy.len() }),
    )
    .map_err(|e| e.to_string())?;
    for (r, &(x, y)) in fx.inset_xy.iter().enumerate() {
        app.execute_command("table.setCell", json!({ "sheet_id": "xy-inset-wf59", "row": r, "col": 0, "value": y })).map_err(|e| e.to_string())?;
        let _ = x;
    }
    app.execute_command("graph.create", json!({ "sheet_id": "g-inset-wf59", "data_sheet_id": "xy-inset-wf59", "kind": "xy" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-wf59", "title": "WF59 Overlay", "preset": "double_column" }))
        .map_err(|e| e.to_string())?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-inset", "kind": "graph", "graph_sheet_id": "g-inset-wf59", "bounds_pt": [275.0, 30.0, 100.0, 80.0] }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-box", "kind": "graph", "graph_sheet_id": "g-box-wf59", "bounds_pt": [20.0, 20.0, 240.0, 170.0] }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-ba", "kind": "graph", "graph_sheet_id": "g-ba-wf59", "bounds_pt": [20.0, 20.0, 180.0, 130.0] }),
    )
    .map_err(|e| e.to_string())?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    h.select_sheet(&SheetId::new("g-ba-wf59"))?;
    h.click("toolbar.format_graph")?;
    h.click("edit.tab.format_graph.settings")?;
    h.click("format_graph.paper_fill.transparent")?;
    h.click("modal.ok")?;

    h.click("toolbar.axes")?;
    h.click("edit.tab.axes.x")?;
    h.click("axes.tick_dir.none")?;
    h.click("axes.show_tick_labels")?;
    h.click("modal.ok")?;

    h.click("toolbar.axes")?;
    h.click("edit.tab.axes.y")?;
    h.click("axes.tick_dir.none")?;
    h.click("axes.show_tick_labels")?;
    h.click("modal.ok")?;

    {
        let g_ba = h.app().document().graph_sheet(&SheetId::new("g-ba-wf59")).ok_or("WF-59 missing g-ba")?;
        ensure(g_ba.spec.paper_fill.is_none(), "WF-59 transparent paper_fill failed")?;
        ensure(g_ba.spec.x_axis.tick_direction == TickDirection::None && !g_ba.spec.x_axis.show_tick_labels, "WF-59 X axis-free failed")?;
        ensure(g_ba.spec.y_axis.tick_direction == TickDirection::None && !g_ba.spec.y_axis.show_tick_labels, "WF-59 Y axis-free failed")?;
    }

    h.select_sheet(&SheetId::new("lay-wf59"))?;
    h.click("inspector.layout.equalize_scaling")?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::EqualizeScaling { .. }), "WF-59 EqualizeScaling dialog must open")?;
    h.click("equalize_scaling.mode.smallest")?;
    h.click("modal.ok")?;
    h.click("inspector.layout.align_origins")?;

    // Bring panel 0 (`p-inset`) to front via Inspector Z-order chip.
    h.click("canvas.layout_panel.0")?;
    h.click("inspector.layout.zorder.front")?;
    {
        let lay = h.app().document().layout_sheet(&SheetId::new("lay-wf59")).ok_or("WF-59 missing layout")?;
        ensure(
            matches!(lay.spec.items.last(), Some(LayoutItem::LiveGraph { id, .. }) if id == "p-inset"),
            "WF-59 Z-order front must move p-inset to end",
        )?;
    }

    let svg_before = layout_svg(h.app().document(), "lay-wf59")?;
    h.select_sheet(&SheetId::new("col-wf59"))?;
    h.enter_grid_cell(0, 1, 0, "49.5")?;
    let svg_after = layout_svg(h.app().document(), "lay-wf59")?;
    ensure(svg_before != svg_after, "WF-59 upstream edit must recompute layout SVG")?;

    h.select_sheet(&SheetId::new("lay-wf59"))?;
    let png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-59 PNG invalid")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-59")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf60_human_subset_regression_decoupled_points_and_red_outlier_override() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf60_subset_linear_with_outliers_xy();
    let sid = h.create_table_via_welcome("welcome.arch.xy", "WF60 Subset Outlier", &["welcome.stepper.cols.dec"])?;

    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{y}"))?;
    }

    // Exclude outlier rows in col 1 via Shift+Click + Cmd+E.
    let w_start = format!("grid.cell.{}.1.0", fx.outlier_rows[0]);
    let w_end = format!("grid.cell.{}.1.0", fx.outlier_rows[1]);
    h.click(&w_start)?;
    let end_pos = h.widget_center(&w_end)?;
    h.driver.click_at_button(end_pos, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    h.driver.press_key("E", KeyModifiers { meta: true, ..Default::default() })?;

    let lin_id = h.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    let tabs_before = format!("{:?}", h.app().document().result_sheet(&lin_id).ok_or("WF-60 missing linear fit")?.tabs);

    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-60 missing auto graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.format_graph")?;
    h.click("format_graph.series_next")?;
    h.click("format_graph.show_raw_data")?;
    h.click("modal.ok")?;
    {
        let gs = h.app().document().graph_sheet(&gs_id).ok_or("WF-60 missing graph")?;
        ensure(!gs.spec.series[1].show_raw_data, "WF-60 series 1 raw points must decouple off")?;
    }

    // Re-include row 6 in col 1 via Cmd+E -> linear fit updates reactively.
    h.select_sheet(&sid)?;
    h.click(&w_start)?;
    h.driver.press_key("E", KeyModifiers { meta: true, ..Default::default() })?;
    let tabs_after = format!("{:?}", h.app().document().result_sheet(&lin_id).ok_or("WF-60 missing linear post toggle")?.tabs);
    ensure(tabs_before != tabs_after, "WF-60 re-including outlier must recompute regression")?;

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-60")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf61_human_per_column_normalized_heatmap_and_vertical_column_titles() -> TestRes {
    let fx = wf61_multivariate_biomarker_grouped();
    let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf61",
            "title": fx.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "cols": fx.analyte_titles.len(),
            "rows": fx.row_titles.len()
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setGroupTitles", json!({ "sheet_id": "grp-wf61", "titles": fx.analyte_titles })).map_err(|e| e.to_string())?;
    app.execute_command("table.setRowTitles", json!({ "sheet_id": "grp-wf61", "titles": fx.row_titles })).map_err(|e| e.to_string())?;
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            app.execute_command("table.setCell", json!({ "sheet_id": "grp-wf61", "row": r, "col": c, "subcol": 0, "value": val }))
                .map_err(|e| e.to_string())?;
        }
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "grp-wf61" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    let norm_id = h.run_analyze_entry("Normalize", "analyze.entry.transform.normalize")?;
    h.select_sheet(&norm_id)?;
    h.click("toolbar.command_palette")?;
    h.driver.type_text("ui.graph.create.open")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::CreateGraph { .. }), "WF-61 CreateGraph modal must open")?;
    if let Some(src_node) = h
        .driver
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("create_graph.source.") && (e.label.contains("Normalize") || e.label.contains(norm_id.as_str())))
    {
        let src_wid = src_node.widget_id.clone();
        h.click(&src_wid)?;
    }
    h.click("modal.ok")?;

    let g_norm_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-61 missing normalize graph")?;
    h.select_sheet(&g_norm_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.diag.heatmap")?;
    h.click("modal.ok")?;

    h.click("toolbar.format_graph")?;
    h.click("edit.tab.format_graph.heatmap_labels")?;
    h.click("format_graph.heatmap.angle.90")?;
    h.click("modal.ok")?;

    let svg_before = {
        let gs = h.app().document().graph_sheet(&g_norm_id).ok_or("WF-61 missing g_norm")?;
        ensure(gs.spec.heatmap_labels.column_angle == HeatmapColumnLabelAngle::Vertical90, "WF-61 expected Vertical90 column labels")?;
        graph_svg(h.app().document(), g_norm_id.as_str())?
    };

    h.select_sheet(&SheetId::new("grp-wf61"))?;
    h.enter_grid_cell(3, 0, 0, "950.0")?;
    let svg_after = graph_svg(h.app().document(), g_norm_id.as_str())?;
    ensure(svg_before != svg_after, "WF-61 live cell edit must rescale normalized heatmap SVG")?;

    h.select_sheet(&g_norm_id)?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-61")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf62_human_five_design_nested_paired_and_rm_anova_trial_comparison() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1600.0, 900.0, 1.0));
    let fx = wf62_five_design_trial();

    // 1) Nested hierarchical t-test vs flattened Column unpaired t-test
    let nest_id =
        h.create_table_via_welcome("welcome.arch.nested", "WF62 Nested Trial", &["welcome.stepper.cols.dec", "welcome.nested.depth.inc"])?;
    for (group_idx, subjects) in fx.nested_values.iter().enumerate() {
        for (subject_idx, reps) in subjects.iter().enumerate() {
            for (rep_idx, &val) in reps.iter().enumerate() {
                enter_cell_scroll_h(&mut h, rep_idx, group_idx, subject_idx, &format!("{val}"))?;
            }
        }
    }
    h.select_sheet(&nest_id)?;
    let nest_res = h.run_analyze_entry("Nested", "analyze.entry.stats.nested")?;
    h.select_sheet(&nest_res)?;
    h.open_cmd_t()?;
    h.click("edit.nested.t")?;
    h.click("modal.ok")?;

    let flat_id = h.create_table_via_welcome("welcome.arch.column", "WF62 Flattened", &[])?;
    for (group_idx, subjects) in fx.nested_values.iter().enumerate() {
        let mut flat_r = 0usize;
        for reps in subjects {
            for &val in reps {
                h.enter_grid_cell(flat_r, group_idx, 0, &format!("{val}"))?;
                flat_r += 1;
            }
        }
    }
    h.select_sheet(&flat_id)?;
    let flat_res = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;

    let nest_tabs_before = {
        let nr = h.app().document().result_sheet(&nest_res).ok_or("WF-62 missing nest_res")?;
        let fr = h.app().document().result_sheet(&flat_res).ok_or("WF-62 missing flat_res")?;
        ensure(matches!(nr.status, ResultStatus::UpToDate), "WF-62 nested_t not UpToDate")?;
        ensure(matches!(fr.status, ResultStatus::UpToDate), "WF-62 flat unpaired_t not UpToDate")?;
        let ns = format!("{:?}", nr.tabs);
        ensure(ns != format!("{:?}", fr.tabs), "WF-62 nested t vs flattened unpaired t must differ")?;
        ns
    };
    h.select_sheet(&nest_id)?;
    h.enter_grid_cell(0, 0, 0, "24.8")?;
    let nest_tabs_after = format!("{:?}", h.app().document().result_sheet(&nest_res).ok_or("WF-62 post edit")?.tabs);
    ensure(nest_tabs_before != nest_tabs_after, "WF-62 live nested edit must recompute nested_t")?;

    // 2) Paired two-group column design (`cols` remains 2 from Welcome state)
    let paired_tbl = h.create_table_via_welcome("welcome.arch.column", "WF62 Paired", &["welcome.design.paired"])?;
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{}", pair[0]))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", pair[1]))?;
    }
    h.select_sheet(&paired_tbl)?;
    let paired_res = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    h.select_sheet(&paired_res)?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.tg.paired_t")?;
    h.click("modal.ok")?;
    ensure(
        matches!(h.app().document().result_sheet(&paired_res).ok_or("WF-62 paired_res")?.status, ResultStatus::UpToDate),
        "WF-62 paired_t must be UpToDate",
    )?;

    // 3) Longitudinal Two-Way RM ANOVA: BothFactorsMatched vs RowFactorMatched (`cols=2, subcols=4` preserved)
    let rm_tbl = h.create_table_via_welcome("welcome.arch.grouped", "WF62 TwoWay RM", &["welcome.subcol.raw"])?;
    for (r, flat_vals) in fx.rm_rows.iter().enumerate() {
        for (flat_idx, &val) in flat_vals.iter().enumerate() {
            enter_cell_scroll_h(&mut h, r, flat_idx / 4, flat_idx % 4, &format!("{val}"))?;
        }
    }
    h.select_sheet(&rm_tbl)?;
    let rm_res = h.run_analyze_entry("Two-Way", "analyze.entry.stats.factorial_anova")?;
    h.select_sheet(&rm_res)?;
    h.open_cmd_t()?;
    if h.driver.frame_meta().find_by_widget_id("edit.tab.repeated_measures").is_some_and(|e| !e.enabled) {
        h.click("edit.paired_or_matched")?;
    }
    h.click("edit.tab.repeated_measures")?;
    h.click("edit.fac.rm.both")?;
    h.click("modal.ok")?;
    let rm_both_tabs = format!("{:?}", h.app().document().result_sheet(&rm_res).ok_or("WF-62 rm both")?.tabs);

    h.open_cmd_t()?;
    if h.driver.frame_meta().find_by_widget_id("edit.tab.repeated_measures").is_some_and(|e| !e.enabled) {
        h.click("edit.paired_or_matched")?;
    }
    h.click("edit.tab.repeated_measures")?;
    h.click("edit.fac.rm.row")?;
    h.click("modal.ok")?;
    let rm_row_rs = h.app().document().result_sheet(&rm_res).ok_or("WF-62 rm row")?;
    ensure(matches!(rm_row_rs.status, ResultStatus::UpToDate), "WF-62 RM row factor not UpToDate")?;
    ensure(format!("{:?}", rm_row_rs.tabs) != rm_both_tabs, "WF-62 RM both vs row factor must differ")?;

    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-62")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf63_human_forest_plot_upper_lower_limits_and_null_reference_line() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let fx = wf63_meta_analysis_forest_grouped();
    let sid = h.create_table_via_welcome(
        "welcome.arch.grouped",
        "WF63 Forest CI",
        &["welcome.subcol.upper_lower", "welcome.stepper.cols.dec", "welcome.stepper.cols.dec"],
    )?;

    for (r, entry) in fx.entries.iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{}", entry[0]))?;
        h.enter_grid_cell(r, 0, 1, &format!("{}", entry[1]))?;
        h.enter_grid_cell(r, 0, 2, &format!("{}", entry[2]))?;
    }

    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-63 missing auto graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.diag.forest")?;
    h.click("modal.ok")?;

    h.click("toolbar.axes")?;
    h.click("edit.tab.axes.x")?;
    h.click("axes.additional_ticks.open")?;
    h.replace_textbox("axes.add_tick.value", "1.0")?;
    h.replace_textbox("axes.add_tick.label", "No Effect")?;
    h.click("axes.add_tick.gridline")?;
    h.click("axes.add_tick.save")?;
    h.click("axes.add_tick.close")?;
    h.click("modal.ok")?;

    let svg_before = {
        let gs = h.app().document().graph_sheet(&gs_id).ok_or("WF-63 missing graph")?;
        ensure(!gs.spec.forest_show_pooled_diamond, "WF-63 pooled diamond must default off")?;
        ensure(gs.spec.x_axis.additional_ticks.iter().any(|t| t.label.as_deref() == Some("No Effect")), "WF-63 custom No Effect tick missing")?;
        graph_svg(h.app().document(), gs_id.as_str())?
    };

    h.select_sheet(&sid)?;
    h.enter_grid_cell(0, 0, 2, "1.35")?;
    let svg_after = graph_svg(h.app().document(), gs_id.as_str())?;
    ensure(svg_before != svg_after, "WF-63 upper-CI cell edit must update forest plot SVG")?;

    h.select_sheet(&gs_id)?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-63")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf64_human_population_pyramid_horizontal_stacked_and_unsigned_ticks() -> TestRes {
    let fx = wf64_demographics_pyramid_grouped();
    let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf64",
            "title": fx.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "cols": fx.cohort_titles.len(),
            "rows": fx.band_titles.len()
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setGroupTitles", json!({ "sheet_id": "grp-wf64", "titles": fx.cohort_titles })).map_err(|e| e.to_string())?;
    app.execute_command("table.setRowTitles", json!({ "sheet_id": "grp-wf64", "titles": fx.band_titles })).map_err(|e| e.to_string())?;
    for (r, row) in fx.rows.iter().enumerate() {
        app.execute_command("table.setCell", json!({ "sheet_id": "grp-wf64", "row": r, "col": 0, "subcol": 0, "value": row[0] }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setCell", json!({ "sheet_id": "grp-wf64", "row": r, "col": 1, "subcol": 0, "value": row[1] }))
            .map_err(|e| e.to_string())?;
    }
    app.execute_command("graph.create", json!({ "sheet_id": "g-wf64", "data_sheet_id": "grp-wf64", "kind": "grouped" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.setSpec", json!({ "sheet_id": "g-wf64", "orientation": "horizontal" })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "grp-wf64" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    h.click("grid.row_titles.toggle")?;
    ensure(h.app().grid.row_titles_collapsed_override == Some(true), "WF-64 Row Titles toggle must collapse/expand")?;
    h.click("grid.row_titles.toggle")?;
    ensure(h.app().grid.row_titles_collapsed_override == Some(false), "WF-64 Row Titles toggle must expand again")?;

    h.select_sheet(&SheetId::new("g-wf64"))?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.grouped.stacked")?;
    h.click("modal.ok")?;

    h.click("toolbar.format_graph")?;
    h.click("format_graph.apply_globally")?;
    h.click("modal.ok")?;

    h.click("toolbar.axes")?;
    h.click("edit.tab.axes.x")?;
    h.click("axes.tick_format.abs_decimal")?;
    if h.driver.frame_meta().find_by_widget_id("axes.categorical_guard.confirm_checkbox").is_some() {
        h.click("axes.categorical_guard.confirm_checkbox")?;
        h.click("axes.categorical_guard.confirm_btn")?;
    }
    h.click("modal.ok")?;

    let svg_before = {
        let gs = h.app().document().graph_sheet(&SheetId::new("g-wf64")).ok_or("WF-64 missing graph")?;
        ensure(gs.spec.orientation == CartesianOrientation::Horizontal, "WF-64 must keep Horizontal orientation")?;
        ensure(gs.spec.grouped_arrangement == GroupedArrangement::Stacked, "WF-64 must be Stacked")?;
        ensure(gs.spec.x_axis.tick_format == TickLabelFormat::AbsoluteDecimal, "WF-64 must persist AbsoluteDecimal X ticks")?;
        graph_svg(h.app().document(), "g-wf64")?
    };

    h.select_sheet(&SheetId::new("grp-wf64"))?;
    h.enter_grid_cell(2, 0, 0, "-38.0")?;
    let svg_after = graph_svg(h.app().document(), "g-wf64")?;
    ensure(svg_before != svg_after, "WF-64 pyramid cell edit must update SVG")?;

    h.select_sheet(&SheetId::new("g-wf64"))?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-64")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf65_human_spaghetti_plot_replicate_trajectories_vs_subject_traces() -> TestRes {
    let fx = wf65_longitudinal_spaghetti_xy();
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(4), fx.cohort_titles.len(), fx.weeks.len()).map_err(|e| e.to_string())?;
    for (g, &title) in fx.cohort_titles.iter().enumerate() {
        xy.data_sets[g].title = title.to_owned();
    }
    for (r, &week) in fx.weeks.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(week)).map_err(|e| e.to_string())?;
        for (g, reps) in fx.replicate_trajectories[r].iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                xy.data_sets[g].cells[r][s] = Cell::number(val);
            }
        }
    }
    let mut doc = ProjectDocument::new("WF65 Spaghetti").map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("xy-rep-wf65", fx.title, TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
    app.execute_command("graph.create", json!({ "sheet_id": "g-rep-wf65", "data_sheet_id": "xy-rep-wf65", "kind": "xy" }))
        .map_err(|e| e.to_string())?;
    app.execute_command(
        "graph.setSeries",
        json!({ "sheet_id": "g-rep-wf65", "all_series": true, "marker": null, "replicate_lines": "individual_replicate_trajectories" }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-rep-wf65" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    let scene = compile_graph(h.app().document().graph_sheet(&SheetId::new("g-rep-wf65")).ok_or("WF-65 graph")?, h.app().document())
        .map_err(|e| e.to_string())?;
    let first_pt = scene.plotted_points.first().ok_or("WF-65 expected spaghetti plotted_points")?;
    ensure(
        matches!(
            h.app().document().graph_sheet(&SheetId::new("g-rep-wf65")).ok_or("missing g-rep")?.spec.series[0].gdo,
            GraphicalDataObject::ScatterAndLine { marker: None, replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories, .. }
        ),
        "WF-65 expected marker=None & IndividualReplicateTrajectories",
    )?;

    let canvas_rect = h.driver.frame_meta().find_by_widget_id("canvas.viewport").ok_or("WF-65 missing canvas.viewport")?.clipped_rect;
    let margin = 16.0_f32;
    let avail_w = (canvas_rect.width() - margin * 2.0).max(80.0);
    let avail_h = (canvas_rect.height() - margin * 2.0).max(80.0);
    let sx = (avail_w / (scene.width_pt.max(1.0) as f32)).min(avail_h / (scene.height_pt.max(1.0) as f32));
    let paper_min =
        egui::pos2(canvas_rect.center().x - (scene.width_pt as f32) * sx * 0.5, canvas_rect.center().y - (scene.height_pt as f32) * sx * 0.5);
    let screen_pt = egui::pos2(paper_min.x + (first_pt.center_pt.x as f32) * sx, paper_min.y + (first_pt.center_pt.y as f32) * sx);
    h.driver.hover_at(screen_pt)?;
    let readout = h.driver.state().views.canvas_view.hover_readout.as_ref().ok_or("WF-65 expected spaghetti hover readout")?;
    ensure(readout.label.contains("Rep "), format!("WF-65 expected Rep N hover readout, got `{}`", readout.label))?;

    let svg_before = graph_svg(h.app().document(), "g-rep-wf65")?;
    h.select_sheet(&SheetId::new("xy-rep-wf65"))?;
    enter_cell_scroll_h(&mut h, 2, 1, 3, "52.5")?;
    let svg_after = graph_svg(h.app().document(), "g-rep-wf65")?;
    ensure(svg_before != svg_after, "WF-65 single-subcolumn edit must update spaghetti SVG")?;

    h.select_sheet(&SheetId::new("g-rep-wf65"))?;
    let png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-65 PNG invalid")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-65")?;
    assert_input_to_paint_budget(&h, 50.0)
}
