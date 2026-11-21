#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-25`–`WF-31`, `GWF-11` / `R10.28`).

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, DiagnosticPlotKind, ErrorBarDirection, ErrorBarMetric, FigureCanvasPreset, FramePreset, GraphKind, GraphSheet, GraphicalDataObject,
    LayoutItem, MathTransformPreset, MultivariateSpec, PValueFormat, ProjectDocument, ResultSheet, SheetId, TableKind, TransformAnalysisSpec,
};
use curvebook_palette::PaletteId;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{CanvasPreviewFilter, UiThemeMode};
use egui::Pos2;
use serde_json::json;

use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use human_wf_common::{EguiWfHarness, TestRes, ensure};

fn find_graph_for_source(doc: &ProjectDocument, source_id: &SheetId, predicate: impl Fn(&GraphSheet) -> bool) -> Result<SheetId, String> {
    doc.graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id == *source_id && predicate(g))
        .map(|g| g.id.clone())
        .ok_or_else(|| format!("no matching GraphSheet bound to `{source_id}`"))
}

fn find_col_in_tab(rs: &ResultSheet, tab_id: &str, col_name: &str) -> Result<(usize, usize), String> {
    let tab_idx = rs.tabs.iter().position(|t| t.id == tab_id).ok_or_else(|| format!("missing tab `{tab_id}` in `{}`", rs.id))?;
    let col_idx =
        rs.tabs[tab_idx].column_headers.iter().position(|h| h == col_name).ok_or_else(|| format!("missing col `{col_name}` in `{tab_id}`"))?;
    Ok((tab_idx, col_idx))
}

fn find_row_col_in_tab(rs: &ResultSheet, tab_id: &str, row_label: &str, col_name: &str) -> Result<(usize, usize, usize), String> {
    let (tab_idx, col_idx) = find_col_in_tab(rs, tab_id, col_name)?;
    let row_idx =
        rs.tabs[tab_idx].row_labels.iter().position(|r| r == row_label).ok_or_else(|| format!("missing row `{row_label}` in `{tab_id}`"))?;
    Ok((tab_idx, row_idx, col_idx))
}

fn create_mv_table_via_welcome(run: &mut WorkflowRun, title: &str, roles: &[&str], col_titles: &[&str], tsv: &str) -> Result<SheetId, String> {
    run.click("welcome.arch.multiple_variables")?;
    let cols = roles.len();
    for _ in 3..cols {
        run.click("welcome.stepper.cols.inc")?;
    }
    let ds_id = run.create_table_via_welcome("welcome.arch.multiple_variables", title, &[])?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard(tsv)?;
    run.click("mv_formula.role_badge.0")?;
    run.click("mv_formula.role_manage.0")?;
    for (col_idx, &title_text) in col_titles.iter().enumerate() {
        run.click(&format!("mv_formula.vars.col.{col_idx}"))?;
        run.replace_textbox("mv_formula.vars.title_input", title_text)?;
        run.click("mv_formula.vars.meta.apply")?;
        run.click(&format!("mv_formula.vars.role.{}", roles[col_idx]))?;
    }
    run.click("mv_formula.vars.close")?;
    Ok(ds_id)
}

fn apply_mv_formula(run: &mut WorkflowRun, col_idx: usize, formula: &str) -> TestRes {
    run.click(&format!("inspector.mv.col.{col_idx}"))?;
    run.replace_textbox("inspector.mv.formula_input", formula)?;
    let can_apply = run.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing inspector.mv.apply_formula")?.enabled;
    ensure(can_apply, format!("expected valid MV formula `{formula}` to enable inspector.mv.apply_formula"))?;
    run.click("inspector.mv.apply_formula")
}

// ============================================================================
// GWF-11: Pure GUI MV / Layout / 14-Locale Gate
// ============================================================================

#[test]
fn r15_4_phase_a_mv_layout_and_14_locale_gate() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    h.assert_parity_and_layout("Phase A MV/Layout initial Welcome")?;

    h.click("welcome.arch.multiple_variables")?;
    h.click("welcome.stepper.cols.inc")?;
    let ds_id = h.create_table_via_welcome("welcome.arch.multiple_variables", "Phase A MV Cohort", &[])?;
    for (r, row) in
        [["S1", "Ctrl", "10", "20"], ["S2", "Ctrl", "12", "22"], ["S3", "Drug", "18", "31"], ["S4", "Drug", "20", "35"]].iter().enumerate()
    {
        for (c, v) in row.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, v)?;
        }
    }
    h.set_mv_role(0, "label")?;
    h.set_mv_role(1, "categorical")?;
    h.set_mv_role(2, "continuous")?;
    h.set_mv_role(3, "continuous")?;

    h.click("menu.insert")?;
    h.click("menu.layout.create")?;
    let ls_id = h.app().document().layout_sheets.last().map(|l| l.id.clone()).ok_or("expected LayoutSheet via Insert > New Layout")?;
    h.select_sheet(&ls_id)?;
    h.click("studio.layout.add_panel.0")?;
    h.click("inspector.layout.badge.uppercase")?;
    h.click("inspector.layout.align_origins")?;
    ensure(h.app().document().layout_sheet(&ls_id).is_some_and(|l| !l.spec.items.is_empty()), "expected LayoutSheet to contain added graph panel")?;

    let shot = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(shot.starts_with(b"\x89PNG\r\n\x1a\n") && shot.len() > 256, "invalid Phase A MV/Layout PNG screenshot")?;
    h.assert_parity_and_layout("Phase A MV/Layout 1280x800")?;
    h.driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    h.assert_parity_and_layout("Phase A MV/Layout 1440x600")?;
    h.select_sheet(&ds_id)?;
    h.verify_14_locales_section_9_4("Phase A MV/Layout")?;
    Ok(())
}

// ============================================================================
// WF-25: Column Plots (Box-and-Whisker, Violin/Raincloud, Bar+Replicates, Error Bars)
// ============================================================================

#[test]
fn wf25_human_violin_boxplot_bars_and_error_bars() -> TestRes {
    let mut run = WorkflowRun::start("WF-25");
    let ds_id = run.create_table_via_welcome("welcome.arch.column", "Biomarker Treatments", &[])?;
    let tsv = "\
10.1\t15.2\t22.0
11.4\t17.8\t25.5
9.8\t14.9\t20.4
12.2\t19.1\t28.1
10.9\t16.4\t24.0
13.0\t20.5\t30.2
9.5\t13.8\t19.7
11.7\t18.0\t26.8";
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard(tsv)?;
    for (c, t) in ["Vehicle", "LowDose", "HighDose"].iter().enumerate() {
        run.enter_column_title(c, t)?;
    }

    let stats_id = run.run_analyze_entry("descriptive", "analyze.entry.stats.descriptive")?;
    let graph_id = find_graph_for_source(run.document(), &ds_id, |_| true)?;
    run.select_sheet(&graph_id)?;

    // 1. Box-and-Whisker plot via Change Graph Type.
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.box_whisker")?;
    run.commit_modal_ok()?;
    ensure(
        run.document()
            .graph_sheet(&graph_id)
            .is_some_and(|g| matches!(g.spec.series.first().map(|s| &s.gdo), Some(GraphicalDataObject::BoxAndWhisker { .. }))),
        "expected BoxAndWhisker GDO",
    )?;
    let box_shot = run.screenshot_png(ScreenshotRegionTarget::Canvas)?;

    // 2. Violin / Half-Violin (Raincloud) plot via Change Graph Type + Format Graph.
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.violin")?;
    run.commit_modal_ok()?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.format.open")?;
    run.click("format_graph.apply_globally")?;
    run.click("format_graph.half_violin_with_points")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().graph_sheet(&graph_id).is_some_and(|g| {
            matches!(g.spec.series.first().map(|s| &s.gdo), Some(GraphicalDataObject::Violin { half_violin_with_points: true, .. }))
        }),
        "expected Violin(half_violin_with_points=true) GDO",
    )?;
    let violin_shot = run.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(violin_shot != box_shot, "expected Violin canvas PNG to differ from BoxAndWhisker PNG")?;

    // 3. Switch to Bar with Replicates + non-default SEM UpperOnly error bars + scatter overlay.
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.bar_replicates")?;
    run.click("graph_type.eb.sem")?;
    run.commit_modal_ok()?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.format.open")?;
    run.click("format_graph.apply_globally")?;
    run.click("format_graph.error_bar_metric.sem")?;
    run.click("format_graph.error_bar_dir.upper")?;
    run.commit_modal_ok()?;

    let ref_ds = run.document().data_sheet(&ds_id).ok_or("missing ds")?.clone();
    let ref_ds_id = ref_ds.id.clone();
    let (tab_idx, mean_row, col0) = {
        let rs = run.document().result_sheet(&stats_id).ok_or("missing stats result")?;
        find_row_col_in_tab(rs, "desc-summary", "Mean", "Vehicle")?
    };

    run.verify_reference(
        ReferenceSpec::for_result(stats_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": "descriptive_stats" })).map_err(|e| e.to_string())?;
            let g_val = serde_json::to_value(GraphKind::Column).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": g_val })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_ds)?
        .with_graph(graph_id.clone())
        .tab_cell(tab_idx, mean_row, col0, 1e-6, 1e-6)
        .check_graph(|gui_gs, _ref_gs| {
            let s0 = gui_gs.spec.series.first().ok_or("missing series 0")?;
            ensure(
                matches!(s0.gdo, GraphicalDataObject::BarWithReplicates { replicate_overlay: Some(_), .. }),
                format!("expected BarWithReplicates with replicate overlay, got {:?}", s0.gdo),
            )?;
            let eb = s0.y_error_bars.ok_or("expected y_error_bars on BarWithReplicates")?;
            ensure(
                eb.metric == ErrorBarMetric::Sem && eb.direction == ErrorBarDirection::UpperOnly,
                format!("expected non-default SEM UpperOnly error bars, got {eb:?}"),
            )?;
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 0, 0, 0, "45.0"),
        vec![LiveEditExpectation::tab_cell_changed(stats_id, tab_idx, mean_row, col0), LiveEditExpectation::graph_svg_changed(graph_id)],
    )?;
    run.finish()
}

// ============================================================================
// WF-26: Correlation Heatmap, Bubble Plot, Volcano Plot, and R7-R10 Predicates
// ============================================================================

#[test]
fn wf26_human_bubble_volcano_heatmap_and_r7_to_r10() -> TestRes {
    let mut run = WorkflowRun::start("WF-26");
    let mv_tsv = "\
-2.4\t1.1\t15.0\tCtrl\tS1
-1.8\t0.6\t18.0\tCtrl\tS2
-2.1\t1.4\t22.0\tCtrl\tS3
-1.5\t0.9\t19.0\tCtrl\tS4
2.2\t-1.2\t64.0\tTreat\tS5
2.7\t-0.8\t72.0\tTreat\tS6
1.9\t-1.5\t58.0\tTreat\tS7
2.5\t-1.0\t69.0\tTreat\tS8";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Multivariate Cohort",
        &["continuous", "continuous", "continuous", "categorical", "label"],
        &["PC1", "PC2", "Biomarker", "Cluster", "Sample"],
        mv_tsv,
    )?;

    // 1. Correlation Matrix + R7 (`all_pairs` controls `correlation.heatmap` enablement) + Heatmap graph.
    let corr_id = run.run_analyze_entry("correl", "analyze.entry.stats.corr_matrix")?;
    run.open_cmd_t()?;
    run.click("edit.corr.all_pairs")?;
    let hm_disabled = !run.frame_meta().find_by_widget_id("edit.corr.heatmap").ok_or("missing edit.corr.heatmap")?.enabled;
    ensure(hm_disabled, "R7: edit.corr.heatmap must disable when all_pairs is false")?;
    run.click("edit.corr.all_pairs")?;
    let hm_enabled = run.frame_meta().find_by_widget_id("edit.corr.heatmap").ok_or("missing edit.corr.heatmap")?.enabled;
    ensure(hm_enabled, "R7: edit.corr.heatmap must re-enable when all_pairs is true")?;
    run.click("edit.corr.heatmap")?;
    run.commit_modal_ok()?;

    let corr_hm_id = find_graph_for_source(run.document(), &corr_id, |g| g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))?;
    run.select_sheet(&corr_hm_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.format.open")?;
    run.click("format_graph.heatmap.cmap.Plasma")?;
    run.click("edit.tab.format_graph.heatmap_labels")?;
    run.click("format_graph.heatmap.annotate_cells")?;
    run.commit_modal_ok()?;

    // 2. R10 PCR leakage guard: selecting predictor 0 disables it in outcome selector while column 2 remains selectable.
    run.select_sheet(&mv_ds_id)?;
    run.click("toolbar.analyze")?;
    run.replace_textbox("analyze.search", "pcr")?;
    run.click("analyze.entry.stats.pcr")?;
    run.click("analyze.pca.pred.0")?;
    let out0_enabled = run.frame_meta().find_by_widget_id("analyze.pcr.outcome.0").ok_or("missing analyze.pcr.outcome.0")?.enabled;
    let out2_enabled = run.frame_meta().find_by_widget_id("analyze.pcr.outcome.2").ok_or("missing analyze.pcr.outcome.2")?.enabled;
    ensure(!out0_enabled && out2_enabled, format!("R10 expected outcome.0 disabled and outcome.2 enabled ({out0_enabled}, {out2_enabled})"))?;
    run.click("modal.cancel")?;

    // 3. Configure 5-channel Bubble Scatter graph.
    let bubble_id = find_graph_for_source(run.document(), &mv_ds_id, |g| g.spec.kind == GraphKind::MultipleVariables)?;
    run.select_sheet(&bubble_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.format.open")?;
    run.click("format_graph.mv.color_continuous")?;
    run.click("edit.tab.format_graph.data_objects")?;
    run.click("format_graph.mv.size_variable")?;
    run.click("format_graph.mv.confidence_ellipses")?;
    run.click("format_graph.mv.convex_hulls")?;
    run.commit_modal_ok()?;

    // 4. Grouped table -> Multiple Row t-tests -> Volcano plot + R8/R9 checks.
    let grp_ds_id =
        run.create_table_via_welcome("welcome.arch.grouped", "Gene Expression", &["welcome.subcol.raw", "welcome.stepper.replicates.inc"])?;
    run.click("grid.cell.0.0.0")?;
    let grp_tsv = "\
10.0\t10.5\t9.8\t18.5\t19.2\t18.9
20.1\t19.9\t20.4\t20.2\t20.0\t20.3
5.0\t5.2\t4.9\t12.1\t12.6\t12.4
15.0\t15.2\t14.8\t15.1\t14.9\t15.3";
    run.paste_clipboard(grp_tsv)?;
    run.enter_column_title(0, "Control")?;
    run.enter_column_title(1, "Treated")?;
    let _mr_id = run.run_analyze_entry("multiple row", "analyze.entry.stats.multiple_row_t_tests")?;

    let volcano_id = find_graph_for_source(run.document(), &grp_ds_id, |_| true)?;
    run.select_sheet(&volcano_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.family.all")?;
    run.click("graph_type.card.diag.volcano")?;
    run.commit_modal_ok()?;

    run.select_sheet(&grp_ds_id)?;
    let desc_id = run.run_analyze_entry("descriptive", "analyze.entry.stats.descriptive")?;
    run.open_cmd_t()?;
    let r8_on = run.frame_meta().find_by_widget_id("edit.desc.row_means").ok_or("missing R8 edit.desc.row_means")?.enabled;
    ensure(r8_on, format!("expected R8 edit.desc.row_means enabled initially (r8={r8_on})"))?;
    run.click("edit.desc.per_subcol")?;
    run.click("edit.tab.options")?;
    let r9_off = !run.frame_meta().find_by_widget_id("edit.pvalue.digits.inc").ok_or("missing R9 pvalue.digits.inc")?.enabled;
    ensure(r9_off, format!("expected R9 digits disabled initially (r9_off={r9_off})"))?;
    run.click("edit.pval.custom")?;
    let r9_on = run.frame_meta().find_by_widget_id("edit.pvalue.digits.inc").ok_or("missing R9 pvalue.digits.inc")?.enabled;
    ensure(r9_on, "expected R9 pvalue.digits.inc enabled after custom P format")?;
    run.click("edit.pvalue.digits.inc")?;
    run.commit_modal_ok()?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (corr_tab, corr_r, corr_c) = {
        let rs = run.document().result_sheet(&corr_id).ok_or("missing corr_id")?;
        find_row_col_in_tab(rs, "corr-matrix-r", "PC1", "Biomarker")?
    };

    run.verify_reference(
        ReferenceSpec::for_result(corr_id.clone(), move |sess| {
            let k = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }))
                .map_err(|e| e.to_string())?;
            let res_out = sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": k })).map_err(|e| e.to_string())?;
            let corr_sid = res_out.payload["sheet_id"].as_str().ok_or("missing ref corr sheet_id")?;
            let hm_kind = serde_json::to_value(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": corr_sid, "kind": hm_kind })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .with_graph(corr_hm_id.clone())
        .tab_cell(corr_tab, corr_r, corr_c, 1e-6, 1e-6)
        .check_graph(|hm_gs, _ref| {
            ensure(hm_gs.spec.heatmap_labels.annotate_cells, "expected annotate_cells true on correlation heatmap")?;
            ensure(hm_gs.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "expected Heatmap graph kind")?;
            Ok(())
        }),
    )?;

    ensure(
        run.document()
            .graph_sheet(&bubble_id)
            .is_some_and(|g| g.spec.multivariate.as_ref().is_some_and(|mv| mv.show_convex_hulls && mv.show_confidence_ellipses)),
        "expected bubble graph with hulls and ellipses",
    )?;
    ensure(
        run.document().graph_sheet(&volcano_id).is_some_and(|g| g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Volcano)),
        "expected Volcano GraphKind",
    )?;
    ensure(
        matches!(run.document().result_sheet(&desc_id).map(|r| &r.analysis.p_value_format), Some(PValueFormat::Custom { decimals: 5, .. })),
        "expected non-default Custom(5) PValueFormat on DescriptiveStats",
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 0, 2, 0, "85.0"),
        vec![LiveEditExpectation::tab_cell_changed(corr_id, corr_tab, corr_r, corr_c), LiveEditExpectation::graph_svg_changed(corr_hm_id)],
    )?;
    run.finish()
}

// ============================================================================
// WF-27: Multi-Panel Layout, Pointer Drag/Resize, ApplyStyleFrom, and Preflight
// ============================================================================

#[test]
fn wf27_human_multi_panel_layout_drag_snap_and_style() -> TestRes {
    let mut run = WorkflowRun::start("WF-27");

    let donor_ds = run.create_table_via_welcome("welcome.arch.column", "Donor Data", &[])?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("10\t22\n12\t25\n11\t24\n13\t27")?;
    let donor_g = find_graph_for_source(run.document(), &donor_ds, |_| true)?;

    let target_ds = run.create_table_via_welcome("welcome.arch.column", "Target Data", &[])?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("5\t14\n6\t17\n7\t16\n8\t19")?;
    let target_g = find_graph_for_source(run.document(), &target_ds, |_| true)?;
    let desc_id = run.run_analyze_entry("descriptive", "analyze.entry.stats.descriptive")?;

    // Style donor graph with non-default palette, bordered legend, and PlainBoxed frame.
    run.select_sheet(&donor_g)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.format.open")?;
    run.click("edit.tab.format_graph.settings")?;
    run.click("format_graph.palette.PaulTolVibrant")?;
    run.click("format_graph.legend_bordered")?;
    run.commit_modal_ok()?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.axes.open")?;
    run.click("axes.frame.plain_boxed")?;
    run.commit_modal_ok()?;

    // Clone styling from Donor onto Target via Graph > Apply Style From...
    run.select_sheet(&target_g)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.applyStyleFrom.open")?;
    run.click("apply_style.donor.0")?;
    run.click("apply_style.target.select_all")?;
    run.commit_modal_ok()?;

    ensure(
        run.document().graph_sheet(&target_g).is_some_and(|g| {
            g.spec.palette == PaletteId::PaulTolVibrant
                && g.spec.frame_preset == FramePreset::PlainBoxed
                && g.spec.legend.bordered
                && g.spec.primary_source_sheet_id == target_ds
        }),
        "ApplyStyleFrom did not clone donor styling while preserving target data binding",
    )?;

    // Create Layout via Insert > New Layout, add both graph panels, move & corner-resize panel 1, run Preflight.
    run.open_menu_and_click("menu.insert", &[], "menu.layout.create")?;
    let ls_id = run.document().layout_sheets.last().map(|l| l.id.clone()).ok_or("missing created LayoutSheet")?;
    run.select_sheet(&ls_id)?;
    run.click("studio.layout.add_panel.0")?;
    run.click("studio.layout.add_panel.1")?;
    run.click("inspector.layout.badge.uppercase")?;
    run.click("inspector.layout.align_origins")?;

    let b1_before = match run.document().layout_sheet(&ls_id).and_then(|l| l.spec.items.get(1)) {
        Some(LayoutItem::LiveGraph { bounds_pt, .. }) => *bounds_pt,
        other => return Err(format!("expected panel 1 LiveGraph before drag, got {other:?}")),
    };
    let p1_center = run.widget_center("canvas.layout_panel.1")?;
    run.drag(p1_center, Pos2::new(p1_center.x + 28.0, p1_center.y + 32.0))?;
    run.click("canvas.layout_panel.1")?;
    let se_center = run.widget_center("canvas.layout_panel.1.resize_se")?;
    run.drag(se_center, Pos2::new(se_center.x + 30.0, se_center.y + 24.0))?;
    let b1_after = match run.document().layout_sheet(&ls_id).and_then(|l| l.spec.items.get(1)) {
        Some(LayoutItem::LiveGraph { bounds_pt, .. }) => *bounds_pt,
        other => return Err(format!("expected panel 1 LiveGraph after drag/resize, got {other:?}")),
    };
    ensure(
        (b1_after.x - b1_before.x).abs() > 1.0 && (b1_after.width - b1_before.width).abs() > 1.0,
        format!("expected panel 1 move & corner resize, before={b1_before:?}, after={b1_after:?}"),
    )?;

    run.open_menu_and_click("menu.graph", &[], "menu.ui.canvas.preflight")?;

    let ref_target = run.document().data_sheet(&target_ds).ok_or("missing target_ds")?.clone();
    let ref_target_id = ref_target.id.clone();
    let (tab_idx, mean_row, col0) = {
        let rs = run.document().result_sheet(&desc_id).ok_or("missing desc_id")?;
        find_row_col_in_tab(rs, "desc-summary", "Mean", "A")?
    };

    run.verify_reference(
        ReferenceSpec::for_result(desc_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_target_id.as_str(), "kind": "descriptive_stats" }))
                .map_err(|e| e.to_string())?;
            let g_val = serde_json::to_value(GraphKind::Column).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_target_id.as_str(), "kind": g_val })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_target)?
        .with_graph(target_g.clone())
        .tab_cell(tab_idx, mean_row, col0, 1e-6, 1e-6)
        .check_graph(|gs, _ref| {
            ensure(gs.spec.palette == PaletteId::PaulTolVibrant && gs.spec.frame_preset == FramePreset::PlainBoxed, "cloned style mismatch")
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(target_ds, 0, 0, 0, "42.0"),
        vec![LiveEditExpectation::tab_cell_changed(desc_id, tab_idx, mean_row, col0), LiveEditExpectation::graph_svg_changed(target_g)],
    )?;
    run.finish()
}

// ============================================================================
// WF-28: Journal Canvas Presets, CVD Preview, Print/Export, Themes & Preferences
// ============================================================================

#[test]
fn wf28_human_canvas_preset_cvd_print_export_and_themes() -> TestRes {
    let mut run = WorkflowRun::start("WF-28");
    let ds_id = run.create_table_via_welcome("welcome.arch.xy", "Dose Response", &[])?;
    for (r, xv) in ["1", "3", "10", "30", "100"].iter().enumerate() {
        run.enter_x_cell(r, 0, xv)?;
    }
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("8\t14\n24\t39\n52\t68\n81\t89\n95\t98")?;

    let fit_id = run.run_analyze_entry("linear", "analyze.entry.fit.linear")?;
    let graph_id = find_graph_for_source(run.document(), &ds_id, |_| true)?;
    run.select_sheet(&graph_id)?;

    // 1. Journal canvas preset via Graph > Apply Canvas Preset > SingleColumn88mm.
    run.open_menu_and_click("menu.graph", &["menu.ui.canvas.applyPreset"], "popup.menu.ui.canvas.applyPreset.opt.SingleColumn88mm")?;
    let expected_pt = FigureCanvasPreset::SingleColumn88mm.dimensions_pt();
    let actual_pt = run.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.canvas_size_pt;
    ensure(
        (actual_pt.width - expected_pt.width).abs() < 0.5 && (actual_pt.height - expected_pt.height).abs() < 0.5,
        format!("expected SingleColumn88mm {expected_pt:?}, got {actual_pt:?}"),
    )?;

    // 2. Non-mutating CVD preview filter (`deuteranopia`) alters rendered canvas without mutating GraphSpec.
    let spec_before = run.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.clone();
    let normal_png = run.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    run.open_menu_and_click("menu.view", &["menu.ui.previewFilter.set"], "popup.menu.ui.previewFilter.set.opt.deuteranopia")?;
    ensure(run.app().preview_filter == CanvasPreviewFilter::Deuteranopia, "expected Deuteranopia preview filter")?;
    let cvd_png = run.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(cvd_png != normal_png, "expected CVD preview filter to alter canvas PNG")?;
    ensure(run.document().graph_sheet(&graph_id).map(|g| &g.spec) == Some(&spec_before), "CVD filter must not mutate GraphSpec")?;
    run.open_menu_and_click("menu.view", &["menu.ui.previewFilter.set"], "popup.menu.ui.previewFilter.set.opt.normal")?;

    // 3. Print Setup modal (non-OS in-memory PDF spool) and Export Figure modal (SVG & PDF).
    run.open_menu_and_click("menu.file", &[], "menu.ui.print.open")?;
    run.click("print.crop_marks")?;
    run.commit_modal_ok()?;
    ensure(run.app().canvas_dialogs.print.last_spool_bytes >= 100, "expected Print modal spool output")?;

    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.figure.open")?;
    run.click("export.fmt.svg")?;
    run.commit_modal_ok()?;

    // 4. Switch Dark / Light / HighContrast via View > Theme + Edit > Preferences...
    run.open_menu_and_click("menu.view", &["menu.ui.theme.set"], "popup.menu.ui.theme.set.opt.light")?;
    let light_shot = run.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    run.open_menu_and_click("menu.view", &["menu.ui.theme.set"], "popup.menu.ui.theme.set.opt.dark")?;
    ensure(run.app().theme.mode == UiThemeMode::Dark, "expected Dark theme via View > Theme")?;
    let dark_shot = run.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    run.open_menu_and_click("menu.view", &["menu.ui.theme.set"], "popup.menu.ui.theme.set.opt.high_contrast")?;
    ensure(run.app().theme.mode == UiThemeMode::HighContrast, "expected HighContrast theme via View > Theme")?;
    let hc_shot = run.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(dark_shot != light_shot && hc_shot != dark_shot, "expected distinct Light/Dark/HighContrast full-window screenshots")?;

    run.open_menu_and_click("menu.edit", &[], "menu.app.preferences")?;
    run.click("edit.tab.prefs.appearance")?;
    run.click("prefs.appearance.theme.light")?;
    run.click("modal.ok")?;
    ensure(run.app().theme.mode == UiThemeMode::Light, "expected Light theme after Preferences")?;

    let ref_ds = run.document().data_sheet(&ds_id).ok_or("missing xy ds")?.clone();
    let ref_ds_id = ref_ds.id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_ds_id.as_str(), "kind": "linear_regression" })).map_err(|e| e.to_string())?;
            let g_val = serde_json::to_value(GraphKind::Xy).map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "data_sheet_id": ref_ds_id.as_str(), "kind": g_val })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_ds)?
        .with_graph(graph_id.clone())
        .scalar("Slope", Some(0), 1e-6, 1e-6)
        .check_graph(move |gs, _ref| {
            ensure(
                (gs.spec.canvas_size_pt.width - expected_pt.width).abs() < 0.5,
                format!("expected SingleColumn88mm canvas width, got {:?}", gs.spec.canvas_size_pt),
            )
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(ds_id, 4, 0, 0, "140.0"),
        vec![LiveEditExpectation::scalar_changed(fit_id, "Slope", Some(0), 1e-3), LiveEditExpectation::graph_svg_changed(graph_id)],
    )?;
    run.finish()
}

// ============================================================================
// WF-29: Select & Transform + Chained Log10 + Row Stats + Remove Baseline
// ============================================================================

#[test]
fn wf29_human_select_transform_row_stats_and_baseline() -> TestRes {
    let mut run = WorkflowRun::start("WF-29");
    let tsv = "\
Control\t0.01\tS1\t100.0\t105.0\t110.0
Control\t0.1\tS2\t100.0\t110.0\t120.0
Control\t1.0\tS3\t100.0\t112.0\t115.0
Control\t10.0\tS4\t100.0\t108.0\t118.0
Treated\t0.01\tS5\t100.0\t125.0\t145.0
Treated\t0.1\tS6\t100.0\t140.0\t190.0
Treated\t1.0\tS7\t100.0\t160.0\t230.0
Treated\t10.0\tS8\t100.0\t180.0\t270.0";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Baseline & Dose Cohort",
        &["categorical", "continuous", "label", "continuous", "continuous", "continuous"],
        &["Condition", "Conc_uM", "Sample", "Signal_T0", "Signal_T1", "Signal_T2"],
        tsv,
    )?;

    // 1. Select & Transform: derived column + row filter formula via Cmd+T (`GWF-19`).
    let sel_id = run.run_analyze_entry("select", "analyze.entry.transform.select")?;
    run.open_cmd_t()?;
    run.click("edit.transform.derived_title")?;
    run.type_text("Gain")?;
    run.click("edit.transform.formula")?;
    run.type_text("[Signal_T2] - [Signal_T0]")?;
    run.click("edit.transform.add_derived")?;
    run.click("edit.tab.select_rows")?;
    run.replace_textbox("edit.transform.filter_formula", "[Condition] == \"Treated\" && [Conc_uM] >= 0.1")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().result_sheet(&sel_id).and_then(|r| r.analyzable_data.as_ref()).is_some_and(|d| d.archetype.row_count() == 3),
        "expected 3 filtered Treated rows with Conc_uM >= 0.1",
    )?;

    // 2. Chained Transform Math (`Log10(Y)`) on the derived SelectAndTransform sheet.
    run.select_sheet(&sel_id)?;
    let math_id = run.run_analyze_entry("log", "analyze.entry.transform.math")?;
    run.open_cmd_t()?;
    run.click("edit.transform.y_transform.log10")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(
            run.document().result_sheet(&math_id).map(|r| &r.analysis.kind),
            Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: Some(MathTransformPreset::Log10), .. }))
        ),
        "expected chained Log10 Y transform",
    )?;

    // 3. Row Descriptive Statistics on Signal_T0..T2.
    run.select_sheet(&mv_ds_id)?;
    let stats_id = run.run_analyze_entry("descriptive", "analyze.entry.stats.descriptive")?;
    let (row_tab_idx, mean_col) = {
        let rs = run.document().result_sheet(&stats_id).ok_or("missing stats_id")?;
        find_col_in_tab(rs, "row-descriptive-stats", "Mean")?
    };

    // 4. Remove Baseline on Signal_T0..T2 using Signal_T0 (col 3) as baseline Difference.
    run.select_sheet(&mv_ds_id)?;
    let base_id = run.run_analyze_entry("baseline", "analyze.entry.transform.baseline")?;
    run.open_cmd_t()?;
    run.click("edit.transform.baseline_col.0")?; // uncheck col 0 if default
    run.click("edit.transform.baseline_col.3")?; // check Signal_T0
    run.click("edit.transform.baseline_operation.difference")?;
    run.commit_modal_ok()?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds_id")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(stats_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": "descriptive_stats" })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .tab_cell(row_tab_idx, 7, mean_col, 1e-6, 1e-6),
    )?;

    let s8_t2_diff_before = run
        .document()
        .result_sheet(&base_id)
        .and_then(|r| r.analyzable_data.as_ref())
        .and_then(|d| d.archetype.data_cell(curvebook_doc::CellCoord::new(7, 5, 0)))
        .and_then(|c| c.active_number())
        .ok_or("missing baseline Signal_T2 diff on row 7")?;
    ensure((s8_t2_diff_before - 170.0).abs() < 1e-6, format!("expected 170.0 baseline diff, got {s8_t2_diff_before}"))?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 7, 5, 0, "300.0"),
        vec![LiveEditExpectation::tab_cell_changed(stats_id, row_tab_idx, 7, mean_col)],
    )?;
    let s8_t2_diff_after = run
        .document()
        .result_sheet(&base_id)
        .and_then(|r| r.analyzable_data.as_ref())
        .and_then(|d| d.archetype.data_cell(curvebook_doc::CellCoord::new(7, 5, 0)))
        .and_then(|c| c.active_number())
        .ok_or("missing baseline Signal_T2 diff after edit")?;
    ensure((s8_t2_diff_after - 200.0).abs() < 1e-6, format!("expected 200.0 baseline diff after edit, got {s8_t2_diff_after}"))?;
    run.finish()
}

// ============================================================================
// WF-30: Calculated Variables Formula Bar, Functions, and Syntax/Cycle Guards
// ============================================================================

#[test]
fn wf30_human_calculated_variables_formula_bar_and_guards() -> TestRes {
    let mut run = WorkflowRun::start("WF-30");
    let tsv = "\
S1\tCtrl\t9.0\t10.23\t20.46
S2\tCtrl\t99.0\t12.00\t14.00
S3\tTreat\t9.0\t8.50\t11.50
S4\tTreat\t99.0\t22.10\t18.10
S5\tTreat\t9.0\t15.00\t25.00
S6\tTreat\t99.0\t30.00\t24.00";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Formula Pipeline MV",
        &["label", "categorical", "continuous", "continuous", "continuous", "continuous", "continuous", "continuous"],
        &["Sample", "Arm", "Dose", "X1", "X2", "SumRound", "MeanLog", "GateMax"],
        tsv,
    )?;

    // Configure 3 calculated columns via Trailing Studio MV Formula Bar using ROUND, SUM, MEAN, LOG10, IF.
    apply_mv_formula(&mut run, 5, "= ROUND(SUM([X1], [X2]), 1)")?;
    apply_mv_formula(&mut run, 6, "= MEAN([X1], [X2]) + LOG10([Dose] + 1)")?;
    apply_mv_formula(&mut run, 7, "= IF([X1] > [X2], [X1], [X2])")?;

    // Syntax error guard: malformed expression disables inspector.mv.apply_formula cleanly (no intent_error).
    run.click("inspector.mv.col.7")?;
    run.replace_textbox("inspector.mv.formula_input", "= LOG10([UnclosedRef")?;
    let syntax_blocked = !run.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing apply_formula")?.enabled;
    ensure(syntax_blocked, "expected malformed syntax to disable inspector.mv.apply_formula")?;

    // Circularity guard: referencing self or cyclic chain disables inspector.mv.apply_formula cleanly.
    run.replace_textbox("inspector.mv.formula_input", "= [GateMax] + 1")?;
    let cycle_blocked = !run.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing apply_formula")?.enabled;
    ensure(cycle_blocked, "expected self-reference cycle to disable inspector.mv.apply_formula")?;
    apply_mv_formula(&mut run, 7, "= IF([X1] > [X2], [X1], [X2])")?;

    // Downstream analysis on calculated columns for reference oracle & live edit cascade.
    let desc_id = run.run_analyze_entry("descriptive", "analyze.entry.stats.descriptive")?;
    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds_id")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    let (tab_idx, mean_row, col_sum) = {
        let rs = run.document().result_sheet(&desc_id).ok_or("missing desc_id")?;
        find_row_col_in_tab(rs, "desc-summary", "Mean", "SumRound")?
    };

    run.verify_reference(
        ReferenceSpec::for_result(desc_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": "descriptive_stats" })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .tab_cell(tab_idx, mean_row, col_sum, 1e-6, 1e-6),
    )?;

    run.live_edit(
        LiveEditTarget::sheet_grid_cell(mv_ds_id, 0, 3, 0, "40.23"),
        LiveEditExpectation::tab_cell_changed(desc_id.clone(), tab_idx, mean_row, col_sum),
    )?;
    run.select_sheet(&desc_id)?;
    run.finish()
}

// ============================================================================
// WF-31: Cross-Schema Select & Transform (MV -> XY / Grouped) + Chained Analysis
// ============================================================================

#[test]
fn wf31_human_extract_and_rearrange_cross_schema() -> TestRes {
    let mut run = WorkflowRun::start("WF-31");
    let tsv = "\
W1\tWT\tResponder\t1.0\t12.0
W2\tWT\tResponder\t2.0\t24.5
W3\tWT\tResponder\t4.0\t47.0
W4\tWT\tNonResponder\t8.0\t95.0
M1\tMut\tResponder\t1.0\t10.0
M2\tMut\tResponder\t2.0\t14.0
M3\tMut\tNonResponder\t4.0\t18.0
M4\tMut\tNonResponder\t8.0\t26.0";
    let mv_ds_id = create_mv_table_via_welcome(
        &mut run,
        "Compound Kinetic Screen",
        &["label", "categorical", "categorical", "continuous", "continuous"],
        &["SampleID", "Genotype", "Phenotype", "TimeH", "Response"],
        tsv,
    )?;

    // Chain 1: MV -> XY via SelectAndTransform (filter WT, keep TimeH & Response).
    let xy_res_id = run.run_analyze_entry("select", "analyze.entry.transform.select")?;
    run.open_cmd_t()?;
    run.click("edit.tab.select_rows")?;
    run.replace_textbox("edit.transform.filter_formula", "[Genotype] == \"WT\"")?;
    run.click("edit.tab.select_cols")?;
    run.click("edit.transform.col.0")?;
    run.click("edit.transform.col.1")?;
    run.click("edit.transform.col.2")?;
    run.click("edit.tab.target_format")?;
    run.click("edit.transform.target_archetype.xy")?;
    run.commit_modal_ok()?;

    ensure(
        run.document()
            .result_sheet(&xy_res_id)
            .and_then(|r| r.analyzable_data.as_ref())
            .is_some_and(|d| d.kind() == TableKind::Xy && d.archetype.row_count() == 4),
        "expected SelectAndTransform to produce 4-row XY analyzable sheet",
    )?;

    // Chain downstream Linear Regression on the derived XY ResultSheet (not 4PL).
    run.select_sheet(&xy_res_id)?;
    let lin_id = run.run_analyze_entry("linear", "analyze.entry.fit.linear")?;

    // Chain 2: Also test MV -> Grouped cross-schema projection (`edit.transform.target_archetype.grouped`).
    run.select_sheet(&mv_ds_id)?;
    let grp_res_id = run.run_analyze_entry("select", "analyze.entry.transform.select")?;
    run.open_cmd_t()?;
    run.click("edit.tab.select_cols")?;
    run.click("edit.transform.col.0")?;
    run.click("edit.transform.col.1")?;
    run.click("edit.transform.col.2")?;
    run.click("edit.tab.target_format")?;
    run.click("edit.transform.target_archetype.grouped")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().result_sheet(&grp_res_id).and_then(|r| r.analyzable_data.as_ref()).is_some_and(|d| d.kind() == TableKind::Grouped),
        "expected Grouped target archetype from SelectAndTransform",
    )?;

    let ref_mv = run.document().data_sheet(&mv_ds_id).ok_or("missing mv_ds_id")?.clone();
    let ref_mv_id = ref_mv.id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(lin_id.clone(), move |sess| {
            let sel_kind = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                filter_formula: Some(String::from("[Genotype] == \"WT\"")),
                target_archetype: TableKind::Xy,
                derived_columns: Vec::new(),
            }))
            .map_err(|e| e.to_string())?;
            let sel_out = sess
                .execute("analysis.run", json!({ "source_sheet_id": ref_mv_id.as_str(), "kind": sel_kind, "selected_data_sets": [3, 4] }))
                .map_err(|e| e.to_string())?;
            let xy_sid = sel_out.payload["sheet_id"].as_str().ok_or("missing ref sel sheet_id")?;
            sess.execute("analysis.run", json!({ "source_sheet_id": xy_sid, "kind": "linear_regression" })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_mv)?
        .scalar("Slope", Some(0), 1e-6, 1e-6),
    )?;

    run.live_edit(LiveEditTarget::sheet_grid_cell(mv_ds_id, 3, 4, 0, "180.0"), LiveEditExpectation::scalar_changed(lin_id, "Slope", Some(0), 0.1))?;
    run.finish()
}
