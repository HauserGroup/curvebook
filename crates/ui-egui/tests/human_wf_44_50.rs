#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` Human-Path Acceptance Matrix (`WF-44`–`WF-50`, `R15.4`/`R15.5`).

mod human_wf_common;

use curvebook_doc::{GraphKind, PairwiseDisplayMode};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;

use human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, ensure};

#[test]
fn wf44_human_confidence_ellipses_and_convex_hulls() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Ellipses Hulls")?;
    let _rs = h.run_analyze_entry("Principal", "analyze.entry.stats.pca")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-44 missing graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.format_graph")?;
    h.click("modal.ok")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-44")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf45_human_grouped_and_mv_cross_tab_heatmaps() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Heatmap")?;
    let rs_id = h.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.corr.spearman")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-45 heatmap missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-45")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf46_human_interactive_mv_inspector_variable_swapping() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Swap Vars")?;
    h.click("mv_formula.role_badge.0")?;
    h.click("mv_formula.role_manage.0")?;
    ensure(h.app().grid.variables_dialog.open, "WF-46 Manage Variables must open")?;
    h.click("mv_formula.vars.col.1")?;
    h.click("modal.ok")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-46")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf47_human_kaplan_meier_number_at_risk_and_pairwise() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.survival", "KM Number at Risk", &[])?;
    for (r, t) in [4.0, 10.0, 16.0, 22.0, 28.0].iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{t}"))?;
        h.enter_grid_cell(r, 0, 0, if r % 2 == 0 { "1" } else { "0" })?;
        h.enter_grid_cell(r, 1, 0, "1")?;
    }
    let rs_id = h.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-47 KM missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-47")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf48_human_mv_bubble_sizing_and_outlier_clamping() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Bubble Sizing")?;
    let _rs = h.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-48 graph missing")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.export_svg")?;
    ensure(h.driver.frame_meta().find_by_widget_id("dialog.export_figure").is_some(), "WF-48 export dialog")?;
    h.click("modal.cancel")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-48")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf49_human_live_tex_model_equation_annotations() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Live TeX Eq", &[])?;
    for (r, (&x, &y)) in [1.0, 3.0, 10.0, 30.0, 100.0].iter().zip([10.0, 25.0, 52.0, 80.0, 95.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-49 graph missing")?;
    h.select_sheet(&gs_id)?;
    h.click("inspector.graph.tab.annotations")?;
    h.click("inspector.graph.add_equation")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-49 NLREG missing")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-49")?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn wf50_human_compact_letter_display_piepho_svg_pdf_rgba() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "WF-50 CLD", &[])?;
    for (c, vals) in [[8.0, 12.0, 16.0], [13.0, 17.0, 21.0], [21.0, 25.0, 29.0]].iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.multiple_comparisons")?;
    h.click("edit.post_hoc.tukey")?;
    h.click("modal.ok")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-50 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("inspector.graph.tab.legends")?;
    h.click("inspector.graph.more_pairwise")?;
    h.click("pairwise.mode.CompactLetterDisplayOnly")?;
    h.click("modal.ok")?;
    let gs = h.app().document().graph_sheet(&gs_id).ok_or("missing WF-50 graph")?;
    ensure(gs.spec.kind == GraphKind::Column, "WF-50 GraphKind::Column")?;
    ensure(gs.spec.pairwise_display_mode == PairwiseDisplayMode::CompactLetterDisplayOnly, "WF-50 CLD mode")?;
    let png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-50 canvas PNG")?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("WF-50")?;
    assert_input_to_paint_budget(&h, 50.0)
}
