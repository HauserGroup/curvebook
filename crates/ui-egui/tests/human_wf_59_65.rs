#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-59`–`WF-65`, Shard 9, `R13.6`).
//!
//! Exercises workflows `WF-59` through `WF-65` on [`EguiHumanPathDriver`] via real pointer,
//! wheel, drag, hover, keyboard, and IME events under `GuiOnlyPolicy::Strict` and
//! `HumanPathOnly`, passing `14`-locale `gui-layout-spec.md` §9.4 parity and latency budgets.

mod human_wf_common;

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, CartesianOrientation, Cell, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphKind, GraphicalDataObject,
    GroupedArrangement, GroupedTable, HeatmapColumnLabelAngle, LayoutItem, NestedTable, ProjectDocument, RectPt, RepeatedMeasuresDesign,
    ReplicateLineMode, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TickDirection, TickLabelFormat, XyTable,
};
use curvebook_palette::LineDashStyle;
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_testkit::r13_datasets::{
    wf59_paired_before_after_and_inset, wf60_subset_linear_with_outliers_xy, wf61_multivariate_biomarker_grouped, wf62_five_design_trial,
    wf63_meta_analysis_forest_grouped, wf64_demographics_pyramid_grouped, wf65_longitudinal_spaghetti_xy,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use egui::{Key, Modifiers};
use serde_json::json;

use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use human_wf_common::{TestRes, ensure};

fn graph_svg(doc: &ProjectDocument, graph_id: &str) -> Result<String, String> {
    let gs = doc.graph_sheet(&SheetId::new(graph_id)).ok_or_else(|| format!("missing graph `{graph_id}`"))?;
    let scene: ChartScene = compile_graph(gs, doc).map_err(|e| format!("compile_graph({graph_id}): {e}"))?;
    ensure(scene.warnings.is_empty(), format!("warnings on `{graph_id}`: {:?}", scene.warnings))?;
    render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())
}

fn graph_for_source(doc: &ProjectDocument, source_id: &SheetId) -> Result<SheetId, String> {
    doc.graph_sheets
        .iter()
        .rfind(|g| g.spec.primary_source_sheet_id == *source_id)
        .map(|g| g.id.clone())
        .ok_or_else(|| format!("missing GraphSheet for `{}`", source_id.as_str()))
}

fn enter_cell_scroll_h(run: &mut WorkflowRun, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
    let wid = format!("grid.cell.{row}.{col}.{subcol}");
    for _ in 0..8 {
        if run.frame_meta().entries.iter().any(|e| e.widget_id == wid && e.clipped_rect.is_positive()) {
            return run.enter_grid_cell(row, col, subcol, text);
        }
        let anchor_pos = run
            .frame_meta()
            .entries
            .iter()
            .find(|e| e.widget_id.starts_with("grid.cell.") && e.clipped_rect.is_positive())
            .map(|e| e.clipped_rect.center());
        if let Some(pos) = anchor_pos {
            run.wheel(pos, egui::vec2(-220.0, 0.0))?;
        } else {
            break;
        }
    }
    run.enter_grid_cell(row, col, subcol, text)
}

fn set_scrub_field_wf(run: &mut WorkflowRun, wid: &str, chip_offset_from_right_pt: f32, text: &str) -> TestRes {
    let r = run
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id == wid && e.clipped_rect.is_positive())
        .map(|e| e.clipped_rect)
        .ok_or_else(|| format!("missing scrub widget `{wid}`"))?;
    let click_pt = egui::pos2((r.right() - chip_offset_from_right_pt).max(r.left() + 6.0), r.center().y);
    run.click_at(click_pt)?;
    for _ in 0..12 {
        run.press_egui_key(Key::Backspace, Modifiers::NONE)?;
    }
    run.type_text(text)?;
    run.press_egui_key(Key::Enter, Modifiers::NONE)
}

fn layout_svg(doc: &ProjectDocument, layout_id: &str) -> Result<String, String> {
    let lay = doc.layout_sheet(&SheetId::new(layout_id)).ok_or_else(|| format!("missing layout `{layout_id}`"))?;
    let scene = compile_layout(lay, doc).map_err(|e| format!("compile_layout({layout_id}): {e}"))?;
    render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())
}

#[test]
fn wf59_human_layout_equalize_scaling_transparent_overlay_and_inset() -> TestRes {
    let mut run = WorkflowRun::start("WF-59");
    let fx = wf59_paired_before_after_and_inset();

    // 1) Create paired Column cohort via Welcome Hub (`cols` 3 -> 2).
    let col_id = run.create_table_via_welcome("welcome.arch.column", fx.title, &["welcome.stepper.cols.dec"])?;
    for (c, &ct) in fx.paired_cols.iter().enumerate() {
        run.enter_column_title(c, ct)?;
    }
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        run.enter_grid_cell(r, 0, 0, &format!("{}", pair[0]))?;
        run.enter_grid_cell(r, 1, 0, &format!("{}", pair[1]))?;
    }

    let g_ba_id = graph_for_source(run.document(), &col_id)?;
    run.select_sheet(&g_ba_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.col.before_after")?;
    run.commit_modal_ok()?;

    // Second graph of the paired table -> transparent axis-free BoxAndWhisker overlay (Tukey).
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    let col_src_wid = run
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("create_graph.source.") && e.label.contains(fx.title))
        .map(|e| e.widget_id.clone())
        .ok_or("WF-59 missing paired table source in CreateGraph")?;
    run.click(&col_src_wid)?;
    run.click("create_graph.kind.column")?;
    run.commit_modal_ok()?;
    let g_box_id = run
        .document()
        .graph_sheets
        .iter()
        .rfind(|g| g.id != g_ba_id && g.spec.primary_source_sheet_id == col_id)
        .map(|g| g.id.clone())
        .ok_or("WF-59 missing second paired graph")?;
    run.select_sheet(&g_box_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.col.box_whisker")?;
    run.commit_modal_ok()?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.settings")?;
    run.click("format_graph.paper_fill.transparent")?;
    run.commit_modal_ok()?;
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.x")?;
    run.click("axes.tick_dir.none")?;
    run.click("axes.show_tick_labels")?;
    run.commit_modal_ok()?;
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.y")?;
    run.click("axes.tick_dir.none")?;
    run.click("axes.show_tick_labels")?;
    run.commit_modal_ok()?;
    {
        let g_box = run.document().graph_sheet(&g_box_id).ok_or("WF-59 missing g_box")?;
        ensure(g_box.spec.paper_fill.is_none(), "WF-59 transparent paper_fill failed")?;
        ensure(g_box.spec.x_axis.tick_direction == TickDirection::None && !g_box.spec.x_axis.show_tick_labels, "WF-59 X axis-free failed")?;
        ensure(g_box.spec.y_axis.tick_direction == TickDirection::None && !g_box.spec.y_axis.show_tick_labels, "WF-59 Y axis-free failed")?;
    }

    // 2) Create linearized XY inset table (`cols` 2 -> 1) + inset graph.
    let inset_id = run.create_table_via_welcome("welcome.arch.xy", "WF59 Inset", &["welcome.stepper.cols.dec"])?;
    for (r, &(x, y)) in fx.inset_xy.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let g_inset_id = graph_for_source(run.document(), &inset_id)?;

    // 3) Create LayoutSheet via GUI (Insert > New Layout…), add all 3 panels via studio.layout.add_panel.*.
    run.open_menu_and_click("menu.insert", &[], "menu.layout.create")?;
    let lay_id = run.document().layout_sheets.last().ok_or("WF-59 missing created LayoutSheet")?.id.clone();
    run.select_sheet(&lay_id)?;
    let idx_ba = run.document().graph_sheets.iter().position(|g| g.id == g_ba_id).ok_or("WF-59 idx g_ba")?;
    let idx_box = run.document().graph_sheets.iter().position(|g| g.id == g_box_id).ok_or("WF-59 idx g_box")?;
    let idx_inset = run.document().graph_sheets.iter().position(|g| g.id == g_inset_id).ok_or("WF-59 idx g_inset")?;
    run.click(&format!("studio.layout.add_panel.{idx_inset}"))?;
    run.click(&format!("studio.layout.add_panel.{idx_ba}"))?;
    run.click(&format!("studio.layout.add_panel.{idx_box}"))?;

    // Equalize Scaling + Align Origins.
    run.click("inspector.layout.equalize_scaling")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::EqualizeScaling { .. }), "WF-59 EqualizeScaling dialog must open")?;
    run.click("equalize_scaling.mode.smallest")?;
    run.commit_modal_ok()?;
    run.click("inspector.layout.align_origins")?;

    // Resize inset panel (panel 0) via SE handle and Bring to Front (`inspector.layout.zorder.front`).
    run.click("canvas.layout_panel.0")?;
    let bounds_before = match &run.document().layout_sheet(&lay_id).ok_or("WF-59 layout")?.spec.items[0] {
        &LayoutItem::LiveGraph { bounds_pt, .. } => *bounds_before_unwrap(&bounds_pt),
        _ => return Err(String::from("WF-59 expected inset LiveGraph at panel 0")),
    };
    let se_pos = run.widget_center("canvas.layout_panel.0.resize_se")?;
    run.drag(se_pos, se_pos + egui::vec2(-35.0, -25.0))?;
    let bounds_after = match &run.document().layout_sheet(&lay_id).ok_or("WF-59 layout")?.spec.items[0] {
        &LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt,
        _ => return Err(String::from("WF-59 expected inset LiveGraph post resize")),
    };
    ensure((bounds_after.width - bounds_before.width).abs() > 1.0, "WF-59 resize_se must change inset panel width")?;

    run.click("canvas.layout_panel.0")?;
    run.click("inspector.layout.zorder.front")?;
    {
        let lay = run.document().layout_sheet(&lay_id).ok_or("WF-59 missing layout")?;
        ensure(
            matches!(lay.spec.items.last(), Some(LayoutItem::LiveGraph { graph_sheet_id, .. }) if *graph_sheet_id == g_inset_id),
            "WF-59 Z-order front must move inset panel to end",
        )?;
    }

    // Stage 4: verify transparent axis-free BoxAndWhisker graph against headless Session oracle.
    let mut ref_col =
        ColumnTable::new(ColumnDesignHint::UnpairedReplicates, fx.paired_cols.len(), fx.paired_rows.len()).map_err(|e| e.to_string())?;
    for (c, &ct) in fx.paired_cols.iter().enumerate() {
        ref_col.data_sets[c].title = ct.to_owned();
    }
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        ref_col.data_sets[0].cells[r][0] = Cell::number(pair[0]);
        ref_col.data_sets[1].cells[r][0] = Cell::number(pair[1]);
    }
    let ref_sheet = DataSheet::new("ref-col-59", fx.title, TableArchetype::Column(ref_col)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_graph(g_box_id.clone(), |sess| {
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-box-59", "data_sheet_id": "ref-col-59", "kind": "column" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-box-59", "kind": "column", "option_id": "col.box_tukey" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setSpec", json!({ "sheet_id": "ref-g-box-59", "paper_fill": null })).map_err(|e| e.to_string())?;
            sess.execute(
                "graph.setAxes",
                json!({
                    "sheet_id": "ref-g-box-59",
                    "x_axis": { "tick_direction": "none", "show_tick_labels": false },
                    "y_axis": { "tick_direction": "none", "show_tick_labels": false }
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_graph_id(SheetId::new("ref-g-box-59"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.paper_fill == ref_gs.spec.paper_fill, "WF-59 paper_fill mismatch vs ref")?;
            ensure(gui_gs.spec.x_axis.tick_direction == ref_gs.spec.x_axis.tick_direction, "WF-59 x tick_dir mismatch vs ref")?;
            ensure(gui_gs.spec.y_axis.show_tick_labels == ref_gs.spec.y_axis.show_tick_labels, "WF-59 y tick labels mismatch vs ref")
        }),
    )?;

    // Stage 5: upstream cell edit updates Box/BeforeAfter graphs AND compiled layout SVG.
    let svg_lay_before = layout_svg(run.document(), lay_id.as_str())?;
    let lay_for_chk = lay_id.clone();
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id, 0, 1, 0, "49.5"),
        vec![
            LiveEditExpectation::graph_svg_changed(g_ba_id),
            LiveEditExpectation::graph_svg_changed(g_box_id),
            LiveEditExpectation::custom(move |_, _, doc| {
                let svg_lay_after = layout_svg(doc, lay_for_chk.as_str())?;
                ensure(svg_lay_after != svg_lay_before, "WF-59 upstream edit must recompute layout SVG")
            }),
        ],
    )?;

    run.select_sheet(&lay_id)?;
    run.finish()
}

fn bounds_before_unwrap(b: &RectPt) -> &RectPt {
    b
}

#[test]
fn wf60_human_subset_regression_decoupled_points_and_red_outlier_override() -> TestRes {
    let mut run = WorkflowRun::start("WF-60");
    run.set_viewport(ViewportMetrics::new(1440.0, 900.0, 1.0));
    let fx = wf60_subset_linear_with_outliers_xy();
    let sid = run.create_table_via_welcome("welcome.arch.xy", fx.title, &["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"])?;
    run.enter_column_title(0, "All Points")?;

    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }

    // 1) Hover plotted point on canvas and assert `(Row ...)` cursor readout appears (`GWF-21`).
    let gs_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&gs_id)?;
    let scene_raw = {
        let gs = run.document().graph_sheet(&gs_id).ok_or("WF-60 missing auto graph")?;
        compile_graph(gs, run.document()).map_err(|e| e.to_string())?
    };
    let pt0 = scene_raw.plotted_points.first().ok_or("WF-60 expected plotted points")?;
    let canvas_rect = run.frame_meta().find_by_widget_id("canvas.viewport").ok_or("WF-60 missing canvas.viewport")?.clipped_rect;
    let margin = 16.0_f32;
    let avail_w = (canvas_rect.width() - margin * 2.0).max(80.0);
    let avail_h = (canvas_rect.height() - margin * 2.0).max(80.0);
    let sx = (avail_w / (scene_raw.width_pt.max(1.0) as f32)).min(avail_h / (scene_raw.height_pt.max(1.0) as f32));
    let paper_min =
        egui::pos2(canvas_rect.center().x - (scene_raw.width_pt as f32) * sx * 0.5, canvas_rect.center().y - (scene_raw.height_pt as f32) * sx * 0.5);
    let pt0_screen = egui::pos2(paper_min.x + (pt0.center_pt.x as f32) * sx, paper_min.y + (pt0.center_pt.y as f32) * sx);
    run.hover_at(pt0_screen)?;
    let readout = run.state().views.canvas_view.hover_readout.as_ref().ok_or("WF-60 expected canvas point hover readout")?;
    ensure(readout.value_summary.contains("Row 1"), format!("WF-60 hover readout must include Row 1, got `{}`", readout.value_summary))?;

    // 2) Duplicate Y column via DataGrid context menu (`grid.ctx.duplicate_column`), title Col 1, and exclude outlier rows [6, 7] via Cmd+E.
    run.select_sheet(&sid)?;
    let cell00 = run.widget_center("grid.cell.0.0.0")?;
    run.click_at_button(cell00, PointerButton::Secondary, KeyModifiers::default())?;
    run.click("grid.ctx.duplicate_column")?;
    ensure(
        run.document().data_sheet(&sid).is_some_and(|ds| ds.archetype.column_count() == 2),
        "WF-60 duplicate_column must create Included Subset column",
    )?;
    run.enter_column_title(1, "Included Subset")?;

    let w_start = format!("grid.cell.{}.1.0", fx.outlier_rows[0]);
    let w_end = format!("grid.cell.{}.1.0", fx.outlier_rows[1]);
    run.click(&w_start)?;
    let end_pos = run.widget_center(&w_end)?;
    run.click_at_button(end_pos, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    run.press_key("E", KeyModifiers { meta: true, ..Default::default() })?;

    // 3) Run linear regression selectively on duplicate column (uncheck Dataset 0 -> selected_data_sets = [1]).
    let before_rs = run.document().result_sheets.len();
    run.click("toolbar.analyze")?;
    run.replace_textbox("analyze.search", "Linear")?;
    run.click("analyze.entry.fit.linear")?;
    run.click("analyze.mv.var.0")?;
    run.commit_modal_ok()?;
    ensure(run.document().result_sheets.len() == before_rs + 1, "WF-60 selective Linear must create ResultSheet")?;
    let lin_id = run.document().result_sheets.last().map(|r| r.id.clone()).ok_or("WF-60 missing linear fit")?;

    // 4) Format full Series 0 (rows 0..8) red via Format Selected Points (`inspector.table.style_subset`, zero guarded clicks).
    run.select_sheet(&sid)?;
    run.click("grid.cell.0.0.0")?;
    let p_last = run.widget_center("grid.cell.7.0.0")?;
    run.click_at_button(p_last, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    run.click("inspector.table.style_subset")?;
    run.click("fmt_pts.col.Red")?;
    run.commit_modal_ok()?;

    // 5) Attach linear regression overlay on Series 1 AND decouple Show data points off while keeping Show fitted curve on.
    run.select_sheet(&gs_id)?;
    run.click("toolbar.format_graph")?;
    run.click("format_graph.series_next")?;
    run.click("format_graph.overlay.attach.0")?;
    run.click("format_graph.show_raw_data")?;
    run.commit_modal_ok()?;
    {
        let doc = run.document();
        let gs = doc.graph_sheet(&gs_id).ok_or("WF-60 missing graph after FormatGraph")?;
        ensure(!gs.spec.series[1].show_raw_data, "WF-60 series 1 Show data points must be decoupled off")?;
        ensure(
            gs.spec.series[1].regression_overlay.as_ref().is_some_and(|ov| ov.result_sheet_id == lin_id),
            "WF-60 series 1 must carry linear regression overlay",
        )?;
        ensure(
            gs.spec.point_overrides.len() >= fx.rows.len(),
            format!("WF-60 expected >= {} red point overrides on Series 0, got {}", fx.rows.len(), gs.spec.point_overrides.len()),
        )?;
        ensure(
            gs.spec.point_overrides.iter().any(|ov| ov.series_index == 0 && ov.fill_color.is_some()),
            "WF-60 Series 0 point overrides must carry red fill color",
        )?;
        let _svg = graph_svg(doc, gs_id.as_str())?;
    }

    // Stage 4: Oracle verification of selective subset linear regression on Col 1 (`excluded = [6, 7]`).
    let mut ref_xy = XyTable::new(SubcolumnMode::Single, 2, fx.rows.len()).map_err(|e| e.to_string())?;
    ref_xy.data_sets[0].title = String::from("All Points");
    ref_xy.data_sets[1].title = String::from("Included Subset");
    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(y);
        ref_xy.data_sets[1].cells[r][0] = Cell::number(y);
    }
    let ref_sheet = DataSheet::new("ref-xy-60", fx.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    let excl_cells: Vec<_> = fx.outlier_rows.iter().map(|&row| json!({ "row": row, "col": 1, "subcol": 0 })).collect();
    run.verify_reference(
        ReferenceSpec::for_result(lin_id.clone(), move |sess| {
            sess.execute("table.excludeCells", json!({ "sheet_id": "ref-xy-60", "cells": excl_cells, "excluded": true }))
                .map_err(|e| e.to_string())?;
            sess.execute(
                "analysis.run",
                json!({ "sheet_id": "ref-lin-60", "source_sheet_id": "ref-xy-60", "kind": "linear_regression", "selected_data_sets": [1] }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-lin-60"))
        .scalar("Slope", Some(1), 1e-6, 1e-6)
        .scalar("YIntercept", Some(1), 1e-6, 1e-6)
        .scalar("R2", Some(1), 1e-6, 1e-6),
    )?;

    // Stage 5: Re-include outlier row 6 on Col 1 via Cmd+E -> linear slope and fitted curve SVG update reactively.
    let sid_for_edit = sid;
    let cell_reincl = w_start;
    run.live_edit_all(
        LiveEditTarget::gui(move |r| {
            r.select_sheet(&sid_for_edit)?;
            r.click(&cell_reincl)?;
            r.press_key("E", KeyModifiers { meta: true, ..Default::default() })
        }),
        vec![LiveEditExpectation::scalar_changed(lin_id, "Slope", Some(1), 1e-2), LiveEditExpectation::graph_svg_changed(gs_id)],
    )?;

    run.finish()
}

#[test]
fn wf61_human_per_column_normalized_heatmap_and_vertical_column_titles() -> TestRes {
    let mut run = WorkflowRun::start("WF-61");
    run.set_viewport(ViewportMetrics::new(1600.0, 900.0, 1.0));
    let fx = wf61_multivariate_biomarker_grouped();

    // Grouped table: starts at cols=3, rows=12, subcols=3 -> RawReplicates(2) (min stepper=2), +10 cols = 13 analytes, -6 rows = 6 conditions.
    let mut extras = vec!["welcome.subcol.raw", "welcome.stepper.replicates.dec"];
    extras.extend(std::iter::repeat_n("welcome.stepper.cols.inc", fx.analyte_titles.len() - 3));
    extras.extend(std::iter::repeat_n("welcome.stepper.rows.dec", 12 - fx.row_titles.len()));
    let sid = run.create_table_via_welcome("welcome.arch.grouped", fx.title, &extras)?;

    for (r, &rt) in fx.row_titles.iter().enumerate() {
        run.enter_row_title(r, rt)?;
    }
    // Paste full 6x13 matrix into subcol 0 of each column (`val\t` across 2-subcol RawReplicates(2) layout).
    run.click("grid.cell.0.0.0")?;
    let tsv = fx.rows.iter().map(|row| row.iter().map(|v| format!("{v}\t")).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n");
    run.paste_clipboard(&tsv)?;

    for (c, &at) in fx.analyte_titles.iter().enumerate() {
        let wid = format!("grid.col_title.{c}");
        for _ in 0..10 {
            if run.frame_meta().entries.iter().any(|e| e.widget_id == wid && e.clipped_rect.is_positive()) {
                break;
            }
            if let Some(pos) = run
                .frame_meta()
                .entries
                .iter()
                .find(|e| e.widget_id.starts_with("grid.cell.") && e.clipped_rect.is_positive())
                .map(|e| e.clipped_rect.center())
            {
                run.wheel(pos, egui::vec2(-260.0, 0.0))?;
            }
        }
        run.enter_column_title(c, at)?;
    }

    // Switch raw grouped graph to Viridis heatmap & verify zero clipping/warnings.
    let g_raw_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&g_raw_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.diag.heatmap")?;
    run.commit_modal_ok()?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.color_mapping")?;
    run.click("format_graph.heatmap.cmap.Viridis")?;
    run.commit_modal_ok()?;
    let _raw_svg = graph_svg(run.document(), g_raw_id.as_str())?;

    // Run Analyze > Normalize (0%-100% per dataset + `[x] Create a new graph of the results`), creating linked Heatmap directly.
    run.select_sheet(&sid)?;
    let norm_id = run.run_analyze_entry("Normalize", "analyze.entry.transform.normalize")?;
    run.select_sheet(&norm_id)?;
    run.open_cmd_t()?;
    ensure(
        run.frame_meta().find_by_widget_id("edit.transform.norm_percent").is_some()
            && run.frame_meta().find_by_widget_id("edit.transform.norm_per_dataset").is_some()
            && run.frame_meta().find_by_widget_id("edit.transform.normalize.create_graph").is_some(),
        "WF-61 Normalize Cmd+T modal must expose norm_percent, norm_per_dataset, and normalize.create_graph",
    )?;
    run.click("edit.transform.normalize.create_graph")?;
    run.commit_modal_ok()?;

    let g_norm_id = graph_for_source(run.document(), &norm_id)?;
    run.select_sheet(&g_norm_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.diag.heatmap")?;
    run.commit_modal_ok()?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.heatmap_labels")?;
    run.click("format_graph.heatmap.angle.90")?;
    run.commit_modal_ok()?;

    {
        let gs = run.document().graph_sheet(&g_norm_id).ok_or("WF-61 missing g_norm")?;
        ensure(gs.spec.heatmap_labels.column_angle == HeatmapColumnLabelAngle::Vertical90, "WF-61 expected Vertical90 column labels")?;
        let scene = compile_graph(gs, run.document()).map_err(|e| e.to_string())?;
        ensure(scene.warnings.is_empty(), format!("WF-61 normalized heatmap warnings: {:?}", scene.warnings))?;
        let _svg = graph_svg(run.document(), g_norm_id.as_str())?;
    }

    // Stage 4: verify 0%-100% per-column normalization + Vertical90 heatmap against headless Session oracle.
    let mut ref_grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), fx.analyte_titles.len(), fx.row_titles.len()).map_err(|e| e.to_string())?;
    for (r, &rt) in fx.row_titles.iter().enumerate() {
        ref_grp.row_titles[r] = rt.to_owned();
    }
    for (c, &at) in fx.analyte_titles.iter().enumerate() {
        ref_grp.data_sets[c].title = at.to_owned();
    }
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            ref_grp.data_sets[c].cells[r][0] = Cell::number(val);
        }
    }
    let ref_sheet = DataSheet::new("ref-grp-61", fx.title, TableArchetype::Grouped(ref_grp)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(norm_id.clone(), |sess| {
            sess.execute(
                "transform.run",
                json!({
                    "sheet_id": "ref-norm-61",
                    "source_sheet_id": "ref-grp-61",
                    "kind": "normalize",
                    "zero_anchor": "smallest",
                    "hundred_anchor": "largest",
                    "as_fraction": false,
                    "per_data_set": true
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-norm-61", "data_sheet_id": "ref-norm-61", "kind": "heatmap" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setSpec", json!({ "sheet_id": "ref-g-norm-61", "heatmap_labels": { "column_angle": "vertical_90" } }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-norm-61"))
        .scalar("ZeroAnchor", Some(0), 1e-6, 1e-6)
        .scalar("HundredAnchor", Some(0), 1e-6, 1e-6)
        .scalar("HundredAnchor", Some(12), 1e-6, 1e-6)
        .with_graph(g_norm_id.clone())
        .ref_graph_id(SheetId::new("ref-g-norm-61"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.heatmap_labels.column_angle == ref_gs.spec.heatmap_labels.column_angle, "WF-61 angle mismatch vs ref")
        }),
    )?;

    // Stage 5: peak rescaling live edit recomputes Normalize anchors, analyzable_data & normalized heatmap SVG.
    let norm_for_chk = norm_id.clone();
    let sid_for_edit = sid.clone();
    run.live_edit_all(
        LiveEditTarget::gui(move |r| {
            r.select_sheet(&sid_for_edit)?;
            for _ in 0..10 {
                if r.frame_meta().entries.iter().any(|e| e.widget_id == "grid.cell.3.0.0" && e.clipped_rect.is_positive()) {
                    break;
                }
                if let Some(pos) = r
                    .frame_meta()
                    .entries
                    .iter()
                    .find(|e| e.widget_id.starts_with("grid.cell.") && e.clipped_rect.is_positive())
                    .map(|e| e.clipped_rect.center())
                {
                    r.wheel(pos, egui::vec2(260.0, 0.0))?;
                }
            }
            r.enter_grid_cell(3, 0, 0, "950.0")
        }),
        vec![
            LiveEditExpectation::scalar_changed(norm_id, "HundredAnchor", Some(0), 100.0),
            LiveEditExpectation::custom(move |_, _, doc| {
                let rs = doc.result_sheet(&norm_for_chk).ok_or("WF-61 missing Normalize ResultSheet")?;
                let ds = rs.analyzable_data.as_ref().ok_or("WF-61 missing Normalize analyzable_data")?;
                let v = match &ds.archetype {
                    TableArchetype::Grouped(g) => g.data_sets[0].cells[3][0].active_number(),
                    _ => None,
                }
                .ok_or("WF-61 normalized peak cell not numeric")?;
                ensure((v - 100.0).abs() < 1e-6, format!("WF-61 new peak row 3 must normalize to 100.0%, got {v}"))
            }),
            LiveEditExpectation::graph_svg_changed(g_norm_id.clone()),
        ],
    )?;

    run.select_sheet(&g_norm_id)?;
    run.finish()
}

#[test]
fn wf62_human_five_design_nested_paired_and_rm_anova_trial_comparison() -> TestRes {
    let mut run = WorkflowRun::start("WF-62");
    run.set_viewport(ViewportMetrics::new(1600.0, 900.0, 1.0));
    let fx = wf62_five_design_trial();

    // 1) Nested hierarchical t-test vs flattened Column unpaired t-test
    let nest_id = run.create_table_via_welcome("welcome.arch.nested", fx.nested_title, &["welcome.stepper.cols.dec", "welcome.nested.depth.inc"])?;
    for (group_idx, subjects) in fx.nested_values.iter().enumerate() {
        for (subject_idx, reps) in subjects.iter().enumerate() {
            for (rep_idx, &val) in reps.iter().enumerate() {
                enter_cell_scroll_h(&mut run, rep_idx, group_idx, subject_idx, &format!("{val}"))?;
            }
        }
    }
    run.select_sheet(&nest_id)?;
    let nest_res = run.run_analyze_entry("Nested", "analyze.entry.stats.nested")?;
    run.select_sheet(&nest_res)?;
    run.open_cmd_t()?;
    run.click("edit.nested.t")?;
    run.commit_modal_ok()?;

    let flat_id = run.create_table_via_welcome("welcome.arch.column", "WF62 Flattened", &[])?;
    for (group_idx, subjects) in fx.nested_values.iter().enumerate() {
        let mut flat_r = 0usize;
        for reps in subjects {
            for &val in reps {
                run.enter_grid_cell(flat_r, group_idx, 0, &format!("{val}"))?;
                flat_r += 1;
            }
        }
    }
    run.select_sheet(&flat_id)?;
    let flat_res = run.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;

    {
        let nr = run.document().result_sheet(&nest_res).ok_or("WF-62 missing nest_res")?;
        let fr = run.document().result_sheet(&flat_res).ok_or("WF-62 missing flat_res")?;
        ensure(matches!(nr.status, ResultStatus::UpToDate), "WF-62 nested_t not UpToDate")?;
        ensure(matches!(fr.status, ResultStatus::UpToDate), "WF-62 flat unpaired_t not UpToDate")?;
        let nest_df = nr.scalars.iter().find(|s| s.key == "Group denominator df").ok_or("WF-62 missing Nested Group denominator df")?.value;
        ensure((nest_df - 6.0).abs() < 1e-6, format!("WF-62 expected subject-level df=6, got {nest_df}"))?;
        let nested_t = nr.scalars.iter().find(|s| s.key == "Nested t").ok_or("WF-62 missing Nested t")?.value;
        let flat_t = fr.scalars.iter().find(|s| s.key == "t").ok_or("WF-62 missing flat unpaired t")?.value;
        ensure((nested_t - flat_t).abs() > 1e-3, "WF-62 nested t vs flattened unpaired t must differ")?;
    }

    // 2) Paired two-group column design (`cols` remains 2 from Welcome state)
    let paired_tbl = run.create_table_via_welcome("welcome.arch.column", fx.paired_title, &["welcome.design.paired"])?;
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        run.enter_grid_cell(r, 0, 0, &format!("{}", pair[0]))?;
        run.enter_grid_cell(r, 1, 0, &format!("{}", pair[1]))?;
    }
    run.select_sheet(&paired_tbl)?;
    let paired_res = run.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    run.select_sheet(&paired_res)?;
    run.open_cmd_t()?;
    run.click("edit.paired_or_matched")?;
    run.click("edit.tg.paired_t")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(run.document().result_sheet(&paired_res).ok_or("WF-62 paired_res")?.status, ResultStatus::UpToDate),
        "WF-62 paired_t must be UpToDate",
    )?;

    // 3) Longitudinal Two-Way RM ANOVA: BothFactorsMatched vs RowFactorMatched (unconditional clicks — no guarded find_by_widget_id!)
    let rm_tbl = run.create_table_via_welcome("welcome.arch.grouped", fx.rm_title, &["welcome.subcol.raw"])?;
    for (r, flat_vals) in fx.rm_rows.iter().enumerate() {
        for (flat_idx, &val) in flat_vals.iter().enumerate() {
            enter_cell_scroll_h(&mut run, r, flat_idx / 4, flat_idx % 4, &format!("{val}"))?;
        }
    }
    run.select_sheet(&rm_tbl)?;
    let rm_res = run.run_analyze_entry("Two-Way", "analyze.entry.stats.factorial_anova")?;
    run.select_sheet(&rm_res)?;
    run.open_cmd_t()?;
    run.click("edit.paired_or_matched")?;
    run.click("edit.tab.repeated_measures")?;
    run.click("edit.fac.rm.both")?;
    run.commit_modal_ok()?;
    let rm_both_tabs = format!("{:?}", run.document().result_sheet(&rm_res).ok_or("WF-62 rm both")?.tabs);

    run.open_cmd_t()?;
    run.click("edit.tab.repeated_measures")?;
    run.click("edit.fac.rm.row")?;
    run.commit_modal_ok()?;
    let rm_row_rs = run.document().result_sheet(&rm_res).ok_or("WF-62 rm row")?;
    ensure(matches!(rm_row_rs.status, ResultStatus::UpToDate), "WF-62 RM row factor not UpToDate")?;
    ensure(format!("{:?}", rm_row_rs.tabs) != rm_both_tabs, "WF-62 RM both vs row factor must differ")?;

    // Stage 4: verify Nested t-test, Paired t-test, and Two-Way RM ANOVA against headless Session oracles.
    let mut ref_nest = NestedTable::new(2, fx.nested_subjects_per_group, fx.nested_reps_per_subject).map_err(|e| e.to_string())?;
    for (g, subs) in fx.nested_values.iter().enumerate() {
        for (s, reps) in subs.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                ref_nest.groups[g].cells[r][s] = Cell::number(v);
            }
        }
    }
    let ref_nest_sheet = DataSheet::new("ref-nest-62", fx.nested_title, TableArchetype::Nested(ref_nest)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(nest_res.clone(), |sess| {
            sess.execute("analysis.run", json!({ "sheet_id": "ref-nest-res-62", "source_sheet_id": "ref-nest-62", "kind": "nested_t" }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_nest_sheet)?
        .ref_result_id(SheetId::new("ref-nest-res-62"))
        .scalar("Nested t", None, 1e-6, 1e-6)
        .scalar("P value (groups)", None, 1e-6, 1e-6)
        .scalar("Group denominator df", None, 1e-9, 1e-9)
        .scalar("Subcluster variance", None, 1e-6, 1e-6),
    )?;

    let mut ref_rm = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, fx.rm_rows.len()).map_err(|e| e.to_string())?;
    for (r, flat_vals) in fx.rm_rows.iter().enumerate() {
        for (flat_idx, &v) in flat_vals.iter().enumerate() {
            ref_rm.data_sets[flat_idx / 4].cells[r][flat_idx % 4] = Cell::number(v);
        }
    }
    let ref_rm_sheet = DataSheet::new("ref-rm-62", fx.rm_title, TableArchetype::Grouped(ref_rm)).map_err(|e| e.to_string())?;
    let rm_row_kind = serde_json::to_value(AnalysisKind::FactorialAnova(curvebook_doc::FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
        ..curvebook_doc::FactorialAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rm_res, move |sess| {
            sess.execute("analysis.run", json!({ "sheet_id": "ref-rm-res-62", "source_sheet_id": "ref-rm-62", "kind": rm_row_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_rm_sheet)?
        .ref_result_id(SheetId::new("ref-rm-res-62"))
        .scalar("MS residual", None, 1e-6, 1e-6)
        .tab_cell(0, 0, 2, 1e-6, 1e-6),
    )?;

    // Stage 5: live edit in nested replicate cell recomputes Nested t-test P value & summary tab.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(nest_id, 0, 0, 0, "24.8"),
        vec![LiveEditExpectation::scalar_changed(nest_res, "P value (groups)", None, 1e-4)],
    )?;

    run.select_sheet(&paired_res)?;
    run.finish()
}

#[test]
fn wf63_human_forest_plot_upper_lower_limits_and_null_reference_line() -> TestRes {
    let mut run = WorkflowRun::start("WF-63");
    let fx = wf63_meta_analysis_forest_grouped();
    let sid = run.create_table_via_welcome(
        "welcome.arch.grouped",
        fx.title,
        &["welcome.subcol.upper_lower", "welcome.stepper.cols.dec", "welcome.stepper.cols.dec"],
    )?;

    for (r, &study) in fx.study_titles.iter().enumerate() {
        run.enter_row_title(r, study)?;
    }
    for (r, entry) in fx.entries.iter().enumerate() {
        run.enter_grid_cell(r, 0, 0, &format!("{}", entry[0]))?;
        run.enter_grid_cell(r, 0, 1, &format!("{}", entry[1]))?;
        run.enter_grid_cell(r, 0, 2, &format!("{}", entry[2]))?;
    }

    let gs_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&gs_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.diag.forest")?;
    run.commit_modal_ok()?;

    ensure(
        !run.document().graph_sheet(&gs_id).ok_or("WF-63 missing graph")?.spec.forest_show_pooled_diamond,
        "WF-63 forest_show_pooled_diamond must default to false",
    )?;
    let svg_no_diamond = graph_svg(run.document(), gs_id.as_str())?;

    // Toggle pooled diamond on via Format Graph (`R13.10` forest_show_pooled_diamond).
    run.click("toolbar.format_graph")?;
    run.click("format_graph.forest_show_pooled_diamond")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().graph_sheet(&gs_id).ok_or("WF-63 missing graph")?.spec.forest_show_pooled_diamond,
        "WF-63 format_graph.forest_show_pooled_diamond toggle must enable pooled diamond",
    )?;
    let svg_with_diamond = graph_svg(run.document(), gs_id.as_str())?;
    ensure(svg_with_diamond != svg_no_diamond, "WF-63 pooled diamond toggle must alter forest plot SVG")?;

    // Add X = 1.0 dotted "No Effect" vertical reference line via Format Axes > Additional Ticks (`draw_gridline` defaults true; click twice to verify toggle).
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.x")?;
    run.click("axes.additional_ticks.open")?;
    run.replace_textbox("axes.add_tick.value", "1.0")?;
    run.replace_textbox("axes.add_tick.label", "No Effect")?;
    run.click("axes.add_tick.gridline")?;
    run.click("axes.add_tick.gridline")?;
    run.click("axes.add_tick.save")?;
    run.click("axes.add_tick.close")?;
    run.commit_modal_ok()?;

    {
        let gs = run.document().graph_sheet(&gs_id).ok_or("WF-63 missing graph")?;
        let tick =
            gs.spec.x_axis.additional_ticks.iter().find(|t| t.label.as_deref() == Some("No Effect")).ok_or("WF-63 custom No Effect tick missing")?;
        ensure((tick.value - 1.0).abs() < 1e-9 && tick.draw_gridline, "WF-63 additional tick must be at X=1.0 with gridline=true")?;
        let scene = compile_graph(gs, run.document()).map_err(|e| e.to_string())?;
        let has_dotted =
            scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(st), .. } if st.dash == LineDashStyle::Dotted));
        ensure(has_dotted, "WF-63 compiled Forest scene must emit dotted vertical reference line")?;
        let svg = graph_svg(run.document(), gs_id.as_str())?;
        ensure(svg.contains("No Effect") && svg.contains("stroke-dasharray"), "WF-63 SVG must contain No Effect label and dotted rule")?;
    }

    // Stage 4: verify Forest Plot config against headless Session oracle.
    let mut ref_grp = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, fx.entries.len()).map_err(|e| e.to_string())?;
    for (r, &study) in fx.study_titles.iter().enumerate() {
        ref_grp.row_titles[r] = study.to_owned();
    }
    for (r, entry) in fx.entries.iter().enumerate() {
        ref_grp.data_sets[0].cells[r][0] = Cell::number(entry[0]);
        ref_grp.data_sets[0].cells[r][1] = Cell::number(entry[1]);
        ref_grp.data_sets[0].cells[r][2] = Cell::number(entry[2]);
    }
    let ref_sheet = DataSheet::new("ref-grp-63", fx.title, TableArchetype::Grouped(ref_grp)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_graph(gs_id.clone(), |sess| {
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-63", "data_sheet_id": "ref-grp-63", "kind": "grouped" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-63", "kind": "diagnostic", "option_id": "diag.forest" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setSpec", json!({ "sheet_id": "ref-g-63", "forest_show_pooled_diamond": true })).map_err(|e| e.to_string())?;
            sess.execute(
                "graph.setAxes",
                json!({ "sheet_id": "ref-g-63", "x_axis": { "additional_ticks": [{ "value": 1.0, "label": "No Effect", "draw_gridline": true }] } }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_graph_id(SheetId::new("ref-g-63"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot), "WF-63 kind mismatch")?;
            ensure(gui_gs.spec.forest_show_pooled_diamond == ref_gs.spec.forest_show_pooled_diamond, "WF-63 pooled diamond mismatch vs ref")?;
            ensure(gui_gs.spec.x_axis.additional_ticks == ref_gs.spec.x_axis.additional_ticks, "WF-63 additional ticks mismatch vs ref")
        }),
    )?;

    // Stage 5: live upper-CI cell edit updates Forest Plot SVG.
    run.live_edit(LiveEditTarget::sheet_grid_cell(sid, 0, 0, 2, "1.35"), LiveEditExpectation::graph_svg_changed(gs_id.clone()))?;

    run.select_sheet(&gs_id)?;
    run.finish()
}

#[test]
fn wf64_human_population_pyramid_horizontal_stacked_and_unsigned_ticks() -> TestRes {
    let mut run = WorkflowRun::start("WF-64");
    let fx = wf64_demographics_pyramid_grouped();
    let sid = run.create_table_via_welcome("welcome.arch.grouped", fx.title, &["welcome.subcol.single", "welcome.stepper.cols.dec"])?;

    run.click("grid.row_titles.toggle")?;
    ensure(run.app().grid.row_titles_collapsed_override == Some(true), "WF-64 Row Titles toggle must collapse")?;
    run.click("grid.row_titles.toggle")?;
    ensure(run.app().grid.row_titles_collapsed_override == Some(false), "WF-64 Row Titles toggle must expand again")?;

    for (r, &band) in fx.band_titles.iter().enumerate() {
        run.enter_row_title(r, band)?;
    }
    for (c, &cohort) in fx.cohort_titles.iter().enumerate() {
        run.enter_column_title(c, cohort)?;
    }
    for (r, row) in fx.rows.iter().enumerate() {
        run.enter_grid_cell(r, 0, 0, &format!("{}", row[0]))?;
        run.enter_grid_cell(r, 1, 0, &format!("{}", row[1]))?;
    }

    let gs_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&gs_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.grouped.stacked")?;
    run.commit_modal_ok()?;

    // Switch to Horizontal (`R13.10`) + All Data Sets 0.5 pt bar borders (`bar_border_width_pt` + `Apply Globally`).
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.series")?;
    run.click("format_graph.orientation.horizontal")?;
    run.click("edit.tab.format_graph.appearance")?;
    run.click("format_graph.apply_globally")?;
    set_scrub_field_wf(&mut run, "format_graph.bar_border_width_pt", 56.0, "0.5")?;
    run.commit_modal_ok()?;

    // Set unsigned X ticks (`|x| Abs`) confirming categorical guard.
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.x")?;
    run.click("axes.tick_format.abs_decimal")?;
    run.click("axes.categorical_guard.confirm_checkbox")?;
    run.click("axes.categorical_guard.confirm_btn")?;
    run.commit_modal_ok()?;

    {
        let gs = run.document().graph_sheet(&gs_id).ok_or("WF-64 missing graph")?;
        ensure(gs.spec.orientation == CartesianOrientation::Horizontal, "WF-64 must keep Horizontal orientation")?;
        ensure(gs.spec.grouped_arrangement == GroupedArrangement::Stacked, "WF-64 must be Stacked")?;
        ensure(gs.spec.x_axis.tick_format == TickLabelFormat::AbsoluteDecimal, "WF-64 must persist AbsoluteDecimal X ticks")?;
        for (idx, s) in gs.spec.series.iter().enumerate() {
            ensure(
                matches!(s.gdo, GraphicalDataObject::BarWithReplicates { border_width_pt, .. } if (border_width_pt - 0.5).abs() < 1e-6),
                format!("WF-64 series {idx} must have 0.5 pt bar border"),
            )?;
        }
        let scene = compile_graph(gs, run.document()).map_err(|e| e.to_string())?;
        let has_negative_tick = scene.primitives.iter().any(|p| match p {
            ScenePrimitive::Text(t) => {
                let txt = t.rich.plain_text();
                let trimmed = txt.trim();
                trimmed.strip_prefix('-').is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
            }
            _ => false,
        });
        ensure(!has_negative_tick, "WF-64 unsigned |x| pyramid must not emit negative numeric tick labels")?;
    }

    // Stage 4: verify Horizontal Stacked + 0.5pt borders + AbsoluteDecimal against headless Session oracle.
    let mut ref_grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), fx.cohort_titles.len(), fx.band_titles.len()).map_err(|e| e.to_string())?;
    for (r, &band) in fx.band_titles.iter().enumerate() {
        ref_grp.row_titles[r] = band.to_owned();
    }
    for (c, &cohort) in fx.cohort_titles.iter().enumerate() {
        ref_grp.data_sets[c].title = cohort.to_owned();
    }
    for (r, row) in fx.rows.iter().enumerate() {
        ref_grp.data_sets[0].cells[r][0] = Cell::number(row[0]);
        ref_grp.data_sets[1].cells[r][0] = Cell::number(row[1]);
    }
    let ref_sheet = DataSheet::new("ref-grp-64", fx.title, TableArchetype::Grouped(ref_grp)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_graph(gs_id.clone(), |sess| {
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-64", "data_sheet_id": "ref-grp-64", "kind": "grouped" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-64", "kind": "grouped", "option_id": "grouped.stacked" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setSpec", json!({ "sheet_id": "ref-g-64", "orientation": "horizontal" })).map_err(|e| e.to_string())?;
            sess.execute("graph.setSeries", json!({ "sheet_id": "ref-g-64", "all_series": true, "border_width_pt": 0.5 }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setAxes", json!({ "sheet_id": "ref-g-64", "x_axis": { "tick_format": "absolute_decimal" } }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_graph_id(SheetId::new("ref-g-64"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.orientation == ref_gs.spec.orientation, "WF-64 orientation mismatch")?;
            ensure(gui_gs.spec.grouped_arrangement == ref_gs.spec.grouped_arrangement, "WF-64 arrangement mismatch")?;
            ensure(gui_gs.spec.x_axis.tick_format == ref_gs.spec.x_axis.tick_format, "WF-64 x tick_format mismatch")
        }),
    )?;

    // Stage 5: negative cohort cell edit updates Horizontal Stacked pyramid SVG.
    run.live_edit(LiveEditTarget::sheet_grid_cell(sid, 2, 0, 0, "-38.0"), LiveEditExpectation::graph_svg_changed(gs_id.clone()))?;

    run.select_sheet(&gs_id)?;
    run.finish()
}

#[test]
fn wf65_human_spaghetti_plot_replicate_trajectories_vs_subject_traces() -> TestRes {
    let mut run = WorkflowRun::start("WF-65");
    run.set_viewport(ViewportMetrics::new(1600.0, 900.0, 1.0));
    let fx = wf65_longitudinal_spaghetti_xy();

    // Part (a): XY RawReplicates(4) with IndividualReplicateTrajectories & Show Symbols unchecked (`R13.10`).
    let rep_sid = run.create_table_via_welcome(
        "welcome.arch.xy",
        fx.title,
        &["welcome.subcol.raw", "welcome.stepper.cols.dec", "welcome.stepper.replicates.inc"],
    )?;
    for (g, &title) in fx.cohort_titles.iter().enumerate() {
        run.enter_column_title(g, title)?;
    }
    for (r, &week) in fx.weeks.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{week}"))?;
        for (g, reps) in fx.replicate_trajectories[r].iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                enter_cell_scroll_h(&mut run, r, g, s, &format!("{val}"))?;
            }
        }
    }

    let g_rep_id = graph_for_source(run.document(), &rep_sid)?;
    run.select_sheet(&g_rep_id)?;
    run.click("toolbar.format_graph")?;
    run.click("format_graph.apply_globally")?;
    run.click("format_graph.show_symbols")?;
    run.click("format_graph.replicate_lines.individual_trajectories")?;
    run.commit_modal_ok()?;

    let scene = compile_graph(run.document().graph_sheet(&g_rep_id).ok_or("WF-65 graph")?, run.document()).map_err(|e| e.to_string())?;
    let first_pt = scene.plotted_points.first().ok_or("WF-65 expected spaghetti plotted_points")?;
    for (idx, s) in run.document().graph_sheet(&g_rep_id).ok_or("missing g-rep")?.spec.series.iter().enumerate() {
        ensure(
            matches!(
                s.gdo,
                GraphicalDataObject::ScatterAndLine { marker: None, replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories, .. }
            ),
            format!("WF-65 series {idx} expected marker=None & IndividualReplicateTrajectories"),
        )?;
    }

    let canvas_rect = run.frame_meta().find_by_widget_id("canvas.viewport").ok_or("WF-65 missing canvas.viewport")?.clipped_rect;
    let margin = 16.0_f32;
    let avail_w = (canvas_rect.width() - margin * 2.0).max(80.0);
    let avail_h = (canvas_rect.height() - margin * 2.0).max(80.0);
    let sx = (avail_w / (scene.width_pt.max(1.0) as f32)).min(avail_h / (scene.height_pt.max(1.0) as f32));
    let paper_min =
        egui::pos2(canvas_rect.center().x - (scene.width_pt as f32) * sx * 0.5, canvas_rect.center().y - (scene.height_pt as f32) * sx * 0.5);
    let screen_pt = egui::pos2(paper_min.x + (first_pt.center_pt.x as f32) * sx, paper_min.y + (first_pt.center_pt.y as f32) * sx);
    run.hover_at(screen_pt)?;
    let readout = run.state().views.canvas_view.hover_readout.as_ref().ok_or("WF-65 expected spaghetti hover readout")?;
    ensure(readout.label.contains("Rep "), format!("WF-65 expected Rep N hover readout, got `{}`", readout.label))?;

    // Part (b): single-Y per-subject columns (`cols` starts at 2 -> +2 = 4) with editable headers & palette theme.
    let subj_sid = run.create_table_via_welcome(
        "welcome.arch.xy",
        "Single-Y Subject Traces",
        &["welcome.subcol.single", "welcome.stepper.cols.inc", "welcome.stepper.cols.inc"],
    )?;
    for (c, &title) in fx.subject_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (r, &week) in fx.weeks.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{week}"))?;
        for (c, &val) in fx.single_y_subjects[r].iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{val}"))?;
        }
    }
    let g_subj_id = graph_for_source(run.document(), &subj_sid)?;
    run.select_sheet(&g_subj_id)?;
    run.click("toolbar.format_graph")?;
    run.click("format_graph.apply_globally")?;
    run.click("format_graph.show_symbols")?;
    run.click("edit.tab.format_graph.settings")?;
    run.click("format_graph.palette.PaulTolBright")?;
    run.commit_modal_ok()?;
    let _subj_svg = graph_svg(run.document(), g_subj_id.as_str())?;

    // Stage 4: verify Part (a) replicate trajectories graph against headless Session oracle.
    let mut ref_xy = XyTable::new(SubcolumnMode::RawReplicates(4), fx.cohort_titles.len(), fx.weeks.len()).map_err(|e| e.to_string())?;
    for (g, &title) in fx.cohort_titles.iter().enumerate() {
        ref_xy.data_sets[g].title = title.to_owned();
    }
    for (r, &week) in fx.weeks.iter().enumerate() {
        ref_xy.set_x_cell(r, Cell::number(week)).map_err(|e| e.to_string())?;
        for (g, reps) in fx.replicate_trajectories[r].iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                ref_xy.data_sets[g].cells[r][s] = Cell::number(val);
            }
        }
    }
    let ref_sheet = DataSheet::new("ref-rep-65", fx.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_graph(g_rep_id.clone(), |sess| {
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-rep-65", "data_sheet_id": "ref-rep-65", "kind": "xy" }))
                .map_err(|e| e.to_string())?;
            sess.execute(
                "graph.setSeries",
                json!({ "sheet_id": "ref-g-rep-65", "all_series": true, "marker": null, "replicate_lines": "individual_replicate_trajectories" }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_graph_id(SheetId::new("ref-g-rep-65"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.series.len() == ref_gs.spec.series.len(), "WF-65 series count mismatch")?;
            for (idx, (g, r)) in gui_gs.spec.series.iter().zip(ref_gs.spec.series.iter()).enumerate() {
                ensure(
                    matches!(
                        (&g.gdo, &r.gdo),
                        (
                            GraphicalDataObject::ScatterAndLine {
                                marker: None,
                                replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories,
                                ..
                            },
                            GraphicalDataObject::ScatterAndLine {
                                marker: None,
                                replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories,
                                ..
                            }
                        )
                    ),
                    format!("WF-65 series {idx} replicate GDO mismatch vs ref: gui={:?} ref={:?}", g.gdo, r.gdo),
                )?;
            }
            Ok(())
        }),
    )?;

    // Stage 5: single-subcolumn edit updates replicate spaghetti SVG while leaving independent subject graph invariant.
    let rep_for_edit = rep_sid.clone();
    run.live_edit_all(
        LiveEditTarget::gui(move |r| {
            r.select_sheet(&rep_for_edit)?;
            enter_cell_scroll_h(r, 2, 1, 3, "52.5")
        }),
        vec![LiveEditExpectation::graph_svg_changed(g_rep_id.clone()), LiveEditExpectation::graph_svg_invariant(g_subj_id)],
    )?;

    run.select_sheet(&g_rep_id)?;
    run.finish()
}
