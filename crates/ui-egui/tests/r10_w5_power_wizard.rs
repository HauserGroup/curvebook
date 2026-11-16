#![forbid(unsafe_code)]
//! Wave 5 Shard `w5f4` (`R10.3`) human-path tests for the four-stage Power & Sample Size Wizard.

mod human_wf_common;

use curvebook_doc::AnalysisKind;
use curvebook_stats::power::{PowerSolveTarget, PowerTestFamily};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::layout::paint::PaintOpKind;
use curvebook_ui_egui::power_wizard::{PowerEffectInputMode, PowerWizardStage};
use curvebook_ui_egui::{HumanPathDriver, KeyModifiers, UiApp, ViewportMetrics, WidgetId};

type TestRes = Result<(), String>;

fn open_power_wizard_via_menu(driver: &mut HumanPathDriver) -> TestRes {
    human_wf_common::click_widget_center(driver, &WidgetId::from("menu.analyze"))?;
    {
        let tree = driver.layout_tree();
        if tree.node("popup.menu.analyze").is_none() {
            return Err(String::from("expected popup.menu.analyze after clicking Analyze menu"));
        }
    }
    human_wf_common::scroll_until_widget_visible(
        driver,
        &WidgetId::from("popup.menu.analyze"),
        &WidgetId::from("menu.ui.powerWizard.open"),
        120.0,
        60,
    )?;
    human_wf_common::click_widget_center(driver, &WidgetId::from("menu.ui.powerWizard.open"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::PowerAndSampleSize {
        return Err(format!("expected ActiveDialog::PowerAndSampleSize, got {:?}", driver.app().canvas_dialogs.active_dialog));
    }
    Ok(())
}

fn replace_input_text(driver: &mut HumanPathDriver, id: &str, text: &str) -> TestRes {
    human_wf_common::click_widget_center(driver, &WidgetId::from(id))?;
    let _ = driver.press_key("a", KeyModifiers { meta: true, ..Default::default() })?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    let _ = driver.type_text(text)?;
    Ok(())
}

fn tree_text_contains(driver: &mut HumanPathDriver, needle: &str) -> bool {
    driver.layout_tree().paint_ops.iter().any(|op| matches!(&op.kind, PaintOpKind::TextRun { text, .. } if text.contains(needle)))
}

fn slider_drag_target(driver: &mut HumanPathDriver, id: &str, frac: f64) -> Result<([f64; 2], [f64; 2]), String> {
    let r = {
        let tree = driver.layout_tree();
        let node = tree.node(id).ok_or_else(|| format!("slider widget `{id}` not found in layout tree"))?;
        node.bounds.rect_logical
    };
    let cy = r[1] + r[3] * 0.5;
    let start_x = r[0] + 8.0 + (r[2] - 16.0).max(1.0) * 0.20;
    let end_x = r[0] + 8.0 + (r[2] - 16.0).max(1.0) * frac.clamp(0.05, 0.95);
    Ok(([start_x, cy], [end_x, cy]))
}

#[test]
fn r10_w5_power_wizard_full_four_stage_human_path_and_resulting_sheets() -> TestRes {
    let app = UiApp::new();
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Open via Analyze menu -> Power & Sample Size...
    open_power_wizard_via_menu(&mut driver)?;
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Verify all 4 stage stepper buttons + live sidebar exist in Stage 1
    {
        let tree = driver.layout_tree();
        for id in ["power.stage.1", "power.stage.2", "power.stage.3", "power.stage.4", "power.sidebar"] {
            if tree.node(id).is_none() {
                return Err(format!("expected widget `{id}` on Stage 1"));
            }
        }
    }

    // Stage 1: Test-family selection cards (One-Way ANOVA + spinner, Survival Log-Rank, Paired t, Unpaired t)
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.one_way_anova"))?;
    if driver.app().canvas_dialogs.power_wizard.family != PowerTestFamily::OneWayAnova {
        return Err(String::from("clicking One-Way ANOVA card should select PowerTestFamily::OneWayAnova"));
    }
    let groups_before = driver.app().canvas_dialogs.power_wizard.anova_groups;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.design.anova_groups.inc"))?;
    if driver.app().canvas_dialogs.power_wizard.anova_groups != groups_before + 1 {
        return Err(String::from("ANOVA groups .inc spinner did not increment group count"));
    }

    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.survival_log_rank"))?;
    if !driver.app().view_shards.power_wizard.survival_log_rank || driver.app().canvas_dialogs.power_wizard.family != PowerTestFamily::TwoProportions
    {
        return Err(String::from("Survival Log-Rank card did not activate survival log-rank state"));
    }

    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.paired_t"))?;
    if driver.app().canvas_dialogs.power_wizard.family != PowerTestFamily::TwoSamplePairedT {
        return Err(String::from("clicking Paired t card failed"));
    }

    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.unpaired_t"))?;
    if driver.app().canvas_dialogs.power_wizard.family != PowerTestFamily::TwoSampleUnpairedT {
        return Err(String::from("clicking Unpaired t card failed"));
    }

    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.solve.power"))?;
    if driver.app().canvas_dialogs.power_wizard.solve_target != PowerSolveTarget::Power {
        return Err(String::from("clicking Solve For Power failed"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.solve.sample_size"))?;
    if driver.app().canvas_dialogs.power_wizard.solve_target != PowerSolveTarget::SampleSize {
        return Err(String::from("clicking Solve For Sample Size failed"));
    }
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0).map_err(|e| format!("after stage 1: {e}"))?;
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Advance to Stage 2 via Next footer button
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("dialog.button.next"))?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::EffectSize {
        return Err(format!("expected Stage 2, got {:?}", driver.app().canvas_dialogs.power_wizard.stage));
    }

    // Stage 2: Pilot-Data Effect-Size Calculator (type pilot means & SD + Apply)
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.effect_mode.means_and_sd"))?;
    replace_input_text(&mut driver, "power.pilot.mean1.input", "10")?;
    replace_input_text(&mut driver, "power.pilot.mean2.input", "14")?;
    replace_input_text(&mut driver, "power.pilot.sd.input", "8")?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.apply"))?;

    let derived_d = driver
        .app()
        .canvas_dialogs
        .power_wizard
        .derived_standardized_effect()
        .ok_or_else(|| String::from("expected derived standardized effect from pilot inputs"))?;
    if (derived_d - 0.5).abs() > 1e-6 {
        return Err(format!("expected pilot calculator to derive Cohen's d = 0.500, got {derived_d}"));
    }

    // Exercise pilot SD spinner & Large benchmark preset, then restore Medium (d = 0.5)
    let sd_before = driver.app().canvas_dialogs.power_wizard.sd;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.sd.inc"))?;
    if driver.app().canvas_dialogs.power_wizard.sd <= sd_before {
        return Err(String::from("pilot SD .inc spinner should increase SD"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.preset.large"))?;
    if (driver.app().canvas_dialogs.power_wizard.standardized_effect - 0.8).abs() > 1e-6 {
        return Err(String::from("Large benchmark preset did not set standardized_effect = 0.80"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.preset.medium"))?;
    if (driver.app().canvas_dialogs.power_wizard.standardized_effect - 0.5).abs() > 1e-6 {
        return Err(String::from("Medium benchmark preset did not restore standardized_effect = 0.50"));
    }
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0).map_err(|e| format!("after stage 2: {e}"))?;
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Advance to Stage 3
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("dialog.button.next"))?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::SignificanceAndSolve {
        return Err(format!("expected Stage 3, got {:?}", driver.app().canvas_dialogs.power_wizard.stage));
    }

    // Stage 3: Alpha / Tail / Power controls
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage3.alpha.001"))?;
    if (driver.app().canvas_dialogs.power_wizard.alpha - 0.01).abs() > 1e-6 {
        return Err(String::from("Stage 3 alpha=0.01 preset failed"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.one_sided"))?;
    if driver.app().canvas_dialogs.power_wizard.two_tailed {
        return Err(String::from("Stage 3 One-sided toggle failed"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.two_tailed"))?;
    if !driver.app().canvas_dialogs.power_wizard.two_tailed {
        return Err(String::from("Stage 3 Two-tailed toggle failed"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage3.alpha.005"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage3.power.90"))?;
    if (driver.app().canvas_dialogs.power_wizard.target_power - 0.90).abs() > 1e-6 {
        return Err(String::from("Stage 3 power=0.90 preset failed"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage3.power.80"))?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0).map_err(|e| format!("after stage 3: {e}"))?;
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Advance to Stage 4: Results, Vector Plot & Scrollable Sensitivity Table
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("dialog.button.next"))?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::Results {
        return Err(format!("expected Stage 4, got {:?}", driver.app().canvas_dialogs.power_wizard.stage));
    }

    // Verify vector plot elements (axes + target ref + curve line segments + plot title)
    {
        let tree = driver.layout_tree();
        if tree.node("power.curve_hover").is_none() {
            return Err(String::from("Stage 4 vector plot `power.curve_hover` missing"));
        }
        let line_count = tree
            .paint_ops
            .iter()
            .filter(|op| {
                op.owner_widget_id.as_ref().is_some_and(|w| w.as_str() == "power.curve_hover") && matches!(op.kind, PaintOpKind::LineSegment { .. })
            })
            .count();
        if line_count < 10 {
            return Err(format!("expected vector curve + axes + reference line segments >= 10, got {line_count}"));
        }
    }
    if !tree_text_contains(&mut driver, "Power vs. Sample Size") {
        return Err(String::from("expected plot title 'Power vs. Sample Size'"));
    }

    // Hover callout check across two distinct curve bins
    human_wf_common::hover_widget_center(&mut driver, &WidgetId::from("power.curve_hover.bin.4"))?;
    let callout_left = driver
        .layout_tree()
        .paint_ops
        .iter()
        .find_map(|op| match &op.kind {
            PaintOpKind::TextRun { text, .. } if text.starts_with("Hover: N=") => Some(text.clone()),
            _ => None,
        })
        .ok_or_else(|| String::from("expected hover callout text while hovering bin.4"))?;
    human_wf_common::hover_widget_center(&mut driver, &WidgetId::from("power.curve_hover.bin.15"))?;
    let callout_right = driver
        .layout_tree()
        .paint_ops
        .iter()
        .find_map(|op| match &op.kind {
            PaintOpKind::TextRun { text, .. } if text.starts_with("Hover: N=") => Some(text.clone()),
            _ => None,
        })
        .ok_or_else(|| String::from("expected hover callout text while hovering bin.15"))?;
    if callout_left == callout_right {
        return Err(format!("expected hover readout to change across bins, got `{callout_left}`"));
    }
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0).map_err(|e| format!("after stage 4 hover: {e}"))?;

    // Sensitivity table scroll & row click
    if driver.layout_tree().node("power.sensitivity_table").is_none() {
        return Err(String::from("expected scrollable `power.sensitivity_table`"));
    }
    human_wf_common::scroll_until_widget_visible(
        &mut driver,
        &WidgetId::from("power.sensitivity_table"),
        &WidgetId::from("power.sensitivity.row.11"),
        120.0,
        24,
    )?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.sensitivity.row.11"))?;
    let sens_alpha = driver.app().canvas_dialogs.power_wizard.alpha;
    let sens_power = driver.app().canvas_dialogs.power_wizard.target_power;
    if (sens_alpha - 0.05).abs() < 1e-4 && (sens_power - 0.80).abs() < 1e-3 {
        return Err(String::from("clicking sensitivity row 11 should select another (alpha, target_power) pair"));
    }

    // Live sidebar slider drag & spinner adjustments
    let n_before_slider = driver.app().canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.n_per_group);
    let (sl_start, sl_end) = slider_drag_target(&mut driver, "power.sidebar.effect_size.slider", 0.80)?;
    let _ = driver.drag_logical(sl_start[0], sl_start[1], sl_end[0], sl_end[1]);
    let eff_after_slider = driver.app().canvas_dialogs.power_wizard.standardized_effect;
    if eff_after_slider <= 0.6 {
        return Err(format!("dragging effect_size slider to 80% should increase standardized_effect, got {eff_after_slider}"));
    }
    let n_after_slider = driver.app().canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.n_per_group);
    if n_after_slider >= n_before_slider {
        return Err(format!("increasing effect_size should decrease required n_per_group ({n_before_slider} -> {n_after_slider})"));
    }

    // Exercise allocation & dropout spinners and confirm dropout inflation readout updates
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.sidebar.dropout.inc"))?;
    if driver.app().view_shards.power_wizard.dropout_rate <= 0.0 {
        return Err(String::from("dropout .inc spinner did not increase dropout_rate"));
    }
    if !tree_text_contains(&mut driver, "Adj +") && !tree_text_contains(&mut driver, "(Adj:") {
        return Err(String::from("expected live readout to display dropout-adjusted N"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.sidebar.dropout.dec"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.sidebar.allocation_ratio.inc"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.sidebar.allocation_ratio.dec"))?;

    // Reset via Stage stepper click back to Stage 2 -> Medium (d=0.50), Stage 3 -> alpha=0.05, power=0.80, then Stage 4 -> Create
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.2"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.preset.medium"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.3"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage3.alpha.005"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage3.power.80"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.4"))?;

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;

    // Click Create (modal.ok) and verify resulting sheets
    let res_before = driver.app().document().result_sheets.len();
    let data_before = driver.app().document().data_sheets.len();
    let graph_before = driver.app().document().graph_sheets.len();
    let info_before = driver.app().document().info_sheets.len();

    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(format!("expected wizard modal to close on Create, got {:?}", driver.app().canvas_dialogs.active_dialog));
    }

    let doc = driver.app().document();
    if doc.result_sheets.len() != res_before + 1
        || doc.data_sheets.len() != data_before + 1
        || doc.graph_sheets.len() != graph_before + 1
        || doc.info_sheets.len() != info_before + 1
    {
        return Err(format!(
            "expected +1 result/data/graph/info sheet on Create, got res={} data={} graph={} info={}",
            doc.result_sheets.len(),
            doc.data_sheets.len(),
            doc.graph_sheets.len(),
            doc.info_sheets.len()
        ));
    }

    let rs = doc.result_sheets.last().ok_or_else(|| String::from("missing created power ResultSheet"))?;
    if !matches!(rs.analysis.kind, AnalysisKind::PowerAnalysis(_)) {
        return Err(format!("expected AnalysisKind::PowerAnalysis, got {:?}", rs.analysis.kind));
    }
    let n_per_group = rs
        .scalars
        .iter()
        .find(|s| s.key == "NPerGroup")
        .map(|s| s.value)
        .ok_or_else(|| String::from("missing NPerGroup scalar in created Power ResultSheet"))?;
    let total_n = rs
        .scalars
        .iter()
        .find(|s| s.key == "TotalN")
        .map(|s| s.value)
        .ok_or_else(|| String::from("missing TotalN scalar in created Power ResultSheet"))?;
    if (n_per_group - 64.0).abs() > 1e-6 || (total_n - 128.0).abs() > 1e-6 {
        return Err(format!("expected NPerGroup=64, TotalN=128 for d=0.5 alpha=0.05 power=0.80, got {n_per_group}/{total_n}"));
    }

    Ok(())
}

#[test]
fn r10_w5_power_wizard_sequential_validation_and_pilot_derivations() -> TestRes {
    let mut app = UiApp::new();
    app.canvas_dialogs.power_wizard.standardized_effect = 0.0;
    app.canvas_dialogs.power_wizard.effect_mode = PowerEffectInputMode::Standardized;
    app.canvas_dialogs.power_wizard.stage = PowerWizardStage::EffectSize;
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;

    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // R2-S10: Invalid effect size on Stage 2 must disable Next and Stage 3/4 forward tabs, while keeping Stage 1 enabled
    let (stage3_en, stage1_en, next_en) = {
        let tree = driver.layout_tree();
        let stage3 = tree.node("power.stage.3").ok_or_else(|| String::from("missing power.stage.3"))?;
        let stage1 = tree.node("power.stage.1").ok_or_else(|| String::from("missing power.stage.1"))?;
        let next_btn = tree.node("dialog.button.next").ok_or_else(|| String::from("missing dialog.button.next"))?;
        (stage3.state.enablement.enabled, stage1.state.enablement.enabled, next_btn.state.enablement.enabled)
    };
    if stage3_en || next_en {
        return Err(String::from("expected invalid effect_size=0 to disable forward Stage 3 & Next button"));
    }
    if !stage1_en {
        return Err(String::from("backward Stage 1 tab should remain enabled"));
    }
    // Clicking disabled Stage 3 must be rejected cleanly and must NOT bypass validation
    if human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.3")).is_ok() {
        return Err(String::from("clicking disabled Stage 3 tab should return error"));
    }
    let stage3_center = {
        let tree = driver.layout_tree();
        let node = tree.node("power.stage.3").ok_or_else(|| String::from("missing power.stage.3"))?;
        let r = node.bounds.rect_logical;
        [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5]
    };
    let _ = driver.click_logical(stage3_center[0], stage3_center[1]);
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::EffectSize {
        return Err(String::from("clicking disabled Stage 3 tab should not bypass validation"));
    }

    // Fix effect via Stage 1 -> Paired t -> Stage 2 Means & Correlation pilot calculation -> n = 34
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.1"))?;
    if driver.app().canvas_dialogs.power_wizard.stage != PowerWizardStage::ExperimentDesign {
        return Err(String::from("clicking Stage 1 backward should work"));
    }
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.paired_t"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("dialog.button.next"))?;

    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.effect_mode.means_and_sd"))?;
    replace_input_text(&mut driver, "power.pilot.mean1.input", "10")?;
    replace_input_text(&mut driver, "power.pilot.mean2.input", "11")?;
    replace_input_text(&mut driver, "power.pilot.sd.input", "2")?;
    replace_input_text(&mut driver, "power.pilot.corr.input", "0.5")?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.apply"))?;

    let paired_n = driver.app().canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.n_per_group);
    if paired_n != 34 {
        return Err(format!("expected paired t with mean_diff=1, sd=2, r=0.5 (dz=0.5) to yield n=34, got {paired_n}"));
    }

    // One-Way ANOVA in Stage 3 must disable one-sided hypothesis mode (`R2-S10`)
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.1"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.one_way_anova"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.3"))?;
    let one_sided_en = {
        let tree = driver.layout_tree();
        tree.node("power.one_sided").ok_or_else(|| String::from("missing power.one_sided on Stage 3"))?.state.enablement.enabled
    };
    if one_sided_en {
        return Err(String::from("expected One-Way ANOVA to disable `power.one_sided` per R2-S10"));
    }

    // Survival Log-Rank family: HR=0.65 & event_prob=0.50 updates Schoenfeld estimate and runs clean parity
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.1"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.family.survival_log_rank"))?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.stage.2"))?;
    replace_input_text(&mut driver, "power.pilot.hr.input", "0.65")?;
    replace_input_text(&mut driver, "power.pilot.evprob.input", "0.50")?;
    human_wf_common::click_widget_center(&mut driver, &WidgetId::from("power.pilot.apply"))?;
    if !tree_text_contains(&mut driver, "HR = 0.65") || !tree_text_contains(&mut driver, "events)") {
        return Err(String::from("expected Survival Log-Rank pilot calculator to show HR and Schoenfeld events estimate"));
    }
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn r10_w5_power_wizard_locales_rtl_and_hidpi_parity() -> TestRes {
    for locale in ["en", "de", "ja", "ar"] {
        let mut app = UiApp::new();
        let _ = app.set_locale(locale);
        app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;

        let mut driver = human_wf_common::new_human_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
        for stage_idx in 1..=4 {
            let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)
                .map_err(|e| format!("locale={locale} stage={stage_idx} parity failed: {e}"))?;
            if stage_idx == 4 {
                human_wf_common::hover_widget_center(&mut driver, &WidgetId::from("power.curve_hover.bin.9"))?;
                let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)
                    .map_err(|e| format!("locale={locale} stage=4 hover parity failed: {e}"))?;
            } else {
                human_wf_common::click_widget_center(&mut driver, &WidgetId::from("dialog.button.next"))?;
            }
        }
    }

    // HiDPI 2.0x verification across all 4 stages (checking every ModalDialog/power.* widget)
    for locale in ["en", "ar"] {
        let mut app = UiApp::new();
        let _ = app.set_locale(locale);
        app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
        let mut driver = human_wf_common::new_human_driver(app, ViewportMetrics::new(1280.0, 800.0, 2.0));
        for stage_idx in 1..=4 {
            let report = driver.verify_parity();
            let modal_violations: Vec<_> = report
                .violations
                .iter()
                .filter(|v| v.widget_id.starts_with("power.") || v.widget_id.starts_with("modal.") || v.widget_id.starts_with("dialog."))
                .collect();
            if !modal_violations.is_empty() {
                return Err(format!("HiDPI 2.0x locale={locale} stage={stage_idx} modal parity violations: {modal_violations:?}"));
            }
            if stage_idx < 4 {
                human_wf_common::click_widget_center(&mut driver, &WidgetId::from("dialog.button.next"))?;
            }
        }
    }
    Ok(())
}
