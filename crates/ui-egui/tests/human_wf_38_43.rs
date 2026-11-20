#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-38`–`WF-43`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{AnalysisKind, CoxTieMethod, GlmFamily, SurvivalAnalysisSpec};
use curvebook_ui_egui::interaction::ViewportMetrics;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn wf38_human_kmeans_clustering_silhouette_elbow_and_scatter() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("KMeans Auto")?;
    let rs_id = h.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-38 missing KMeans")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-38")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf39_human_cox_ph_efron_ties_and_covariate_curves() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Cox PH")?;
    let rs_id = h.run_analyze_entry("Cox", "analyze.entry.stats.survival_cox")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.cox.efron")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-39")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Efron, .. })),
        "WF-39 expected Efron Cox PH",
    )?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-39")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf40_human_multiple_linear_regression_type_iii_and_vif() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MLR Type III")?;
    let rs_id = h.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.glm.linear")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-40")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Linear), "WF-40 Linear GLM")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-40")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf41_human_multiple_logistic_classification_and_roc() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Logistic GLM")?;
    let rs_id = h.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.data")?;
    h.click("edit.glm.resp.2")?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.glm.logistic")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-41")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Logistic), "WF-41 Logistic GLM")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-41")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf42_human_poisson_count_glm_dispersion_and_vif() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Poisson GLM")?;
    let rs_id = h.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.glm.poisson")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-42")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Poisson), "WF-42 Poisson GLM")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-42")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf43_human_xy_serial_dilution_constrained_4pl_and_auc() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Serial Dilution 4PL", &[])?;
    for (r, (&x, &y)) in [0.1, 1.0, 10.0, 100.0, 1000.0].iter().zip([12.0, 28.0, 55.0, 82.0, 96.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.constraints")?;
    h.click("fit.tab.output")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-43 missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-43")?;
    assert_input_to_paint_budget(&h, 50.0)
}
