//! End-to-end spawned-stdio MCP `--gui-only` tests covering `R13` workflows `WF-51`–`WF-65`.
//!
//! Completes full 65/65 workflow parity (`WF-01`–`WF-65`) over a real spawned
//! `curvebook-cli mcp --gui-only --workdir <dir>` child process with strict
//! `save_and_verify_headless` persistence checks.
use curvebook_doc::{SheetId, SubcolumnMode};
use curvebook_format::open_archive_bytes;
use curvebook_render::{RasterImage, encode_png};
use curvebook_svg::validate_svg_structure;
use curvebook_testkit::r13_datasets::{
    wf51_inhibitor_triplicate_xy, wf52_sparse_agonist_4pl_xy, wf53_three_arm_survival, wf54_dual_axis_mean_sd_n_xy, wf55_multisite_grouped_cohort,
    wf56_exponential_growth_xy, wf57_schematic_rgba, wf58_descriptive_column, wf59_paired_before_after_and_inset,
    wf60_subset_linear_with_outliers_xy, wf61_multivariate_biomarker_grouped, wf62_five_design_trial, wf63_meta_analysis_forest_grouped,
    wf64_demographics_pyramid_grouped, wf65_longitudinal_spaghetti_xy,
};
use serde_json::{Value, json};

use super::common::{McpChild, TempWorkdir, decode_base64, encode_base64, read_result_json, result_tab, save_and_verify_headless};

fn ensure(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn export_graph_svg(gui: &mut McpChild, sheet_id: &str) -> Result<String, String> {
    let out = gui.ui_invoke("graph.exportSvg", json!({ "sheet_id": sheet_id }))?;
    let svg = out.pointer("/payload/svg").and_then(Value::as_str).ok_or_else(|| format!("missing SVG payload for `{sheet_id}`: {out}"))?.to_owned();
    validate_svg_structure(&svg).map_err(|e| format!("invalid SVG for `{sheet_id}`: {e}"))?;
    Ok(svg)
}

fn export_figure_svg(gui: &mut McpChild, sheet_id: &str) -> Result<String, String> {
    gui.ui_invoke("ui.export.figure.open", json!({ "sheet_id": sheet_id }))?;
    let out = gui.ui_answer_ok(json!({ "format": "svg" }))?;
    let svg =
        out.pointer("/payload/svg").and_then(Value::as_str).ok_or_else(|| format!("missing figure SVG payload for `{sheet_id}`: {out}"))?.to_owned();
    validate_svg_structure(&svg).map_err(|e| format!("invalid figure SVG for `{sheet_id}`: {e}"))?;
    Ok(svg)
}

#[test]
fn r73_gui_only_wf51_to_wf58_r13_regression_survival_batch_and_objects() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf51_58")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // Verify strict --gui-only screenshot path guard over live MCP stdio.
    let blocked = gui.call_tool_raw("ui_screenshot", json!({ "target": "workspace", "width_px": 320, "height_px": 240, "path": "shot.png" }))?;
    ensure(
        blocked.get("isError") == Some(&json!(true)) && blocked.pointer("/structuredContent/code") == Some(&json!("gui_only_violation")),
        format!("expected --gui-only screenshot path rejection, got {blocked}"),
    )?;

    // WF-51: Nonlinear ROUT, Dual Replicate Interpolation, 3PL/4PL & Normalize
    let fx51 = wf51_inhibitor_triplicate_xy();
    let total_rows_51 = fx51.known_rows.len() + fx51.unknown_rows.len();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "xy-wf51",
            "title": fx51.title,
            "kind": "xy",
            "cols": 1,
            "rows": total_rows_51,
            "subcolumn_mode": "replicates",
            "replicates": 3,
            "auto_graph": true,
            "graph_sheet_id": "g-wf51"
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "xy-wf51", "titles": [fx51.data_set_title] }))?;
    for (r, pt) in fx51.known_rows.iter().enumerate() {
        gui.set_x_cell("xy-wf51", r, 0, &format!("{}", pt.log_dose))?;
        for (s, val) in pt.replicates.iter().enumerate() {
            gui.set_cell("xy-wf51", r, 0, s, &format!("{val}"))?;
        }
    }
    for (u_idx, reps) in fx51.unknown_rows.iter().enumerate() {
        let r = fx51.known_rows.len() + u_idx;
        for (s, val) in reps.iter().enumerate() {
            gui.set_cell("xy-wf51", r, 0, s, &format!("{val}"))?;
        }
    }
    let res51 = gui.run_analyze(
        "xy-wf51",
        json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl", "fit_method": "rout", "rout_q": 1.0 }),
    )?;
    gui.ui_switch_sheet(&res51)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let dlg51 = gui.ui_inspect("dialog", false)?;
    ensure(!dlg51["dialog"].is_null(), "WF-51 Cmd+T must open EditAnalysisParameters dialog")?;
    gui.ui_answer_dialog(json!({ "action": "cancel" }))?;
    gui.ui_invoke(
        "analysis.updateParams",
        json!({
            "sheet_id": res51,
            "interpolate_unknowns": true,
            "flag_r_squared_below": 0.999999,
            "comparison": { "method": "extra_sum_of_squares_f", "alternative_equation_id": "log_inhibitor_vs_response_3pl" }
        }),
    )?;
    gui.ui_invoke(
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf51",
            "series_index": 0,
            "regression_overlay": {
                "result_sheet_id": res51,
                "sample_segments": 256,
                "show_confidence_band": false,
                "show_prediction_band": false,
                "shaded_ribbon": false
            }
        }),
    )?;
    let _svg51 = export_graph_svg(&mut gui, "g-wf51")?;
    gui.ui_invoke(
        "transform.run",
        json!({ "sheet_id": "res-norm-wf51", "source_sheet_id": "xy-wf51", "kind": "normalize", "zero_anchor": "smallest", "hundred_anchor": "largest" }),
    )?;
    // Edit unknown replicate cell to exercise live reactive unknown interpolation.
    gui.set_cell("xy-wf51", fx51.known_rows.len(), 0, 1, "64.0")?;
    gui.ui_switch_sheet("g-wf51")?;
    let _png51 = gui.ui_screenshot("canvas", 640, 400)?;

    // WF-52: 4PL Identifiability Recovery, Extrapolation Banner & Normal Q-Q Plot
    let fx52 = wf52_sparse_agonist_4pl_xy();
    let s0_52 = fx52.series.first().ok_or("missing WF-52 series 0")?;
    let s1_52 = fx52.series.get(1).ok_or("missing WF-52 series 1")?;
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "xy-wf52", "title": fx52.title, "kind": "xy", "cols": 2, "rows": s0_52.rows.len(), "auto_graph": true, "graph_sheet_id": "g-wf52" }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "xy-wf52", "titles": [s0_52.title, s1_52.title] }))?;
    for (r, (&(x, y0), &(_, y1))) in s0_52.rows.iter().zip(s1_52.rows.iter()).enumerate() {
        gui.set_x_cell("xy-wf52", r, 0, &format!("{x}"))?;
        gui.set_cell("xy-wf52", r, 0, 0, &format!("{y0}"))?;
        gui.set_cell("xy-wf52", r, 1, 0, &format!("{y1}"))?;
    }
    let res52 = gui.run_analyze("xy-wf52", json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_4pl" }))?;
    gui.ui_invoke("analysis.updateParams", json!({ "sheet_id": res52, "identify_unstable_parameters": true }))?;
    gui.ui_invoke(
        "analysis.updateParams",
        json!({
            "sheet_id": res52,
            "identify_unstable_parameters": true,
            "parameter_rules": [
                { "name": "Bottom", "scope": "constant", "value": 0.0 },
                { "name": "HillSlope", "scope": "shared_global" },
                { "name": "Top", "scope": "shared_global" }
            ]
        }),
    )?;
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-wf52-qq", "data_sheet_id": res52, "kind": "qq" }))?;
    let svg52_before = export_graph_svg(&mut gui, "g-wf52-qq")?;
    gui.set_cell("xy-wf52", 4, 0, 0, "58.0")?;
    let svg52_after = export_graph_svg(&mut gui, "g-wf52-qq")?;
    ensure(svg52_before != svg52_after, "WF-52 cell correction must update Normal Q-Q diagnostic SVG")?;

    // WF-53: Kaplan-Meier Display Modes & Column Exclude Transition to HR CI
    let fx53 = wf53_three_arm_survival();
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "surv-wf53", "title": fx53.title, "kind": "survival", "cols": 3, "rows": fx53.rows.len(), "auto_graph": true, "graph_sheet_id": "g-wf53" }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "surv-wf53", "titles": fx53.cohorts }))?;
    for (r, entry) in fx53.rows.iter().enumerate() {
        if let Some(t) = entry.elapsed_days {
            gui.set_x_cell("surv-wf53", r, 0, &format!("{t}"))?;
        }
        for (c, status) in entry.status.iter().enumerate() {
            if let Some(code) = status {
                gui.set_cell("surv-wf53", r, c, 0, &format!("{code}"))?;
            }
        }
    }
    let res53 = gui.run_analyze("surv-wf53", json!({ "entry_id": "stats.survival_km" }))?;
    let svg53_default = export_graph_svg(&mut gui, "g-wf53")?;
    gui.ui_invoke(
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf53",
            "all_series": true,
            "display_metric": "percent_cumulative_event",
            "staircase": false,
            "error_display": "sem"
        }),
    )?;
    let svg53_event = export_graph_svg(&mut gui, "g-wf53")?;
    ensure(svg53_default != svg53_event, "WF-53 KM CumulativeEvent + PointToPoint + SEM must alter SVG")?;
    let cells53: Vec<Value> = (0..fx53.rows.len()).map(|r| json!({ "row": r, "col": 0, "subcol": 0 })).collect();
    gui.ui_invoke("table.excludeCells", json!({ "sheet_id": "surv-wf53", "cells": cells53, "excluded": true }))?;

    // WF-54: Dual-Axis Nudged XY Bars, 50% Alpha Lines & Selective Decay Fit
    let fx54 = wf54_dual_axis_mean_sd_n_xy();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "xy-wf54",
            "title": fx54.title,
            "kind": "xy",
            "cols": 2,
            "rows": fx54.x_hours.len(),
            "subcolumn_mode": "mean_sd_n",
            "auto_graph": true,
            "graph_sheet_id": "g-wf54"
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "xy-wf54", "titles": fx54.series_titles }))?;
    for (r, ((&x, f), m)) in fx54.x_hours.iter().zip(fx54.fluorescence.iter()).zip(fx54.metabolite.iter()).enumerate() {
        gui.set_x_cell("xy-wf54", r, 0, &format!("{x}"))?;
        for (s, v) in [f.mean, f.sd, f.n].iter().enumerate() {
            gui.set_cell("xy-wf54", r, 0, s, &format!("{v}"))?;
        }
        for (s, v) in [m.mean, m.sd, m.n].iter().enumerate() {
            gui.set_cell("xy-wf54", r, 1, s, &format!("{v}"))?;
        }
    }
    let fit54 = gui.run_analyze("xy-wf54", json!({ "entry_id": "fit.nonlinear", "equation_id": "one_phase_decay", "selected_data_sets": [0] }))?;
    gui.ui_invoke("graph.setAxes", json!({ "sheet_id": "g-wf54", "y2_axis": { "title": "Metabolite Pool (uM)" } }))?;
    gui.ui_invoke(
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf54",
            "series_index": 0,
            "alpha": 0.5,
            "regression_overlay": {
                "result_sheet_id": fit54,
                "sample_segments": 256,
                "show_confidence_band": false,
                "show_prediction_band": false,
                "shaded_ribbon": false
            }
        }),
    )?;
    gui.ui_invoke(
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf54",
            "series_index": 1,
            "bind_to_right_y2": true,
            "gdo": {
                "BarWithReplicates": {
                    "summary": "Mean",
                    "fill_color": { "r": 210, "g": 120, "b": 40, "a": 200 },
                    "border_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "border_width_pt": 0.8,
                    "replicate_overlay": null
                }
            },
            "offset": { "x_data": 0.5, "x_pt": 0.0, "y_pt": 0.0 },
            "bar_width_pt": 14.0
        }),
    )?;
    let svg54_before = export_graph_svg(&mut gui, "g-wf54")?;
    gui.set_cell("xy-wf54", 1, 0, 0, "84.5")?;
    let svg54_after = export_graph_svg(&mut gui, "g-wf54")?;
    ensure(svg54_before != svg54_after, "WF-54 live mean edit must update dual-axis decay curve & bars")?;

    // WF-55: Grouped Transpose -> 12 Batch Graphs & Batch Apply Style From
    let fx55 = wf55_multisite_grouped_cohort();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf55",
            "title": fx55.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "rows": fx55.row_titles.len(),
            "cols": fx55.col_titles.len()
        }),
    )?;
    gui.ui_invoke("table.setRowTitles", json!({ "sheet_id": "grp-wf55", "titles": fx55.row_titles }))?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "grp-wf55", "titles": fx55.col_titles }))?;
    for (r, row) in fx55.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            gui.set_cell("grp-wf55", r, c, 0, &format!("{val}"))?;
        }
    }
    let tr55_out = gui.ui_invoke(
        "transform.run",
        json!({ "sheet_id": "res-tr-wf55", "source_sheet_id": "grp-wf55", "kind": "transpose", "swap_row_and_column_titles": true }),
    )?;
    let tr55_id = tr55_out.pointer("/payload/sheet_id").and_then(Value::as_str).unwrap_or("res-tr-wf55").to_owned();
    gui.ui_invoke("ui.graph.create.open", json!({ "source_sheet_id": tr55_id }))?;
    let batch55_out = gui.ui_answer_ok(json!({ "source_sheet_id": tr55_id, "one_per_data_set": true, "y_axis_title": "Quarter Index" }))?;
    let created55: Vec<String> = batch55_out
        .pointer("/payload/sheet_ids")
        .and_then(Value::as_array)
        .ok_or("WF-55 expected batch created sheet_ids")?
        .iter()
        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();
    ensure(created55.len() == 12, format!("WF-55 expected 12 site graphs, got {}", created55.len()))?;
    let donor55 = created55.first().ok_or("WF-55 missing donor graph")?.clone();
    let targets55: Vec<String> = created55.iter().skip(1).cloned().collect();
    gui.ui_invoke("graph.setSpec", json!({ "sheet_id": donor55, "paper_fill": "#F3F5FA" }))?;
    gui.ui_invoke("graph.applyStyleFrom", json!({ "donor_sheet_id": donor55, "target_sheet_ids": targets55 }))?;
    let svg55_before = export_graph_svg(&mut gui, &donor55)?;
    gui.set_cell("grp-wf55", 0, 2, 0, "140.0")?;
    let svg55_after = export_graph_svg(&mut gui, &donor55)?;
    ensure(svg55_before != svg55_after, "WF-55 2-hop cell edit -> transpose -> batch site graph must re-render")?;

    // WF-56: Log10 Transform, Antilog Ticks & Restricted X Range Fit Invariance
    let fx56 = wf56_exponential_growth_xy();
    gui.ui_invoke("sheet.createData", json!({ "sheet_id": "xy-wf56", "title": fx56.title, "kind": "xy", "cols": 1, "rows": fx56.rows.len() }))?;
    for (r, &(x, y)) in fx56.rows.iter().enumerate() {
        gui.set_x_cell("xy-wf56", r, 0, &format!("{x}"))?;
        gui.set_cell("xy-wf56", r, 0, 0, &format!("{y}"))?;
    }
    gui.ui_invoke("transform.run", json!({ "sheet_id": "res-log-wf56", "source_sheet_id": "xy-wf56", "kind": "math", "y_transform": "log10" }))?;
    gui.ui_invoke(
        "analysis.run",
        json!({ "sheet_id": "res-lin-wf56", "source_sheet_id": "res-log-wf56", "kind": "linear_regression", "x_range_filter": [10.0, 20.0] }),
    )?;
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-wf56", "data_sheet_id": "res-log-wf56", "kind": "xy" }))?;
    gui.ui_invoke(
        "graph.setAxes",
        json!({ "sheet_id": "g-wf56", "y_axis": { "tick_format": "antilog", "segments": [{ "minor_ticks_per_interval": 9 }] } }),
    )?;
    gui.ui_invoke(
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf56",
            "series_index": 0,
            "regression_overlay": {
                "result_sheet_id": "res-lin-wf56",
                "sample_segments": 256,
                "show_confidence_band": false,
                "show_prediction_band": false,
                "shaded_ribbon": false
            }
        }),
    )?;
    let _svg56 = export_graph_svg(&mut gui, "g-wf56")?;
    gui.ui_switch_sheet("res-lin-wf56")?;
    let shot56_before = gui.ui_screenshot("canvas", 640, 400)?;
    // Edit point at X < 10 -> restricted [10, 20] fit result canvas stays invariant.
    gui.set_cell("xy-wf56", 0, 0, 0, "999.0")?;
    gui.ui_switch_sheet("res-lin-wf56")?;
    let shot56_invar = gui.ui_screenshot("canvas", 640, 400)?;
    ensure(shot56_before == shot56_invar, "WF-56 edit at X < 10.0 must leave [10, 20] restricted fit result invariant")?;
    // Edit point at X >= 10 -> restricted fit recomputes.
    gui.set_cell("xy-wf56", 8, 0, 0, "500.0")?;
    gui.ui_switch_sheet("res-lin-wf56")?;
    let shot56_after = gui.ui_screenshot("canvas", 640, 400)?;
    ensure(shot56_after != shot56_before, "WF-56 edit at X >= 10.0 must recompute restricted fit result")?;

    // WF-57: Layout Embedded Image Crop/Rotate & Non-Exported Grid Overlay
    let fx57 = wf57_schematic_rgba();
    let png57 = encode_png(&RasterImage { width_px: fx57.width_px, height_px: fx57.height_px, dpi: 96, rgba: fx57.rgba })
        .map_err(|e| format!("WF-57 encode_png failed: {e}"))?;
    let png57_b64 = encode_base64(&png57);
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "xy-wf57", "title": "Panel Assay", "kind": "xy", "cols": 1, "rows": fx57.assay_xy.len(), "auto_graph": true, "graph_sheet_id": "g-wf57" }),
    )?;
    for (r, &(x, y)) in fx57.assay_xy.iter().enumerate() {
        gui.set_x_cell("xy-wf57", r, 0, &format!("{x}"))?;
        gui.set_cell("xy-wf57", r, 0, 0, &format!("{y}"))?;
    }
    gui.ui_invoke("layout.create", json!({ "sheet_id": "lay-wf57", "title": "2x2 Schematic Layout", "preset": "double_column" }))?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf57", "panel_id": "p-graph", "kind": "graph", "graph_sheet_id": "g-wf57", "bounds_pt": [20.0, 20.0, 220.0, 150.0] }),
    )?;
    gui.ui_invoke("layout.insertImage", json!({ "sheet_id": "lay-wf57", "bytes_base64": png57_b64, "bounds_pt": [260.0, 24.0, 120.0, 80.0] }))?;
    gui.ui_invoke("ui.image.format.open", json!({ "sheet_id": "lay-wf57", "item_index": 1 }))?;
    gui.ui_answer_ok(json!({
        "lock_aspect_ratio": false,
        "width_pt": 150.0,
        "height_pt": 95.0,
        "rotation_deg": 15.0,
        "crop_left_pt": 4.0,
        "crop_right_pt": 4.0,
        "crop_top_pt": 2.0,
        "crop_bottom_pt": 2.0,
        "border_width_pt": 1.5,
        "border_color": "#1E3A8A"
    }))?;
    let grid_on = gui.ui_invoke("ui.gridOverlay.toggle", json!({ "visible": true, "spacing_pt": 24.0 }))?;
    ensure(grid_on.pointer("/result/grid_overlay_visible") == Some(&json!(true)), "WF-57 grid overlay toggle must activate viewport grid")?;
    let svg57_grid_on = export_figure_svg(&mut gui, "lay-wf57")?;
    ensure(svg57_grid_on.contains("data:image/png;base64,"), "WF-57 layout SVG missing embedded PNG data URI")?;
    gui.ui_invoke("ui.export.figure.open", json!({ "sheet_id": "lay-wf57" }))?;
    let pdf57_b64 = gui
        .ui_answer_ok(json!({ "format": "pdf" }))?
        .pointer("/payload/base64")
        .and_then(Value::as_str)
        .ok_or("WF-57 missing layout PDF base64")?
        .to_owned();
    let pdf57 = decode_base64(&pdf57_b64)?;
    ensure(pdf57.starts_with(b"%PDF-1.7"), "WF-57 layout PDF missing %PDF-1.7 header")?;
    gui.ui_invoke("ui.gridOverlay.toggle", json!({ "visible": false }))?;
    let svg57_grid_off = export_figure_svg(&mut gui, "lay-wf57")?;
    ensure(svg57_grid_on == svg57_grid_off, "WF-57 canvas grid overlay must never alter exported vector output")?;

    // WF-58: Result Slice Paste Table, Borderless Transpose & Upstream Reactive
    let fx58 = wf58_descriptive_column();
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "col-wf58", "title": fx58.title, "kind": "column", "cols": 4, "rows": fx58.rows.len(), "auto_graph": true, "graph_sheet_id": "g-wf58" }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "col-wf58", "titles": fx58.group_titles }))?;
    for (r, row) in fx58.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            gui.set_cell("col-wf58", r, c, 0, &format!("{val}"))?;
        }
    }
    let res58 = gui.run_analyze("col-wf58", json!({ "entry_id": "stats.descriptive" }))?;
    gui.ui_invoke(
        "edit.pasteResultTable",
        json!({
            "source_sheet_id": res58,
            "target_sheet_id": "g-wf58",
            "selected_rows": [0, 1, 2, 3],
            "selected_cols": [0, 1, 2],
            "bounds_pt": [48.0, 28.0, 260.0, 110.0]
        }),
    )?;
    gui.ui_invoke("ui.embeddedTable.format.open", json!({ "sheet_id": "g-wf58", "annotation_index": 0 }))?;
    gui.ui_answer_ok(json!({ "outer_border_width_pt": 0.0, "gridline_width_pt": 0.0, "transpose": true }))?;
    let svg58_before = export_graph_svg(&mut gui, "g-wf58")?;
    gui.set_cell("col-wf58", 0, 1, 0, "")?;
    let svg58_after = export_graph_svg(&mut gui, "g-wf58")?;
    ensure(svg58_before != svg58_after, "WF-58 deleting upstream cell must reactively update embedded result table SVG")?;

    // Save in --gui-only mode and verify full persistence in a fresh headless MCP child.
    save_and_verify_headless(&mut gui, &wd.dir, "wf51_58.curvebook", |headless| {
        let archive_bytes = std::fs::read(wd.dir.join("wf51_58.curvebook")).map_err(|e| format!("read wf51_58.curvebook: {e}"))?;
        let (_kind, saved_doc) = open_archive_bytes(&archive_bytes).map_err(|e| format!("open wf51_58.curvebook: {e}"))?;
        let rs51_doc = saved_doc.result_sheet(&SheetId::new(&res51)).ok_or("WF-51 missing persisted ResultSheet in archive")?;
        ensure(!rs51_doc.eliminated_outliers.is_empty(), "WF-51 persisted archive must retain ROUT eliminated outliers")?;

        let rs51 = read_result_json(headless, &res51)?;
        let tabs51 = rs51.get("tabs").and_then(Value::as_array).ok_or("WF-51 missing persisted tabs")?;
        ensure(
            tabs51.iter().any(|t| t.get("title").and_then(Value::as_str) == Some("Interpolated Mean X")),
            "WF-51 missing Interpolated Mean X tab",
        )?;
        ensure(
            tabs51.iter().any(|t| t.get("title").and_then(Value::as_str) == Some("Interpolated Replicates")),
            "WF-51 missing Interpolated Replicates tab",
        )?;
        ensure(
            tabs51.iter().any(|t| {
                t.get("title").and_then(Value::as_str).is_some_and(|s| s.contains("Comparison"))
                    || t.get("id").and_then(Value::as_str).is_some_and(|s| s.contains("cmp"))
            }),
            "WF-51 missing 3PL vs 4PL comparison tab",
        )?;
        ensure(
            rs51.get("checklist").and_then(Value::as_array).is_some_and(|c| {
                c.iter().any(|it| it.get("assumption").and_then(Value::as_str).is_some_and(|s| s.contains("R squared below threshold")))
            }),
            "WF-51 missing Flag R^2 checklist note",
        )?;
        let _rs51_norm = read_result_json(headless, "res-norm-wf51")?;

        let rs53 = read_result_json(headless, &res53)?;
        let _hr_tab = result_tab(&rs53, "hazard-ratio")?;
        ensure(
            rs53.get("scalars")
                .and_then(Value::as_array)
                .is_some_and(|s| s.iter().any(|it| it.get("key").and_then(Value::as_str).is_some_and(|k| k.contains("Hazard ratio")))),
            "WF-53 persisted 2-arm KM must expose Hazard ratio scalar",
        )?;

        let rs56 = read_result_json(headless, "res-lin-wf56")?;
        let curve56 = rs56
            .get("fitted_curves")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|c| c.get("points"))
            .and_then(Value::as_array)
            .ok_or("WF-56 missing persisted fitted curve points")?;
        let (min_x, max_x) = curve56
            .iter()
            .filter_map(|p| p.get("x").and_then(Value::as_f64))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| (a.min(x), b.max(x)));
        ensure(min_x >= 10.0 - 1e-9 && max_x <= 20.0 + 1e-9, format!("WF-56 persisted fitted curve escaped [10, 20]: ({min_x}, {max_x})"))?;

        let doc = headless.call_tool_payload("doc_inspect", json!({}))?;
        let graphs = doc.get("graphs").and_then(Value::as_array).ok_or("doc_inspect missing graphs")?;
        for tid in &created55 {
            ensure(
                graphs.iter().any(|g| g.get("id").and_then(Value::as_str) == Some(tid.as_str())),
                format!("WF-55 persisted document missing batch site graph `{tid}`"),
            )?;
        }

        headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "lay-wf57", "path": "wf57_reopened.svg" }))?;
        let lay_svg = std::fs::read_to_string(wd.dir.join("wf57_reopened.svg")).map_err(|e| format!("read wf57_reopened.svg: {e}"))?;
        validate_svg_structure(&lay_svg).map_err(|e| format!("reopened WF-57 layout SVG invalid: {e}"))?;
        ensure(lay_svg.contains("data:image/png;base64,"), "reopened WF-57 layout SVG missing embedded PNG")?;
        Ok(())
    })?;

    Ok(())
}

#[test]
fn r73_gui_only_wf59_to_wf65_r13_layouts_subset_heatmaps_and_designs() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf59_65")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-59: Layout Equalize Scaling, Transparent Overlay, Align Origins & Inset
    let fx59 = wf59_paired_before_after_and_inset();
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "col-wf59", "title": fx59.title, "kind": "column", "cols": 2, "rows": fx59.paired_rows.len() }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "col-wf59", "titles": fx59.paired_cols }))?;
    for (r, pair) in fx59.paired_rows.iter().enumerate() {
        gui.set_cell("col-wf59", r, 0, 0, &format!("{}", pair[0]))?;
        gui.set_cell("col-wf59", r, 1, 0, &format!("{}", pair[1]))?;
    }
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-box-wf59", "data_sheet_id": "col-wf59", "kind": "column" }))?;
    gui.ui_invoke("graph.setKind", json!({ "sheet_id": "g-box-wf59", "kind": "column", "option_id": "col.box_tukey" }))?;
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-ba-wf59", "data_sheet_id": "col-wf59", "kind": "column" }))?;
    gui.ui_invoke("graph.setKind", json!({ "sheet_id": "g-ba-wf59", "kind": "column", "option_id": "col.before_after" }))?;
    gui.ui_invoke(
        "graph.setAxes",
        json!({
            "sheet_id": "g-ba-wf59",
            "x_axis": { "tick_direction": "none", "show_tick_labels": false },
            "y_axis": { "tick_direction": "none", "show_tick_labels": false }
        }),
    )?;
    gui.ui_invoke("graph.setSpec", json!({ "sheet_id": "g-ba-wf59", "paper_fill": null }))?;
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "xy-inset-wf59", "title": "Linearized Inset", "kind": "xy", "cols": 1, "rows": fx59.inset_xy.len() }),
    )?;
    for (r, &(x, y)) in fx59.inset_xy.iter().enumerate() {
        gui.set_x_cell("xy-inset-wf59", r, 0, &format!("{x}"))?;
        gui.set_cell("xy-inset-wf59", r, 0, 0, &format!("{y}"))?;
    }
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-inset-wf59", "data_sheet_id": "xy-inset-wf59", "kind": "xy" }))?;
    gui.ui_invoke("layout.create", json!({ "sheet_id": "lay-wf59", "title": "Overlay Layout WF-59", "preset": "double_column" }))?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-box", "kind": "graph", "graph_sheet_id": "g-box-wf59", "bounds_pt": [20.0, 20.0, 240.0, 170.0] }),
    )?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-ba", "kind": "graph", "graph_sheet_id": "g-ba-wf59", "bounds_pt": [20.0, 20.0, 180.0, 130.0] }),
    )?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf59", "panel_id": "p-inset", "kind": "graph", "graph_sheet_id": "g-inset-wf59", "bounds_pt": [160.0, 30.0, 100.0, 80.0] }),
    )?;
    gui.ui_invoke("ui.layout.equalizeScaling.open", json!({ "sheet_id": "lay-wf59" }))?;
    let eq59 = gui.ui_answer_ok(json!({ "mode": "match_smallest", "panel_ids": ["p-box", "p-ba"] }))?;
    ensure(
        eq59.pointer("/payload/panels").and_then(Value::as_array).is_some_and(|arr| arr.len() == 2),
        "WF-59 EqualizeScaling must report both equalized panels",
    )?;
    gui.ui_invoke("layout.alignOrigins", json!({ "sheet_id": "lay-wf59", "align": true, "normalize_bounds": true }))?;
    gui.ui_invoke("layout.reorderPanel", json!({ "sheet_id": "lay-wf59", "panel_id": "p-inset", "action": "bring_to_front" }))?;
    let svg59_before = export_figure_svg(&mut gui, "lay-wf59")?;
    gui.set_cell("col-wf59", 0, 1, 0, "49.5")?;
    let svg59_after = export_figure_svg(&mut gui, "lay-wf59")?;
    ensure(svg59_before != svg59_after, "WF-59 upstream edit must recompute composite layout SVG")?;
    gui.ui_switch_sheet("lay-wf59")?;
    let _png59 = gui.ui_screenshot("canvas", 640, 400)?;

    // WF-60: Subset Linear Regression, Decoupled Points & Red Outlier Overrides
    let fx60 = wf60_subset_linear_with_outliers_xy();
    gui.ui_invoke("sheet.createData", json!({ "sheet_id": "xy-wf60", "title": fx60.title, "kind": "xy", "cols": 2, "rows": fx60.rows.len() }))?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "xy-wf60", "titles": ["All Points", "Included Subset"] }))?;
    for (r, &(x, y)) in fx60.rows.iter().enumerate() {
        gui.set_x_cell("xy-wf60", r, 0, &format!("{x}"))?;
        gui.set_cell("xy-wf60", r, 0, 0, &format!("{y}"))?;
        gui.set_cell("xy-wf60", r, 1, 0, &format!("{y}"))?;
    }
    gui.ui_invoke(
        "table.excludeCells",
        json!({
            "sheet_id": "xy-wf60",
            "cells": [
                { "row": fx60.outlier_rows[0], "col": 1, "subcol": 0 },
                { "row": fx60.outlier_rows[1], "col": 1, "subcol": 0 }
            ],
            "excluded": true
        }),
    )?;
    gui.ui_invoke(
        "analysis.run",
        json!({ "sheet_id": "res-lin-wf60", "source_sheet_id": "xy-wf60", "kind": "linear_regression", "selected_data_sets": [1] }),
    )?;
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-wf60", "data_sheet_id": "xy-wf60", "kind": "xy" }))?;
    gui.ui_invoke(
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
    gui.ui_invoke(
        "graph.formatSelectedPoints",
        json!({
            "sheet_id": "g-wf60",
            "points": [
                { "series_index": 0, "point_index": fx60.outlier_rows[0] },
                { "series_index": 0, "point_index": fx60.outlier_rows[1] }
            ],
            "color": "#E11D48"
        }),
    )?;
    let svg60_before = export_graph_svg(&mut gui, "g-wf60")?;
    ensure(svg60_before.to_ascii_lowercase().contains("#e11d48"), "WF-60 SVG must render red #E11D48 outlier point overrides")?;
    gui.ui_invoke(
        "table.excludeCells",
        json!({ "sheet_id": "xy-wf60", "cells": [{ "row": fx60.outlier_rows[0], "col": 1, "subcol": 0 }], "excluded": false }),
    )?;
    let svg60_after = export_graph_svg(&mut gui, "g-wf60")?;
    ensure(svg60_before != svg60_after, "WF-60 re-including outlier row must recompute subset regression SVG")?;

    // WF-61: Per-Column Normalized Heatmap & Vertical 90° Column Titles
    let fx61 = wf61_multivariate_biomarker_grouped();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf61",
            "title": fx61.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "cols": fx61.analyte_titles.len(),
            "rows": fx61.row_titles.len()
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "grp-wf61", "titles": fx61.analyte_titles }))?;
    gui.ui_invoke("table.setRowTitles", json!({ "sheet_id": "grp-wf61", "titles": fx61.row_titles }))?;
    for (r, row) in fx61.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            gui.set_cell("grp-wf61", r, c, 0, &format!("{val}"))?;
        }
    }
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-raw-wf61", "data_sheet_id": "grp-wf61", "kind": "heatmap" }))?;
    let _svg61_raw = export_graph_svg(&mut gui, "g-raw-wf61")?;
    gui.ui_invoke(
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
    gui.ui_invoke("graph.setKind", json!({ "sheet_id": "g-norm-wf61", "kind": "heatmap" }))?;
    gui.ui_invoke(
        "graph.setSpec",
        json!({
            "sheet_id": "g-norm-wf61",
            "heatmap_labels": { "row_labels": "row_titles", "column_labels": "column_titles", "column_angle": "vertical_90" }
        }),
    )?;
    let svg61_before = export_graph_svg(&mut gui, "g-norm-wf61")?;
    for title in &fx61.analyte_titles {
        ensure(svg61_before.contains(title), format!("WF-61 normalized heatmap SVG missing analyte label `{title}`"))?;
    }
    gui.set_cell("grp-wf61", 3, 0, 0, "950.0")?;
    let svg61_after = export_graph_svg(&mut gui, "g-norm-wf61")?;
    ensure(svg61_before != svg61_after, "WF-61 live cell peak edit must rescale normalized heatmap SVG")?;

    // WF-62: Five Experimental Designs (Nested, Paired Column & RM Two-Way ANOVA)
    let fx62 = wf62_five_design_trial();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "nest-wf62",
            "title": fx62.nested_title,
            "kind": "nested",
            "cols": fx62.nested_groups.len(),
            "replicates": fx62.nested_subjects_per_group,
            "rows": fx62.nested_reps_per_subject
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "nest-wf62", "titles": fx62.nested_groups }))?;
    for (group_idx, subjects) in fx62.nested_values.iter().enumerate() {
        for (subject_idx, reps) in subjects.iter().enumerate() {
            for (rep_idx, &val) in reps.iter().enumerate() {
                gui.set_cell("nest-wf62", rep_idx, group_idx, subject_idx, &format!("{val}"))?;
            }
        }
    }
    gui.ui_invoke("analysis.run", json!({ "sheet_id": "res-nest-wf62", "source_sheet_id": "nest-wf62", "kind": "nested_t" }))?;
    let flat_rows_62 = fx62.nested_subjects_per_group * fx62.nested_reps_per_subject;
    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "col-flat-wf62", "title": "Flattened Cohort", "kind": "column", "cols": fx62.nested_groups.len(), "rows": flat_rows_62 }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "col-flat-wf62", "titles": fx62.nested_groups }))?;
    for (group_idx, subjects) in fx62.nested_values.iter().enumerate() {
        let mut flat_r = 0usize;
        for reps in subjects {
            for &val in reps {
                gui.set_cell("col-flat-wf62", flat_r, group_idx, 0, &format!("{val}"))?;
                flat_r += 1;
            }
        }
    }
    let flat62_id = gui.run_analyze("col-flat-wf62", json!({ "entry_id": "stats.two_group", "two_group_method": "unpaired_t" }))?;
    gui.ui_switch_sheet("res-nest-wf62")?;
    let nest62_shot_before = gui.ui_screenshot("canvas", 640, 400)?;
    gui.set_cell("nest-wf62", 0, 0, 0, "24.8")?;
    gui.ui_switch_sheet("res-nest-wf62")?;
    let nest62_shot_after = gui.ui_screenshot("canvas", 640, 400)?;
    ensure(nest62_shot_before != nest62_shot_after, "WF-62 live nested replicate edit must recompute nested_t result")?;

    gui.ui_invoke(
        "sheet.createData",
        json!({ "sheet_id": "col-paired-wf62", "title": fx62.paired_title, "kind": "column", "cols": 2, "rows": fx62.paired_rows.len() }),
    )?;
    for (r, pair) in fx62.paired_rows.iter().enumerate() {
        gui.set_cell("col-paired-wf62", r, 0, 0, &format!("{}", pair[0]))?;
        gui.set_cell("col-paired-wf62", r, 1, 0, &format!("{}", pair[1]))?;
    }
    let paired62_id = gui.run_analyze("col-paired-wf62", json!({ "entry_id": "stats.two_group", "two_group_method": "paired_t" }))?;

    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "grp-rm-wf62",
            "title": fx62.rm_title,
            "kind": "grouped",
            "cols": 2,
            "replicates": 4,
            "rows": fx62.rm_timepoints.len()
        }),
    )?;
    gui.ui_invoke("table.setRowTitles", json!({ "sheet_id": "grp-rm-wf62", "titles": fx62.rm_timepoints }))?;
    for (r, flat_vals) in fx62.rm_rows.iter().enumerate() {
        for (flat_idx, &val) in flat_vals.iter().enumerate() {
            gui.set_cell("grp-rm-wf62", r, flat_idx / 4, flat_idx % 4, &format!("{val}"))?;
        }
    }
    let rm62_both_ref = gui.run_analyze("grp-rm-wf62", json!({ "entry_id": "stats.factorial_anova", "repeated_measures": "both" }))?;
    let rm62_id = gui.run_analyze("grp-rm-wf62", json!({ "entry_id": "stats.factorial_anova", "repeated_measures": "both" }))?;
    gui.ui_invoke("analysis.updateParams", json!({ "sheet_id": rm62_id, "repeated_measures": "row_factor" }))?;
    gui.ui_switch_sheet(&rm62_id)?;
    let _rm62_shot = gui.ui_screenshot("workspace", 1024, 640)?;

    // WF-63: Forest Plot via Upper/Lower Limits & Custom X=1.0 Null Reference
    let fx63 = wf63_meta_analysis_forest_grouped();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf63",
            "title": fx63.title,
            "kind": "grouped",
            "cols": 1,
            "rows": fx63.study_titles.len(),
            "subcolumn_mode": "upper_lower_limits"
        }),
    )?;
    gui.ui_invoke("table.setRowTitles", json!({ "sheet_id": "grp-wf63", "titles": fx63.study_titles }))?;
    for (r, entry) in fx63.entries.iter().enumerate() {
        gui.set_cell("grp-wf63", r, 0, 0, &format!("{}", entry[0]))?;
        gui.set_cell("grp-wf63", r, 0, 1, &format!("{}", entry[1]))?;
        gui.set_cell("grp-wf63", r, 0, 2, &format!("{}", entry[2]))?;
    }
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-wf63", "data_sheet_id": "grp-wf63", "kind": "forest" }))?;
    gui.ui_invoke(
        "graph.setAdditionalTicks",
        json!({ "sheet_id": "g-wf63", "axis": "x", "ticks": [{ "value": 1.0, "label": "No Effect", "draw_gridline": true }] }),
    )?;
    let svg63_before = export_graph_svg(&mut gui, "g-wf63")?;
    ensure(svg63_before.contains("No Effect"), "WF-63 forest SVG missing `No Effect` tick label")?;
    for title in &fx63.study_titles {
        ensure(svg63_before.contains(title), format!("WF-63 forest SVG missing study title `{title}`"))?;
    }
    gui.set_cell("grp-wf63", 0, 0, 2, "1.35")?;
    let svg63_after = export_graph_svg(&mut gui, "g-wf63")?;
    ensure(svg63_before != svg63_after, "WF-63 live upper-CI limit edit must update forest plot SVG")?;

    // WF-64: Population Pyramid via Horizontal Stacked Bars & |x| Unsigned Ticks
    let fx64 = wf64_demographics_pyramid_grouped();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "grp-wf64",
            "title": fx64.title,
            "kind": "grouped",
            "subcolumn_mode": SubcolumnMode::RawReplicates(1),
            "cols": fx64.cohort_titles.len(),
            "rows": fx64.band_titles.len()
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "grp-wf64", "titles": fx64.cohort_titles }))?;
    gui.ui_invoke("table.setRowTitles", json!({ "sheet_id": "grp-wf64", "titles": fx64.band_titles }))?;
    for (r, row) in fx64.rows.iter().enumerate() {
        gui.set_cell("grp-wf64", r, 0, 0, &format!("{}", row[0]))?;
        gui.set_cell("grp-wf64", r, 1, 0, &format!("{}", row[1]))?;
    }
    gui.ui_invoke("ui.view.rowTitles.toggle", json!({ "collapsed": false }))?;
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-wf64", "data_sheet_id": "grp-wf64", "kind": "grouped" }))?;
    gui.ui_invoke("graph.setKind", json!({ "sheet_id": "g-wf64", "kind": "grouped", "option_id": "grouped.stacked" }))?;
    gui.ui_invoke("graph.setSpec", json!({ "sheet_id": "g-wf64", "orientation": "horizontal" }))?;
    gui.ui_invoke("graph.setSeries", json!({ "sheet_id": "g-wf64", "all_series": true, "border_width_pt": 0.5 }))?;
    gui.ui_invoke("graph.setAxes", json!({ "sheet_id": "g-wf64", "x_axis": { "tick_format": "absolute_decimal" } }))?;
    let svg64_before = export_graph_svg(&mut gui, "g-wf64")?;
    ensure(
        !svg64_before.contains(">-20<") && !svg64_before.contains(">−20<"),
        "WF-64 AbsoluteDecimal pyramid SVG must not render signed negative X tick labels",
    )?;
    gui.set_cell("grp-wf64", 2, 0, 0, "-38.0")?;
    let svg64_after = export_graph_svg(&mut gui, "g-wf64")?;
    ensure(svg64_before != svg64_after, "WF-64 live pyramid value edit must update diverging bar SVG")?;

    // WF-65: Spaghetti Plots (Individual Replicate Trajectories & Multi-Subject Y)
    let fx65 = wf65_longitudinal_spaghetti_xy();
    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "xy-rep-wf65",
            "title": fx65.title,
            "kind": "xy",
            "cols": fx65.cohort_titles.len(),
            "replicates": 4,
            "rows": fx65.weeks.len()
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "xy-rep-wf65", "titles": fx65.cohort_titles }))?;
    for (r, (&week, cohorts)) in fx65.weeks.iter().zip(fx65.replicate_trajectories.iter()).enumerate() {
        gui.set_x_cell("xy-rep-wf65", r, 0, &format!("{week}"))?;
        for (g, reps) in cohorts.iter().enumerate() {
            for (s, &val) in reps.iter().enumerate() {
                gui.set_cell("xy-rep-wf65", r, g, s, &format!("{val}"))?;
            }
        }
    }
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-rep-wf65", "data_sheet_id": "xy-rep-wf65", "kind": "xy" }))?;
    gui.ui_invoke(
        "graph.setSeries",
        json!({ "sheet_id": "g-rep-wf65", "all_series": true, "marker": null, "replicate_lines": "individual_replicate_trajectories" }),
    )?;
    let svg65_before = export_graph_svg(&mut gui, "g-rep-wf65")?;
    gui.set_cell("xy-rep-wf65", 2, 1, 3, "52.5")?;
    let svg65_after = export_graph_svg(&mut gui, "g-rep-wf65")?;
    ensure(svg65_before != svg65_after, "WF-65 single-subcolumn replicate edit must update spaghetti trajectory SVG")?;

    gui.ui_invoke(
        "sheet.createData",
        json!({
            "sheet_id": "xy-subj-wf65",
            "title": "Single-Y Subject Traces",
            "kind": "xy",
            "cols": fx65.subject_titles.len(),
            "rows": fx65.weeks.len()
        }),
    )?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": "xy-subj-wf65", "titles": fx65.subject_titles }))?;
    for (r, (&week, vals)) in fx65.weeks.iter().zip(fx65.single_y_subjects.iter()).enumerate() {
        gui.set_x_cell("xy-subj-wf65", r, 0, &format!("{week}"))?;
        for (c, &val) in vals.iter().enumerate() {
            gui.set_cell("xy-subj-wf65", r, c, 0, &format!("{val}"))?;
        }
    }
    gui.ui_invoke("graph.create", json!({ "sheet_id": "g-subj-wf65", "data_sheet_id": "xy-subj-wf65", "kind": "xy" }))?;
    gui.ui_invoke("graph.setSeries", json!({ "sheet_id": "g-subj-wf65", "all_series": true, "marker": null }))?;
    let _svg65_subj = export_graph_svg(&mut gui, "g-subj-wf65")?;
    gui.ui_switch_sheet("g-rep-wf65")?;
    let _png65 = gui.ui_screenshot("canvas", 640, 400)?;

    // Save in --gui-only mode and verify full persistence in a fresh headless MCP child.
    save_and_verify_headless(&mut gui, &wd.dir, "wf59_65.curvebook", |headless| {
        let rs60 = read_result_json(headless, "res-lin-wf60")?;
        ensure(rs60.get("status") == Some(&json!("UpToDate")), format!("WF-60 persisted subset regression status: {}", rs60["status"]))?;

        let rs_nest = read_result_json(headless, "res-nest-wf62")?;
        let rs_flat = read_result_json(headless, &flat62_id)?;
        let rs_paired = read_result_json(headless, &paired62_id)?;
        let rs_rm_both = read_result_json(headless, &rm62_both_ref)?;
        let rs_rm = read_result_json(headless, &rm62_id)?;
        for (label, rs) in [
            ("nested_t", &rs_nest),
            ("unpaired_t", &rs_flat),
            ("paired_t", &rs_paired),
            ("factorial_rm_both", &rs_rm_both),
            ("factorial_rm_row", &rs_rm),
        ] {
            ensure(rs.get("status") == Some(&json!("UpToDate")), format!("WF-62 persisted {label} status: {}", rs["status"]))?;
        }
        ensure(rs_nest["tabs"] != rs_flat["tabs"], "WF-62 nested_t and flattened unpaired_t must retain distinct DF/summaries")?;
        ensure(rs_rm_both["tabs"] != rs_rm["tabs"], "WF-62 switching RM design between both and row_factor must update ANOVA tabs")?;

        for (sheet, fname) in
            [("lay-wf59", "wf59.svg"), ("g-norm-wf61", "wf61.svg"), ("g-wf63", "wf63.svg"), ("g-wf64", "wf64.svg"), ("g-rep-wf65", "wf65.svg")]
        {
            headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": sheet, "path": fname }))?;
            let svg = std::fs::read_to_string(wd.dir.join(fname)).map_err(|e| format!("read {fname}: {e}"))?;
            validate_svg_structure(&svg).map_err(|e| format!("reopened {sheet} SVG invalid: {e}"))?;
        }
        Ok(())
    })?;

    Ok(())
}
