//! Strict `--gui-only` end-to-end GUI acceptance workflows `WF-51`..`WF-58` (Shard 8, `R13.6`).
//!
//! Every workflow boots [`UiApp::new()`] with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`),
//! seeds clean-room synthetic fixtures from `curvebook_testkit::r13_datasets`, and drives all
//! interactions exclusively through [`curvebook_ui_egui::dispatch_gui_action`]: `ui.invoke_menu`,
//! `ui.switch_tab`, `ui.type_text`, `ui.press_key`, `ui.pointer_gesture`, `ui.answer_dialog`,
//! `ui.inspect`, and `ui.screenshot`. Zero direct `app.execute_command` calls appear anywhere in
//! this suite.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    CartesianOrientation, EmbeddedTableStyle, GraphAnnotation, GraphicalDataObject, LayoutItem, ProjectDocument, RegressionOverlaySpec, SheetId,
    SummaryMeasure, SurvivalDisplayMetric, TickDirection, TickLabelFormat,
};
use curvebook_palette::Rgba8;
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_render::{RasterImage, RenderTargetSpec, encode_png, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_testkit::r13_datasets::{
    wf51_inhibitor_triplicate_xy, wf52_sparse_agonist_4pl_xy, wf53_three_arm_survival, wf54_dual_axis_mean_sd_n_xy, wf55_multisite_grouped_cohort,
    wf56_exponential_growth_xy, wf57_schematic_rgba, wf58_descriptive_column,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

use crate::common::{decode_base64, default_metrics, encode_base64, ensure, new_strict_gui_app};

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

fn path_x_bounds(path: &curvebook_chart::Path2D) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for cmd in &path.commands {
        let pts: &[curvebook_chart::PointPt] = match cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => std::slice::from_ref(p),
            PathCmd::QuadTo { to, .. } | PathCmd::CubicTo { to, .. } => std::slice::from_ref(to),
            PathCmd::Close => &[],
        };
        for p in pts {
            lo = lo.min(p.x);
            hi = hi.max(p.x);
        }
    }
    (lo, hi)
}

// ---------------------------------------------------------------------------
// WF-51: Nonlinear ROUT, Dual Replicate Interpolation, 3PL/4PL & Normalize
// ---------------------------------------------------------------------------

#[test]
fn wf51_nonlinear_rout_replicate_interp_compare_and_relative_ic50() -> TestResult {
    let mut app = new_strict_gui_app();
    assert_strict_gui_only_guards(&mut app)?;
    let fx = wf51_inhibitor_triplicate_xy();
    let total_rows = fx.known_rows.len() + fx.unknown_rows.len();

    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "xy-wf51",
            "title": fx.title,
            "kind": "xy",
            "cols": 1,
            "rows": total_rows,
            "subcolumn_mode": "replicates",
            "replicates": 3,
            "auto_graph": true,
            "graph_sheet_id": "g-wf51"
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "xy-wf51", "titles": [fx.data_set_title] }))?;
    for (r, pt) in fx.known_rows.iter().enumerate() {
        type_x_cell(&mut app, "xy-wf51", r, 0, &format!("{}", pt.log_dose))?;
        for (s, val) in pt.replicates.iter().enumerate() {
            type_grid_cell(&mut app, "xy-wf51", r, 0, s, &format!("{val}"))?;
        }
    }
    for (u_idx, reps) in fx.unknown_rows.iter().enumerate() {
        let r = fx.known_rows.len() + u_idx;
        for (s, val) in reps.iter().enumerate() {
            type_grid_cell(&mut app, "xy-wf51", r, 0, s, &format!("{val}"))?;
        }
    }

    let res_id = run_analyze_dialog(
        &mut app,
        "xy-wf51",
        json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl", "fit_method": "rout", "rout_q": 1.0 }),
    )?;
    // Apply Extra-SS F comparison vs 3PL, unknown interpolation, and Flag R^2 < 0.9999 via Cmd+T -> analysis.updateParams.
    switch_sheet(&mut app, &res_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }), "Cmd+T opens EditAnalysisParameters")?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "cancel" }))?;
    menu(
        &mut app,
        "analysis.updateParams",
        json!({
            "sheet_id": res_id,
            "interpolate_unknowns": true,
            "flag_r_squared_below": 0.999999,
            "comparison": { "method": "extra_sum_of_squares_f", "alternative_equation_id": "log_inhibitor_vs_response_3pl" }
        }),
    )?;

    let rep_val_before = {
        let rs = app.document().result_sheet(&SheetId::new(&res_id)).ok_or("missing WF-51 result")?;
        ensure(!rs.eliminated_outliers.is_empty(), "ROUT Q=1% must eliminate injected outlier")?;
        ensure(rs.tabs.iter().any(|t| t.title == "Interpolated Mean X"), "missing Interpolated Mean X tab")?;
        let rep_tab = rs.tabs.iter().find(|t| t.title == "Interpolated Replicates").ok_or("missing Interpolated Replicates tab")?;
        ensure(rs.tabs.iter().any(|t| t.title.contains("Comparison") || t.id.contains("cmp")), "missing 3PL vs 4PL comparison tab")?;
        ensure(
            rs.checklist.iter().any(|c| c.assumption.contains("R squared below threshold")),
            format!("missing Flag R^2 checklist note: {:?}", rs.checklist),
        )?;
        format!("{:?}", rep_tab.rows)
    };

    // Attach overlay + confirm outlier marker badge compiles on GraphSheet.
    menu(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf51",
            "series_index": 0,
            "regression_overlay": {
                "result_sheet_id": res_id,
                "sample_segments": 256,
                "show_confidence_band": false,
                "show_prediction_band": false,
                "shaded_ribbon": false
            }
        }),
    )?;
    let _scene = compile_clean(app.document(), "g-wf51")?;

    // Chain Normalize for relative IC50 scale + edit unknown replicate cell and observe Interpolated Replicates change.
    menu(
        &mut app,
        "transform.run",
        json!({ "sheet_id": "res-norm-wf51", "source_sheet_id": "xy-wf51", "kind": "normalize", "zero_anchor": "smallest", "hundred_anchor": "largest" }),
    )?;
    ensure(app.document().result_sheet(&SheetId::new("res-norm-wf51")).is_some(), "missing chained normalize sheet")?;

    type_grid_cell(&mut app, "xy-wf51", fx.known_rows.len(), 0, 1, "64.0")?;
    let rs_after = app.document().result_sheet(&SheetId::new(&res_id)).ok_or("missing WF-51 result after edit")?;
    let rep_val_after =
        format!("{:?}", rs_after.tabs.iter().find(|t| t.title == "Interpolated Replicates").ok_or("missing reps tab after edit")?.rows);
    ensure(rep_val_after != rep_val_before, "editing unknown replicate must update Interpolated Replicates tab")?;

    switch_sheet(&mut app, "g-wf51")?;
    let _png = assert_screenshot_png(&mut app, 320, 200)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-52: 4PL Identifiability Recovery, Extrapolation Banner & Normal Q-Q Drill
// ---------------------------------------------------------------------------

#[test]
fn wf52_4pl_identifiability_recovery_extrapolation_and_qq_drillback() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf52_sparse_agonist_4pl_xy();

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "xy-wf52", "title": fx.title, "kind": "xy", "cols": 2, "rows": 5, "auto_graph": true, "graph_sheet_id": "g-wf52" }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "xy-wf52", "titles": [fx.series[0].title, fx.series[1].title] }))?;
    for (r, &(x, y0)) in fx.series[0].rows.iter().enumerate() {
        type_x_cell(&mut app, "xy-wf52", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-wf52", r, 0, 0, &format!("{y0}"))?;
        type_grid_cell(&mut app, "xy-wf52", r, 1, 0, &format!("{}", fx.series[1].rows[r].1))?;
    }

    let res_id = run_analyze_dialog(&mut app, "xy-wf52", json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_4pl" }))?;
    menu(&mut app, "analysis.updateParams", json!({ "sheet_id": res_id, "identify_unstable_parameters": true }))?;

    {
        let rs = app.document().result_sheet(&SheetId::new(&res_id)).ok_or("missing WF-52 result")?;
        let flagged = rs.checklist.iter().any(|c| c.assumption.contains("Ambiguous") || c.assumption.to_ascii_lowercase().contains("extrapol"))
            || rs.tabs.iter().any(|t| t.row_labels.iter().any(|l| l.contains("(Unstable)")));
        ensure(flagged, "sparse top-less 4PL must flag identifiability ambiguity or midpoint extrapolation")?;
    }

    // Recover via Bottom=Constant(0) + SharedGlobal HillSlope & Top.
    menu(
        &mut app,
        "analysis.updateParams",
        json!({
            "sheet_id": res_id,
            "identify_unstable_parameters": true,
            "parameter_rules": [
                { "name": "Bottom", "scope": "constant", "value": 0.0 },
                { "name": "HillSlope", "scope": "shared_global" },
                { "name": "Top", "scope": "shared_global" }
            ]
        }),
    )?;

    // Build Normal Q-Q diagnostic plot and check plotted_points source_cell drill-back coordinates.
    menu(&mut app, "graph.create", json!({ "sheet_id": "g-wf52-qq", "data_sheet_id": res_id, "kind": "qq" }))?;
    let qq_scene = compile_clean(app.document(), "g-wf52-qq")?;
    ensure(
        !qq_scene.plotted_points.is_empty() && qq_scene.plotted_points.iter().any(|p| !p.source_cell.sheet_id.as_str().is_empty()),
        "Normal Q-Q points must carry source_cell drill-back",
    )?;

    // Live cell correction recomputes ResultSheet cleanly.
    let before_tabs = format!("{:?}", app.document().result_sheet(&SheetId::new(&res_id)).ok_or("missing rs")?.tabs.first());
    type_grid_cell(&mut app, "xy-wf52", 4, 0, 0, "58.0")?;
    let after_tabs = format!("{:?}", app.document().result_sheet(&SheetId::new(&res_id)).ok_or("missing rs2")?.tabs.first());
    ensure(before_tabs != after_tabs, "cell correction must update 4PL recovery result sheet")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-53: Kaplan-Meier Display Modes & Column Exclude Transition to HR CI
// ---------------------------------------------------------------------------

#[test]
fn wf53_kaplan_meier_display_modes_and_column_exclude_hr() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf53_three_arm_survival();

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "surv-wf53", "title": fx.title, "kind": "survival", "cols": 3, "rows": fx.rows.len(), "auto_graph": true, "graph_sheet_id": "g-wf53" }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "surv-wf53", "titles": fx.cohorts }))?;
    for (r, entry) in fx.rows.iter().enumerate() {
        if let Some(t) = entry.elapsed_days {
            type_x_cell(&mut app, "surv-wf53", r, 0, &format!("{t}"))?;
        }
        for (c, status) in entry.status.iter().enumerate() {
            if let Some(code) = status {
                type_grid_cell(&mut app, "surv-wf53", r, c, 0, &format!("{code}"))?;
            }
        }
    }

    let res_id = run_analyze_dialog(&mut app, "surv-wf53", json!({ "entry_id": "stats.survival_km" }))?;
    let _scene_default = compile_clean(app.document(), "g-wf53")?;

    menu(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf53",
            "all_series": true,
            "display_metric": "percent_cumulative_event",
            "staircase": false,
            "error_display": "sem"
        }),
    )?;
    let gs = app.document().graph_sheet(&SheetId::new("g-wf53")).ok_or("missing g-wf53")?;
    ensure(
        gs.spec.series.iter().all(|s| {
            matches!(
                s.gdo,
                GraphicalDataObject::KaplanMeierStep {
                    display_metric: SurvivalDisplayMetric::PercentCumulativeEvent,
                    staircase: false,
                    error_display: Some(curvebook_doc::ErrorBarMetric::Sem),
                    ..
                }
            )
        }),
        "KM series must adopt CumulativeEvent + PointToPoint + SEM",
    )?;
    let _scene_ev = compile_clean(app.document(), "g-wf53")?;

    // Exclude Vehicle column (col 0) across all rows -> reduces 3-arm KM to 2-arm Hazard Ratio + 95% CI.
    let cells: Vec<Value> = (0..fx.rows.len()).map(|r| json!({ "row": r, "col": 0, "subcol": 0 })).collect();
    menu(&mut app, "table.excludeCells", json!({ "sheet_id": "surv-wf53", "cells": cells, "excluded": true }))?;

    let rs_after = app.document().result_sheet(&SheetId::new(&res_id)).ok_or("missing KM result after exclusion")?;
    let hr_present = rs_after.tabs.iter().any(|t| t.id == "hazard-ratio") && rs_after.scalars.iter().any(|s| s.key.contains("Hazard ratio"));
    ensure(hr_present, "excluding 3rd cohort must expose 2-arm Hazard ratio tab and scalar")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-54: Dual-Axis Nudged XY Bars, 50% Alpha Lines & Selective Decay Fit
// ---------------------------------------------------------------------------

#[test]
fn wf54_dual_axis_nudged_bars_alpha_lines_and_selective_decay_fit() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf54_dual_axis_mean_sd_n_xy();

    menu(
        &mut app,
        "sheet.createData",
        json!({
            "sheet_id": "xy-wf54",
            "title": fx.title,
            "kind": "xy",
            "cols": 2,
            "rows": fx.x_hours.len(),
            "subcolumn_mode": "mean_sd_n",
            "auto_graph": true,
            "graph_sheet_id": "g-wf54"
        }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "xy-wf54", "titles": fx.series_titles }))?;
    for (r, &x) in fx.x_hours.iter().enumerate() {
        type_x_cell(&mut app, "xy-wf54", r, 0, &format!("{x}"))?;
        let f = fx.fluorescence[r];
        let m = fx.metabolite[r];
        for (s, v) in [f.mean, f.sd, f.n].iter().enumerate() {
            type_grid_cell(&mut app, "xy-wf54", r, 0, s, &format!("{v}"))?;
        }
        for (s, v) in [m.mean, m.sd, m.n].iter().enumerate() {
            type_grid_cell(&mut app, "xy-wf54", r, 1, s, &format!("{v}"))?;
        }
    }

    let fit_id =
        run_analyze_dialog(&mut app, "xy-wf54", json!({ "entry_id": "fit.nonlinear", "equation_id": "one_phase_decay", "selected_data_sets": [0] }))?;
    menu(&mut app, "graph.setAxes", json!({ "sheet_id": "g-wf54", "y2_axis": { "title": "Metabolite Pool (uM)" } }))?;
    menu(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf54",
            "series_index": 0,
            "alpha": 0.5,
            "regression_overlay": RegressionOverlaySpec {
                result_sheet_id: SheetId::new(&fit_id),
                sample_segments: 256,
                show_confidence_band: false,
                show_prediction_band: false,
                shaded_ribbon: false
            }
        }),
    )?;
    let bar_gdo = GraphicalDataObject::BarWithReplicates {
        summary: SummaryMeasure::Mean,
        fill_color: Rgba8::rgba(210, 120, 40, 200),
        border_color: Rgba8::BLACK,
        border_width_pt: 0.8,
        replicate_overlay: None,
    };
    menu(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-wf54",
            "series_index": 1,
            "bind_to_right_y2": true,
            "gdo": bar_gdo,
            "offset": { "x_data": 0.5, "x_pt": 0.0, "y_pt": 0.0 },
            "bar_width_pt": 14.0
        }),
    )?;

    let scene_before = compile_clean(app.document(), "g-wf54")?;
    let svg_before = render_svg(&scene_before, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    type_grid_cell(&mut app, "xy-wf54", 1, 0, 0, "84.5")?;
    let scene_after = compile_clean(app.document(), "g-wf54")?;
    let svg_after = render_svg(&scene_after, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    ensure(svg_before != svg_after, "live mean edit must update dual-axis decay curve & bars")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-55: Grouped Transpose -> 12 Batch Graphs & Batch Apply Style From
// ---------------------------------------------------------------------------

#[test]
fn wf55_grouped_transpose_batch_graphs_and_gallery_apply_style() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf55_multisite_grouped_cohort();

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "grp-wf55", "title": fx.title, "kind": "grouped", "subcolumn_mode": curvebook_doc::SubcolumnMode::RawReplicates(1), "rows": fx.row_titles.len(), "cols": fx.col_titles.len() }),
    )?;
    menu(&mut app, "table.setRowTitles", json!({ "sheet_id": "grp-wf55", "titles": fx.row_titles }))?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "grp-wf55", "titles": fx.col_titles }))?;
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            type_grid_cell(&mut app, "grp-wf55", r, c, 0, &format!("{val}"))?;
        }
    }

    let tr_out = menu(
        &mut app,
        "transform.run",
        json!({ "sheet_id": "res-tr-wf55", "source_sheet_id": "grp-wf55", "kind": "transpose", "swap_row_and_column_titles": true }),
    )?;
    let tr_id = tr_out.pointer("/payload/sheet_id").and_then(Value::as_str).unwrap_or("res-tr-wf55").to_owned();

    menu(&mut app, "ui.graph.create.open", json!({ "source_sheet_id": tr_id }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CreateGraph { .. }), "CreateGraph dialog must open")?;
    let batch_out = gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": { "source_sheet_id": tr_id, "one_per_data_set": true, "y_axis_title": "Quarter Index" },
            "action": "ok"
        }),
    )?;
    let created_ids: Vec<String> = batch_out
        .pointer("/payload/sheet_ids")
        .and_then(Value::as_array)
        .ok_or("expected batch created sheet_ids")?
        .iter()
        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();
    ensure(created_ids.len() == 12, format!("expected 12 site graphs, got {}", created_ids.len()))?;

    let donor = &created_ids[0];
    let targets: Vec<String> = created_ids[1..].to_vec();
    menu(&mut app, "graph.setSpec", json!({ "sheet_id": donor, "paper_fill": "#F3F5FA" }))?;
    menu(&mut app, "graph.applyStyleFrom", json!({ "donor_sheet_id": donor, "target_sheet_ids": targets }))?;
    for (offset, tid) in targets.iter().enumerate() {
        let gs = app.document().graph_sheet(&SheetId::new(tid)).ok_or("missing batch target graph")?;
        ensure(gs.spec.paper_fill == Some(Rgba8::rgb(243, 245, 250)), "paper_fill style must clone across batch targets")?;
        ensure(gs.spec.series[0].data_set_index == offset + 1, "batch applyStyleFrom must preserve dataset bindings")?;
    }

    let scene_before = compile_clean(app.document(), &created_ids[0])?;
    type_grid_cell(&mut app, "grp-wf55", 0, 2, 0, "140.0")?;
    let scene_after = compile_clean(app.document(), &created_ids[0])?;
    ensure(
        render_svg(&scene_before, &SvgExportOptions::default()).map_err(|e| e.to_string())?
            != render_svg(&scene_after, &SvgExportOptions::default()).map_err(|e| e.to_string())?,
        "2-hop cell edit -> transpose -> batch site graph must re-render",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-56: Log10 Transform, Antilog Ticks & Restricted X Range Fit Invariance
// ---------------------------------------------------------------------------

#[test]
fn wf56_log_transform_antilog_axis_and_restricted_x_range_fit() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf56_exponential_growth_xy();

    menu(&mut app, "sheet.createData", json!({ "sheet_id": "xy-wf56", "title": fx.title, "kind": "xy", "cols": 1, "rows": fx.rows.len() }))?;
    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        type_x_cell(&mut app, "xy-wf56", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-wf56", r, 0, 0, &format!("{y}"))?;
    }

    menu(&mut app, "transform.run", json!({ "sheet_id": "res-log-wf56", "source_sheet_id": "xy-wf56", "kind": "math", "y_transform": "log10" }))?;
    menu(
        &mut app,
        "analysis.run",
        json!({ "sheet_id": "res-lin-wf56", "source_sheet_id": "res-log-wf56", "kind": "linear_regression", "x_range_filter": [10.0, 20.0] }),
    )?;
    menu(&mut app, "graph.create", json!({ "sheet_id": "g-wf56", "data_sheet_id": "res-log-wf56", "kind": "xy" }))?;
    menu(
        &mut app,
        "graph.setAxes",
        json!({ "sheet_id": "g-wf56", "y_axis": { "tick_format": "antilog", "segments": [{ "minor_ticks_per_interval": 9 }] } }),
    )?;
    menu(
        &mut app,
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

    let (x0, x1) = {
        let rs = app.document().result_sheet(&SheetId::new("res-lin-wf56")).ok_or("missing restricted linear fit")?;
        let curve = rs.fitted_curves.first().ok_or("missing fitted curve overlay")?;
        let xs = curve.points.iter().map(|pt| pt.x).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| (a.min(x), b.max(x)));
        ensure(xs.0 >= 10.0 - 1e-9 && xs.1 <= 20.0 + 1e-9, format!("fitted curve escaped [10, 20]: {xs:?}"))?;
        let gs = app.document().graph_sheet(&SheetId::new("g-wf56")).ok_or("missing g-wf56")?;
        ensure(gs.spec.y_axis.tick_format == TickLabelFormat::Antilog, "expected Antilog Y tick format")?;
        let _scene = compile_clean(app.document(), "g-wf56")?;
        let summary_before = format!("{:?}", rs.tabs.first());
        // Edit point inside X < 10 -> fit parameters stay unchanged.
        type_grid_cell(&mut app, "xy-wf56", 0, 0, 0, "999.0")?;
        let rs_invar = app.document().result_sheet(&SheetId::new("res-lin-wf56")).ok_or("missing res after X<10 edit")?;
        let summary_invar = format!("{:?}", rs_invar.tabs.first());
        ensure(summary_invar == summary_before, "edit at X < 10.0 must leave [10, 20] restricted fit unchanged")?;
        xs
    };
    let _ = (x0, x1);

    let before_recompute = format!("{:?}", app.document().result_sheet(&SheetId::new("res-lin-wf56")).ok_or("missing rs")?.tabs.first());
    type_grid_cell(&mut app, "xy-wf56", 8, 0, 0, "500.0")?;
    let after_recompute = format!("{:?}", app.document().result_sheet(&SheetId::new("res-lin-wf56")).ok_or("missing rs2")?.tabs.first());
    ensure(after_recompute != before_recompute, "edit at X >= 10.0 must recompute restricted fit")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-57: Layout Embedded Image Crop/Rotate & Non-Exported Grid Overlay
// ---------------------------------------------------------------------------

#[test]
fn wf57_layout_embedded_image_crop_rotate_and_non_exported_grid() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf57_schematic_rgba();
    let png = encode_png(&RasterImage { width_px: fx.width_px, height_px: fx.height_px, dpi: 96, rgba: fx.rgba })
        .map_err(|e| format!("encode_png failed: {e}"))?;
    let png_b64 = encode_base64(&png);

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "xy-wf57", "title": "Panel Assay", "kind": "xy", "cols": 1, "rows": fx.assay_xy.len(), "auto_graph": true, "graph_sheet_id": "g-wf57" }),
    )?;
    for (r, &(x, y)) in fx.assay_xy.iter().enumerate() {
        type_x_cell(&mut app, "xy-wf57", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-wf57", r, 0, 0, &format!("{y}"))?;
    }
    menu(&mut app, "layout.create", json!({ "sheet_id": "lay-wf57", "title": "2x2 Schematic Layout", "preset": "double_column" }))?;
    menu(
        &mut app,
        "layout.addPanel",
        json!({ "sheet_id": "lay-wf57", "panel_id": "p-graph", "kind": "graph", "graph_sheet_id": "g-wf57", "bounds_pt": [20.0, 20.0, 220.0, 150.0] }),
    )?;
    menu(&mut app, "layout.insertImage", json!({ "sheet_id": "lay-wf57", "bytes_base64": png_b64, "bounds_pt": [260.0, 24.0, 120.0, 80.0] }))?;

    menu(&mut app, "ui.image.format.open", json!({ "sheet_id": "lay-wf57", "item_index": 1 }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatImage { .. }), "FormatImage dialog must open")?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
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
            },
            "action": "ok"
        }),
    )?;

    // Toggle viewport-only canvas grid overlay on; verify exports include raster image cleanly without grid overlay.
    let grid_on = menu(&mut app, "ui.gridOverlay.toggle", json!({ "visible": true, "spacing_pt": 24.0 }))?;
    ensure(grid_on["result"]["grid_overlay_visible"] == json!(true), "grid overlay toggle must activate viewport grid")?;

    let lay = app.document().layout_sheet(&SheetId::new("lay-wf57")).ok_or("missing lay-wf57")?;
    ensure(
        matches!(
            lay.spec.items.get(1),
            Some(LayoutItem::EmbeddedImage { bounds_pt, rotation_deg, lock_aspect_ratio: false, border: Some(_), .. })
            if (bounds_pt.width - 150.0).abs() < 1e-6 && (*rotation_deg - 15.0).abs() < 1e-6
        ),
        "FormatImage updates not persisted on LayoutItem",
    )?;
    let l_scene = compile_layout(lay, app.document()).map_err(|e| format!("compile_layout failed: {e}"))?;
    let raster_count = l_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::RasterImage(_))).count();
    ensure(raster_count == 1, format!("expected 1 RasterImage primitive in layout export, got {raster_count}"))?;
    let svg = render_svg(&l_scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    validate_svg_structure(&svg).map_err(|e| e.to_string())?;
    ensure(svg.contains("data:image/png;base64,"), "SVG export missing embedded PNG data URI")?;
    let pdf = render_pdf(&l_scene, &PdfExportOptions::default()).map_err(|e| e.to_string())?;
    ensure(pdf.starts_with(b"%PDF-1.7"), "PDF export missing header")?;
    let rgba = render_scene(&l_scene, RenderTargetSpec { width_px: 400, height_px: 260, dpi: 96 }).map_err(|e| e.to_string())?;
    let exported_png = encode_png(&rgba).map_err(|e| e.to_string())?;
    ensure(exported_png.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG export missing header")?;

    // Toggle grid off again: exported SVG bytes must be byte-for-byte identical.
    menu(&mut app, "ui.gridOverlay.toggle", json!({ "visible": false }))?;
    let svg_off = render_svg(
        &compile_layout(app.document().layout_sheet(&SheetId::new("lay-wf57")).ok_or("missing lay2")?, app.document()).map_err(|e| e.to_string())?,
        &SvgExportOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    ensure(svg == svg_off, "canvas grid overlay must never alter exported vector output")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-58: Result Slice Paste Table, Borderless Transpose & Upstream Reactive
// ---------------------------------------------------------------------------

#[test]
fn wf58_result_slice_paste_table_transpose_borderless_reactive() -> TestResult {
    let mut app = new_strict_gui_app();
    let fx = wf58_descriptive_column();

    menu(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "col-wf58", "title": fx.title, "kind": "column", "cols": 4, "rows": fx.rows.len(), "auto_graph": true, "graph_sheet_id": "g-wf58" }),
    )?;
    menu(&mut app, "table.setGroupTitles", json!({ "sheet_id": "col-wf58", "titles": fx.group_titles }))?;
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            type_grid_cell(&mut app, "col-wf58", r, c, 0, &format!("{val}"))?;
        }
    }

    let res_id = run_analyze_dialog(&mut app, "col-wf58", json!({ "entry_id": "stats.descriptive" }))?;
    menu(
        &mut app,
        "edit.pasteResultTable",
        json!({
            "source_sheet_id": res_id,
            "target_sheet_id": "g-wf58",
            "selected_rows": [0, 1, 2, 3],
            "selected_cols": [0, 1, 2],
            "bounds_pt": [48.0, 28.0, 260.0, 110.0]
        }),
    )?;

    menu(&mut app, "ui.embeddedTable.format.open", json!({ "sheet_id": "g-wf58", "annotation_index": 0 }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatEmbeddedTable { .. }), "FormatEmbeddedTable dialog must open")?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({ "fields": { "outer_border_width_pt": 0.0, "gridline_width_pt": 0.0, "transpose": true }, "action": "ok" }),
    )?;

    let scene_before = compile_clean(app.document(), "g-wf58")?;
    let gs = app.document().graph_sheet(&SheetId::new("g-wf58")).ok_or("missing g-wf58")?;
    ensure(
        matches!(
            gs.spec.annotations.first(),
            Some(GraphAnnotation::EmbeddedResultTable {
                style: EmbeddedTableStyle { outer_border_width_pt, gridline_width_pt, transpose: true, .. },
                ..
            }) if *outer_border_width_pt == 0.0 && *gridline_width_pt == 0.0
        ),
        "FormatEmbeddedTable must persist borderless transpose settings",
    )?;
    let texts_before = scene_texts(&scene_before);
    ensure(!texts_before.is_empty(), "embedded table must render text cells")?;

    // Delete upstream cell -> descriptive summary & embedded table update automatically.
    type_grid_cell(&mut app, "col-wf58", 0, 1, 0, "")?;
    let scene_after = compile_clean(app.document(), "g-wf58")?;
    let set_before: BTreeSet<String> = texts_before.into_iter().collect();
    let set_after: BTreeSet<String> = scene_texts(&scene_after).into_iter().collect();
    ensure(set_before != set_after, "deleting upstream cell must reactively update embedded result table")?;
    let _ = (CartesianOrientation::Vertical, TickDirection::Outside, path_x_bounds);
    Ok(())
}
