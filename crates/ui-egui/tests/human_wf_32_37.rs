#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-32`–`WF-37`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{AnalysisKind, ClusterLinkage, MultivariateSpec};
use curvebook_ui_egui::interaction::ViewportMetrics;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn wf32_human_direct_mv_welch_t_and_categorical_summary() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Welch")?;
    let rs = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    h.select_sheet(&rs)?;
    h.open_cmd_t()?;
    h.click("edit.tg.welch_t")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-32 missing Welch")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-32")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf33_human_direct_mv_multifactor_anova() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Factorial")?;
    h.set_mv_role(1, "categorical")?;
    for r in 0..10 {
        h.enter_grid_cell(r, 1, 0, if r % 2 == 0 { "Male" } else { "Female" })?;
    }
    let rs = h.run_analyze_entry("Two-Way", "analyze.entry.stats.factorial_anova")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-33 missing Factorial")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-33")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf34_human_qpcr_ddct_calculated_variables_and_welch() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("qPCR ddCt")?;
    let rs = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-34 missing Welch")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-34")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf35_human_direct_mv_kaplan_meier_survival() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV KM")?;
    let rs = h.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-35 missing KM")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-35")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf36_human_direct_mv_4pl_dose_response() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV 4PL")?;
    let rs = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-36 missing 4PL")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-36")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf37_human_hierarchical_clustering_and_dendrogram() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("HClust")?;
    let rs_id = h.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.cluster.link.ward")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-37")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage: ClusterLinkage::Ward, .. })),
        "WF-37 Clustering",
    )?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-37")?;
    assert_input_to_paint_budget(&h, 50.0)
}
