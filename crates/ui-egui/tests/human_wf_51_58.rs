#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-51`–`WF-58`, Shard 8, `R13.6`).
//!
//! Exercises workflows `WF-51` through `WF-58` on [`EguiHumanPathDriver`] via real pointer,
//! wheel, drag, modifier-click, keyboard, and IME events under `GuiOnlyPolicy::Strict` and
//! `HumanPathOnly`, passing `14`-locale `gui-layout-spec.md` §9.4 parity and latency budgets.

mod human_wf_common;

use std::sync::Arc;

use curvebook_chart::{compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorBarMetric, FramePreset, GraphAnnotation, GraphicalDataObject, GroupedTable,
    LayoutItem, ParameterScope, ProjectDocument, ResultCell, ResultStatus, ResultTabKind, SheetId, SubcolumnMode, SurvivalDisplayMetric,
    SurvivalTable, TableArchetype, TickLabelFormat, XyTable,
};
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_render::{RasterImage, RenderTargetSpec, encode_png, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_testkit::r13_datasets::{
    wf51_inhibitor_triplicate_xy, wf52_sparse_agonist_4pl_xy, wf53_three_arm_survival, wf54_dual_axis_mean_sd_n_xy, wf55_multisite_grouped_cohort,
    wf56_exponential_growth_xy, wf57_schematic_rgba, wf58_descriptive_column,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use egui::{Key, Modifiers};
use serde_json::json;

use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use human_wf_common::{TestRes, ensure};

fn graph_for_source(doc: &ProjectDocument, source_id: &SheetId) -> Result<SheetId, String> {
    doc.graph_sheets
        .iter()
        .rfind(|g| g.spec.primary_source_sheet_id == *source_id)
        .map(|g| g.id.clone())
        .ok_or_else(|| format!("missing GraphSheet for primary source `{}`", source_id.as_str()))
}

fn graph_svg_rendered(doc: &ProjectDocument, graph_id: &SheetId) -> Result<String, String> {
    let gs = doc.graph_sheet(graph_id).ok_or_else(|| format!("missing graph `{}`", graph_id.as_str()))?;
    let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
    render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())
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

#[test]
fn wf51_human_nonlinear_rout_replicate_interp_compare_and_relative_ic50() -> TestRes {
    let mut run = WorkflowRun::start("WF-51");
    let fx = wf51_inhibitor_triplicate_xy();
    let sid = run.create_table_via_welcome(
        "welcome.arch.xy",
        "WF51 Inhibitor ROUT",
        &["welcome.subcol.raw", "welcome.stepper.cols.dec", "welcome.stepper.cols.dec"],
    )?;
    run.enter_column_title(0, fx.data_set_title)?;

    for (r, pt) in fx.known_rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{}", pt.log_dose))?;
        for (s, &v) in pt.replicates.iter().enumerate() {
            run.enter_grid_cell(r, 0, s, &format!("{v}"))?;
        }
    }
    for (u_idx, reps) in fx.unknown_rows.iter().enumerate() {
        let r = fx.known_rows.len() + u_idx;
        for (s, &v) in reps.iter().enumerate() {
            run.enter_grid_cell(r, 0, s, &format!("{v}"))?;
        }
    }

    run.select_sheet(&sid)?;
    let rs_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.replace_textbox("fit.eq.search", "inhibitor")?;
    run.click("fit.eq.item.log_inhibitor_vs_response_4pl")?;
    run.click("fit.tab.weight")?;
    run.click("fit.method.rout")?;
    run.click("fit.tab.compare")?;
    run.click("fit.cmp.extra_f")?;
    let cmp_pt = run.widget_center("fit.cmp.extra_f")?;
    run.wheel(cmp_pt, egui::vec2(0.0, -280.0))?;
    run.click("fit.cmp.alt.log_inhibitor_vs_response_3pl")?;
    run.click("fit.tab.output")?;
    run.click("fit.out.interp")?;
    run.replace_textbox("fit.diagnostics.r2.input", "0.999999")?;
    run.commit_modal_ok()?;

    let (rep_tab_idx, rep_num_col) = {
        let rs = run.document().result_sheet(&rs_id).ok_or("WF-51 missing result sheet")?;
        ensure(matches!(rs.status, ResultStatus::UpToDate), "WF-51 nonlinear result not UpToDate")?;
        ensure(!rs.eliminated_outliers.is_empty(), "WF-51 ROUT Q=1% must eliminate injected outlier")?;
        ensure(rs.tabs.iter().any(|t| t.id == "nonlinear_interpolated_mean_x"), "WF-51 missing Mean X interp tab")?;
        let rep_pos = rs.tabs.iter().position(|t| t.id == "nonlinear_interpolated_replicates").ok_or("WF-51 missing Replicates interp tab")?;
        let rep_col = rs.tabs[rep_pos]
            .rows
            .first()
            .and_then(|row| row.iter().position(|c| matches!(c, ResultCell::Number { value, .. } if value.is_finite())))
            .ok_or("WF-51 Replicates interp row 0 missing numeric interpolated value")?;
        ensure(rs.tabs.iter().any(|t| t.kind == ResultTabKind::ModelComparison || t.id.contains("cmp")), "WF-51 missing 3PL vs 4PL comparison tab")?;
        ensure(
            rs.checklist.iter().any(|c| c.assumption.contains("R squared below threshold")),
            format!("WF-51 missing Flag R^2 checklist note: {:?}", rs.checklist),
        )?;
        (rep_pos, rep_col)
    };

    // Attach overlay to graph and verify ROUT outlier Cross badge compiles onto GraphSheet.
    let gs_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&gs_id)?;
    run.click("toolbar.format_graph")?;
    run.click("format_graph.overlay.attach.0")?;
    run.commit_modal_ok()?;
    {
        let doc = run.document();
        let gs = doc.graph_sheet(&gs_id).ok_or("WF-51 missing graph after overlay")?;
        let ov = gs.spec.series[0].regression_overlay.as_ref().ok_or("WF-51 expected regression_overlay on series 0")?;
        ensure(ov.result_sheet_id == rs_id, "WF-51 overlay must point to ROUT ResultSheet")?;
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        ensure(scene.warnings.is_empty(), format!("WF-51 graph compile warnings: {:?}", scene.warnings))?;
        ensure(
            scene.plotted_points.iter().any(|p| p.value_summary.contains("ROUT outlier")),
            "WF-51 graph scene must emit ROUT outlier marker metadata",
        )?;
        ensure(!render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?.is_empty(), "WF-51 SVG must render non-empty")?;
    }

    // Chain Normalize -> Nonlinear 4PL relative IC50 (`analyze.entry.fit.nonlinear` on Normalize output).
    run.select_sheet(&sid)?;
    let norm_id = run.run_analyze_entry("Normalize", "analyze.entry.transform.normalize")?;
    ensure(
        run.document().result_sheet(&norm_id).is_some_and(|r| matches!(r.status, ResultStatus::UpToDate) && r.analyzable_data.is_some()),
        "WF-51 Normalize must produce UpToDate analyzable_data",
    )?;
    run.select_sheet(&norm_id)?;
    let rel_ic50_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&rel_ic50_id)?;
    run.open_cmd_t()?;
    run.replace_textbox("fit.eq.search", "inhibitor")?;
    run.click("fit.eq.item.log_inhibitor_vs_response_4pl")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().result_sheet(&rel_ic50_id).is_some_and(|r| matches!(r.status, ResultStatus::UpToDate)),
        "WF-51 chained relative IC50 fit must be UpToDate",
    )?;

    // Stage 4: Oracle verification for both ROUT 4PL + unknowns and chained Normalize relative IC50.
    let total_rows = fx.known_rows.len() + fx.unknown_rows.len();
    let mut ref_xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, total_rows).map_err(|e| e.to_string())?;
    ref_xy.data_sets[0].title = String::from(fx.data_set_title);
    for (r, pt) in fx.known_rows.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(pt.log_dose);
        for (s, &v) in pt.replicates.iter().enumerate() {
            ref_xy.data_sets[0].cells[r][s] = Cell::number(v);
        }
    }
    for (u_idx, reps) in fx.unknown_rows.iter().enumerate() {
        let r = fx.known_rows.len() + u_idx;
        for (s, &v) in reps.iter().enumerate() {
            ref_xy.data_sets[0].cells[r][s] = Cell::number(v);
        }
    }
    let ref_sheet = DataSheet::new("ref-xy-51", fx.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "sheet_id": "ref-nl-51",
                    "source_sheet_id": "ref-xy-51",
                    "kind": "nonlinear_regression",
                    "equation_id": "log_inhibitor_vs_response_4pl",
                    "method": "rout",
                    "q_percent": 1.0,
                    "interpolate_unknowns": true,
                    "flag_r_squared_below": 0.999999,
                    "comparison": {
                        "method": "extra_sum_of_squares_f",
                        "alternative_equation_id": "log_inhibitor_vs_response_3pl"
                    }
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet.clone())?
        .ref_result_id(SheetId::new("ref-nl-51"))
        .scalar("LogIC50", Some(0), 1e-4, 1e-4)
        .scalar("IC50", Some(0), 1e-4, 1e-4)
        .scalar("R2", None, 1e-5, 1e-5)
        .tab_cell(rep_tab_idx, 0, rep_num_col, 1e-4, 1e-4),
    )?;

    run.verify_reference(
        ReferenceSpec::for_result(rel_ic50_id, |sess| {
            sess.execute(
                "transform.run",
                json!({
                    "sheet_id": "ref-norm-51",
                    "source_sheet_id": "ref-xy-51",
                    "kind": "normalize",
                    "zero_anchor": "smallest",
                    "hundred_anchor": "largest"
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute(
                "analysis.run",
                json!({
                    "sheet_id": "ref-rel-ic50-51",
                    "source_sheet_id": "ref-norm-51",
                    "kind": "nonlinear_regression",
                    "equation_id": "log_inhibitor_vs_response_4pl"
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-rel-ic50-51"))
        .scalar("LogIC50", Some(0), 1e-4, 1e-4)
        .scalar("IC50", Some(0), 1e-4, 1e-4)
        .scalar("R2", None, 1e-5, 1e-5),
    )?;

    // Stage 5: Live unknown replicate edit recomputes Interpolated Replicates tab cleanly.
    let rs_for_chk = rs_id.clone();
    let before_rep_rows =
        run.document().result_sheet(&rs_id).and_then(|r| r.tabs.get(rep_tab_idx)).map(|t| t.rows.clone()).ok_or("WF-51 missing rep tab rows")?;
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid, fx.known_rows.len(), 0, 1, "64.0"),
        vec![LiveEditExpectation::custom(move |_, _, doc| {
            let rs_after = doc.result_sheet(&rs_for_chk).ok_or("WF-51 missing result after edit")?;
            let after_rows = &rs_after.tabs[rep_tab_idx].rows;
            ensure(after_rows != &before_rep_rows, "WF-51 unknown replicate edit must recompute Interpolated Replicates")
        })],
    )?;

    run.select_sheet(&gs_id)?;
    run.finish()
}

#[test]
fn wf52_human_4pl_identifiability_recovery_extrapolation_and_qq_drillback() -> TestRes {
    let mut run = WorkflowRun::start("WF-52");
    run.set_viewport(ViewportMetrics::new(1440.0, 900.0, 1.0));
    let fx = wf52_sparse_agonist_4pl_xy();
    let sid = run.create_table_via_welcome("welcome.arch.xy", "WF52 Ambiguous 4PL", &["welcome.stepper.cols.dec"])?;
    for (c, s) in fx.series.iter().enumerate() {
        run.enter_column_title(c, s.title)?;
    }

    for (r, &(x, y)) in fx.series[0].rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    for (r, &(_x, y)) in fx.series[1].rows.iter().enumerate() {
        run.enter_grid_cell(r, 1, 0, &format!("{y}"))?;
    }

    // 1) Initial sparse-plateau fit with Identify unstable parameters enabled -> Ambiguous / (Unstable) + Extrapolation.
    run.select_sheet(&sid)?;
    let rs_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.output")?;
    run.click("fit.out.unstable")?;
    run.commit_modal_ok()?;

    {
        let rs = run.document().result_sheet(&rs_id).ok_or("WF-52 missing sparse fit result")?;
        ensure(
            matches!(&rs.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.identify_unstable_parameters),
            "WF-52 identify_unstable_parameters must persist",
        )?;
        let has_ambig = rs.checklist.iter().any(|c| c.assumption.contains("Ambiguous"))
            || rs.tabs.iter().any(|t| t.row_labels.iter().any(|l| l.contains("(Unstable)")));
        ensure(has_ambig, "WF-52 sparse 4PL must flag Ambiguous / (Unstable) parameters")?;
        ensure(
            rs.checklist.iter().any(|c| c.assumption.contains("outside observed X range")),
            format!("WF-52 missing midpoint extrapolation checklist item: {:?}", rs.checklist),
        )?;
    }

    // 2) Recover identifiability via Bottom=Constant(0) + SharedGlobal HillSlope & Top (zero guarded clicks).
    run.open_cmd_t()?;
    run.click("fit.tab.constraints")?;
    run.click("fit.con.rule.Bottom.fixed")?;
    run.replace_textbox("fit.con.const.Bottom", "0")?;
    let con_pt = run.widget_center("fit.con.rule.Bottom.fixed")?;
    run.wheel(con_pt, egui::vec2(0.0, -180.0))?;
    run.click("fit.con.rule.Top.shared")?;
    let top_pt = run.widget_center("fit.con.rule.Top.shared")?;
    run.wheel(top_pt, egui::vec2(0.0, -280.0))?;
    run.click("fit.con.rule.HillSlope.shared")?;
    run.commit_modal_ok()?;

    {
        let rs = run.document().result_sheet(&rs_id).ok_or("WF-52 missing recovered fit")?;
        ensure(matches!(rs.status, ResultStatus::UpToDate), "WF-52 recovered fit must be UpToDate")?;
        let AnalysisKind::NonlinearRegression(nl) = &rs.analysis.kind else {
            return Err(String::from("WF-52 expected NonlinearRegression"));
        };
        let b_rule = nl.parameter_rules.iter().find(|r| r.name == "Bottom").ok_or("WF-52 missing Bottom rule")?;
        let h_rule = nl.parameter_rules.iter().find(|r| r.name == "HillSlope").ok_or("WF-52 missing HillSlope rule")?;
        let t_rule = nl.parameter_rules.iter().find(|r| r.name == "Top").ok_or("WF-52 missing Top rule")?;
        ensure(b_rule.scope == ParameterScope::Constant(0.0), format!("WF-52 expected Bottom Constant(0.0), got {:?}", b_rule.scope))?;
        ensure(h_rule.scope == ParameterScope::SharedGlobal, format!("WF-52 expected HillSlope SharedGlobal, got {:?}", h_rule.scope))?;
        ensure(t_rule.scope == ParameterScope::SharedGlobal, format!("WF-52 expected Top SharedGlobal, got {:?}", t_rule.scope))?;
    }

    // 3) Create Normal Q-Q diagnostic graph from data via Insert -> Create Graph, click plotted point, and drill back to source cell.
    let before_graphs = run.document().graph_sheets.len();
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    run.click("create_graph.source.0")?;
    run.commit_modal_ok()?;
    ensure(run.document().graph_sheets.len() == before_graphs + 1, "WF-52 CreateGraph must add Normal Q-Q target graph")?;
    let g_qq_id = run.document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-52 missing g_qq")?;

    run.select_sheet(&g_qq_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.card.diag.normal_qq")?;
    run.commit_modal_ok()?;

    let qq_scene = {
        let gs = run.document().graph_sheet(&g_qq_id).ok_or("WF-52 missing g_qq")?;
        compile_graph(gs, run.document()).map_err(|e| e.to_string())?
    };
    let first_pt = qq_scene
        .plotted_points
        .iter()
        .find(|p| p.source_cell.sheet_id == sid)
        .ok_or("WF-52 Normal Q-Q plotted points must carry source_cell back to XY sheet")?
        .clone();
    let canvas_rect = run.frame_meta().find_by_widget_id("canvas.viewport").ok_or("WF-52 missing canvas.viewport")?.clipped_rect;
    let margin = 16.0_f32;
    let avail_w = (canvas_rect.width() - margin * 2.0).max(80.0);
    let avail_h = (canvas_rect.height() - margin * 2.0).max(80.0);
    let sx = (avail_w / (qq_scene.width_pt.max(1.0) as f32)).min(avail_h / (qq_scene.height_pt.max(1.0) as f32));
    let paper_min =
        egui::pos2(canvas_rect.center().x - (qq_scene.width_pt as f32) * sx * 0.5, canvas_rect.center().y - (qq_scene.height_pt as f32) * sx * 0.5);
    let hit_screen = egui::pos2(paper_min.x + (first_pt.center_pt.x as f32) * sx, paper_min.y + (first_pt.center_pt.y as f32) * sx);
    run.click_at(hit_screen)?;
    ensure(
        run.frame_meta().find_by_widget_id("inspector.drill_back.jump").is_some(),
        "WF-52 clicking Q-Q point must surface drill-back jump button",
    )?;
    run.click("inspector.drill_back.jump")?;
    ensure(run.document().active_sheet_id.as_ref() == Some(&sid), "WF-52 drill-back jump must activate source XY sheet")?;

    // Stage 4: Oracle verification of recovered 4PL constrained fit.
    let mut ref_xy = XyTable::new(SubcolumnMode::Single, 2, fx.series[0].rows.len()).map_err(|e| e.to_string())?;
    for (c, s) in fx.series.iter().enumerate() {
        ref_xy.data_sets[c].title = String::from(s.title);
    }
    for (r, &(x, y0)) in fx.series[0].rows.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(y0);
        ref_xy.data_sets[1].cells[r][0] = Cell::number(fx.series[1].rows[r].1);
    }
    let ref_sheet = DataSheet::new("ref-xy-52", fx.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "sheet_id": "ref-nl-52",
                    "source_sheet_id": "ref-xy-52",
                    "kind": "nonlinear_regression",
                    "equation_id": "log_agonist_vs_response_4pl",
                    "identify_unstable_parameters": true,
                    "parameter_rules": [
                        { "name": "Bottom", "scope": "constant", "value": 0.0 },
                        { "name": "HillSlope", "scope": "shared_global" },
                        { "name": "Top", "scope": "shared_global" }
                    ]
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-nl-52"))
        .scalar("LogEC50", Some(0), 1e-4, 1e-4)
        .scalar("LogEC50", Some(1), 1e-4, 1e-4)
        .scalar("Top", None, 1e-3, 1e-3)
        .scalar("HillSlope", None, 1e-3, 1e-3),
    )?;

    // Stage 5: Live cell correction updates LogEC50 and Q-Q graph SVG cleanly.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid, 4, 1, 0, "58.0"),
        vec![LiveEditExpectation::scalar_changed(rs_id, "LogEC50", Some(1), 1e-4), LiveEditExpectation::graph_svg_changed(g_qq_id)],
    )?;

    run.finish()
}

#[test]
fn wf53_human_kaplan_meier_display_modes_and_column_exclude_hr() -> TestRes {
    let mut run = WorkflowRun::start("WF-53");
    let fx = wf53_three_arm_survival();
    let sid = run.create_table_via_welcome("welcome.arch.survival", "WF53 3-Arm KM", &[])?;

    for (c, &cohort) in fx.cohorts.iter().enumerate() {
        run.enter_column_title(c, cohort)?;
    }
    for (r, row) in fx.rows.iter().enumerate() {
        if let Some(t) = row.elapsed_days {
            run.enter_x_cell(r, 0, &format!("{t}"))?;
        }
        for (c, status) in row.status.iter().enumerate() {
            if let Some(v) = status {
                run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
            }
        }
    }

    run.select_sheet(&sid)?;
    let km_id = run.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    {
        let rs = run.document().result_sheet(&km_id).ok_or("WF-53 missing KM result")?;
        ensure(matches!(rs.status, ResultStatus::UpToDate), "WF-53 KM result must be UpToDate")?;
        let nar = rs.tabs.iter().find(|t| t.kind == ResultTabKind::NumberAtRisk).ok_or("WF-53 missing Number at risk tab")?;
        ensure(nar.column_headers.iter().any(|h| h.contains("CI")) && !nar.rows.is_empty(), "WF-53 Number at risk must report rows & CI")?;
    }

    let gs_id = graph_for_source(run.document(), &sid)?;
    let svg_default = graph_svg_rendered(run.document(), &gs_id)?;

    // Switch Survival -> % Cumulative Event + PointToPoint + SEM error bars.
    run.select_sheet(&gs_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.survival.metric.pct_event")?;
    run.click("graph_type.survival.point_to_point")?;
    run.click("graph_type.survival.err.sem")?;
    run.commit_modal_ok()?;
    let svg_event_ptp = {
        let gs = run.document().graph_sheet(&gs_id).ok_or("WF-53 missing g-surv")?;
        ensure(
            matches!(
                gs.spec.series[0].gdo,
                GraphicalDataObject::KaplanMeierStep {
                    display_metric: SurvivalDisplayMetric::PercentCumulativeEvent,
                    staircase: false,
                    error_display: Some(ErrorBarMetric::Sem),
                    show_greenwood_ci: true,
                    ..
                }
            ),
            "WF-53 survival display options failed to persist",
        )?;
        graph_svg_rendered(run.document(), &gs_id)?
    };
    ensure(svg_event_ptp != svg_default, "WF-53 switching to Cumulative Event + Point-to-Point + SEM must update SVG")?;

    // Switch back -> % Survival + Staircase + Greenwood 95% CI and confirm bidirectional toggle in GraphSpec & SVG.
    run.click("toolbar.graph_type")?;
    run.click("graph_type.survival.metric.pct_survival")?;
    run.click("graph_type.survival.staircase")?;
    run.click("graph_type.survival.err.ci95")?;
    run.commit_modal_ok()?;
    let svg_surv_stair = {
        let gs = run.document().graph_sheet(&gs_id).ok_or("WF-53 missing g-surv roundtrip")?;
        ensure(
            matches!(
                gs.spec.series[0].gdo,
                GraphicalDataObject::KaplanMeierStep {
                    display_metric: SurvivalDisplayMetric::PercentSurvival,
                    staircase: true,
                    error_display: Some(ErrorBarMetric::ConfidenceInterval95),
                    show_greenwood_ci: true,
                    ..
                }
            ),
            "WF-53 reverse toggle back to PercentSurvival + Staircase failed",
        )?;
        graph_svg_rendered(run.document(), &gs_id)?
    };
    ensure(svg_surv_stair != svg_event_ptp, "WF-53 reverse toggle must update survival SVG")?;

    // Stage 4: verify 3-arm KM numerical parity & graph parity against headless Session oracle.
    let mut ref_surv = SurvivalTable::new(fx.cohorts.len(), fx.rows.len()).map_err(|e| e.to_string())?;
    for (c, &cohort) in fx.cohorts.iter().enumerate() {
        ref_surv.cohorts[c].title = cohort.to_owned();
    }
    for (r, row) in fx.rows.iter().enumerate() {
        if let Some(t) = row.elapsed_days {
            ref_surv.times[r] = Cell::number(t);
        }
        for (c, status) in row.status.iter().enumerate() {
            if let Some(v) = status {
                ref_surv.cohorts[c].cells[r][0] = Cell::number(f64::from(*v));
            }
        }
    }
    let ref_sheet = DataSheet::new("ref-surv-53", fx.title, TableArchetype::Survival(ref_surv)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(km_id.clone(), |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "sheet_id": "ref-km-53",
                    "source_sheet_id": "ref-surv-53",
                    "kind": "survival"
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet.clone())?
        .ref_result_id(SheetId::new("ref-km-53"))
        .scalar("Log-rank chi-square", None, 1e-6, 1e-6)
        .scalar("Log-rank P value", None, 1e-6, 1e-6)
        .tab_cell(1, 0, 2, 1e-6, 1e-6),
    )?;

    // Stage 5: Exclude entire 100%-censored Vehicle arm (col 0 rows 0..=8) via Shift+Click + Cmd+E -> 2-arm Hazard Ratio + 95% CI.
    let sid_for_edit = sid.clone();
    let km_for_check = km_id.clone();
    let last_r = fx.rows.len() - 1;
    run.live_edit(
        LiveEditTarget::gui(move |r| {
            r.select_sheet(&sid_for_edit)?;
            r.click("grid.cell.0.0.0")?;
            let p_end = r.widget_center(&format!("grid.cell.{last_r}.0.0"))?;
            r.click_at_button(p_end, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
            r.press_key("E", KeyModifiers { meta: true, ..Default::default() })
        }),
        LiveEditExpectation::custom(move |before, after, doc| {
            let b_chi = before.scalar(&km_for_check, "Log-rank chi-square", None).ok_or("WF-53 before missing chi-square")?;
            let a_chi = after.scalar(&km_for_check, "Log-rank chi-square", None).ok_or("WF-53 after missing chi-square")?;
            ensure((b_chi - a_chi).abs() > 1e-3, format!("WF-53 3-arm vs 2-arm log-rank chi-square must change ({b_chi} -> {a_chi})"))?;
            let hr = after.scalar(&km_for_check, "Hazard ratio (log-rank)", None).ok_or("WF-53 missing Hazard ratio (log-rank) scalar")?;
            ensure(hr.is_finite() && hr > 0.0, format!("WF-53 expected finite positive HR, got {hr}"))?;
            let rs = doc.result_sheet(&km_for_check).ok_or("WF-53 missing ResultSheet after exclusion")?;
            let hr_tab = rs.tabs.iter().find(|t| t.id == "hazard-ratio").ok_or("WF-53 missing hazard-ratio tab")?;
            let has_ci = hr_tab.rows.iter().any(|row| row.iter().any(|c| matches!(c, curvebook_doc::ResultCell::ConfidenceInterval { .. })));
            ensure(has_ci, "WF-53 hazard-ratio tab must include finite 95% CI")
        }),
    )?;

    // Also verify post-exclusion 2-arm Hazard Ratio & 95% CI numerically against headless Session oracle.
    let exclude_cells: Vec<_> = (0..fx.rows.len()).map(|row| json!({ "row": row, "col": 0, "subcol": 0 })).collect();
    run.verify_reference(
        ReferenceSpec::for_result(km_id, move |sess| {
            sess.execute("table.excludeCells", json!({ "sheet_id": "ref-surv-53", "cells": exclude_cells, "excluded": true }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "sheet_id": "ref-km-2arm", "source_sheet_id": "ref-surv-53", "kind": "survival" }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-km-2arm"))
        .scalar("Log-rank chi-square", None, 1e-6, 1e-6)
        .scalar("Hazard ratio (log-rank)", None, 1e-6, 1e-6)
        .scalar("Hazard ratio (Mantel-Haenszel)", None, 1e-6, 1e-6),
    )?;

    run.select_sheet(&gs_id)?;
    run.finish()
}

#[test]
fn wf54_human_dual_axis_nudged_bars_alpha_lines_and_selective_decay_fit() -> TestRes {
    let mut run = WorkflowRun::start("WF-54");
    let fx = wf54_dual_axis_mean_sd_n_xy();
    let sid = run.create_table_via_welcome("welcome.arch.xy", fx.title, &["welcome.subcol.mean_sd_n", "welcome.stepper.cols.dec"])?;
    for (c, &title) in fx.series_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }

    for (r, &x) in fx.x_hours.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        let m0 = &fx.fluorescence[r];
        let m1 = &fx.metabolite[r];
        for (c, m) in [(0, m0), (1, m1)] {
            run.enter_grid_cell(r, c, 0, &format!("{}", m.mean))?;
            run.enter_grid_cell(r, c, 1, &format!("{}", m.sd))?;
            run.enter_grid_cell(r, c, 2, &format!("{}", m.n))?;
        }
    }

    // Selective single-series fit (Series 0 only) via Analyze Data checkbox + One-Phase Exponential Decay.
    run.select_sheet(&sid)?;
    let before_rs = run.document().result_sheets.len();
    run.click("toolbar.analyze")?;
    run.replace_textbox("analyze.search", "Nonlinear")?;
    run.click("analyze.entry.fit.nonlinear")?;
    run.click("analyze.mv.var.1")?;
    run.commit_modal_ok()?;
    ensure(run.document().result_sheets.len() == before_rs + 1, "WF-54 selective Nonlinear must create ResultSheet")?;
    let fit_id = run.document().result_sheets.last().map(|r| r.id.clone()).ok_or("WF-54 missing fit ResultSheet")?;

    run.select_sheet(&fit_id)?;
    run.open_cmd_t()?;
    run.replace_textbox("fit.eq.search", "one_phase_decay")?;
    run.click("fit.eq.item.one_phase_decay")?;
    run.commit_modal_ok()?;
    {
        let rs = run.document().result_sheet(&fit_id).ok_or("WF-54 missing decay fit")?;
        ensure(matches!(rs.status, ResultStatus::UpToDate), "WF-54 selective One-Phase Decay fit must be UpToDate")?;
        ensure(
            rs.analysis.selected_data_sets == vec![0],
            format!("WF-54 expected fit selected_data_sets=[0], got {:?}", rs.analysis.selected_data_sets),
        )?;
        ensure(
            matches!(&rs.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "one_phase_decay"),
            "WF-54 fit must persist one_phase_decay equation",
        )?;
    }

    // Configure Dual-Y2 axis + 50% alpha overlay on Series 0 + Right-Y2 nudged BarWithReplicates on Series 1.
    let gs_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&gs_id)?;
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.y2")?;
    run.click("axes.enable_y2")?;
    run.click("edit.tab.axes.titles")?;
    run.replace_textbox("axes.title", "Metabolite Pool (uM)")?;
    run.commit_modal_ok()?;

    run.click("toolbar.format_graph")?;
    run.click("format_graph.alpha.50")?;
    run.click("format_graph.overlay.attach.0")?;
    run.click("modal.apply")?;
    run.click("edit.tab.format_graph.series")?;
    run.click("format_graph.series_select.1")?;
    run.click("format_graph.series_axis.right_y2")?;
    set_scrub_field_wf(&mut run, "format_graph.series_offset_x", 24.0, "0.5")?;
    run.click("edit.tab.format_graph.appearance")?;
    run.click("format_graph.series_gdo.bar_replicates")?;
    run.commit_modal_ok()?;

    {
        let doc = run.document();
        let gs = doc.graph_sheet(&gs_id).ok_or("WF-54 missing dual-axis graph")?;
        let a0 = match gs.spec.series[0].gdo {
            GraphicalDataObject::ScatterAndLine { marker: Some(ref m), .. } => m.fill_color.a,
            _ => 255,
        };
        ensure((120..=135).contains(&a0), format!("WF-54 expected ~50% opacity on Series 0, got {a0}"))?;
        ensure(
            gs.spec.series[0].regression_overlay.as_ref().is_some_and(|ov| ov.result_sheet_id == fit_id),
            "WF-54 Series 0 must carry One-Phase Decay regression overlay",
        )?;
        ensure(!gs.spec.series[0].bind_to_right_y2 && gs.spec.series[1].bind_to_right_y2, "WF-54 Series 1 must bind to Right Y2")?;
        ensure(
            matches!(gs.spec.series[1].gdo, GraphicalDataObject::BarWithReplicates { .. }),
            format!("WF-54 Series 1 expected BarWithReplicates, got {:?}", gs.spec.series[1].gdo),
        )?;
        ensure((gs.spec.series[1].offset.x_data - 0.5).abs() < 1e-6, "WF-54 Series 1 offset_x must be +0.5")?;
        let _svg = graph_svg_rendered(doc, &gs_id)?;
    }

    // Stage 4: Oracle verification for selective One-Phase Decay fit + dual-axis GraphSpec.
    let mut ref_xy = XyTable::new(SubcolumnMode::MeanSdN, 2, fx.x_hours.len()).map_err(|e| e.to_string())?;
    for (c, &title) in fx.series_titles.iter().enumerate() {
        ref_xy.data_sets[c].title = String::from(title);
    }
    for (r, &x) in fx.x_hours.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(x);
        for (c, m) in [(0, &fx.fluorescence[r]), (1, &fx.metabolite[r])] {
            ref_xy.data_sets[c].cells[r][0] = Cell::number(m.mean);
            ref_xy.data_sets[c].cells[r][1] = Cell::number(m.sd);
            ref_xy.data_sets[c].cells[r][2] = Cell::number(m.n);
        }
    }
    let ref_sheet = DataSheet::new("ref-xy-54", fx.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "sheet_id": "ref-decay-54",
                    "source_sheet_id": "ref-xy-54",
                    "kind": "nonlinear_regression",
                    "equation_id": "one_phase_decay",
                    "selected_data_sets": [0]
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-decay-54"))
        .scalar("Y0", Some(0), 1e-4, 1e-4)
        .scalar("Plateau", Some(0), 1e-4, 1e-4)
        .scalar("K", Some(0), 1e-5, 1e-5)
        .scalar("R2", None, 1e-5, 1e-5),
    )?;

    // Stage 5: Live mean cell edit updates One-Phase Decay K and dual-axis SVG.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid, 1, 0, 0, "84.5"),
        vec![LiveEditExpectation::scalar_changed(fit_id, "K", Some(0), 1e-4), LiveEditExpectation::graph_svg_changed(gs_id)],
    )?;

    run.finish()
}

#[test]
fn wf55_human_grouped_transpose_batch_graphs_and_gallery_apply_style() -> TestRes {
    let mut run = WorkflowRun::start("WF-55");
    run.set_viewport(ViewportMetrics::new(1600.0, 900.0, 1.0));
    let fx = wf55_multisite_grouped_cohort();
    let sid = run.create_table_via_welcome("welcome.arch.grouped", fx.title, &["welcome.subcol.single", "welcome.stepper.cols.inc"])?;

    for (r, &rt) in fx.row_titles.iter().enumerate() {
        run.enter_row_title(r, rt)?;
    }
    for (c, &ct) in fx.col_titles.iter().enumerate() {
        run.enter_column_title(c, ct)?;
    }
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{val}"))?;
        }
    }

    run.select_sheet(&sid)?;
    let tr_id = run.run_analyze_entry("Transpose", "analyze.entry.transform.transpose")?;
    run.select_sheet(&tr_id)?;
    run.open_cmd_t()?;
    run.click("edit.transform.transpose.swap_titles")?;
    run.click("edit.transform.transpose.swap_titles")?;
    run.commit_modal_ok()?;

    let before_graphs = run.document().graph_sheets.len();
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::CreateGraph { .. }), "WF-55 CreateGraph modal must open")?;
    let src_wid = run
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("create_graph.source.") && e.label.contains("Transpose"))
        .map(|e| e.widget_id.clone())
        .ok_or("WF-55 missing Transpose source option in CreateGraph")?;
    run.click(&src_wid)?;
    run.click("create_graph.one_per_data_set")?;
    run.replace_textbox("create_graph.y_axis_title", "Quarter Index")?;
    run.commit_modal_ok()?;

    let batch_graphs: Vec<SheetId> =
        run.document().graph_sheets.iter().filter(|g| g.spec.primary_source_sheet_id == tr_id).map(|g| g.id.clone()).collect();
    ensure(
        batch_graphs.len() == fx.row_titles.len(),
        format!("WF-55 expected {} batch graphs for transposed sites, got {}", fx.row_titles.len(), batch_graphs.len()),
    )?;
    ensure(run.document().graph_sheets.len() >= before_graphs + 12, "WF-55 batch graph count mismatch")?;
    let donor_id = batch_graphs[0].clone();
    let last_id = batch_graphs[batch_graphs.len() - 1].clone();

    // Format donor frame preset via inspector then batch ApplyStyleFrom through Portfolio Gallery real Shift+Click range (no select_all).
    run.select_sheet(&donor_id)?;
    run.click("inspector.graph.tab.axes")?;
    run.click("inspector.graph.frame.PlainBoxed")?;

    let donor_title = run.document().graph_sheet(&donor_id).ok_or("WF-55 missing donor graph")?.title.clone();
    run.click("status.section.gallery")?;
    let p_first = run.widget_center(&format!("gallery.card.{}", donor_id.as_str()))?;
    run.click_at_button(p_first, PointerButton::Primary, KeyModifiers { ctrl: true, ..Default::default() })?;
    let p_last = run.widget_center(&format!("gallery.card.{}", last_id.as_str()))?;
    run.click_at_button(p_last, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    run.click("gallery.apply_style_from")?;
    let donor_wid = run
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("apply_style.donor.") && e.label == donor_title)
        .map(|e| e.widget_id.clone())
        .ok_or("WF-55 missing donor graph in ApplyStyleFrom")?;
    run.click(&donor_wid)?;
    run.commit_modal_ok()?;

    for gid in &batch_graphs {
        let gs = run.document().graph_sheet(gid).ok_or_else(|| format!("WF-55 missing batch graph `{}`", gid.as_str()))?;
        ensure(gs.spec.frame_preset == FramePreset::PlainBoxed, "WF-55 batch ApplyStyleFrom must propagate FramePreset::PlainBoxed")?;
        ensure(gs.spec.y_axis.title == "Quarter Index", "WF-55 batch graphs must keep shared Y title")?;
    }
    ensure(!run.app().nav.gallery.open, "WF-55 Apply Style From must close Portfolio Gallery cleanly")?;

    // Stage 4: verify transposed result cells and batch graph style against headless Session oracle.
    let mut ref_grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), fx.col_titles.len(), fx.row_titles.len()).map_err(|e| e.to_string())?;
    for (r, &rt) in fx.row_titles.iter().enumerate() {
        ref_grp.row_titles[r] = rt.to_owned();
    }
    for (c, &ct) in fx.col_titles.iter().enumerate() {
        ref_grp.data_sets[c].title = ct.to_owned();
    }
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            ref_grp.data_sets[c].cells[r][0] = Cell::number(val);
        }
    }
    let ref_sheet = DataSheet::new("ref-grp-55", fx.title, TableArchetype::Grouped(ref_grp)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(tr_id.clone(), |sess| {
            sess.execute(
                "transform.run",
                json!({
                    "sheet_id": "ref-tr-55",
                    "source_sheet_id": "ref-grp-55",
                    "kind": "transpose",
                    "swap_row_and_column_titles": true
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute(
                "graph.create",
                json!({
                    "sheet_id": "ref-g-55",
                    "data_sheet_id": "ref-tr-55",
                    "kind": "grouped",
                    "one_per_data_set": true,
                    "y_axis_title": "Quarter Index"
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute("graph.setSpec", json!({ "sheet_id": "ref-g-55_12", "frame_preset": "PlainBoxed" })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-tr-55"))
        .tab_cell(0, 0, 0, 1e-9, 1e-9)
        .tab_cell(0, 1, 0, 1e-9, 1e-9)
        .with_graph(last_id.clone())
        .ref_graph_id(SheetId::new("ref-g-55_12"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.frame_preset == ref_gs.spec.frame_preset, "WF-55 FramePreset mismatch vs ref")?;
            ensure(gui_gs.spec.y_axis.title == ref_gs.spec.y_axis.title, "WF-55 Y-axis title mismatch vs ref")
        }),
    )?;

    // Stage 5: two-hop reactive live_edit from original grouped cell -> Transpose analyzable_data -> donor graph SVG.
    let edited_val = fx.rows[0][0] + 150.0;
    let tr_for_chk = tr_id.clone();
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid, 0, 0, 0, format!("{edited_val}")),
        vec![
            LiveEditExpectation::custom(move |_, _, doc| {
                let rs = doc.result_sheet(&tr_for_chk).ok_or("WF-55 missing Transpose ResultSheet")?;
                let ds = rs.analyzable_data.as_ref().ok_or("WF-55 missing Transpose analyzable_data")?;
                let v = match &ds.archetype {
                    TableArchetype::Grouped(g) => g.data_sets[0].cells[0][0].active_number(),
                    _ => None,
                }
                .ok_or("WF-55 transposed cell (0,0) not numeric")?;
                ensure((v - edited_val).abs() < 1e-6, format!("WF-55 expected transposed (0,0)={edited_val}, got {v}"))
            }),
            LiveEditExpectation::graph_svg_changed(donor_id.clone()),
        ],
    )?;

    run.select_sheet(&donor_id)?;
    run.finish()
}

#[test]
fn wf56_human_log_transform_antilog_axis_and_restricted_x_range_fit() -> TestRes {
    let mut run = WorkflowRun::start("WF-56");
    let fx = wf56_exponential_growth_xy();
    let sid = run.create_table_via_welcome("welcome.arch.xy", "WF56 Growth Log", &["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"])?;

    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }

    run.select_sheet(&sid)?;
    let log_id = run.run_analyze_entry("Transform", "analyze.entry.transform.math")?;
    run.select_sheet(&log_id)?;
    run.open_cmd_t()?;
    run.click("edit.transform.y_transform.log10")?;
    run.commit_modal_ok()?;

    // Create graph of the transformed result, switch Y ticks to Antilog + 9 logarithmic minor ticks (GWF-18).
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    let log_src_wid = run
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("create_graph.source.") && e.label.contains("Transform"))
        .map(|e| e.widget_id.clone())
        .ok_or("WF-56 missing Transform source in CreateGraph")?;
    run.click(&log_src_wid)?;
    run.commit_modal_ok()?;
    let log_graph_id = graph_for_source(run.document(), &log_id)?;
    run.select_sheet(&log_graph_id)?;
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.y")?;
    run.click("axes.tick_format.antilog")?;
    run.replace_textbox("axes.seg.0.minor_ticks_per_interval", "9")?;
    run.commit_modal_ok()?;
    {
        let gs = run.document().graph_sheet(&log_graph_id).ok_or("WF-56 missing log graph")?;
        ensure(gs.spec.y_axis.tick_format == TickLabelFormat::Antilog, "WF-56 expected Antilog Y tick format")?;
        ensure(gs.spec.y_axis.segments[0].minor_ticks_per_interval == 9, "WF-56 expected 9 Y minor ticks per interval")?;
    }

    run.select_sheet(&log_id)?;
    let lin_id = run.run_analyze_entry("Linear", "analyze.entry.fit.linear")?;
    run.select_sheet(&lin_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.range")?;
    run.replace_textbox("fit.lr.range.input.x_min", "10")?;
    run.replace_textbox("fit.lr.range.input.x_max", "20")?;
    run.commit_modal_ok()?;

    {
        let rs = run.document().result_sheet(&lin_id).ok_or("WF-56 missing restricted fit")?;
        ensure(
            matches!(&rs.analysis.kind, AnalysisKind::LinearRegression(spec) if spec.x_range_filter == Some((Some(10.0), Some(20.0)))),
            "WF-56 expected restricted [10, 20] linear range",
        )?;
        let curve = rs.fitted_curves.first().ok_or("WF-56 missing fitted polyline")?;
        let (xmin, xmax) = curve.points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), pt| (a.min(pt.x), b.max(pt.x)));
        ensure(xmin >= 10.0 - 1e-9 && xmax <= 20.0 + 1e-9, format!("WF-56 fitted polyline escaped [10, 20]: ({xmin}, {xmax})"))?;
    }

    // Stage 4: verify restricted [10, 20] slope/intercept/R2 + graph Y Antilog ticks vs headless Session oracle.
    let mut ref_xy = XyTable::new(SubcolumnMode::Single, 1, fx.rows.len()).map_err(|e| e.to_string())?;
    for (r, &(x, y)) in fx.rows.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ref_sheet = DataSheet::new("ref-xy-56", fx.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(lin_id.clone(), |sess| {
            sess.execute(
                "transform.run",
                json!({ "sheet_id": "ref-log-56", "source_sheet_id": "ref-xy-56", "kind": "math", "y_transform": "log10" }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute(
                "analysis.run",
                json!({ "sheet_id": "ref-lin-56", "source_sheet_id": "ref-log-56", "kind": "linear_regression", "x_range_filter": [10.0, 20.0] }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-56", "data_sheet_id": "ref-log-56", "kind": "xy" }))
                .map_err(|e| e.to_string())?;
            sess.execute(
                "graph.setAxes",
                json!({ "sheet_id": "ref-g-56", "y_axis": { "tick_format": "antilog", "segments": [{ "minor_ticks_per_interval": 9 }] } }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-lin-56"))
        .scalar("Slope", Some(0), 1e-6, 1e-6)
        .scalar("YIntercept", Some(0), 1e-6, 1e-6)
        .scalar("R2", Some(0), 1e-6, 1e-6)
        .with_graph(log_graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-56"))
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.y_axis.tick_format == ref_gs.spec.y_axis.tick_format, "WF-56 Y tick_format mismatch")?;
            ensure(
                gui_gs.spec.y_axis.segments[0].minor_ticks_per_interval == ref_gs.spec.y_axis.segments[0].minor_ticks_per_interval,
                "WF-56 minor_ticks mismatch",
            )
        }),
    )?;

    // Stage 5a: edit lag-phase X < 10 cell (row 1: X=4.0) -> transform analyzable_data updates, restricted linear slope remains INVARIANT.
    let log_for_chk = log_id.clone();
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid.clone(), 1, 0, 0, "45.0"),
        vec![
            LiveEditExpectation::custom(move |_, _, doc| {
                let rs = doc.result_sheet(&log_for_chk).ok_or("WF-56 missing Math ResultSheet")?;
                let ds = rs.analyzable_data.as_ref().ok_or("WF-56 missing Math analyzable_data")?;
                let v = match &ds.archetype {
                    TableArchetype::Xy(t) => t.data_sets[0].cells[1][0].active_number(),
                    _ => None,
                }
                .ok_or("WF-56 transformed cell (1,0) not numeric")?;
                let expected = 45.0_f64.log10();
                ensure((v - expected).abs() < 1e-6, format!("WF-56 expected log10(45.0)={expected}, got {v}"))
            }),
            LiveEditExpectation::scalar_invariant(lin_id.clone(), "Slope", Some(0), 1e-12, 1e-12),
        ],
    )?;

    // Stage 5b: edit log-linear X >= 10 cell (row 6: X=14.0) -> restricted linear slope CHANGES.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid, 6, 0, 0, "250.0"),
        vec![LiveEditExpectation::scalar_changed(lin_id, "Slope", Some(0), 1e-3), LiveEditExpectation::graph_svg_changed(log_graph_id.clone())],
    )?;

    run.select_sheet(&log_graph_id)?;
    run.finish()
}

#[test]
fn wf57_human_layout_embedded_image_crop_rotate_and_non_exported_grid() -> TestRes {
    let fx = wf57_schematic_rgba();
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    let png = encode_png(&RasterImage { width_px: fx.width_px, height_px: fx.height_px, dpi: 96, rgba: fx.rgba }).map_err(|e| e.to_string())?;
    clip.seed_image_bytes(png);

    let mut run = WorkflowRun::start_with_clipboard("WF-57", clip);
    let sid = run.create_table_via_welcome("welcome.arch.xy", "Assay WF57", &["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"])?;
    for (r, &(x, y)) in fx.assay_xy.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let g_id = graph_for_source(run.document(), &sid)?;

    // Create LayoutSheet via GUI (Insert > New Layout…) and add the graph panel via Trailing Studio.
    run.open_menu_and_click("menu.insert", &[], "menu.layout.create")?;
    let lay_id = run.document().layout_sheets.last().ok_or("WF-57 missing created LayoutSheet")?.id.clone();
    run.select_sheet(&lay_id)?;
    run.click("studio.layout.add_panel.0")?;

    // Paste schematic EmbeddedImage from clipboard via Cmd+V.
    run.press_key("V", KeyModifiers { meta: true, ..Default::default() })?;
    let img_idx = run
        .document()
        .layout_sheet(&lay_id)
        .and_then(|ls| ls.spec.items.iter().position(|it| matches!(it, LayoutItem::EmbeddedImage { .. })))
        .ok_or("WF-57 expected Cmd+V EmbeddedImage on LayoutSheet")?;
    let panel_wid = format!("canvas.layout_panel.{img_idx}");
    run.click(&panel_wid)?;

    // Corner-resize with Shift (free aspect) + drag rotation handle.
    let (w_before, h_before, rot_before) = match &run.document().layout_sheet(&lay_id).ok_or("WF-57 layout")?.spec.items[img_idx] {
        &LayoutItem::EmbeddedImage { bounds_pt, rotation_deg, .. } => (bounds_pt.width, bounds_pt.height, rotation_deg),
        _ => return Err(String::from("WF-57 expected EmbeddedImage item")),
    };
    let se_center = run.widget_center("canvas.image.handle.se")?;
    run.drag_with_modifiers(se_center, se_center + egui::vec2(54.0, -16.0), KeyModifiers { shift: true, ..Default::default() })?;
    let rot_center = run.widget_center("canvas.image.handle.rotate")?;
    run.drag(rot_center, rot_center + egui::vec2(30.0, 15.0))?;
    let (w_after, h_after, rot_after) = match &run.document().layout_sheet(&lay_id).ok_or("WF-57 layout")?.spec.items[img_idx] {
        &LayoutItem::EmbeddedImage { bounds_pt, rotation_deg, .. } => (bounds_pt.width, bounds_pt.height, rotation_deg),
        _ => return Err(String::from("WF-57 expected EmbeddedImage item post handle drag")),
    };
    ensure((w_after - w_before).abs() > 1.0 && (h_after - h_before).abs() > 1.0, "WF-57 Shift+corner resize must change image bounds")?;
    ensure((rot_after - rot_before).abs() > 0.5, "WF-57 rotate handle drag must change rotation")?;

    // Open Format Image via context menu, apply 4-edge crop + border + 90 deg rotation.
    let img_pos = run.widget_center(&panel_wid)?;
    run.click_at_button(img_pos, PointerButton::Secondary, KeyModifiers::default())?;
    run.click("canvas.layout_ctx.format_image")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::FormatImage { .. }), "WF-57 FormatImage dialog must open")?;
    run.click("format_image.crop.left_plus4")?;
    run.click("format_image.crop.right_plus4")?;
    run.click("format_image.crop.top_plus4")?;
    run.click("format_image.crop.bottom_plus4")?;
    run.click("format_image.border.enabled")?;
    run.click("format_image.rotation.90")?;
    run.commit_modal_ok()?;
    match &run.document().layout_sheet(&lay_id).ok_or("WF-57 layout")?.spec.items[img_idx] {
        LayoutItem::EmbeddedImage { crop, border, rotation_deg, .. } => {
            ensure(
                crop.left_pt > 0.0 && crop.right_pt > 0.0 && crop.top_pt > 0.0 && crop.bottom_pt > 0.0,
                "WF-57 expected non-zero 4-edge crop insets",
            )?;
            ensure(border.is_some(), "WF-57 expected border enabled")?;
            ensure((*rotation_deg - 90.0).abs() < 1e-6, "WF-57 expected 90 deg rotation")?;
        }
        _ => return Err(String::from("WF-57 expected EmbeddedImage after FormatImage")),
    }

    // Export SVG, PDF, PNG before & after toggling View > Rulers and View > Canvas Grid -> must be byte-identical.
    let render_exports = |r: &WorkflowRun| -> Result<(String, Vec<u8>, Vec<u8>), String> {
        let lay = r.document().layout_sheet(&lay_id).ok_or("missing lay-wf57")?;
        let scene = compile_layout(lay, r.document()).map_err(|e| e.to_string())?;
        let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
        let pdf = render_pdf(&scene, &PdfExportOptions::default()).map_err(|e| e.to_string())?;
        let raster = render_scene(&scene, RenderTargetSpec { width_px: 320, height_px: 220, dpi: 96 }).map_err(|e| e.to_string())?;
        let png = encode_png(&raster).map_err(|e| e.to_string())?;
        Ok((svg, pdf, png))
    };
    let (svg_off, pdf_off, png_off) = render_exports(&run)?;

    run.open_menu_and_click("menu.view", &[], "menu.ui.rulers.toggle")?;
    run.open_menu_and_click("menu.view", &[], "menu.ui.gridOverlay.toggle")?;
    ensure(run.app().canvas_dialogs.rulers_visible, "WF-57 View > Rulers must toggle rulers_visible")?;
    ensure(run.app().canvas_dialogs.grid_overlay_visible, "WF-57 View > Canvas Grid must toggle grid_overlay_visible")?;

    let (svg_on, pdf_on, png_on) = render_exports(&run)?;
    ensure(svg_off == svg_on, "WF-57 rulers & canvas grid must not alter exported SVG")?;
    ensure(pdf_off == pdf_on, "WF-57 rulers & canvas grid must not alter exported PDF")?;
    ensure(png_off == png_on, "WF-57 rulers & canvas grid must not alter exported PNG")?;

    // Stage 4: verify embedded companion XY graph against headless Session oracle.
    let mut ref_xy = XyTable::new(SubcolumnMode::Single, 1, fx.assay_xy.len()).map_err(|e| e.to_string())?;
    for (r, &(x, y)) in fx.assay_xy.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ref_sheet = DataSheet::new("ref-xy-57", "Assay WF57", TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_graph(g_id.clone(), |sess| {
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-57", "data_sheet_id": "ref-xy-57", "kind": "xy" }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_graph_id(SheetId::new("ref-g-57"))
        .check_graph(|gui_gs, ref_gs| ensure(gui_gs.spec.series.len() == ref_gs.spec.series.len(), "WF-57 series mismatch vs ref")),
    )?;

    // Stage 5: live_edit on Assay XY cell propagates to embedded LiveGraph panel inside LayoutSheet SVG.
    let lay_svg_before = svg_on;
    let lay_for_chk = lay_id.clone();
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(sid, 0, 0, 0, "88.0"),
        vec![
            LiveEditExpectation::graph_svg_changed(g_id),
            LiveEditExpectation::custom(move |_, _, doc| {
                let lay = doc.layout_sheet(&lay_for_chk).ok_or("WF-57 missing layout post edit")?;
                let lay_svg_after =
                    render_svg(&compile_layout(lay, doc).map_err(|e| e.to_string())?, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
                ensure(lay_svg_after != lay_svg_before, "WF-57 upstream cell edit must update layout SVG")
            }),
        ],
    )?;

    run.select_sheet(&lay_id)?;
    run.finish()
}

#[test]
fn wf58_human_result_slice_paste_table_transpose_borderless_reactive() -> TestRes {
    let mut run = WorkflowRun::start("WF-58");
    let fx = wf58_descriptive_column();
    let sid = run.create_table_via_welcome("welcome.arch.column", fx.title, &["welcome.stepper.cols.inc"])?;

    for (c, &title) in fx.group_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{val}"))?;
        }
    }

    run.select_sheet(&sid)?;
    let desc_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    run.select_sheet(&desc_id)?;

    // Drag-copy rectangular slice (rows 0..=2, cols 0..=2) from ResultsView via real pointer drag.
    let c0_pos = run.widget_center("result.cell.0.0")?;
    let c1_pos = run.widget_center("result.cell.2.2")?;
    run.drag(c0_pos, c1_pos)?;
    run.click("result.copy_selection")?;

    let gs_id = graph_for_source(run.document(), &sid)?;
    run.select_sheet(&gs_id)?;
    let canvas_rect = run.frame_meta().find_by_widget_id("canvas.viewport").ok_or("WF-58 missing canvas.viewport")?.clipped_rect;
    let anchor = egui::pos2(canvas_rect.right() - 40.0, canvas_rect.bottom() - 40.0);
    run.click_at_button(anchor, PointerButton::Secondary, KeyModifiers::default())?;
    run.click("canvas.graph_ctx.paste_table")?;

    run.dbl_click("canvas.graph_ann.0")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::FormatEmbeddedTable { .. }), "WF-58 FormatEmbeddedTable must open")?;
    run.click("embedded_table.transpose")?;
    run.click("embedded_table.outer_border.0")?;
    run.click("embedded_table.gridline.0")?;
    run.commit_modal_ok()?;

    let svg_before = {
        let gs = run.document().graph_sheet(&gs_id).ok_or("WF-58 missing graph post embed")?;
        let Some(GraphAnnotation::EmbeddedResultTable { style, .. }) = gs.spec.annotations.first() else {
            return Err(String::from("WF-58 missing EmbeddedResultTable annotation"));
        };
        ensure(
            style.transpose && style.outer_border_width_pt == 0.0 && style.gridline_width_pt == 0.0,
            "WF-58 borderless transpose settings must persist",
        )?;
        let rs = run.document().result_sheet(&desc_id).ok_or("WF-58 missing desc result")?;
        let tab0 = rs.tabs.first().ok_or("WF-58 missing desc tab 0")?;
        let hdr_text = tab0.column_headers.first().map(String::as_str).unwrap_or("");
        let cell_text = tab0
            .rows
            .first()
            .and_then(|r| r.first())
            .map(|c| match c {
                ResultCell::Number { value, .. } => format!("{value}"),
                ResultCell::Text(s) => s.clone(),
                _ => String::new(),
            })
            .unwrap_or_default();
        let svg = graph_svg_rendered(run.document(), &gs_id)?;
        ensure(!hdr_text.is_empty() && svg.contains(hdr_text), format!("WF-58 SVG missing embedded slice header `{hdr_text}`"))?;
        ensure(!cell_text.is_empty() && svg.contains(&cell_text), format!("WF-58 SVG missing embedded slice cell `{cell_text}`"))?;
        svg
    };

    // Stage 4: verify Descriptive Statistics values against headless Session oracle.
    let mut ref_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, fx.group_titles.len(), fx.rows.len()).map_err(|e| e.to_string())?;
    for (c, &title) in fx.group_titles.iter().enumerate() {
        ref_col.data_sets[c].title = title.to_owned();
    }
    for (r, row) in fx.rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            ref_col.data_sets[c].cells[r][0] = Cell::number(val);
        }
    }
    let ref_sheet = DataSheet::new("ref-col-58", fx.title, TableArchetype::Column(ref_col)).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(desc_id.clone(), |sess| {
            sess.execute("analysis.run", json!({ "sheet_id": "ref-desc-58", "source_sheet_id": "ref-col-58", "kind": "descriptive" }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-desc-58"))
        .scalar("Mean", Some(1), 1e-9, 1e-9)
        .scalar("SD", Some(1), 1e-9, 1e-9)
        .tab_cell(0, 0, 1, 1e-9, 1e-9),
    )?;

    // Stage 5: deleting upstream cell (row 0, col 1) recomputes Descriptive Stats & embedded table SVG.
    let sid_del = sid.clone();
    let gs_for_chk = gs_id.clone();
    run.live_edit_all(
        LiveEditTarget::gui(move |r| {
            r.select_sheet(&sid_del)?;
            r.dbl_click("grid.cell.0.1.0")?;
            r.press_egui_key(Key::A, Modifiers::COMMAND)?;
            r.press_key("Backspace", KeyModifiers::default())?;
            r.press_key("Enter", KeyModifiers::default())
        }),
        vec![
            LiveEditExpectation::scalar_changed(desc_id, "Mean", Some(1), 1e-3),
            LiveEditExpectation::custom(move |_, _, doc| {
                let svg_after = graph_svg_rendered(doc, &gs_for_chk)?;
                ensure(svg_after != svg_before, "WF-58 deleting upstream cell must update embedded table SVG")
            }),
        ],
    )?;

    run.select_sheet(&gs_id)?;
    run.finish()
}
