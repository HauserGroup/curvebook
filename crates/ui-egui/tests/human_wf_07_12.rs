#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-07`–`WF-12`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{AnalysisKind, GraphKind, GroupedArrangement, MultiplicityMethod, OneWayMethod, TwoGroupMethod};
use curvebook_ui_egui::interaction::ViewportMetrics;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn wf07_human_unpaired_student_and_welch_t_test_with_predicates() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Unpaired T Cohort", &[])?;
    for (r, (a, b)) in [(10.0, 15.0), (11.0, 18.0), (12.0, 21.0), (10.5, 19.0)].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let rs_id = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    let paired_t = h.driver.frame_meta().find_by_widget_id("edit.tg.paired_t").ok_or("missing edit.tg.paired_t")?;
    ensure(!paired_t.enabled, "WF-07 paired_t disabled on unpaired design")?;
    let graph_diff = h.driver.frame_meta().find_by_widget_id("edit.two_group.graph_differences").ok_or("missing edit.two_group.graph_differences")?;
    ensure(!graph_diff.enabled, "WF-07 R4 graph_differences disabled on unpaired design")?;
    h.click("edit.tg.welch_t")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-07 res")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::WelchT), "WF-07 WelchT")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-07")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf08_human_paired_t_test_equal_n_and_before_after_graph() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Paired T", &["welcome.design.paired", "welcome.stepper.cols.dec"])?;
    for (r, (a, b)) in [(10.0, 14.0), (12.0, 17.0), (14.0, 19.0)].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let rs_id = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.tg.paired_t")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("WF-08 missing result")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::PairedT), "WF-08 PairedT")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-08 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.col.before_after")?;
    h.click("modal.ok")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-08")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf09_human_mann_whitney_and_wilcoxon_rank_invariance() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Nonparam Ranks", &["welcome.stepper.cols.dec"])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", (r + 1) * 10))?;
    }
    let rs_id = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.assume_gaussian")?;
    h.click("edit.tg.mann_whitney")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-09")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::MannWhitneyU), "WF-09 MannWhitneyU")?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.tg.wilcoxon")?;
    h.click("modal.ok")?;
    let rs2 = h.app().document().result_sheet(&rs_id).ok_or("missing WF-09 wilcoxon")?;
    ensure(matches!(rs2.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::WilcoxonSignedRank), "WF-09 Wilcoxon")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-09")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf10_human_one_way_anova_welch_dunnett_tukey_and_brackets() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Four Dose ANOVA", &[])?;
    for (r, vals) in [[10.0, 12.0, 20.0], [10.2, 12.5, 21.0], [9.9, 11.8, 19.5], [10.1, 12.2, 20.5]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.multiple_comparisons")?;
    h.click("edit.post_hoc.tukey")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-10")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::TukeyHsd), "WF-10 Tukey")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-10 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("inspector.graph.tab.legends")?;
    h.click("inspector.graph.more_pairwise")?;
    h.click("pairwise.auto_add")?;
    h.click("modal.ok")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-10")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf11_human_kruskal_wallis_and_dunns_multiple_comparisons() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "KW Dunn", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", 5 + r))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", 15 + r))?;
        h.enter_grid_cell(r, 2, 0, &format!("{}", 30 + r))?;
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.assume_gaussian")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-11")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::KruskalWallis), "WF-11 KW")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-11")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf12_human_two_way_anova_interaction_and_grouped_guard() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.grouped", "Grouped 2Way", &["welcome.subcol.raw"])?;
    for r in 0..2 {
        for c in 0..2 {
            for s in 0..2 {
                h.enter_grid_cell(r, c, s, &format!("{:.1}", 10.0 + (r * 4 + c * 2 + s) as f64 * 1.5))?;
            }
        }
    }
    let rs_id = h.run_analyze_entry("Two-Way", "analyze.entry.stats.factorial_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.fac.ss_type_iii")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-12")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(_)), "WF-12 FactorialAnova")?;

    // Verify Grouped Interleaved retains Y error bars and Grouped Stacked strips them (R1 grouped guard).
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-12 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.grouped.interleaved")?;
    h.click("modal.ok")?;
    let gs_inter = h.app().document().graph_sheet(&gs_id).ok_or("missing interleaved graph")?;
    ensure(
        gs_inter.spec.kind == GraphKind::Grouped
            && gs_inter.spec.grouped_arrangement == GroupedArrangement::Interleaved
            && gs_inter.spec.series.iter().any(|s| s.y_error_bars.is_some()),
        "WF-12 expected Grouped Interleaved with Y error bars",
    )?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.grouped.stacked")?;
    h.click("modal.ok")?;
    let gs_stack = h.app().document().graph_sheet(&gs_id).ok_or("missing stacked graph")?;
    ensure(
        gs_stack.spec.kind == GraphKind::Grouped
            && gs_stack.spec.grouped_arrangement == GroupedArrangement::Stacked
            && !gs_stack.spec.series.iter().any(|s| s.y_error_bars.is_some()),
        "WF-12 expected Grouped Stacked to strip Y error bars",
    )?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-12")?;
    assert_input_to_paint_budget(&h, 50.0)
}
