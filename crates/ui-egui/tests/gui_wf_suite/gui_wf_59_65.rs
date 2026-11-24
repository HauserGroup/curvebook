//! Strict `--gui-only` end-to-end GUI acceptance workflows `WF-59`..`WF-65` (Shard 9, `R13.6`).
//!
//! Every workflow boots [`UiApp::new()`] with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`),
//! seeds clean-room synthetic fixtures from `curvebook_testkit::r13_datasets`, and drives all
//! interactions exclusively through [`curvebook_ui_egui::dispatch_gui_action`]: `ui.invoke_menu`,
//! `ui.switch_tab`, `ui.type_text`, `ui.answer_dialog`, `ui.inspect`, and `ui.screenshot`. Zero
//! direct `app.execute_command` calls appear anywhere in this suite.
#![forbid(unsafe_code)]

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    CartesianOrientation, GraphicalDataObject, GroupedArrangement, HeatmapColumnLabelAngle, LayoutItem, ProjectDocument, ReplicateLineMode,
    ResultStatus, SheetId, SubcolumnMode, TickDirection, TickLabelFormat,
};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_testkit::r13_datasets::{
    wf59_paired_before_after_and_inset, wf60_subset_linear_with_outliers_xy, wf61_multivariate_biomarker_grouped, wf62_five_design_trial,
    wf63_meta_analysis_forest_grouped, wf64_demographics_pyramid_grouped, wf65_longitudinal_spaghetti_xy,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

use crate::common::{decode_base64, default_metrics, ensure, new_strict_gui_app};

type TestResult = Result<(), String>;

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed: {err}"))
}

fn menu(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "command_id": command_id, "params": params }))
}

fn switch_sheet(app: &mut UiApp, sheet_id: &str) -> Result<Value, String> {
    gui(app, "ui.switch_tab", json!({ "sheet_id": sheet_id }))
}

fn type_grid_cell(app: &mut UiApp, sheet_id: &str, row: usize, col: usize, subcol: usize, text: &str) -> TestResult {
    if app.document().active_sheet_id.as_ref().map(SheetId::as_str) != Some(sheet_id) {
        switch_sheet(app, sheet_id)?;
    }
    let widget_id = format!("grid.cell.{row}.{col}.{subcol}");
    gui(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "commit_enter": true }))?;
    Ok(())
}

fn type_x_cell(app: &mut UiApp, sheet_id: &str, row: usize, subcol: usize, text: &str) -> TestResult {
    if app.document().active_sheet_id.as_ref().map(SheetId::as_str) != Some(sheet_id) {
        switch_sheet(app, sheet_id)?;
    }
    let widget_id = format!("grid.xcell.{row}.{subcol}");
    gui(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "commit_enter": true }))?;
    Ok(())
}

fn run_analyze_dialog(app: &mut UiApp, data_sheet_id: &str, fields: Value) -> Result<String, String> {
    switch_sheet(app, data_sheet_id)?;
    menu(app, "ui.analyze.open", json!({ "sheet_id": data_sheet_id }))?;
    let out = gui(app, "ui.answer_dialog", json!({ "fields": fields, "force_create_new": true, "action": "ok" }))?;
    let res_id = out
        .get("payload")
        .and_then(|p| p.get("sheet_id"))
        .and_then(Value::as_str)
        .or_else(|| app.document().active_sheet_id.as_ref().map(SheetId::as_str))
        .ok_or_else(|| String::from("Analyze dialog did not return or select a result sheet_id"))?;
    Ok(res_id.to_owned())
}

fn compile_clean(doc: &ProjectDocument, graph_id: &str) -> Result<ChartScene, String> {
    let gs = doc.graph_sheet(&SheetId::new(graph_id)).ok_or_else(|| format!("missing graph sheet `{graph_id}`"))?;
    let scene = compile_graph(gs, doc).map_err(|err| format!("compile_graph({graph_id}) failed: {err}"))?;
    ensure(!scene.primitives.is_empty(), format!("compiled scene `{graph_id}` unexpectedly empty"))?;
    ensure(scene.warnings.is_empty(), format!("unexpected warnings on `{graph_id}`: {:?}", scene.warnings))?;
    Ok(scene)
}

fn assert_screenshot_png(app: &mut UiApp, min_w: u32, min_h: u32) -> Result<Vec<u8>, String> {
    let shot = gui(
        app,
        "ui.screenshot",
        json!({ "target": "canvas", "width_px": min_w.max(640), "height_px": min_h.max(400), "dpi": 144, "metrics": default_metrics() }),
    )?;
    ensure(shot["format"] == json!("png"), format!("expected png format, got {}", shot["format"]))?;
    ensure(shot["path"].is_null(), format!("expected null path under strict gui_only, got {}", shot["path"]))?;
    let b64 = shot.get("data_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing data_base64"))?;
    let png = decode_base64(b64)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "invalid PNG screenshot")?;
    Ok(png)
}

fn assert_strict_gui_only_guards(app: &mut UiApp) -> TestResult {
    ensure(app.is_gui_only(), "app must remain in GuiOnlyPolicy::Strict")?;
    let blocked = dispatch_gui_action(app, "ui.screenshot", json!({ "target": "workspace", "width_px": 320, "height_px": 240, "path": "shot.png" }));
    match blocked {
        Err(err) if err.to_string().to_ascii_lowercase().contains("gui") => Ok(()),
        other => Err(format!("expected gui_only violation on screenshot disk path override, got {other:?}")),
    }
}

fn scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn layout_svg(doc: &ProjectDocument, layout_id: &str) -> Result<String, String> {
    let lay = doc.layout_sheet(&SheetId::new(layout_id)).ok_or_else(|| format!("missing layout sheet `{layout_id}`"))?;
    let scene = compile_layout(lay, doc).map_err(|e| format!("compile_layout({layout_id}) failed: {e}"))?;
    let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    validate_svg_structure(&svg).map_err(|e| e.to_string())?;
    Ok(svg)
}

fn rendered_panel_plot_width_pt(doc: &ProjectDocument, layout_id: &str, panel_id: &str) -> Result<f64, String> {
    let lay = doc.layout_sheet(&SheetId::new(layout_id)).ok_or_else(|| format!("missing layout `{layout_id}`"))?;
    let item = lay
        .spec
        .items
        .iter()
        .find_map(|it| match it {
            LayoutItem::LiveGraph { id, graph_sheet_id, bounds_pt, scale_percent, .. } if id == panel_id => {
                Some((graph_sheet_id, bounds_pt, *scale_percent))
            }
            _ => None,
        })
        .ok_or_else(|| format!("missing live graph panel `{panel_id}` on `{layout_id}`"))?;
    let gs = doc.graph_sheet(item.0).ok_or_else(|| format!("missing graph `{}`", item.0))?;
    let sub = compile_graph(gs, doc).map_err(|e| format!("compile_graph failed: {e}"))?;
    let fit = (item.1.width / sub.width_pt.max(1.0)).min(item.1.height / sub.height_pt.max(1.0));
    let pct_frac = if item.2.is_finite() && item.2 > 0.0 { item.2 / 100.0 } else { 1.0 };
    Ok(sub.plot_rect_pt.width * fit * pct_frac)
}

// ---------------------------------------------------------------------------
// WF-59: Layout Equalize Scaling, Transparent Overlay, Align Origins & Inset
// ---------------------------------------------------------------------------

#[test]
fn wf59_layout_equalize_scaling_transparent_overlay_and_inset() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf59_paired_before_after_and_inset();

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "col-wf59", "title": fx.title, "kind": "column", "cols": 2, "rows": fx.paired_rows.len() }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "col-wf59", "titles": fx.paired_cols }))?;
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        type_grid_cell(&mut app, "col-wf59", r, 0, 0, &format!("{}", pair[0]))?;
        type_grid_cell(&mut app, "col-wf59", r, 1, 0, &format!("{}", pair[1]))?;
    }

    menu(&mut app, "graph.create", json!({ "sheet_id": "g-box-wf59", "data_sheet_id": "col-wf59", "kind": "column" }))?;
    menu(&mut app, "graph.setKind", json!({ "sheet_id": "g-box-wf59", "kind": "column", "option_id": "col.box_tukey" }))?;
    menu(&mut app, "graph.create", json!({ "sheet_id": "g-ba-wf59", "data_sheet_id": "col-wf59", "kind": "column" }))?;
    menu(&mut app, "graph.setKind", json!({ "sheet_id": "g-ba-wf59", "kind": "column", "option_id": "col.before_after" }))?;
    menu(
        &mut app,
        "graph.setAxes",
        json!({
            "sheet_id": "g-ba-wf59",
            "x_axis": { "tick_direction": "none", "show_tick_labels": false },
            "y_axis": { "tick_direction": "none", "show_tick_labels": false }
        }),
    )?;
    menu(&mut app, "graph.setSpec", json!({ "sheet_id": "g-ba-wf59", "paper_fill": null }))?;

    {
        let g_ba = app.document().graph_sheet(&SheetId::new("g-ba-wf59")).ok_or("missing g-ba-wf59")?;
        ensure(g_ba.spec.paper_fill.is_none(), "expected transparent paper_fill on before-after overlay")?;
        ensure(
            g_ba.spec.x_axis.tick_direction == TickDirection::None && !g_ba.spec.x_axis.show_tick_labels,
            "expected axis-free X settings on g-ba-wf59",
        )?;
        ensure(
            g_ba.spec.y_axis.tick_direction == TickDirection::None && !g_ba.spec.y_axis.show_tick_labels,
            "expected axis-free Y settings on g-ba-wf59",
        )?;
    }

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "xy-inset-wf59", "title": "Linearized Inset", "kind": "xy", "cols": 1, "rows": fx.inset_xy.len() }),
    )?;
    for (r, &(x, y)) in fx.inset_xy.iter().enumerate() {
        type_x_cell(&mut app, "xy-inset-wf59", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-inset-wf59", r, 0, 0, &format!("{y}"))?;
    }
    menu(&mut app, "graph.create", json!({ "sheet_id": "g-inset-wf59", "data_sheet_id": "xy-inset-wf59", "kind": "xy" }))?;

    menu(&mut app, "layout.create", json!({ "sheet_id": "lay-wf59", "title": "Overlay Layout WF-59", "preset": "double_column" }))?;
    menu(
        &mut app,
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-box", "kind": "graph", "graph_sheet_id": "g-box-wf59", "bounds_pt": [20.0, 20.0, 240.0, 170.0] }),
    )?;
    menu(
        &mut app,
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-ba", "kind": "graph", "graph_sheet_id": "g-ba-wf59", "bounds_pt": [20.0, 20.0, 180.0, 130.0] }),
    )?;
    menu(
        &mut app,
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-inset", "kind": "graph", "graph_sheet_id": "g-inset-wf59", "bounds_pt": [160.0, 30.0, 100.0, 80.0] }),
    )?;

    menu(&mut app, "ui.layout.equalizeScaling.open", json!({ "sheet_id": "lay-wf59" }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EqualizeScaling { .. }), "EqualizeScaling dialog must open")?;
    let eq_out =
        gui(&mut app, "ui.answer_dialog", json!({ "fields": { "mode": "match_smallest", "panel_ids": ["p-box", "p-ba"] }, "action": "ok" }))?;
    ensure(
        eq_out.pointer("/payload/panels").and_then(Value::as_array).is_some_and(|arr| arr.len() == 2),
        "EqualizeScaling should report both equalized panels",
    )?;
    let box_w = rendered_panel_plot_width_pt(app.document(), "lay-wf59", "p-box")?;
    let ba_w = rendered_panel_plot_width_pt(app.document(), "lay-wf59", "p-ba")?;
    ensure((box_w - ba_w).abs() <= 0.25, format!("equalized plot widths must match within 0.25 pt: {box_w} vs {ba_w}"))?;

    menu(&mut app, "layout.alignOrigins", json!({ "sheet_id": "lay-wf59", "align": true, "normalize_bounds": true }))?;
    menu(&mut app, "layout.reorderPanel", json!({ "sheet_id": "lay-wf59", "panel_id": "p-inset", "action": "bring_to_front" }))?;
    {
        let lay = app.document().layout_sheet(&SheetId::new("lay-wf59")).ok_or("missing lay-wf59")?;
        ensure(
            matches!(lay.spec.items.last(), Some(LayoutItem::LiveGraph { id, .. }) if id == "p-inset"),
            "bring_to_front must place p-inset last in z-order",
        )?;
    }

    let svg_before = layout_svg(app.document(), "lay-wf59")?;
    type_grid_cell(&mut app, "col-wf59", 0, 1, 0, "49.5")?;
    let svg_after = layout_svg(app.document(), "lay-wf59")?;
    ensure(svg_before != svg_after, "upstream edit must recompute composite layout SVG")?;

    switch_sheet(&mut app, "lay-wf59")?;
    let _png = assert_screenshot_png(&mut app, 640, 400)?;
    assert_strict_gui_only_guards(&mut app)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-60: Subset Linear Regression, Decoupled Points & Red Outlier Overrides
// ---------------------------------------------------------------------------

#[test]
fn wf60_subset_regression_decoupled_points_and_red_outlier_override() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf60_subset_linear_with_outliers_xy();

    menu(&mut app, "sheet.createData", json!({ "sheet_id": "xy-wf60", "title": fx.title, "kind": "xy", "cols": 2, "rows": fx.rows.len() }))?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "xy-wf60", "titles": ["All Points", "Included Subset"] }))?;
    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        type_x_cell(&mut app, "xy-wf60", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-wf60", r, 0, 0, &format!("{y}"))?;
        type_grid_cell(&mut app, "xy-wf60", r, 1, 0, &format!("{y}"))?;
    }

    menu(
        &mut app,
        "table.excludeCells",
        json!({
            "sheet_id": "xy-wf60",
            "cells": [
                { "row": fx.outlier_rows[0], "col": 1, "subcol": 0 },
                { "row": fx.outlier_rows[1], "col": 1, "subcol": 0 }
            ],
            "excluded": true
        }),
    )?;
    menu(
        &mut app,
        "analysis.run",
        json!({ "sheet_id": "res-lin-wf60", "source_sheet_id": "xy-wf60", "kind": "linear_regression", "selected_data_sets": [1] }),
    )?;
    menu(&mut app, "graph.create", json!({ "sheet_id": "g-wf60", "data_sheet_id": "xy-wf60", "kind": "xy" }))?;
    menu(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf60",
            "series_index": 1,
            "show_raw_data": false,
            "regression_overlay": {
                "result_sheet_id": "res-lin-wf60",
                "sample_segments": 256,
                "show_confidence_band": false,
                "show_prediction_band": false,
                "shaded_ribbon": false
            }
        }),
    )?;
    menu(
        &mut app,
        "graph.formatSelectedPoints",
        json!({
            "sheet_id": "g-wf60",
            "points": [
                { "series_index": 0, "point_index": fx.outlier_rows[0] },
                { "series_index": 0, "point_index": fx.outlier_rows[1] }
            ],
            "color": "#E11D48"
        }),
    )?;

    {
        let gs = app.document().graph_sheet(&SheetId::new("g-wf60")).ok_or("missing g-wf60")?;
        ensure(!gs.spec.series[1].show_raw_data, "series 1 raw markers must be decoupled via show_raw_data=false")?;
        ensure(gs.spec.point_overrides.len() == 2, format!("expected 2 outlier point overrides, got {}", gs.spec.point_overrides.len()))?;
        let _scene = compile_clean(app.document(), "g-wf60")?;
    }

    let summary_before = format!("{:?}", app.document().result_sheet(&SheetId::new("res-lin-wf60")).ok_or("missing res-lin-wf60")?.tabs);
    menu(
        &mut app,
        "table.excludeCells",
        json!({ "sheet_id": "xy-wf60", "cells": [{ "row": fx.outlier_rows[0], "col": 1, "subcol": 0 }], "excluded": false }),
    )?;
    let rs_after = app.document().result_sheet(&SheetId::new("res-lin-wf60")).ok_or("missing res-lin-wf60 after re-include")?;
    ensure(matches!(rs_after.status, ResultStatus::UpToDate), "re-included subset regression must remain Ok")?;
    let summary_after = format!("{:?}", rs_after.tabs);
    ensure(summary_after != summary_before, "re-including outlier row must recompute regression slope/intercept")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-61: Per-Column Normalized Heatmap & Vertical 90° Column Titles
// ---------------------------------------------------------------------------

#[test]
fn wf61_per_column_normalized_heatmap_and_vertical_column_titles() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf61_multivariate_biomarker_grouped();

    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf61",
            "title": fx.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "cols": fx.analyte_titles.len(),
            "rows": fx.row_titles.len()
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "grp-wf61", "titles": fx.analyte_titles }))?;
    menu(&mut app, "table.setRowTitles", json!({ "sheet_id": "grp-wf61", "titles": fx.row_titles }))?;
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            type_grid_cell(&mut app, "grp-wf61", r, c, 0, &format!("{val}"))?;
        }
    }

    menu(&mut app, "graph.create", json!({ "sheet_id": "g-raw-wf61", "data_sheet_id": "grp-wf61", "kind": "heatmap" }))?;
    let _raw_scene = compile_clean(app.document(), "g-raw-wf61")?;

    menu(
        &mut app,
        "transform.run",
        json!({
            "sheet_id": "res-norm-wf61",
            "source_sheet_id": "grp-wf61",
            "kind": "normalize",
            "per_data_set": true,
            "create_graph": true,
            "graph_sheet_id": "g-norm-wf61"
        }),
    )?;
    menu(&mut app, "graph.setKind", json!({ "sheet_id": "g-norm-wf61", "kind": "heatmap" }))?;
    menu(
        &mut app,
        "graph.setSpec",
        json!({
            "sheet_id": "g-norm-wf61",
            "heatmap_labels": { "row_labels": "row_titles", "column_labels": "column_titles", "column_angle": "vertical_90" }
        }),
    )?;

    let svg_before = {
        let gs = app.document().graph_sheet(&SheetId::new("g-norm-wf61")).ok_or("missing g-norm-wf61")?;
        ensure(gs.spec.heatmap_labels.column_angle == HeatmapColumnLabelAngle::Vertical90, "heatmap column_angle must be Vertical90")?;
        let scene = compile_clean(app.document(), "g-norm-wf61")?;
        let texts = scene_texts(&scene);
        for title in &fx.analyte_titles {
            ensure(texts.iter().any(|t| t == title), format!("missing analyte label `{title}` in normalized heatmap scene"))?;
        }
        ensure(
            scene.primitives.iter().all(|p| match p {
                ScenePrimitive::Text(t) => t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite(),
                _ => true,
            }),
            "all heatmap text primitives must have finite coordinates",
        )?;
        render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?
    };

    type_grid_cell(&mut app, "grp-wf61", 3, 0, 0, "950.0")?;
    let scene_after = compile_clean(app.document(), "g-norm-wf61")?;
    let svg_after = render_svg(&scene_after, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    ensure(svg_before != svg_after, "live cell peak edit must rescale normalized heatmap")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-62: Five Experimental Designs (Nested, Paired Column & RM Two-Way ANOVA)
// ---------------------------------------------------------------------------

#[test]
fn wf62_five_design_nested_paired_and_rm_anova_trial_comparison() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf62_five_design_trial();

    // 1) Nested hierarchy vs pseudoreplicated flattened Column table
    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "nest-wf62",
            "title": fx.nested_title,
            "kind": "nested",
            "cols": fx.nested_groups.len(),
            "replicates": fx.nested_subjects_per_group,
            "rows": fx.nested_reps_per_subject
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "nest-wf62", "titles": fx.nested_groups }))?;
    for (group_idx, subjects) in fx.nested_values.iter().enumerate() {
        for (subject_idx, reps) in subjects.iter().enumerate() {
            for (rep_idx, &val) in reps.iter().enumerate() {
                type_grid_cell(&mut app, "nest-wf62", rep_idx, group_idx, subject_idx, &format!("{val}"))?;
            }
        }
    }
    menu(&mut app, "analysis.run", json!({ "sheet_id": "res-nest-wf62", "source_sheet_id": "nest-wf62", "kind": "nested_t" }))?;

    let flat_rows = fx.nested_subjects_per_group * fx.nested_reps_per_subject;
    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "col-flat-wf62", "title": "Flattened Cohort", "kind": "column", "cols": fx.nested_groups.len(), "rows": flat_rows }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "col-flat-wf62", "titles": fx.nested_groups }))?;
    for (group_idx, subjects) in fx.nested_values.iter().enumerate() {
        let mut flat_r = 0usize;
        for reps in subjects {
            for &val in reps {
                type_grid_cell(&mut app, "col-flat-wf62", flat_r, group_idx, 0, &format!("{val}"))?;
                flat_r += 1;
            }
        }
    }
    let flat_res_id = run_analyze_dialog(&mut app, "col-flat-wf62", json!({ "entry_id": "stats.two_group", "two_group_method": "unpaired_t" }))?;

    let nest_tabs_before = {
        let nest_rs = app.document().result_sheet(&SheetId::new("res-nest-wf62")).ok_or("missing res-nest-wf62")?;
        let flat_rs = app.document().result_sheet(&SheetId::new(&flat_res_id)).ok_or("missing flat two_group result")?;
        ensure(matches!(nest_rs.status, ResultStatus::UpToDate), "nested_t result must be Ok")?;
        ensure(matches!(flat_rs.status, ResultStatus::UpToDate), "flattened unpaired_t result must be Ok")?;
        let nest_str = format!("{:?}", nest_rs.tabs);
        let flat_str = format!("{:?}", flat_rs.tabs);
        ensure(nest_str != flat_str, "nested t-test and pseudoreplicated unpaired t-test must produce distinct DF/summaries")?;
        nest_str
    };
    type_grid_cell(&mut app, "nest-wf62", 0, 0, 0, "24.8")?;
    let nest_tabs_after =
        format!("{:?}", app.document().result_sheet(&SheetId::new("res-nest-wf62")).ok_or("missing res-nest-wf62 after edit")?.tabs);
    ensure(nest_tabs_after != nest_tabs_before, "live nested replicate edit must recompute nested_t result")?;

    // 2) Paired two-group comparison
    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "col-paired-wf62", "title": fx.paired_title, "kind": "column", "cols": 2, "rows": fx.paired_rows.len() }),
    )?;
    for (r, pair) in fx.paired_rows.iter().enumerate() {
        type_grid_cell(&mut app, "col-paired-wf62", r, 0, 0, &format!("{}", pair[0]))?;
        type_grid_cell(&mut app, "col-paired-wf62", r, 1, 0, &format!("{}", pair[1]))?;
    }
    let paired_id = run_analyze_dialog(&mut app, "col-paired-wf62", json!({ "entry_id": "stats.two_group", "two_group_method": "paired_t" }))?;
    let paired_rs = app.document().result_sheet(&SheetId::new(&paired_id)).ok_or("missing paired result")?;
    ensure(matches!(paired_rs.status, ResultStatus::UpToDate), "paired_t analysis must be Ok")?;

    // 3) Longitudinal Grouped Two-Way RM ANOVA: both_factors vs row_factor
    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "grp-rm-wf62",
            "title": fx.rm_title,
            "kind": "grouped",
            "cols": 2,
            "replicates": 4,
            "rows": fx.rm_timepoints.len()
        }),
    )?;
    menu(&mut app, "table.setRowTitles", json!({ "sheet_id": "grp-rm-wf62", "titles": fx.rm_timepoints }))?;
    for (r, flat_vals) in fx.rm_rows.iter().enumerate() {
        for (flat_idx, &val) in flat_vals.iter().enumerate() {
            type_grid_cell(&mut app, "grp-rm-wf62", r, flat_idx / 4, flat_idx % 4, &format!("{val}"))?;
        }
    }
    let rm_id = run_analyze_dialog(&mut app, "grp-rm-wf62", json!({ "entry_id": "stats.factorial_anova", "repeated_measures": "both" }))?;
    let rm_both_tabs = {
        let rm_rs = app.document().result_sheet(&SheetId::new(&rm_id)).ok_or("missing factorial RM result")?;
        ensure(matches!(rm_rs.status, ResultStatus::UpToDate), "factorial_anova both_factors must be Ok")?;
        format!("{:?}", rm_rs.tabs)
    };
    menu(&mut app, "analysis.updateParams", json!({ "sheet_id": rm_id, "repeated_measures": "row_factor" }))?;
    let rm_row_rs = app.document().result_sheet(&SheetId::new(&rm_id)).ok_or("missing factorial RM row_factor result")?;
    ensure(matches!(rm_row_rs.status, ResultStatus::UpToDate), "factorial_anova row_factor must be Ok")?;
    ensure(format!("{:?}", rm_row_rs.tabs) != rm_both_tabs, "switching RM design between both_factors and row_factor must update ANOVA table")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-63: Forest Plot via Upper/Lower Limits & Custom X=1.0 Null Reference
// ---------------------------------------------------------------------------

#[test]
fn wf63_forest_plot_upper_lower_limits_and_null_reference_line() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf63_meta_analysis_forest_grouped();

    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf63",
            "title": fx.title,
            "kind": "grouped",
            "cols": 1,
            "rows": fx.study_titles.len(),
            "subcolumn_mode": "upper_lower_limits"
        }),
    )?;
    menu(&mut app, "table.setRowTitles", json!({ "sheet_id": "grp-wf63", "titles": fx.study_titles }))?;
    for (r, entry) in fx.entries.iter().enumerate() {
        type_grid_cell(&mut app, "grp-wf63", r, 0, 0, &format!("{}", entry[0]))?;
        type_grid_cell(&mut app, "grp-wf63", r, 0, 1, &format!("{}", entry[1]))?;
        type_grid_cell(&mut app, "grp-wf63", r, 0, 2, &format!("{}", entry[2]))?;
    }

    menu(&mut app, "graph.create", json!({ "sheet_id": "g-wf63", "data_sheet_id": "grp-wf63", "kind": "forest" }))?;
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-wf63")).ok_or("missing g-wf63")?;
        ensure(!gs.spec.forest_show_pooled_diamond, "forest_show_pooled_diamond must default to false")?;
    }
    menu(
        &mut app,
        "graph.setAdditionalTicks",
        json!({ "sheet_id": "g-wf63", "axis": "x", "ticks": [{ "value": 1.0, "label": "No Effect", "draw_gridline": true }] }),
    )?;

    let svg_before = {
        let scene = compile_clean(app.document(), "g-wf63")?;
        let texts = scene_texts(&scene);
        ensure(texts.iter().any(|t| t == "No Effect"), "missing `No Effect` additional X tick label")?;
        for title in &fx.study_titles {
            ensure(texts.iter().any(|t| t == title), format!("missing study row label `{title}` in forest scene"))?;
        }
        render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?
    };

    type_grid_cell(&mut app, "grp-wf63", 0, 0, 2, "1.35")?;
    let scene_after = compile_clean(app.document(), "g-wf63")?;
    let svg_after = render_svg(&scene_after, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    ensure(svg_before != svg_after, "live upper-CI limit edit must update forest plot SVG")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-64: Population Pyramid via Horizontal Stacked Bars & |x| Unsigned Ticks
// ---------------------------------------------------------------------------

#[test]
fn wf64_population_pyramid_horizontal_stacked_and_unsigned_ticks() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf64_demographics_pyramid_grouped();

    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf64",
            "title": fx.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "cols": fx.cohort_titles.len(),
            "rows": fx.band_titles.len()
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "grp-wf64", "titles": fx.cohort_titles }))?;
    menu(&mut app, "table.setRowTitles", json!({ "sheet_id": "grp-wf64", "titles": fx.band_titles }))?;
    for (r, row) in fx.rows.iter().enumerate() {
        type_grid_cell(&mut app, "grp-wf64", r, 0, 0, &format!("{}", row[0]))?;
        type_grid_cell(&mut app, "grp-wf64", r, 1, 0, &format!("{}", row[1]))?;
    }

    menu(&mut app, "ui.view.rowTitles.toggle", json!({ "collapsed": false }))?;
    app.sync_views();
    ensure(
        app.grid.row_titles_collapsed_override == Some(false) && !app.egui_views.column_grid.row_titles_collapsed,
        "expected Row Titles column expanded after toggle",
    )?;

    menu(&mut app, "graph.create", json!({ "sheet_id": "g-wf64", "data_sheet_id": "grp-wf64", "kind": "grouped" }))?;
    menu(&mut app, "graph.setKind", json!({ "sheet_id": "g-wf64", "kind": "grouped", "option_id": "grouped.stacked" }))?;
    menu(&mut app, "graph.setSpec", json!({ "sheet_id": "g-wf64", "orientation": "horizontal" }))?;
    menu(&mut app, "graph.setSeries", json!({ "sheet_id": "g-wf64", "all_series": true, "border_width_pt": 0.5 }))?;
    menu(&mut app, "graph.setAxes", json!({ "sheet_id": "g-wf64", "x_axis": { "tick_format": "absolute_decimal" } }))?;

    let svg_before = {
        let gs = app.document().graph_sheet(&SheetId::new("g-wf64")).ok_or("missing g-wf64")?;
        ensure(gs.spec.orientation == CartesianOrientation::Horizontal, "pyramid must use Horizontal orientation")?;
        ensure(gs.spec.grouped_arrangement == GroupedArrangement::Stacked, "pyramid must use Stacked arrangement")?;
        ensure(gs.spec.x_axis.tick_format == TickLabelFormat::AbsoluteDecimal, "pyramid must use AbsoluteDecimal X ticks")?;
        let scene = compile_clean(app.document(), "g-wf64")?;
        let numeric_ticks: Vec<String> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) if t.anchor_pt.y > scene.plot_rect_pt.bottom() && t.anchor_pt.y < scene.height_pt - 15.0 => {
                    let s = t.rich.plain_text();
                    s.chars().any(|ch| ch.is_ascii_digit()).then_some(s)
                }
                _ => None,
            })
            .collect();
        ensure(!numeric_ticks.is_empty(), "expected unsigned X tick labels below pyramid plot")?;
        ensure(
            numeric_ticks.iter().all(|s| !s.starts_with('-') && !s.starts_with('−')),
            format!("AbsoluteDecimal ticks must not include minus signs: {numeric_ticks:?}"),
        )?;
        render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?
    };

    type_grid_cell(&mut app, "grp-wf64", 2, 0, 0, "-38.0")?;
    let scene_after = compile_clean(app.document(), "g-wf64")?;
    let svg_after = render_svg(&scene_after, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    ensure(svg_before != svg_after, "live pyramid value edit must update diverging bar SVG")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-65: Spaghetti Plots (Individual Replicate Trajectories & Multi-Subject Y)
// ---------------------------------------------------------------------------

#[test]
fn wf65_spaghetti_plot_replicate_trajectories_vs_subject_traces() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf65_longitudinal_spaghetti_xy();

    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "xy-rep-wf65",
            "title": fx.title,
            "kind": "xy",
            "cols": fx.cohort_titles.len(),
            "replicates": 4,
            "rows": fx.weeks.len()
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "xy-rep-wf65", "titles": fx.cohort_titles }))?;
    for (r, &week) in fx.weeks.iter().enumerate() {
        type_x_cell(&mut app, "xy-rep-wf65", r, 0, &format!("{week}"))?;
        for (g, reps) in fx.replicate_trajectories[r].iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                type_grid_cell(&mut app, "xy-rep-wf65", r, g, s, &format!("{val}"))?;
            }
        }
    }

    menu(&mut app, "graph.create", json!({ "sheet_id": "g-rep-wf65", "data_sheet_id": "xy-rep-wf65", "kind": "xy" }))?;
    menu(
        &mut app,
        "graph.setSeries",
        json!({ "sheet_id": "g-rep-wf65", "all_series": true, "marker": null, "replicate_lines": "individual_replicate_trajectories" }),
    )?;

    let svg_before = {
        let gs = app.document().graph_sheet(&SheetId::new("g-rep-wf65")).ok_or("missing g-rep-wf65")?;
        for series in &gs.spec.series {
            ensure(
                matches!(
                    series.gdo,
                    GraphicalDataObject::ScatterAndLine { marker: None, replicate_lines: ReplicateLineMode::IndividualReplicateTrajectories, .. }
                ),
                "replicate spaghetti series must persist marker=None and IndividualReplicateTrajectories",
            )?;
        }
        let scene = compile_clean(app.document(), "g-rep-wf65")?;
        ensure(!scene.plotted_points.is_empty(), "spaghetti plot must emit plotted_points vertices")?;
        let line_paths = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { .. })).count();
        ensure(line_paths >= 8, format!("expected at least 8 trajectory/axis paths on spaghetti plot, got {line_paths}"))?;
        render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?
    };

    type_grid_cell(&mut app, "xy-rep-wf65", 2, 1, 3, "52.5")?;
    let scene_after = compile_clean(app.document(), "g-rep-wf65")?;
    let svg_after = render_svg(&scene_after, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    ensure(svg_before != svg_after, "single-subcolumn replicate edit must update spaghetti trajectory SVG")?;

    // Alternative 4-subject single-Y table design
    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "xy-subj-wf65",
            "title": "Single-Y Subject Traces",
            "kind": "xy",
            "cols": fx.subject_titles.len(),
            "rows": fx.weeks.len()
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "xy-subj-wf65", "titles": fx.subject_titles }))?;
    for (r, &week) in fx.weeks.iter().enumerate() {
        type_x_cell(&mut app, "xy-subj-wf65", r, 0, &format!("{week}"))?;
        for (c, &val) in fx.single_y_subjects[r].iter().enumerate() {
            type_grid_cell(&mut app, "xy-subj-wf65", r, c, 0, &format!("{val}"))?;
        }
    }
    menu(&mut app, "graph.create", json!({ "sheet_id": "g-subj-wf65", "data_sheet_id": "xy-subj-wf65", "kind": "xy" }))?;
    menu(&mut app, "graph.setSeries", json!({ "sheet_id": "g-subj-wf65", "all_series": true, "marker": null }))?;
    let _subj_scene = compile_clean(app.document(), "g-subj-wf65")?;

    switch_sheet(&mut app, "g-rep-wf65")?;
    let _png = assert_screenshot_png(&mut app, 640, 400)?;
    Ok(())
}
