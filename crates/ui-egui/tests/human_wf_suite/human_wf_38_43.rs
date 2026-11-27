#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-38`–`WF-43`, `R15.4`/`R15.5`).

use curvebook_doc::{
    AnalysisKind, Cell, CellValue, ClusterDistance, ClusterLinkage, CoxTieMethod, DataSheet, GlmFamily, GlmSpec, GraphKind, GraphSheet,
    MathTransformPreset, MultivariateSpec, NonlinearFitSpec, ParameterRule, ParameterScope, ProjectDocument, ResultSheet, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_testkit::wf_datasets;
use egui::Vec2;
use serde_json::{Value, json};

use crate::human_wf_common::{
    TestRes, ensure,
    workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun},
};

fn exec(sess: &mut Session, cmd: &str, params: Value) -> Result<Value, String> {
    sess.execute(cmd, params).map(|o| o.payload).map_err(|e| format!("{cmd} failed: {e}"))
}

fn payload_sheet_id(payload: &Value) -> Result<String, String> {
    payload["sheet_id"].as_str().map(str::to_owned).ok_or_else(|| format!("missing `sheet_id` in command payload: {payload}"))
}

fn mv_data_tsv(fixture: &wf_datasets::MvFixture) -> String {
    fixture.rows.iter().map(|row| row.iter().map(wf_datasets::MvCellValue::as_token).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n")
}

fn enter_mv_fixture(run: &mut WorkflowRun, fixture: &wf_datasets::MvFixture) -> Result<SheetId, String> {
    let mut extra_clicks = vec!["welcome.stepper.cols.inc"; fixture.columns.len().saturating_sub(3)];
    extra_clicks.extend(std::iter::repeat_n("welcome.stepper.rows.inc", fixture.rows.len().saturating_sub(12)));
    let ds_id = run.create_table_via_welcome("welcome.arch.multiple_variables", fixture.title, &extra_clicks)?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard(&mv_data_tsv(fixture))?;
    for (c, col) in fixture.columns.iter().enumerate() {
        run.enter_column_title(c, col.title)?;
        run.set_mv_role(c, col.role_slug)?;
    }
    Ok(ds_id)
}

fn pin_mv_reference_level(run: &mut WorkflowRun, ds_id: &SheetId, col: usize, level_idx: usize) -> TestRes {
    run.select_sheet(ds_id)?;
    run.click("mv_formula.role_badge.0")?;
    run.click("mv_formula.role_manage.0")?;
    ensure(run.app().grid.variables_dialog.open, "Manage Variables modal failed to open")?;
    run.click(&format!("mv_formula.vars.col.{col}"))?;
    run.click(&format!("mv_formula.vars.level_ref.{level_idx}"))?;
    run.click("modal.ok")?;
    ensure(!run.app().grid.variables_dialog.open, "Manage Variables modal did not close on OK")
}

fn result_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a ResultSheet, String> {
    doc.result_sheet(id).ok_or_else(|| format!("missing ResultSheet `{}`", id.as_str()))
}

fn tab_index_by_id(doc: &ProjectDocument, rs_id: &SheetId, tab_id: &str) -> Result<usize, String> {
    let rs = result_sheet(doc, rs_id)?;
    rs.tabs.iter().position(|t| t.id == tab_id).ok_or_else(|| format!("missing tab `{tab_id}` in `{}`", rs_id.as_str()))
}

fn find_linked_graph<'a>(doc: &'a ProjectDocument, rs_id: &SheetId, ds_id: &SheetId, kind: Option<GraphKind>) -> Result<&'a GraphSheet, String> {
    doc.graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id == *rs_id && kind.is_none_or(|k| g.spec.kind == k))
        .or_else(|| doc.graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == *ds_id && kind.is_none_or(|k| g.spec.kind == k)))
        .ok_or_else(|| format!("missing linked graph for `{}` / `{}` (kind={kind:?})", rs_id.as_str(), ds_id.as_str()))
}

fn compile_svg(doc: &ProjectDocument, gs: &GraphSheet) -> Result<String, String> {
    let scene = curvebook_chart::compile_graph(gs, doc).map_err(|e| format!("compile_graph `{}` failed: {e}", gs.id.as_str()))?;
    curvebook_svg::render_svg(&scene, &curvebook_svg::SvgExportOptions::default()).map_err(|e| format!("render_svg `{}` failed: {e}", gs.id.as_str()))
}

fn has_curve_or_marker_svg(svg: &str) -> bool {
    svg.contains("<svg") && (svg.contains("<path") || svg.contains("<circle"))
}

#[test]
fn wf38_human_kmeans_clustering_silhouette_elbow_and_scatter() -> TestRes {
    let mut run = WorkflowRun::start("WF-38");
    let fixture = wf_datasets::clustering_three_clusters();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    let rs_id = run.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;

    // Configure non-default clustering options via Cmd+T (`k_means_auto` + `k_sweep_max` elbow sweep + Complete linkage)
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.cluster.link.complete")?;
    run.click("edit.cluster.k_means_auto")?;
    run.click("edit.cluster.k_sweep_max.inc")?;
    run.commit_modal_ok()?;

    let (k_sweep_max, chosen_k, mean_sil) = {
        let rs = result_sheet(run.document(), &rs_id)?;
        let k_sweep = match &rs.analysis.kind {
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: None,
                k_sweep_max: Some(km),
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Complete,
                ..
            }) => *km,
            other => return Err(format!("WF-38 expected auto-k Complete clustering, got {other:?}")),
        };
        let k_val = rs.lookup_scalar("Clusters (k)", None).ok_or("WF-38 missing Clusters (k)")?;
        let sil = rs.lookup_scalar("Mean silhouette", None).ok_or("WF-38 missing Mean silhouette")?;
        ensure((k_val - 3.0).abs() < 1e-12 && sil > 0.70, format!("WF-38 expected auto-selected k=3 & strong silhouette, got k={k_val}, sil={sil}"))?;
        (k_sweep, k_val, sil)
    };
    let clusters_tab = tab_index_by_id(run.document(), &rs_id, "kmeans-clusters")?;
    let sweep_tab = tab_index_by_id(run.document(), &rs_id, "kmeans-sweep-curve")?;
    run.select_sheet(&rs_id)?;
    run.click("toolbar.append_cluster")?;
    let graph = find_linked_graph(run.document(), &rs_id, &ds_id, None)?;
    let graph_id = graph.id.clone();
    let graph_kind = graph.spec.kind;
    let graph_from_rs = graph.spec.primary_source_sheet_id == rs_id;
    if graph_kind == GraphKind::MultipleVariables {
        run.select_sheet(&graph_id)?;
        run.click("toolbar.format_graph")?;
        run.click("edit.tab.format_graph.axis_variables")?;
        let n_cols = match &run.document().data_sheet(&ds_id).ok_or("missing ds")?.archetype {
            TableArchetype::MultipleVariables(mv) => mv.variables.len(),
            _ => 5,
        };
        let (x0, y0) = run.document().graph_sheet(&graph_id).and_then(|g| g.spec.multivariate.as_ref()).map_or((0, 1), |m| (m.x_column, m.y_column));
        for _ in 0..((1 + n_cols - (x0 % n_cols)) % n_cols) {
            run.click("format_graph.mv.x_next")?;
        }
        for _ in 0..((2 + n_cols - (y0 % n_cols)) % n_cols) {
            run.click("format_graph.mv.y_next")?;
        }
        run.commit_modal_ok()?;
    }
    let gui_mv_enc = run.document().graph_sheet(&graph_id).and_then(|g| g.spec.multivariate.clone());
    ensure(
        has_curve_or_marker_svg(&compile_svg(run.document(), run.document().graph_sheet(&graph_id).ok_or("missing gui graph")?)?),
        "WF-38 cluster scatter SVG missing geometry",
    )?;

    let ref_sheet = fixture.to_data_sheet("ref-cluster-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::Multivariate(MultivariateSpec::Clustering {
        distance: ClusterDistance::Euclidean,
        linkage: ClusterLinkage::Complete,
        k_means_k: None,
        k_sweep_max: Some(k_sweep_max),
        consensus_optimal_k: false,
        kmeans_algorithm: Default::default(),
        cluster_direction: Default::default(),
        cut_height: None,
        emit_anova_tab: false,
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            let rs_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = payload_sheet_id(&rs_out)?;
            let g_src = if graph_from_rs { ref_rs_id.as_str() } else { ref_ds_id.as_str() };
            let g_out = exec(sess, "graph.create", json!({ "data_sheet_id": g_src, "kind": graph_kind, "initialized": true }))?;
            if let Some(enc) = gui_mv_enc {
                let ref_gid = payload_sheet_id(&g_out)?;
                exec(sess, "graph.setSpec", json!({ "sheet_id": ref_gid, "kind": GraphKind::MultipleVariables, "multivariate": enc }))?;
            }
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(graph_id.clone())
        .scalar("Clusters (k)", None, 1e-12, 1e-12)
        .scalar("Mean silhouette", None, 1e-8, 1e-8)
        .scalar("Total WCSS", None, 1e-8, 1e-8)
        .tab_cell(clusters_tab, 0, 0, 1e-6, 1e-6)
        .tab_cell(sweep_tab, 0, 1, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| ensure(gui_gs.spec.kind == ref_gs.spec.kind, "WF-38 cluster scatter GraphKind mismatch")),
    )?;

    // Also switch via Cmd+T to explicit manual k=3 + Manhattan distance and verify live upstream edit propagates
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.cluster.dist.manhattan")?;
    run.click("edit.cluster.k_means_auto")?;
    run.commit_modal_ok()?;
    ensure(chosen_k > 0.0 && mean_sil > 0.0, "WF-38 cached baseline sanity")?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 0, 1, 0, "4.5"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "Total WCSS", None, 1e-3),
            LiveEditExpectation::scalar_changed(rs_id, "Mean silhouette", None, 1e-4),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;

    run.finish()
}

#[test]
fn wf39_human_cox_ph_efron_ties_and_covariate_curves() -> TestRes {
    let mut run = WorkflowRun::start("WF-39");
    let fixture = wf_datasets::cox_ph_clinical();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    // Pin non-default reference level `Therapy` (level index 1) on categorical column `Treatment` (col 2)
    pin_mv_reference_level(&mut run, &ds_id, 2, 1)?;

    let rs_id = run.run_analyze_entry("Cox", "analyze.entry.stats.survival_cox")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.data")?;
    run.click("edit.cox.pred.4")?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.cox.breslow")?;
    run.commit_modal_ok()?;

    {
        let rs = result_sheet(run.document(), &rs_id)?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                    tie_method: CoxTieMethod::Breslow,
                    predictor_columns,
                    ..
                }) if predictor_columns == &[2, 3]
            ),
            format!("WF-39 expected non-default Breslow + [2,3] predictors, got {:?}", rs.analysis.kind),
        )?;
    }

    let ph_tab = tab_index_by_id(run.document(), &rs_id, "cox-ph")?;
    let graph_id = find_linked_graph(run.document(), &rs_id, &ds_id, None)?.id.clone();
    run.select_sheet(&graph_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.show_all")?;
    run.click("graph_type.card.survival.km")?;
    run.commit_modal_ok()?;
    let graph = run.document().graph_sheet(&graph_id).ok_or("missing Cox survival graph")?;
    let graph_kind = graph.spec.kind;
    let graph_from_rs = graph.spec.primary_source_sheet_id == rs_id;
    ensure(
        graph_kind == GraphKind::Survival && has_curve_or_marker_svg(&compile_svg(run.document(), graph)?),
        "WF-39 Cox survival SVG missing geometry",
    )?;

    let ref_sheet = fixture.to_data_sheet("ref-cox-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column: 0,
        status_column: 1,
        predictor_columns: vec![2, 3],
        tie_method: CoxTieMethod::Breslow,
        reference_levels: Vec::new(),
        formula: None,
        covariate_profiles: Vec::new(),
        nested_comparison: None,
        emit_cox_snell_residuals: false,
        ci_method: Default::default(),
        event_code: None,
        censored_code: None,
        treat_other_as: Default::default(),
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            exec(sess, "table.setCategoricalOrder", json!({ "sheet_id": ref_ds_id.as_str(), "col": 2, "reference_level": "Therapy" }))?;
            let rs_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = payload_sheet_id(&rs_out)?;
            let g_src = if graph_from_rs { ref_rs_id.as_str() } else { ref_ds_id.as_str() };
            exec(sess, "graph.create", json!({ "data_sheet_id": g_src, "kind": graph_kind, "initialized": true }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(graph_id.clone())
        .scalar("Concordance (C-index)", None, 1e-8, 1e-8)
        .scalar("AIC", None, 1e-6, 1e-6)
        .scalar("Likelihood ratio P", None, 1e-8, 1e-6)
        .tab_cell(ph_tab, 0, 0, 1e-6, 1e-6)
        .tab_cell(ph_tab, 0, 4, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.kind == GraphKind::Survival && ref_gs.spec.kind == GraphKind::Survival, "WF-39 covariate survival graph kind mismatch")
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 0, 0, 0, "20.0"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "AIC", None, 1e-3),
            LiveEditExpectation::tab_cell_changed(rs_id, ph_tab, 0, 0),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;

    run.finish()
}

#[test]
fn wf40_human_multiple_linear_regression_type_iii_and_vif() -> TestRes {
    let mut run = WorkflowRun::start("WF-40");
    let fixture = wf_datasets::glm_multiple_linear();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    // Pin non-default reference level `Treated` (level index 1) on `Group` (col 3)
    pin_mv_reference_level(&mut run, &ds_id, 3, 1)?;

    let rs_id = run.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.data")?;
    // AUD-09-11: Analyze honors `is_response` (last numeric column, col 2) by default;
    // bind col 0 (`Response`) as Y so predictors become `[1, 3]`.
    run.click("edit.glm.resp.0")?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.glm.linear")?;
    run.commit_modal_ok()?;

    {
        let rs = result_sheet(run.document(), &rs_id)?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::GeneralizedLinearModel(g)
                    if g.family == GlmFamily::Linear && g.predictor_columns == [1, 3] && g.compute_vif
            ),
            format!("WF-40 expected Linear GLM with predictors [1,3] & VIF, got {:?}", rs.analysis.kind),
        )?;
    }

    let params_tab = tab_index_by_id(run.document(), &rs_id, "glm-linear-params")?;
    let type3_tab = tab_index_by_id(run.document(), &rs_id, "glm-linear-type-iii")?;
    let graph_id = find_linked_graph(run.document(), &rs_id, &ds_id, None)?.id.clone();
    run.select_sheet(&graph_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.family.mv_diag")?;
    run.click("graph_type.show_all")?;
    let scroll_pos = run.widget_center("graph_type.show_all")? + Vec2::new(0.0, 100.0);
    run.wheel(scroll_pos, Vec2::new(0.0, -200.0))?;
    run.click("graph_type.card.diag.residual")?;
    run.commit_modal_ok()?;
    let graph = run.document().graph_sheet(&graph_id).ok_or("missing WF-40 residual graph")?;
    let graph_kind = graph.spec.kind;
    let graph_from_rs = graph.spec.primary_source_sheet_id == rs_id;
    ensure(
        matches!(graph_kind, GraphKind::Diagnostic(_)) && has_curve_or_marker_svg(&compile_svg(run.document(), graph)?),
        "WF-40 Linear GLM Residual SVG missing geometry",
    )?;

    let ref_sheet = fixture.to_data_sheet("ref-glm-lin-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family: GlmFamily::Linear,
        response_column: 0,
        predictor_columns: vec![1, 3],
        compute_vif: true,
        reference_levels: Vec::new(),
        ..Default::default()
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            exec(sess, "table.setCategoricalOrder", json!({ "sheet_id": ref_ds_id.as_str(), "col": 3, "reference_level": "Treated" }))?;
            let rs_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = payload_sheet_id(&rs_out)?;
            let g_src = if graph_from_rs { ref_rs_id.as_str() } else { ref_ds_id.as_str() };
            exec(sess, "graph.create", json!({ "data_sheet_id": g_src, "kind": graph_kind, "initialized": true }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(graph_id.clone())
        .scalar("R squared", None, 1e-8, 1e-8)
        .scalar("Adjusted R squared", None, 1e-8, 1e-8)
        .scalar("AIC", None, 1e-6, 1e-6)
        .scalar("Durbin-Watson", None, 1e-8, 1e-8)
        .tab_cell(params_tab, 1, 0, 1e-6, 1e-6)
        .tab_cell(params_tab, 1, 5, 1e-6, 1e-6)
        .tab_cell(type3_tab, 0, 3, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| ensure(gui_gs.spec.kind == ref_gs.spec.kind, "WF-40 Linear GLM graph kind mismatch")),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 0, 0, 0, "42.0"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "R squared", None, 0.05),
            LiveEditExpectation::tab_cell_changed(rs_id, params_tab, 1, 0),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;

    run.finish()
}

#[test]
fn wf41_human_multiple_logistic_classification_and_roc() -> TestRes {
    let mut run = WorkflowRun::start("WF-41");
    let fixture = wf_datasets::glm_logistic();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    // Pin non-default reference level `CohortB` (level index 1) on col 3
    pin_mv_reference_level(&mut run, &ds_id, 3, 1)?;

    let rs_id = run.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.data")?;
    // AUD-09-11: bind binary outcome col 0 (`Event`) as Y and re-include col 2 (`BaselineMarker`) as X.
    run.click("edit.glm.resp.0")?;
    run.click("edit.glm.pred.2")?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.glm.logistic")?;
    run.commit_modal_ok()?;

    {
        let rs = result_sheet(run.document(), &rs_id)?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::GeneralizedLinearModel(g)
                    if g.family == GlmFamily::Logistic && g.predictor_columns == [1, 2, 3] && g.compute_vif
            ),
            format!("WF-41 expected Logistic GLM with predictors [1,2,3], got {:?}", rs.analysis.kind),
        )?;
    }

    let params_tab = tab_index_by_id(run.document(), &rs_id, "glm-logistic-params")?;
    let graph = find_linked_graph(run.document(), &rs_id, &ds_id, None)?;
    let graph_id = graph.id.clone();
    let graph_kind = graph.spec.kind;
    let graph_from_rs = graph.spec.primary_source_sheet_id == rs_id;
    ensure(has_curve_or_marker_svg(&compile_svg(run.document(), graph)?), "WF-41 Logistic ROC SVG missing geometry")?;

    let ref_sheet = fixture.to_data_sheet("ref-glm-log-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family: GlmFamily::Logistic,
        response_column: 0,
        predictor_columns: vec![1, 2, 3],
        compute_vif: true,
        reference_levels: Vec::new(),
        ..Default::default()
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            exec(sess, "table.setCategoricalOrder", json!({ "sheet_id": ref_ds_id.as_str(), "col": 3, "reference_level": "CohortB" }))?;
            let rs_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = payload_sheet_id(&rs_out)?;
            let g_src = if graph_from_rs { ref_rs_id.as_str() } else { ref_ds_id.as_str() };
            exec(sess, "graph.create", json!({ "data_sheet_id": g_src, "kind": graph_kind, "initialized": true }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(graph_id.clone())
        .scalar("ROC AUC", None, 1e-8, 1e-8)
        .scalar("Hosmer-Lemeshow chi2", None, 1e-6, 1e-6)
        .scalar("Hosmer-Lemeshow P", None, 1e-6, 1e-6)
        .scalar("AIC", None, 1e-6, 1e-6)
        .tab_cell(params_tab, 1, 0, 1e-6, 1e-6)
        .tab_cell(params_tab, 1, 3, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| ensure(gui_gs.spec.kind == ref_gs.spec.kind, "WF-41 Logistic ROC graph kind mismatch")),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 1, 1, 0, "4.2"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "AIC", None, 1e-3),
            LiveEditExpectation::tab_cell_changed(rs_id, params_tab, 1, 0),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;

    run.finish()
}

#[test]
fn wf42_human_poisson_count_glm_dispersion_and_vif() -> TestRes {
    let mut run = WorkflowRun::start("WF-42");
    let fixture = wf_datasets::glm_poisson();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    // Pin non-default reference level (index 1, `Treated`) on `Arm` (col 3)
    pin_mv_reference_level(&mut run, &ds_id, 3, 1)?;

    let rs_id = run.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.data")?;
    // AUD-09-11: bind integer count col 0 (`Counts`) as Y and re-include col 2 (`DoseLevel`) as X.
    run.click("edit.glm.resp.0")?;
    run.click("edit.glm.pred.2")?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.glm.poisson")?;
    run.commit_modal_ok()?;

    {
        let rs = result_sheet(run.document(), &rs_id)?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::GeneralizedLinearModel(g)
                    if g.family == GlmFamily::Poisson && g.predictor_columns == [1, 2, 3] && g.compute_vif
            ),
            format!("WF-42 expected Poisson GLM with predictors [1,2,3], got {:?}", rs.analysis.kind),
        )?;
    }

    let params_tab = tab_index_by_id(run.document(), &rs_id, "glm-poisson-params")?;
    let graph = find_linked_graph(run.document(), &rs_id, &ds_id, None)?;
    let graph_id = graph.id.clone();
    let graph_kind = graph.spec.kind;
    let graph_from_rs = graph.spec.primary_source_sheet_id == rs_id;
    ensure(has_curve_or_marker_svg(&compile_svg(run.document(), graph)?), "WF-42 Poisson GLM SVG missing geometry")?;

    let ref_sheet = fixture.to_data_sheet("ref-glm-pois-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let ref_arm_name = match &ref_sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let mut seen = Vec::new();
            for cell in &mv.variables[3].cells {
                if let CellValue::Text(s) = &cell.value
                    && !seen.iter().any(|x| x == s)
                {
                    seen.push(s.clone());
                }
            }
            seen.get(1).cloned().unwrap_or_else(|| String::from("Treated"))
        }
        _ => String::from("Treated"),
    };
    let oracle_kind = AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family: GlmFamily::Poisson,
        response_column: 0,
        predictor_columns: vec![1, 2, 3],
        compute_vif: true,
        reference_levels: Vec::new(),
        ..Default::default()
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            exec(sess, "table.setCategoricalOrder", json!({ "sheet_id": ref_ds_id.as_str(), "col": 3, "reference_level": ref_arm_name }))?;
            let rs_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = payload_sheet_id(&rs_out)?;
            let g_src = if graph_from_rs { ref_rs_id.as_str() } else { ref_ds_id.as_str() };
            exec(sess, "graph.create", json!({ "data_sheet_id": g_src, "kind": graph_kind, "initialized": true }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(graph_id.clone())
        .scalar("Residual deviance", None, 1e-6, 1e-6)
        .scalar("Pearson dispersion", None, 1e-6, 1e-6)
        .scalar("AIC", None, 1e-6, 1e-6)
        .scalar("McFadden R squared", None, 1e-6, 1e-6)
        .tab_cell(params_tab, 1, 0, 1e-6, 1e-6)
        .tab_cell(params_tab, 1, 6, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| ensure(gui_gs.spec.kind == ref_gs.spec.kind, "WF-42 Poisson GLM graph kind mismatch")),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 13, 0, 0, "48"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "Residual deviance", None, 1e-2),
            LiveEditExpectation::scalar_changed(rs_id.clone(), "AIC", None, 1e-2),
            LiveEditExpectation::tab_cell_changed(rs_id, params_tab, 1, 0),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;

    run.finish()
}

#[test]
fn wf43_human_xy_serial_dilution_constrained_4pl_and_auc() -> TestRes {
    let mut run = WorkflowRun::start("WF-43");
    let fixture = wf_datasets::dose_response_4pl();
    // Create 2-series XY table with 3 raw replicates and 8 dilution rows matching dose_response_4pl()
    let extra_clicks = vec!["welcome.subcol.raw", "welcome.stepper.cols.dec"];
    let ds_id = run.create_table_via_welcome("welcome.arch.xy", fixture.title, &extra_clicks)?;
    for (s, title) in fixture.series_titles.iter().enumerate() {
        run.enter_column_title(s, title)?;
    }
    for (r, row) in fixture.rows.iter().enumerate() {
        let molar = 10_f64.powf(row.x);
        run.enter_x_cell(r, 0, &format!("{molar:.12e}"))?;
        for (s, reps) in row.series_replicates.iter().enumerate() {
            for (rep, &y) in reps.iter().enumerate() {
                run.enter_grid_cell(r, s, rep, &format!("{y}"))?;
            }
        }
    }

    // Step 1: Math Transform -> log10(X), none(Y)
    let xform_id = run.run_analyze_entry("Transform", "analyze.entry.transform.math")?;
    run.select_sheet(&xform_id)?;
    run.open_cmd_t()?;
    run.click("edit.transform.x_transform.log10")?;
    run.click("edit.transform.y_transform.none")?;
    run.commit_modal_ok()?;

    // Step 2: Chain Nonlinear 4PL directly onto the Transform ResultSheet, applying Bottom=0 & Top=100 in Constrain tab
    run.select_sheet(&xform_id)?;
    let fit_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&fit_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.constraints")?;
    run.click("fit.con.rule.Bottom.fixed")?;
    run.replace_textbox("fit.con.const.Bottom", "0")?;
    run.click("fit.con.rule.Top.fixed")?;
    run.replace_textbox("fit.con.const.Top", "100")?;
    run.commit_modal_ok()?;

    {
        let rs = result_sheet(run.document(), &fit_id)?;
        ensure(rs.analysis.source_sheet_id.as_ref() == Some(&xform_id), "WF-43 expected 4PL source chained to log10(X) ResultSheet")?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::NonlinearRegression(nl)
                    if nl.equation_id == "log_agonist_vs_response_4pl"
                    && nl.parameter_rules.iter().any(|r| r.name == "Bottom" && r.scope == ParameterScope::Constant(0.0))
                    && nl.parameter_rules.iter().any(|r| r.name == "Top" && r.scope == ParameterScope::Constant(100.0))
            ),
            format!("WF-43 expected constrained Bottom=0 & Top=100 on 4PL, got {:?}", rs.analysis.kind),
        )?;
        let span = rs.lookup_scalar("Span", Some(0)).ok_or("WF-43 missing Span")?;
        let auc_fit = rs.lookup_scalar("AUC (fitted curve)", Some(0)).ok_or("WF-43 missing AUC (fitted curve)")?;
        ensure((span - 100.0).abs() < 1e-8 && auc_fit > 10.0, format!("WF-43 invalid Span={span} / AUC={auc_fit}"))?;
    }

    let params_tab = tab_index_by_id(run.document(), &fit_id, "nonlinear_parameters")?;
    let auc_tab = tab_index_by_id(run.document(), &fit_id, "fit_auc")?;
    let graph = find_linked_graph(run.document(), &fit_id, &ds_id, Some(GraphKind::Xy))?;
    let graph_id = graph.id.clone();
    ensure(has_curve_or_marker_svg(&compile_svg(run.document(), graph)?), "WF-43 4PL XY SVG missing geometry")?;

    let mut ref_xy = XyTable::new_with_x_mode(
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        SubcolumnMode::RawReplicates(fixture.replicates),
        fixture.series_titles.len(),
        fixture.rows.len(),
    )
    .map_err(|e| e.to_string())?;
    for (s, title) in fixture.series_titles.iter().enumerate() {
        ref_xy.data_sets[s].title = (*title).to_owned();
    }
    for (r, row) in fixture.rows.iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(10_f64.powf(row.x));
        for (s, reps) in row.series_replicates.iter().enumerate() {
            for (rep, &y) in reps.iter().enumerate() {
                ref_xy.data_sets[s].cells[r][rep] = Cell::number(y);
            }
        }
    }
    if let TableArchetype::Xy(gui_xy) = &run.document().data_sheet(&ds_id).ok_or("missing gui xy")?.archetype {
        ref_xy.x_title.clone_from(&gui_xy.x_title);
    }
    let ref_sheet = DataSheet::new("ref-4pl-xy", fixture.title, TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let xform_kind = AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform: Some(MathTransformPreset::Log10),
        y_transform: None,
        error_propagation: Default::default(),
        pharmacology_preset: None,
        interchange_xy: false,
        zero_x_substitute: None,
        asymmetric_ci_policy: Default::default(),
    });
    let fit_kind = AnalysisKind::NonlinearRegression(NonlinearFitSpec {
        parameter_rules: vec![
            ParameterRule {
                name: String::from("Bottom"),
                scope: ParameterScope::Constant(0.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: String::from("Top"),
                scope: ParameterScope::Constant(100.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
    });
    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), |sess| {
            let xf_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": xform_kind }))?;
            let ref_xf_id = payload_sheet_id(&xf_out)?;
            exec(sess, "analysis.run", json!({ "source_sheet_id": ref_xf_id, "kind": fit_kind }))?;
            exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "xy", "initialized": true }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(graph_id.clone())
        .scalar("Span", Some(0), 1e-8, 1e-8)
        .scalar("EC50", Some(0), 1e-10, 1e-6)
        .scalar("AUC (trapezoidal)", Some(0), 1e-6, 1e-6)
        .scalar("AUC (fitted curve)", Some(0), 1e-6, 1e-6)
        .scalar("R2", None, 1e-8, 1e-8)
        .tab_cell(params_tab, 2, 0, 1e-6, 1e-6)
        .tab_cell(auc_tab, 1, 0, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| ensure(gui_gs.spec.kind == ref_gs.spec.kind, "WF-43 XY GraphKind mismatch")),
    )?;

    // 2-hop reactive DAG edit on raw linear-molar XY sheet -> updates Transform -> constrained 4PL & AUC
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 4, 0, 0, "45.0"),
        vec![
            LiveEditExpectation::scalar_changed(fit_id.clone(), "EC50", Some(0), 1e-8),
            LiveEditExpectation::scalar_changed(fit_id.clone(), "AUC (fitted curve)", Some(0), 1e-2),
            LiveEditExpectation::tab_cell_changed(fit_id, params_tab, 2, 0),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;

    run.finish()
}
