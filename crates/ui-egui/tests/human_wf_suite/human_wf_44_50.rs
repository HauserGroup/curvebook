#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-44`–`WF-50`, `R15.4`/`R15.5`).

use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, ColorChannelMapping, DataSheet, DiagnosticPlotKind, EquationDisplayMode, GraphAnnotation,
    GraphKind, GraphSheet, HeatmapColumnLabelAngle, MultiplicityMethod, MultiplicitySpec, MultivariateGraphEncoding, MultivariateSpec,
    NonlinearFitSpec, OneWayAnovaSpec, PairwiseDisplayMode, ProjectDocument, ResultCell, ResultSheet, SheetId, SizeChannelMapping,
    SurvivalAnalysisSpec,
};
use curvebook_engine::Session;
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_testkit::wf_datasets;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use egui::{Pos2, Vec2};
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

fn result_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a ResultSheet, String> {
    doc.result_sheet(id).ok_or_else(|| format!("missing ResultSheet `{}`", id.as_str()))
}

fn graph_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a GraphSheet, String> {
    doc.graph_sheet(id).ok_or_else(|| format!("missing GraphSheet `{}`", id.as_str()))
}

fn compile_svg(doc: &ProjectDocument, gs: &GraphSheet) -> Result<String, String> {
    let scene = curvebook_chart::compile_graph(gs, doc).map_err(|e| format!("compile_graph `{}` failed: {e}", gs.id.as_str()))?;
    curvebook_svg::render_svg(&scene, &curvebook_svg::SvgExportOptions::default()).map_err(|e| format!("render_svg `{}` failed: {e}", gs.id.as_str()))
}

fn tab_index_by_id(doc: &ProjectDocument, rs_id: &SheetId, tab_id: &str) -> Result<usize, String> {
    let rs = result_sheet(doc, rs_id)?;
    rs.tabs.iter().position(|t| t.id == tab_id).ok_or_else(|| format!("missing tab `{tab_id}` in `{}`", rs_id.as_str()))
}

fn find_linked_graph<'a>(doc: &'a ProjectDocument, primary_ids: &[&SheetId], kind: Option<GraphKind>) -> Result<&'a GraphSheet, String> {
    doc.graph_sheets
        .iter()
        .find(|g| primary_ids.contains(&&g.spec.primary_source_sheet_id) && kind.is_none_or(|k| g.spec.kind == k))
        .ok_or_else(|| format!("missing linked graph for sources {:?} (kind={kind:?})", primary_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>()))
}

fn create_mv_graph_via_gui(run: &mut WorkflowRun, ds_id: &SheetId) -> Result<SheetId, String> {
    run.select_sheet(ds_id)?;
    let before_count = run.document().graph_sheets.len();
    run.right_click(&format!("nav.sheet.{}", ds_id.as_str()))?;
    run.click("nav.ctx.new_graph")?;
    let gs = run
        .document()
        .graph_sheets
        .iter()
        .skip(before_count)
        .find(|g| g.spec.primary_source_sheet_id == *ds_id && g.spec.kind == GraphKind::MultipleVariables)
        .ok_or_else(|| format!("new MultipleVariables graph not created for `{}`", ds_id.as_str()))?;
    Ok(gs.id.clone())
}

fn cycle_clicks(current: usize, target: usize, total: usize) -> usize {
    (target + total - (current % total)) % total
}

fn mv_var_count(doc: &ProjectDocument, ds_id: &SheetId) -> Result<usize, String> {
    let ds = doc.data_sheet(ds_id).ok_or_else(|| format!("missing MV DataSheet `{}`", ds_id.as_str()))?;
    match &ds.archetype {
        curvebook_doc::TableArchetype::MultipleVariables(mv) => Ok(mv.variables.len()),
        _ => Err(format!("expected MultipleVariables table for `{}`", ds_id.as_str())),
    }
}

fn mv_encoding<'a>(doc: &'a ProjectDocument, gs_id: &SheetId) -> Result<&'a MultivariateGraphEncoding, String> {
    graph_sheet(doc, gs_id)?.spec.multivariate.as_ref().ok_or_else(|| format!("missing spec.multivariate on `{}`", gs_id.as_str()))
}

fn oracle_append_cluster_column(sess: &mut Session, ref_ds_id: &SheetId, ref_rs_id: &SheetId) -> TestRes {
    let tab = result_sheet(sess.document(), ref_rs_id)?
        .tabs
        .iter()
        .find(|t| t.id == "kmeans-assignments")
        .ok_or("oracle clustering missing kmeans-assignments")?;
    let rows: Vec<Vec<Value>> = tab
        .rows
        .iter()
        .map(|r| {
            let label = match r.first() {
                Some(ResultCell::Number { value, .. }) if value.is_finite() => format!("Cluster {}", (*value).round() as i64),
                Some(ResultCell::Text(s)) => s.trim().to_owned(),
                _ => String::new(),
            };
            vec![json!(label)]
        })
        .collect();
    exec(sess, "table.paste", json!({ "sheet_id": ref_ds_id.as_str(), "mode": "append_columns", "rows": rows }))?;
    exec(sess, "table.setGroupTitles", json!({ "sheet_id": ref_ds_id.as_str(), "col": 4, "title": "Cluster" }))?;
    exec(sess, "table.setVariableRole", json!({ "sheet_id": ref_ds_id.as_str(), "col": 4, "role": "categorical" }))?;
    Ok(())
}

fn count_dashed_cubic_paths(doc: &ProjectDocument, gs: &GraphSheet) -> Result<usize, String> {
    let scene = curvebook_chart::compile_graph(gs, doc).map_err(|e| format!("compile_graph `{}` failed: {e}", gs.id.as_str()))?;
    Ok(scene
        .primitives
        .iter()
        .filter(|p| {
            matches!(
                p,
                curvebook_chart::ScenePrimitive::Path { path, fill: Some(_), stroke: Some(st), .. }
                    if st.dash == curvebook_palette::LineDashStyle::Dashed
                        && path.commands.iter().any(|c| matches!(c, curvebook_chart::PathCmd::CubicTo { .. }))
            )
        })
        .count())
}

fn count_polygon_or_hull_paths(doc: &ProjectDocument, gs: &GraphSheet) -> Result<usize, String> {
    let scene = curvebook_chart::compile_graph(gs, doc).map_err(|e| format!("compile_graph `{}` failed: {e}", gs.id.as_str()))?;
    Ok(scene
        .primitives
        .iter()
        .filter(|p| {
            matches!(
                p,
                curvebook_chart::ScenePrimitive::Path { path, fill: Some(_), stroke: Some(st), .. }
                    if st.dash == curvebook_palette::LineDashStyle::Solid
                        && path.commands.iter().any(|c| matches!(c, curvebook_chart::PathCmd::LineTo(_)))
            )
        })
        .count())
}

#[test]
fn wf44_human_confidence_ellipses_and_convex_hulls() -> TestRes {
    let mut run = WorkflowRun::start("WF-44");
    let fixture = wf_datasets::clustering_three_clusters();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    let clust_rs_id = run.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    run.select_sheet(&clust_rs_id)?;
    run.click("toolbar.append_cluster")?;

    let gs_id = create_mv_graph_via_gui(&mut run, &ds_id)?;
    let n_cols = mv_var_count(run.document(), &ds_id)?;
    let init_enc = mv_encoding(run.document(), &gs_id)?.clone();

    run.select_sheet(&gs_id)?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    for _ in 0..cycle_clicks(init_enc.x_column, 4, n_cols) {
        run.click("format_graph.mv.x_next")?;
    }
    run.click("format_graph.mv.color_categorical")?;
    for _ in 0..cycle_clicks(4, 1, n_cols) {
        run.click("format_graph.mv.x_next")?;
    }
    for _ in 0..cycle_clicks(init_enc.y_column, 2, n_cols) {
        run.click("format_graph.mv.y_next")?;
    }
    run.click("edit.tab.format_graph.data_objects")?;
    run.click("format_graph.mv.confidence_ellipses")?;
    run.click("format_graph.mv.convex_hulls")?;
    run.commit_modal_ok()?;

    let gui_enc = mv_encoding(run.document(), &gs_id)?.clone();
    ensure(
        gui_enc.show_confidence_ellipses
            && gui_enc.show_convex_hulls
            && gui_enc.x_column == 1
            && gui_enc.y_column == 2
            && matches!(gui_enc.color, ColorChannelMapping::Categorical { column: 4, .. }),
        format!("WF-44 expected FeatureA/FeatureB/Color(Cluster) + ellipses & hulls, got {gui_enc:?}"),
    )?;
    let gs_ref = graph_sheet(run.document(), &gs_id)?;
    let _svg_both = compile_svg(run.document(), gs_ref)?;
    let initial_ellipses = count_dashed_cubic_paths(run.document(), gs_ref)?;
    let initial_hulls = count_polygon_or_hull_paths(run.document(), gs_ref)?;
    ensure(
        initial_ellipses == 3 && initial_hulls >= 3,
        format!("WF-44 expected 3 cluster ellipses & hulls, got ellipses={initial_ellipses}, hulls={initial_hulls}"),
    )?;

    let ref_sheet = fixture.to_data_sheet("ref-wf44-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let cluster_kind = AnalysisKind::Multivariate(MultivariateSpec::Clustering {
        distance: ClusterDistance::Euclidean,
        linkage: ClusterLinkage::Ward,
        k_means_k: Some(3),
        k_sweep_max: None,
        consensus_optimal_k: false,
        kmeans_algorithm: Default::default(),
        cluster_direction: Default::default(),
        cut_height: None,
        emit_anova_tab: false,
    });
    run.verify_reference(
        ReferenceSpec::for_graph(gs_id.clone(), |sess| {
            let rs_payload = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": cluster_kind }))?;
            let ref_rs_id = SheetId::new(payload_sheet_id(&rs_payload)?);
            oracle_append_cluster_column(sess, &ref_ds_id, &ref_rs_id)?;
            let g_payload =
                exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "multiple_variables", "initialized": true }))?;
            let ref_gid = payload_sheet_id(&g_payload)?;
            exec(sess, "graph.setSpec", json!({ "sheet_id": ref_gid, "kind": GraphKind::MultipleVariables, "multivariate": gui_enc }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .check_graph(|gui_gs, ref_gs| {
            ensure(
                gui_gs.spec.multivariate == ref_gs.spec.multivariate,
                format!("WF-44 oracle MV encoding mismatch: gui={:?} vs ref={:?}", gui_gs.spec.multivariate, ref_gs.spec.multivariate),
            )
        }),
    )?;

    // Exclude two Cluster-1 points via grid context menu -> Cluster 1 drops below N=3 guard, suppressing its ellipse.
    run.live_edit_all(
        LiveEditTarget::gui(move |run| {
            run.select_sheet(&ds_id)?;
            run.right_click("grid.cell.0.1.0")?;
            run.click("grid.ctx.exclude")?;
            run.right_click("grid.cell.1.1.0")?;
            run.click("grid.ctx.exclude")?;
            Ok(())
        }),
        vec![
            LiveEditExpectation::graph_svg_changed(gs_id.clone()),
            LiveEditExpectation::custom(move |_, after, doc| {
                let _svg_after = after.graph_svg(&gs_id).ok_or("WF-44 missing post-exclude SVG")?;
                let after_ellipses = count_dashed_cubic_paths(doc, graph_sheet(doc, &gs_id)?)?;
                ensure(
                    after_ellipses < initial_ellipses,
                    format!("WF-44 excluding 2 cluster points below N=3 must drop ellipse count ({initial_ellipses} -> {after_ellipses})"),
                )
            }),
        ],
    )?;

    run.finish()
}

#[test]
fn wf45_human_grouped_and_mv_cross_tab_heatmaps() -> TestRes {
    let mut run = WorkflowRun::start("WF-45");
    // Part A: Grouped Replicate-Summary Heatmap (`Viridis`, reversed, explicit bounds, GWF-21 annotations + cell_gap + rotated col labels)
    let grp_fx = wf_datasets::grouped_two_way();
    let mut grp_clicks = vec!["welcome.subcol.raw", "welcome.stepper.cols.dec"];
    grp_clicks.extend(std::iter::repeat_n("welcome.stepper.rows.dec", 12_usize.saturating_sub(grp_fx.row_titles.len())));
    let grp_ds_id = run.create_table_via_welcome("welcome.arch.grouped", grp_fx.title, &grp_clicks)?;
    for (r, rtitle) in grp_fx.row_titles.iter().enumerate() {
        run.enter_row_title(r, rtitle)?;
    }
    for (c, ctitle) in grp_fx.column_titles.iter().enumerate() {
        run.enter_column_title(c, ctitle)?;
    }
    for (r, row_groups) in grp_fx.values.iter().enumerate() {
        for (c, reps) in row_groups.iter().enumerate() {
            for (rep, &v) in reps.iter().enumerate() {
                run.enter_grid_cell(r, c, rep, &format!("{v}"))?;
            }
        }
    }

    run.select_sheet(&grp_ds_id)?;
    let before_grp_g = run.document().graph_sheets.len();
    run.right_click(&format!("nav.sheet.{}", grp_ds_id.as_str()))?;
    run.click("nav.ctx.new_graph")?;
    let grp_hm_id = run
        .document()
        .graph_sheets
        .iter()
        .skip(before_grp_g)
        .find(|g| g.spec.primary_source_sheet_id == grp_ds_id)
        .map(|g| g.id.clone())
        .ok_or("WF-45 missing Grouped graph")?;

    run.select_sheet(&grp_hm_id)?;
    run.click("toolbar.graph_type")?;
    run.click("graph_type.family.mv_diag")?;
    run.click("graph_type.show_all")?;
    let scroll_pos = run.widget_center("graph_type.show_all")? + Vec2::new(0.0, 100.0);
    run.wheel(scroll_pos, Vec2::new(0.0, -180.0))?;
    run.click("graph_type.card.diag.heatmap")?;
    run.commit_modal_ok()?;

    run.select_sheet(&grp_hm_id)?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.color_mapping")?;
    run.click("format_graph.heatmap.cmap.Viridis")?;
    run.click("format_graph.heatmap.reversed")?;
    run.click("format_graph.heatmap.auto_range")?;
    run.replace_textbox("format_graph.heatmap.min", "10")?;
    run.replace_textbox("format_graph.heatmap.max", "30")?;
    run.click("edit.tab.format_graph.heatmap_labels")?;
    run.click("format_graph.heatmap.annotate_cells")?;
    run.click("format_graph.heatmap.angle.45")?;
    run.click("edit.tab.format_graph.heatmap_borders")?;
    let gap_rect = run.frame_meta().find_by_widget_id("format_graph.heatmap.cell_gap").ok_or("WF-45 missing cell_gap")?.clipped_rect;
    run.drag(Pos2::new(gap_rect.min.x + 8.0, gap_rect.center().y), Pos2::new(gap_rect.min.x + 90.0, gap_rect.center().y))?;
    run.commit_modal_ok()?;

    let (gui_grp_enc, gui_grp_labels) = {
        let gs = graph_sheet(run.document(), &grp_hm_id)?;
        ensure(gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "WF-45 expected Heatmap GraphKind")?;
        let enc = gs.spec.multivariate.as_ref().ok_or("WF-45 missing persisted Grouped heatmap multivariate")?;
        ensure(
            matches!(
                enc.color,
                ColorChannelMapping::Continuous {
                    ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: true },
                    domain_clamp: Some((lo, hi)),
                    ..
                } if (lo - 10.0).abs() < 1e-6 && (hi - 30.0).abs() < 1e-6
            ),
            format!("WF-45 expected Viridis reversed [10,30], got {:?}", enc.color),
        )?;
        ensure(
            gs.spec.heatmap_labels.annotate_cells
                && gs.spec.heatmap_labels.cell_gap_pt > 0.0
                && gs.spec.heatmap_labels.column_angle == HeatmapColumnLabelAngle::Angled45,
            format!("WF-45 expected annotate_cells + cell_gap>0 + Angled45, got {:?}", gs.spec.heatmap_labels),
        )?;
        let svg = compile_svg(run.document(), gs)?;
        ensure(svg.contains("Vehicle") && svg.contains("0h"), "WF-45 Grouped heatmap SVG missing factor labels")?;
        (enc.clone(), gs.spec.heatmap_labels)
    };

    // Part B: MV CorrelationMatrix Spearman + Heatmap (`Cividis`) on `clustering_three_clusters()`
    let mv_fx = wf_datasets::clustering_three_clusters();
    let _mv_ds_id = enter_mv_fixture(&mut run, &mv_fx)?;
    let corr_rs_id = run.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    run.select_sheet(&corr_rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.corr.spearman")?;
    run.click("edit.corr.heatmap")?;
    run.commit_modal_ok()?;
    let mv_hm_id = find_linked_graph(run.document(), &[&corr_rs_id], Some(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)))?.id.clone();
    run.select_sheet(&mv_hm_id)?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.color_mapping")?;
    run.click("format_graph.heatmap.cmap.Cividis")?;
    run.commit_modal_ok()?;
    ensure(!compile_svg(run.document(), graph_sheet(run.document(), &mv_hm_id)?)?.is_empty(), "WF-45 MV heatmap SVG empty")?;

    let mut ref_grp = grp_fx.to_data_sheet("ref-grp-hm").map_err(|e| e.to_string())?;
    if let curvebook_doc::TableArchetype::Grouped(ref mut g) = ref_grp.archetype
        && let curvebook_doc::TableArchetype::Grouped(gui_g) = &run.document().data_sheet(&grp_ds_id).ok_or("missing gui grp")?.archetype
    {
        g.row_factor_title.clone_from(&gui_g.row_factor_title);
        g.column_factor_title.clone_from(&gui_g.column_factor_title);
    }
    let ref_grp_id = ref_grp.id.clone();
    run.verify_reference(
        ReferenceSpec::for_graph(grp_hm_id.clone(), |sess| {
            let g_out = exec(
                sess,
                "graph.create",
                json!({ "data_sheet_id": ref_grp_id.as_str(), "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "initialized": true }),
            )?;
            let ref_gid = payload_sheet_id(&g_out)?;
            exec(
                sess,
                "graph.setSpec",
                json!({
                    "sheet_id": ref_gid,
                    "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
                    "multivariate": gui_grp_enc,
                    "heatmap_labels": gui_grp_labels,
                }),
            )?;
            Ok(())
        })
        .with_reference_sheet(ref_grp)?
        .check_graph(|gui_gs, ref_gs| {
            ensure(
                gui_gs.spec.kind == ref_gs.spec.kind
                    && gui_gs.spec.multivariate == ref_gs.spec.multivariate
                    && gui_gs.spec.heatmap_labels == ref_gs.spec.heatmap_labels,
                "WF-45 Grouped heatmap spec mismatch against headless oracle",
            )
        }),
    )?;

    run.live_edit(LiveEditTarget::sheet_grid_cell(grp_ds_id, 0, 0, 0, "29.0"), LiveEditExpectation::graph_svg_changed(grp_hm_id))?;

    run.finish()
}

#[test]
fn wf46_human_interactive_mv_inspector_variable_swapping() -> TestRes {
    let mut run = WorkflowRun::start("WF-46");
    let fixture = wf_datasets::clustering_three_clusters();
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    let clust_rs_id = run.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    run.select_sheet(&clust_rs_id)?;
    run.click("toolbar.append_cluster")?;

    let gs_id = create_mv_graph_via_gui(&mut run, &ds_id)?;
    let n_cols = mv_var_count(run.document(), &ds_id)?;
    let enc0 = mv_encoding(run.document(), &gs_id)?.clone();

    // First interactive assignment: X=FeatureA(1), Y=FeatureB(2), Color=Cat(Cluster=4), Size=Variable(FeatureC=3)
    run.select_sheet(&gs_id)?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    for _ in 0..cycle_clicks(enc0.x_column, 4, n_cols) {
        run.click("format_graph.mv.x_next")?;
    }
    run.click("format_graph.mv.color_categorical")?;
    for _ in 0..cycle_clicks(4, 1, n_cols) {
        run.click("format_graph.mv.x_next")?;
    }
    for _ in 0..cycle_clicks(enc0.y_column, 3, n_cols) {
        run.click("format_graph.mv.y_next")?;
    }
    run.click("edit.tab.format_graph.data_objects")?;
    run.click("format_graph.mv.size_variable")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    for _ in 0..cycle_clicks(3, 2, n_cols) {
        run.click("format_graph.mv.y_next")?;
    }
    run.commit_modal_ok()?;

    let svg_before_swap = compile_svg(run.document(), graph_sheet(run.document(), &gs_id)?)?;
    let enc1 = mv_encoding(run.document(), &gs_id)?.clone();
    ensure(
        enc1.x_column == 1
            && enc1.y_column == 2
            && matches!(enc1.color, ColorChannelMapping::Categorical { column: 4, .. })
            && matches!(enc1.size, SizeChannelMapping::Variable { column: 3, .. }),
        format!("WF-46 expected initial X=1/Y=2/Color=Cat(4)/Size=Var(3), got {enc1:?}"),
    )?;

    // Duplicate the configured MV graph via Navigator context menu before performing live channel swap
    let before_dup_count = run.document().graph_sheets.len();
    run.right_click(&format!("nav.sheet.{}", gs_id.as_str()))?;
    run.click("nav.ctx.duplicate")?;
    ensure(run.document().graph_sheets.len() == before_dup_count + 1, "WF-46 Navigator duplicate must create a second GraphSheet")?;

    // Second swap on original MV graph: swap all 4 channels -> X=FeatureB(2), Y=FeatureC(3), Size=Var(FeatureA=1), Color=Cont(FeatureA=1)
    run.select_sheet(&gs_id)?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    for _ in 0..cycle_clicks(1, 2, n_cols) {
        run.click("format_graph.mv.x_next")?;
    }
    for _ in 0..cycle_clicks(2, 1, n_cols) {
        run.click("format_graph.mv.y_next")?;
    }
    run.click("format_graph.mv.color_continuous")?;
    run.click("edit.tab.format_graph.data_objects")?;
    run.click("format_graph.mv.size_variable")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    for _ in 0..cycle_clicks(1, 3, n_cols) {
        run.click("format_graph.mv.y_next")?;
    }
    run.commit_modal_ok()?;

    let enc2 = mv_encoding(run.document(), &gs_id)?.clone();
    let svg_after_swap = compile_svg(run.document(), graph_sheet(run.document(), &gs_id)?)?;
    ensure(
        enc2.x_column == 2
            && enc2.y_column == 3
            && matches!(enc2.color, ColorChannelMapping::Continuous { column: 1, .. })
            && matches!(enc2.size, SizeChannelMapping::Variable { column: 1, .. })
            && svg_after_swap != svg_before_swap,
        format!("WF-46 expected swapped X=2/Y=3/Color=Cont(1)/Size=Var(1) & SVG change, got {enc2:?}"),
    )?;

    let ref_sheet = fixture.to_data_sheet("ref-wf46-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let cluster_kind = AnalysisKind::Multivariate(MultivariateSpec::Clustering {
        distance: ClusterDistance::Euclidean,
        linkage: ClusterLinkage::Ward,
        k_means_k: Some(3),
        k_sweep_max: None,
        consensus_optimal_k: false,
        kmeans_algorithm: Default::default(),
        cluster_direction: Default::default(),
        cut_height: None,
        emit_anova_tab: false,
    });
    run.verify_reference(
        ReferenceSpec::for_graph(gs_id.clone(), |sess| {
            let rs_payload = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": cluster_kind }))?;
            let ref_rs_id = SheetId::new(payload_sheet_id(&rs_payload)?);
            oracle_append_cluster_column(sess, &ref_ds_id, &ref_rs_id)?;
            let g_payload =
                exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "multiple_variables", "initialized": true }))?;
            let ref_gid = payload_sheet_id(&g_payload)?;
            exec(sess, "graph.setSpec", json!({ "sheet_id": ref_gid, "kind": GraphKind::MultipleVariables, "multivariate": enc2 }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.multivariate == ref_gs.spec.multivariate, "WF-46 swapped MV GraphSpec mismatch against headless oracle")
        }),
    )?;

    run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 0, 2, 0, "6.5"), LiveEditExpectation::graph_svg_changed(gs_id))?;

    run.finish()
}

#[test]
fn wf47_human_kaplan_meier_number_at_risk_and_pairwise() -> TestRes {
    let mut run = WorkflowRun::start("WF-47");
    let fixture = wf_datasets::survival_three_group();
    let mut extra_clicks = vec!["welcome.stepper.cols.inc"; fixture.cohort_titles.len().saturating_sub(3)];
    extra_clicks.extend(std::iter::repeat_n("welcome.stepper.rows.inc", fixture.rows.len().saturating_sub(12)));
    let ds_id = run.create_table_via_welcome("welcome.arch.survival", fixture.title, &extra_clicks)?;
    for (c, title) in fixture.cohort_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (r, (time, cohorts)) in fixture.rows.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{time}"))?;
        for (c, status) in cohorts.iter().enumerate() {
            if let Some(code) = status {
                run.enter_grid_cell(r, c, 0, &format!("{code}"))?;
            }
        }
    }

    let rs_id = run.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.km.trend")?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.holm_sidak")?;
    run.commit_modal_ok()?;

    let gs_id = find_linked_graph(run.document(), &[&rs_id, &ds_id], Some(GraphKind::Survival))?.id.clone();
    run.select_sheet(&gs_id)?;
    run.click("toolbar.axes")?;
    run.click("edit.tab.axes.x")?;
    run.replace_textbox("axes.seg.0.major_tick_interval", "10")?;
    run.click("edit.tab.axes.y")?;
    run.click("axes.additional_ticks.open")?;
    run.replace_textbox("axes.add_tick.value", "0.5")?;
    run.replace_textbox("axes.add_tick.label", "Median 50%")?;
    run.click("axes.add_tick.gridline")?;
    run.click("axes.add_tick.save")?;
    run.click("axes.add_tick.close")?;
    run.commit_modal_ok()?;

    let km_svg = compile_svg(run.document(), graph_sheet(run.document(), &gs_id)?)?;
    ensure(
        graph_sheet(run.document(), &gs_id)?.spec.survival_number_at_risk && km_svg.contains("Median 50%"),
        "WF-47 expected survival number-at-risk enabled and Median 50% line rendered",
    )?;
    ensure(run.frame_meta().find_by_widget_id("canvas.km_number_at_risk").is_some(), "WF-47 canvas.km_number_at_risk strip missing")?;

    let summary_tab = tab_index_by_id(run.document(), &rs_id, "km-summary")?;
    let curve_cmp_tab = tab_index_by_id(run.document(), &rs_id, "curve-comparison")?;
    let pairwise_tab = tab_index_by_id(run.document(), &rs_id, "survival-pairwise")?;

    let mut ref_sheet = fixture.to_data_sheet("ref-km-ds").map_err(|e| e.to_string())?;
    if let curvebook_doc::TableArchetype::Survival(ref mut s) = ref_sheet.archetype {
        s.time_title = match &run.document().data_sheet(&ds_id).ok_or("missing gui surv ds")?.archetype {
            curvebook_doc::TableArchetype::Survival(gui_s) => gui_s.time_title.clone(),
            _ => s.time_title.clone(),
        };
    }
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: false,
        trend_test: true,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::HolmSidak, ..MultiplicitySpec::default() },
        asymmetrical_greenwood_ci: true,
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
        event_code: None,
        censored_code: None,
        treat_other_as: Default::default(),
        ratio_median_ci: false,
        mv_status_columns: Vec::new(),
        mv_group_columns: Vec::new(),
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "survival", "initialized": true }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(gs_id.clone())
        .scalar("Log-rank chi-square", None, 1e-8, 1e-8)
        .scalar("Log-rank P value", None, 1e-8, 1e-6)
        .tab_cell(summary_tab, 0, 2, 1e-8, 1e-8)
        .tab_cell(curve_cmp_tab, 1, 0, 1e-8, 1e-8)
        .tab_cell(curve_cmp_tab, 1, 2, 1e-8, 1e-6)
        .tab_cell(pairwise_tab, 0, 2, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| {
            ensure(
                gui_gs.spec.kind == GraphKind::Survival && ref_gs.spec.kind == GraphKind::Survival && gui_gs.spec.survival_number_at_risk,
                "WF-47 KM graph spec mismatch",
            )
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_x_cell(ds_id, 0, 0, "22.0"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "Log-rank P value", None, 1e-5),
            LiveEditExpectation::tab_cell_changed(rs_id, pairwise_tab, 0, 2),
            LiveEditExpectation::graph_svg_changed(gs_id),
        ],
    )?;

    run.finish()
}

#[test]
fn wf48_human_mv_bubble_sizing_and_outlier_clamping() -> TestRes {
    let mut run = WorkflowRun::start("WF-48");
    let fixture = wf_datasets::MvFixture {
        title: "MV Bubble Outlier Clamping",
        columns: vec![
            wf_datasets::MvColumnSpec { title: "Dose", role: curvebook_doc::VariableRole::Continuous, role_slug: "continuous" },
            wf_datasets::MvColumnSpec { title: "Response", role: curvebook_doc::VariableRole::Continuous, role_slug: "continuous" },
            wf_datasets::MvColumnSpec { title: "SizeMetric", role: curvebook_doc::VariableRole::Continuous, role_slug: "continuous" },
            wf_datasets::MvColumnSpec { title: "Intensity", role: curvebook_doc::VariableRole::Continuous, role_slug: "continuous" },
        ],
        rows: [
            [1.0, 10.0, 2.0, 4.0],
            [2.0, 18.0, 4.0, 8.0],
            [3.0, 27.0, 6.0, 14.0],
            [4.0, 36.0, 8.0, 19.0],
            [5.0, 44.0, 10.0, 24.0],
            [6.0, 52.0, 40.0, 45.0],
        ]
        .into_iter()
        .map(|row| row.into_iter().map(wf_datasets::MvCellValue::Number).collect())
        .collect(),
    };
    let ds_id = enter_mv_fixture(&mut run, &fixture)?;
    let gs_id = create_mv_graph_via_gui(&mut run, &ds_id)?;

    // Part A: Configure continuous color + variable bubble size & radii in BubblePlotInspector
    run.select_sheet(&gs_id)?;
    run.click("inspector.graph.more_bubble")?;
    run.click("bubble.color_col_next")?;
    run.click("bubble.size_variable")?;
    run.click("bubble.size_col_next")?;
    let min_rect = run.frame_meta().find_by_widget_id("bubble.min_radius").ok_or("WF-48 missing bubble.min_radius")?.clipped_rect;
    run.drag(Pos2::new(min_rect.min.x + 8.0, min_rect.center().y), Pos2::new(min_rect.min.x + 56.0, min_rect.center().y))?;
    let max_rect = run.frame_meta().find_by_widget_id("bubble.max_radius").ok_or("WF-48 missing bubble.max_radius")?.clipped_rect;
    run.drag(Pos2::new(max_rect.min.x + 8.0, max_rect.center().y), Pos2::new(max_rect.min.x + 56.0, max_rect.center().y))?;
    run.commit_modal_ok()?;

    // Part B: FormatGraph MV channel cycling + oor_omit vs oor_clamp domain clamping [2.0, 20.0]
    run.select_sheet(&gs_id)?;
    run.click("toolbar.format_graph")?;
    run.click("edit.tab.format_graph.axis_variables")?;
    run.click("format_graph.mv.color_continuous")?; // col 1
    run.click("format_graph.mv.color_col_next")?; // 1 -> 2
    run.click("format_graph.mv.color_col_next")?; // 2 -> 3 (Intensity)
    run.click("edit.tab.format_graph.data_objects")?;
    run.click("format_graph.mv.size_col_next")?; // 1 -> 2 (SizeMetric)
    run.replace_textbox("format_graph.mv.color_clamp_min", "0")?;
    run.replace_textbox("format_graph.mv.color_clamp_max", "30")?;
    run.click("format_graph.mv.oor_omit")?;
    ensure(
        run.state().views.graph_modals.mv_clamp_min_draft.is_empty() && run.state().views.graph_modals.mv_clamp_max_draft.is_empty(),
        "WF-48 oor_omit must clear color clamp bounds",
    )?;
    run.replace_textbox("format_graph.mv.color_clamp_min", "2")?;
    run.replace_textbox("format_graph.mv.color_clamp_max", "20")?;
    run.click("format_graph.mv.oor_clamp")?;
    run.commit_modal_ok()?;

    let gui_enc = mv_encoding(run.document(), &gs_id)?.clone();
    ensure(
        matches!(
            gui_enc.color,
            ColorChannelMapping::Continuous { column: 3, domain_clamp: Some((lo, hi)), .. }
            if (lo - 2.0).abs() < 1e-6 && (hi - 20.0).abs() < 1e-6
        ) && matches!(gui_enc.size, SizeChannelMapping::Variable { column: 2, .. }),
        format!("WF-48 expected Continuous(col=3, clamp=(2,20)) & Size Variable(col=2), got {gui_enc:?}"),
    )?;
    let before_pts =
        curvebook_chart::compile_graph(graph_sheet(run.document(), &gs_id)?, run.document()).map_err(|e| e.to_string())?.plotted_points.len();
    ensure(before_pts == 6, format!("WF-48 expected 6 plotted bubbles before outlier exclude, got {before_pts}"))?;

    let ref_sheet = fixture.to_data_sheet("ref-wf48-ds").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    run.verify_reference(
        ReferenceSpec::for_graph(gs_id.clone(), |sess| {
            let g_out =
                exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "multiple_variables", "initialized": true }))?;
            let ref_gid = payload_sheet_id(&g_out)?;
            exec(sess, "graph.setSpec", json!({ "sheet_id": ref_gid, "kind": GraphKind::MultipleVariables, "multivariate": gui_enc }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .check_graph(|gui_gs, ref_gs| {
            ensure(gui_gs.spec.multivariate == ref_gs.spec.multivariate, "WF-48 bubble spec mismatch against headless oracle")
        }),
    )?;

    // Live exclude the extreme sizing/intensity outlier (row 5) via grid context menu
    run.live_edit_all(
        LiveEditTarget::gui(move |run| {
            run.select_sheet(&ds_id)?;
            run.right_click("grid.cell.5.1.0")?;
            run.click("grid.ctx.exclude")?;
            run.right_click("grid.cell.5.2.0")?;
            run.click("grid.ctx.exclude")?;
            Ok(())
        }),
        vec![
            LiveEditExpectation::graph_svg_changed(gs_id.clone()),
            LiveEditExpectation::custom(move |_, _, doc| {
                let after_pts = curvebook_chart::compile_graph(graph_sheet(doc, &gs_id)?, doc).map_err(|e| e.to_string())?.plotted_points.len();
                ensure(after_pts == 5, format!("WF-48 excluding outlier row 5 must leave 5 plotted bubbles, got {after_pts}"))
            }),
        ],
    )?;

    run.finish()
}

#[test]
fn wf49_human_live_tex_model_equation_annotations() -> TestRes {
    let mut run = WorkflowRun::start("WF-49");
    let ds_id =
        run.create_table_via_welcome("welcome.arch.xy", "One-Phase Decay Live TeX", &["welcome.stepper.cols.dec", "welcome.stepper.cols.dec"])?;
    let points = [(0.0, 100.0), (1.0, 62.5), (2.0, 40.2), (4.0, 20.1), (8.0, 11.2), (12.0, 9.8), (24.0, 9.5)];
    for (r, &(x, y)) in points.iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }

    let rs_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.model")?;
    run.replace_textbox("fit.eq.search", "one_phase_decay")?;
    run.click("fit.eq.item.one_phase_decay")?;
    run.click("fit.tab.output")?;
    run.click("fit.out.pred_bands")?;
    run.commit_modal_ok()?;

    let gs_id = find_linked_graph(run.document(), &[&ds_id, &rs_id], Some(GraphKind::Xy))?.id.clone();
    run.select_sheet(&gs_id)?;
    run.click("inspector.graph.tab.annotations")?;
    run.click("inspector.graph.add_equation")?;
    run.click("eq.mode.substituted")?;
    run.click("eq.bordered")?;
    run.commit_modal_ok()?;

    let gui_ann = graph_sheet(run.document(), &gs_id)?
        .spec
        .annotations
        .iter()
        .find(|a| matches!(a, GraphAnnotation::ModelEquation { .. }))
        .cloned()
        .ok_or("WF-49 missing ModelEquation annotation")?;
    ensure(
        matches!(&gui_ann, GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Substituted, bordered: true, .. }),
        format!("WF-49 expected bordered Substituted ModelEquation, got {gui_ann:?}"),
    )?;
    let svg_before_eq = compile_svg(run.document(), graph_sheet(run.document(), &gs_id)?)?;
    ensure(
        !svg_before_eq.contains("R²") && !svg_before_eq.contains("R^2") && !svg_before_eq.contains("Span"),
        "WF-49 R9.16 ModelEquation SVG must contain formulas only without R2/Span prose",
    )?;
    let k_before = result_sheet(run.document(), &rs_id)?.lookup_scalar("K", Some(0)).ok_or("WF-49 missing fitted scalar K")?;

    let param_tab = tab_index_by_id(run.document(), &rs_id, "nonlinear_parameters")?;

    let mut ref_xy = curvebook_doc::XyTable::new(curvebook_doc::SubcolumnMode::RawReplicates(1), 1, points.len()).map_err(|e| e.to_string())?;
    for (r, &(x, y)) in points.iter().enumerate() {
        ref_xy.x_cells[r][0] = curvebook_doc::Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = curvebook_doc::Cell::number(y);
    }
    let gui_ds = run.document().data_sheet(&ds_id).ok_or("WF-49 missing GUI XY table")?;
    if let curvebook_doc::TableArchetype::Xy(gxy) = &gui_ds.archetype {
        ref_xy.x_title.clone_from(&gxy.x_title);
        ref_xy.data_sets[0].title.clone_from(&gxy.data_sets[0].title);
    }
    let ref_sheet =
        DataSheet::new("ref-decay-xy", "One-Phase Decay Live TeX", curvebook_doc::TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::NonlinearRegression(NonlinearFitSpec { prediction_bands: true, ..NonlinearFitSpec::new("one_phase_decay") });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            let fit_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = SheetId::new(payload_sheet_id(&fit_out)?);
            let g_out = exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "xy", "initialized": true }))?;
            let ref_gid = payload_sheet_id(&g_out)?;
            let mut ref_ann = gui_ann.clone();
            if let GraphAnnotation::ModelEquation { result_sheet_id, .. } = &mut ref_ann {
                *result_sheet_id = ref_rs_id;
            }
            exec(sess, "graph.addAnnotation", json!({ "sheet_id": ref_gid, "annotation": ref_ann }))?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(gs_id.clone())
        .scalar("Y0", Some(0), 1e-6, 1e-6)
        .scalar("Plateau", Some(0), 1e-6, 1e-6)
        .scalar("K", Some(0), 1e-6, 1e-6)
        .scalar("HalfLife", Some(0), 1e-6, 1e-6)
        .scalar("R2", None, 1e-8, 1e-8)
        .tab_cell(param_tab, 0, 0, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| {
            ensure(
                gui_gs.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::ModelEquation { .. }))
                    && ref_gs.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::ModelEquation { .. })),
                "WF-49 ModelEquation annotation mismatch against headless oracle",
            )
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 1, 0, 0, "82.0"),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "K", Some(0), 1e-3),
            LiveEditExpectation::tab_cell_changed(rs_id.clone(), param_tab, 2, 0),
            LiveEditExpectation::graph_svg_changed(gs_id.clone()),
            LiveEditExpectation::custom(move |before, after, _| {
                let k_after = after.scalar(&rs_id, "K", Some(0)).ok_or("missing K after live edit")?;
                let svg_before = before.graph_svg(&gs_id).unwrap_or_default();
                let svg_after = after.graph_svg(&gs_id).unwrap_or_default();
                ensure(
                    (k_after - k_before).abs() > 1e-3 && svg_after != svg_before,
                    format!("WF-49 live equation SVG must update with K ({k_before} -> {k_after})"),
                )
            }),
        ],
    )?;

    run.finish()
}

#[test]
fn wf50_human_compact_letter_display_piepho_svg_pdf_rgba() -> TestRes {
    let mut run = WorkflowRun::start("WF-50");
    let fixture = wf_datasets::column_three_group();
    let ds_id = run.create_table_via_welcome("welcome.arch.column", fixture.title, &[])?;
    for (c, title) in fixture.group_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
    }
    for (c, col_vals) in fixture.columns.iter().enumerate() {
        for (r, &v) in col_vals.iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }

    let rs_id = run.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    run.select_sheet(&rs_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.tukey")?;
    run.commit_modal_ok()?;

    let gs_id = find_linked_graph(run.document(), &[&ds_id, &rs_id], Some(GraphKind::Column))?.id.clone();
    run.select_sheet(&gs_id)?;
    run.click("inspector.graph.tab.legends")?;
    run.click("inspector.graph.more_pairwise")?;
    run.click("pairwise.mode.CompactLetterDisplayOnly")?;
    run.click("pairwise.cld_uppercase")?;
    run.commit_modal_ok()?;

    let svg_before_edit = compile_svg(run.document(), graph_sheet(run.document(), &gs_id)?)?;
    {
        let gs = graph_sheet(run.document(), &gs_id)?;
        ensure(gs.spec.kind == GraphKind::Column, "WF-50 expected GraphKind::Column")?;
        ensure(
            gs.spec.pairwise_display_mode == PairwiseDisplayMode::CompactLetterDisplayOnly
                && gs.spec.compact_letter_display.as_ref().is_some_and(|c| c.uppercase),
            "WF-50 expected uppercase CompactLetterDisplayOnly",
        )?;
        ensure(svg_before_edit.contains(">A<") && svg_before_edit.contains(">B<"), "WF-50 expected uppercase Piepho CLD letters in SVG")?;
    }

    let anova_tab = tab_index_by_id(run.document(), &rs_id, "anova-table")?;
    let mult_tab = tab_index_by_id(run.document(), &rs_id, "multiple-comparisons")?;

    let ref_sheet = fixture.to_data_sheet("ref-col-wf50").map_err(|e| e.to_string())?;
    let ref_ds_id = ref_sheet.id.clone();
    let oracle_kind = AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
        ..OneWayAnovaSpec::default()
    });
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), |sess| {
            let anova_out = exec(sess, "analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": oracle_kind }))?;
            let ref_rs_id = payload_sheet_id(&anova_out)?;
            let g_out = exec(sess, "graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": "column", "initialized": true }))?;
            let ref_gid = payload_sheet_id(&g_out)?;
            exec(
                sess,
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": ref_gid,
                    "result_sheet_id": ref_rs_id,
                    "pairwise_display_mode": "compact_letter_display_only",
                    "uppercase": true
                }),
            )?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .with_graph(gs_id.clone())
        .scalar("F", None, 1e-8, 1e-8)
        .scalar("P value", None, 1e-8, 1e-6)
        .tab_cell(anova_tab, 0, 3, 1e-8, 1e-8)
        .tab_cell(mult_tab, 0, 2, 1e-6, 1e-6)
        .check_graph(|gui_gs, ref_gs| {
            ensure(
                gui_gs.spec.pairwise_display_mode == ref_gs.spec.pairwise_display_mode
                    && gui_gs.spec.compact_letter_display.as_ref().map(|c| c.uppercase)
                        == ref_gs.spec.compact_letter_display.as_ref().map(|c| c.uppercase),
                "WF-50 Piepho CLD GraphSpec mismatch against headless oracle",
            )
        }),
    )?;

    // Shift Baseline group well above Compound_High so CLD letter order flips dynamically
    run.live_edit_all(
        LiveEditTarget::gui(move |run| {
            run.select_sheet(&ds_id)?;
            for r in 0..6 {
                run.enter_grid_cell(r, 0, 0, &format!("{:.1}", 36.0 + r as f64))?;
            }
            Ok(())
        }),
        vec![
            LiveEditExpectation::scalar_changed(rs_id.clone(), "F", None, 0.5),
            LiveEditExpectation::tab_cell_changed(rs_id, mult_tab, 0, 2),
            LiveEditExpectation::graph_svg_changed(gs_id.clone()),
            LiveEditExpectation::custom(move |_, after, _| {
                let svg_after = after.graph_svg(&gs_id).ok_or("WF-50 missing post-edit SVG")?;
                ensure(svg_after != svg_before_edit, "WF-50 expected CLD SVG letters to shift after live edit")
            }),
        ],
    )?;

    let png = run.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-50 canvas PNG")?;
    run.finish()
}
