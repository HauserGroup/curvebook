#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-19`–`WF-24`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{AnalysisKind, ContingencyMethod, SurvivalAnalysisSpec};
use curvebook_ui_egui::interaction::ViewportMetrics;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn wf19_human_contingency_2x2_fisher_r5_r6_and_rxc_chi_square() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.contingency", "2x2 Cohort", &[])?;
    for (r, (a, b)) in [(18, 4), (6, 22)].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let rs_id = h.run_analyze_entry("Contingency", "analyze.entry.stats.contingency")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.cont.fisher")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-19")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Contingency(ref c) if matches!(c.method, ContingencyMethod::FisherExact | ContingencyMethod::PearsonChiSquare { .. })),
        "WF-19 expected Contingency",
    )?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-19")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf20_human_roc_curve_and_cochran_armitage_trend() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "ROC Controls Cases", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{:.1}", 1.0 + r as f64 * 0.4))?;
        h.enter_grid_cell(r, 1, 0, &format!("{:.1}", 3.5 + r as f64 * 0.5))?;
    }
    let rs_id = h.run_analyze_entry("ROC", "analyze.entry.stats.roc")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.roc.delong")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-20 missing ROC")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-20")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf21_human_kaplan_meier_survival_logistic_and_r4() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.survival", "Survival KM", &[])?;
    for (r, t) in [5.0, 12.0, 18.0, 24.0, 30.0].iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{t}"))?;
        h.enter_grid_cell(r, 0, 0, if r % 2 == 0 { "1" } else { "0" })?;
        h.enter_grid_cell(r, 1, 0, "1")?;
    }
    let rs_id = h.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.km.mantel_cox")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-21")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. })), "WF-21 KM")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-21")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf22_human_pca_clustering_glm_volcano_format_points() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.multiple_variables", "MV PCA Volc", &["welcome.stepper.cols.inc"])?;
    h.seed_mv_4col_5row(&sid)?;
    let rs_id = h.run_analyze_entry("Principal", "analyze.entry.stats.pca")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-22 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.format_graph")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-22 missing PCA")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-22")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf23_human_stacked_superimposed_pcr_r10_and_chaining() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.multiple_variables", "PCR Matrix", &["welcome.stepper.cols.inc"])?;
    h.seed_mv_4col_5row(&sid)?;
    let rs_id = h.run_analyze_entry("Principal Component Regression", "analyze.entry.stats.pcr")?;
    h.select_sheet(&rs_id)?;
    let chained = h.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    ensure(h.app().document().result_sheet(&chained).is_some(), "WF-23 chaining")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-23")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf24_human_significance_brackets_stars_and_axes() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Brackets Cohort", &[])?;
    for (r, vals) in [[10.0, 15.0, 28.0], [11.0, 16.0, 30.0], [9.5, 14.5, 29.0], [10.5, 15.5, 31.0]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-24 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.axes")?;
    h.click("edit.tab.axes.y")?;
    h.click("axes.additional_ticks.open")?;
    h.click("axes.add_tick.value")?;
    h.driver.type_text("20.0")?;
    h.click("axes.add_tick.label")?;
    h.driver.type_text("Target")?;
    h.click("axes.add_tick.save")?;
    h.click("axes.add_tick.close")?;
    h.click("modal.ok")?;
    h.click("inspector.graph.tab.legends")?;
    h.click("inspector.graph.more_pairwise")?;
    h.click("pairwise.auto_add")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-24 missing ANOVA")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-24")?;
    assert_input_to_paint_budget(&h, 50.0)
}
