//! Four-stage `Power & Sample Size Studio` (`65%` left 3-tab narrative/curve/sensitivity matrix +
//! `35%` live sensitivity sidebar) in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.13).

use curvebook_doc::AnalysisKind;
use curvebook_stats::power::{CorrelationPowerMethod, PowerAnalysisResult, PowerSolveTarget, PowerTestFamily, TwoProportionsMethod};
use egui::{
    Align2, Context, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, scroll_area::ScrollBarVisibility,
};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_button, evaluate_dialog_widget};
use crate::power_wizard::{PowerEffectInputMode, PowerWizardStage, PowerWizardState};

use super::stat_anova_modal::{
    ClippedTextSpec, StepperSpec, record_widget, render_clipped_text, render_modal_checkbox, render_modal_radio, render_modal_seg_btn,
    render_section_title, render_stepper_row,
};
use super::stats_modals::{Wave4ModalShell, render_modal_action_button, show_wave4_modal};
use super::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use super::{FrameInspectMeta, PendingUiIntents, Tokens};

/// Three tabs rendered inside the `65%` Stage 4 results pane of `Power & Sample Size Studio`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PowerResultsSubTab {
    #[default]
    NarrativeReport,
    PowerCurve,
    SensitivityMatrix,
}

/// Persistent transient UI state for `render_power_studio_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerStudioEguiState {
    pub results_tab: PowerResultsSubTab,
    pub alpha_spinner: ScrubUnitFieldState,
    pub power_spinner: ScrubUnitFieldState,
    pub hover_n: Option<u32>,
    pub narrative_copied: bool,
}

/// Frame geometry and modal capture report emitted by `power_studio`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerStudioFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Return `true` when `ActiveDialog::PowerAndSampleSize` OR `EditAnalysisParameters` (`PowerAnalysis`) is open.
#[must_use]
pub fn is_power_studio_dialog_active(app: &UiApp) -> bool {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => true,
        ActiveDialog::EditAnalysisParameters { result_sheet_id } => {
            if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
                return matches!(spec.kind, AnalysisKind::PowerAnalysis(_));
            }
            app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::PowerAnalysis(_)))
        }
        _ => false,
    }
}

/// Render the 4-stage `Power & Sample Size Studio` modal when active.
pub fn render_power_studio_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut PowerStudioEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> PowerStudioFrameReport {
    if !is_power_studio_dialog_active(app) {
        return PowerStudioFrameReport::default();
    }

    // Initialize clean units for sidebar scrub fields once.
    if state.alpha_spinner.unit != ScrubUnitKind::Generic {
        state.alpha_spinner = ScrubUnitFieldState::new(ScrubUnitKind::Generic);
    }
    if state.power_spinner.unit != ScrubUnitKind::Percent {
        state.power_spinner = ScrubUnitFieldState::new(ScrubUnitKind::Percent);
    }

    let pw = &app.canvas_dialogs.power_wizard;
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "w6.view.modal.power_and_sample_size");
    let solved_summary = format_power_banner(pw);

    let can_next = evaluate_dialog_button(&app.canvas_dialogs, app.document(), "next").unwrap_or_else(ControlEnablement::enabled);
    let can_back = evaluate_dialog_button(&app.canvas_dialogs, app.document(), "back").unwrap_or_else(ControlEnablement::enabled);
    let can_create = evaluate_dialog_button(&app.canvas_dialogs, app.document(), "create").unwrap_or_else(ControlEnablement::enabled);

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.power_and_sample_size",
            title,
            subtitle_or_banner: Some(&solved_summary),
            banner_widget_id: "power.status_banner",
            max_width_pt: 880.0,
            max_height_pt: 590.0,
            use_body_scroll: false,
            footer_reserve_pt: 42.0,
        },
        |ui, _state, meta, intents| {
            ui.add_space(4.0);
            render_power_stage_stepper(ui, pw, tokens, meta, intents);
        },
        |ui, state, avail_w, avail_h, meta, intents| {
            let gap = 10.0;
            let left_w = ((avail_w - gap) * 0.65).floor().max(260.0);
            let right_w = (avail_w - left_w - gap).max(160.0);

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                // 65% Primary Left Stage Pane
                ui.allocate_ui_with_layout(Vec2::new(left_w, avail_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let scroll_w = (left_w - 18.0).max(140.0);
                    egui::ScrollArea::vertical()
                        .id_salt("power_studio.left_scroll")
                        .max_height(avail_h)
                        .min_scrolled_height(avail_h)
                        .auto_shrink([false, false])
                        .scroll_source(egui::scroll_area::ScrollSource {
                            mouse_wheel: true,
                            scroll_bar: true,
                            ..egui::scroll_area::ScrollSource::NONE
                        })
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                        .show(ui, |ui| {
                            ui.set_width(scroll_w);
                            ui.set_max_width(scroll_w);
                            match pw.stage {
                                PowerWizardStage::ExperimentDesign => render_stage1_design(ui, app, pw, state, tokens, meta, intents),
                                PowerWizardStage::EffectSize => render_stage2_effect(ui, pw, tokens, meta, intents),
                                PowerWizardStage::SignificanceAndSolve => render_stage3_alpha_power(ui, app, pw, tokens, meta, intents),
                                PowerWizardStage::Results => render_stage4_results(ctx, ui, pw, state, tokens, meta, intents),
                            }
                        });
                });

                // 35% Persistent Live Sensitivity Sidebar
                ui.allocate_ui_with_layout(Vec2::new(right_w, avail_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let scroll_w = (right_w - 18.0).max(120.0);
                    egui::Frame::NONE
                        .fill(tokens.panel_bg)
                        .stroke(Stroke::new(1.0, tokens.border))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Vec2::new(6.0, 6.0))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("power_studio.sidebar_scroll")
                                .max_height((avail_h - 12.0).max(60.0))
                                .min_scrolled_height((avail_h - 12.0).max(60.0))
                                .auto_shrink([false, false])
                                .scroll_source(egui::scroll_area::ScrollSource {
                                    mouse_wheel: true,
                                    scroll_bar: true,
                                    ..egui::scroll_area::ScrollSource::NONE
                                })
                                .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                                .show(ui, |ui| {
                                    let inner_side_w = (scroll_w - 12.0).max(100.0);
                                    ui.set_width(inner_side_w);
                                    ui.set_max_width(inner_side_w);
                                    render_live_sidebar(ui, pw, state, tokens, meta, intents);
                                });
                        });
                });
            });
        },
        |ui, _state, meta, intents| {
            ui.horizontal(|ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.back", "← Back", false, false, &can_back) {
                    intents.push_commit_power_wizard("back");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pw.stage == PowerWizardStage::Results {
                        if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "w6.view.btn.create"), true, false, &can_create) {
                            intents.push_commit_power_wizard("create");
                        }
                    } else if render_modal_action_button(ui, tokens, meta, "modal.next", "Next →", true, false, &can_next) {
                        intents.push_commit_power_wizard("next");
                    }
                    if render_modal_action_button(
                        ui,
                        tokens,
                        meta,
                        "modal.cancel",
                        tr(locale, "btn.cancel"),
                        false,
                        false,
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push_commit_power_wizard("cancel");
                    }
                });
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_commit_power_wizard("cancel");
    }

    PowerStudioFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn format_power_banner(pw: &PowerWizardState) -> String {
    if let Some(res) = &pw.result {
        let mut s = format!(
            "{}  |  N/group = {} (Total N = {})  |  Power = {:.1}%  |  Effect = {:.3}",
            res.family.label(),
            res.n_per_group,
            res.total_n,
            res.achieved_power * 100.0,
            res.effect_size
        );
        if let Some(n2) = res.n_group2 {
            s.push_str(&format!(" (n₂={n2})"));
        }
        s
    } else {
        format!("{}  |  α = {:.3}  |  Target Power = {:.0}%", pw.family.label(), pw.alpha, pw.target_power * 100.0)
    }
}

fn render_power_stage_stepper(ui: &mut Ui, pw: &PowerWizardState, tokens: &Tokens, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let can_advance = pw.can_advance().is_ok();
    ui.horizontal_wrapped(|ui| {
        for (idx, &stage) in PowerWizardStage::ALL.iter().enumerate() {
            let selected = pw.stage == stage;
            let en = if stage.index() <= pw.stage.index() || can_advance {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled(pw.can_advance().err().unwrap_or_else(|| String::from("Complete current stage parameters first.")))
            };
            if render_modal_seg_btn(ui, tokens, meta, &format!("power.stage.{}", idx + 1), &format!("{}. {}", idx + 1, stage.label()), selected, &en)
            {
                let mut next = pw.clone();
                if next.step_to_stage(stage).is_ok() {
                    intents.push_update_power_wizard_state(next);
                }
            }
        }
    });
}

fn render_stage1_design(
    ui: &mut Ui,
    app: &UiApp,
    pw: &PowerWizardState,
    state: &mut PowerStudioEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let vs = &app.view_shards.power_wizard;
    render_section_title(ui, tokens, "1. Hypothesis Test Family");

    for (key, lbl, fam, is_surv) in [
        ("two_sample_t", "Unpaired Two-Group t-test", PowerTestFamily::TwoSampleUnpairedT, false),
        ("paired_t", "Paired / Matched t-test", PowerTestFamily::TwoSamplePairedT, false),
        ("one_sample_t", "One-Sample t-test", PowerTestFamily::OneSampleT, false),
        ("one_way_anova", "One-Way ANOVA (k groups)", PowerTestFamily::OneWayAnova, false),
        ("two_proportions", "Two Independent Proportions", PowerTestFamily::TwoProportions, false),
        ("survival_log_rank", "Survival Log-Rank (Schoenfeld)", PowerTestFamily::TwoProportions, true),
        ("correlation", "Pearson / Spearman Correlation", PowerTestFamily::Correlation, false),
        ("chi_square", "Chi-Square Goodness / Contingency", PowerTestFamily::ChiSquare, false),
        ("simple_linear", "Simple Linear Regression Slope", PowerTestFamily::SimpleLinearRegression, false),
    ] {
        let active =
            if is_surv { vs.survival_log_rank && pw.family == PowerTestFamily::TwoProportions } else { !vs.survival_log_rank && pw.family == fam };
        if render_modal_radio(ui, tokens, meta, &format!("power.family.{key}"), lbl, active, &ControlEnablement::enabled()) {
            intents.push_custom_view_action("power_wizard", format!("family:{key}"));
        }
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, "Solve For");
    ui.horizontal_wrapped(|ui| {
        for (key, lbl, target) in [
            ("sample_size", "Sample Size (N)", PowerSolveTarget::SampleSize),
            ("power", "Power (1 - β)", PowerSolveTarget::Power),
            ("effect", "Detectable Effect", PowerSolveTarget::DetectableEffect),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, &format!("power.solve.{key}"), lbl, pw.solve_target == target, &ControlEnablement::enabled()) {
                let mut next = pw.clone();
                next.solve_target = target;
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        }
    });

    if pw.family == PowerTestFamily::OneWayAnova {
        ui.add_space(4.0);
        let step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "power.design.anova_groups",
                label: "Number of Groups (k)",
                value_text: &pw.anova_groups.to_string(),
                enablement: &ControlEnablement::enabled(),
            },
        );
        if step != 0 {
            let mut next = pw.clone();
            next.anova_groups = (next.anova_groups as isize + step as isize).clamp(2, 24) as usize;
            let _ = next.recalculate();
            intents.push_update_power_wizard_state(next);
        }
    } else if pw.family == PowerTestFamily::ChiSquare {
        ui.add_space(4.0);
        let step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "power.design.chi2_df",
                label: "Chi-Square Degrees of Freedom (df)",
                value_text: &format!("{:.0}", pw.chi_square_df),
                enablement: &ControlEnablement::enabled(),
            },
        );
        if step != 0 {
            let mut next = pw.clone();
            next.chi_square_df = (next.chi_square_df + f64::from(step)).clamp(1.0, 40.0);
            let _ = next.recalculate();
            intents.push_update_power_wizard_state(next);
        }
    }

    ui.add_space(8.0);
    if render_modal_action_button(
        ui,
        tokens,
        meta,
        "power.compute_defaults",
        "Compute Immediately with Defaults →",
        true,
        false,
        &ControlEnablement::enabled(),
    ) {
        let mut next = pw.clone();
        let _ = next.recalculate();
        next.stage = PowerWizardStage::Results;
        state.results_tab = PowerResultsSubTab::NarrativeReport;
        intents.push_update_power_wizard_state(next);
    }
}

fn render_stage2_effect(ui: &mut Ui, pw: &PowerWizardState, tokens: &Tokens, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    render_section_title(ui, tokens, "2. Effect Size & Pilot Calculator");
    ui.horizontal_wrapped(|ui| {
        if render_modal_seg_btn(
            ui,
            tokens,
            meta,
            "power.effect_mode.pilot",
            "Pilot Data Inputs",
            matches!(pw.effect_mode, PowerEffectInputMode::MeansAndSd | PowerEffectInputMode::Proportions | PowerEffectInputMode::RegressionSlope),
            &ControlEnablement::enabled(),
        ) {
            intents.push_custom_view_action("power_wizard", "effect_mode:pilot");
        }
        if render_modal_seg_btn(
            ui,
            tokens,
            meta,
            "power.effect_mode.direct",
            "Direct Standardized Effect",
            pw.effect_mode == PowerEffectInputMode::Standardized,
            &ControlEnablement::enabled(),
        ) {
            intents.push_custom_view_action("power_wizard", "effect_mode:direct");
        }
    });

    ui.add_space(4.0);
    render_section_title(ui, tokens, "Cohen Benchmarks");
    ui.horizontal_wrapped(|ui| {
        for (k, lbl) in [("small", "Small"), ("medium", "Medium"), ("large", "Large")] {
            if render_modal_seg_btn(ui, tokens, meta, &format!("power.pilot.preset.{k}"), lbl, false, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("power_wizard", format!("pilot_preset:{k}"));
            }
        }
    });

    ui.add_space(6.0);
    let eff = pw.derived_standardized_effect().unwrap_or(pw.standardized_effect);
    match pw.family {
        PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT => {
            let step_md = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "power.pilot.mean_diff",
                    label: "Expected Mean Difference",
                    value_text: &format!("{:.2}", pw.mean_diff),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_md != 0 {
                let mut next = pw.clone();
                next.mean_diff = (next.mean_diff + f64::from(step_md) * 0.5).max(0.05);
                if let Some(d) = next.derived_standardized_effect() {
                    next.standardized_effect = d;
                }
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
            let step_sd = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "power.pilot.sd",
                    label: "Common SD (σ)",
                    value_text: &format!("{:.2}", pw.sd),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_sd != 0 {
                let mut next = pw.clone();
                next.sd = (next.sd + f64::from(step_sd) * 0.5).max(0.05);
                if let Some(d) = next.derived_standardized_effect() {
                    next.standardized_effect = d;
                }
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        }
        PowerTestFamily::TwoProportions => {
            let s1 = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "power.pilot.p1",
                    label: "Proportion p₁",
                    value_text: &format!("{:.2}", pw.p1),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if s1 != 0 {
                let mut next = pw.clone();
                next.p1 = (next.p1 + f64::from(s1) * 0.05).clamp(0.02, 0.98);
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
            let s2 = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "power.pilot.p2",
                    label: "Proportion p₂",
                    value_text: &format!("{:.2}", pw.p2),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if s2 != 0 {
                let mut next = pw.clone();
                next.p2 = (next.p2 + f64::from(s2) * 0.05).clamp(0.02, 0.98);
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        }
        _ => {
            let s_std = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "power.pilot.std",
                    label: pw.family.default_effect_label(),
                    value_text: &format!("{:.3}", pw.standardized_effect),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if s_std != 0 {
                let mut next = pw.clone();
                next.standardized_effect = (next.standardized_effect + f64::from(s_std) * 0.05).clamp(0.05, 5.0);
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        }
    }

    ui.add_space(6.0);
    render_clipped_text(
        ui,
        meta,
        ClippedTextSpec {
            widget_id: "power.effect.derived_summary",
            role: "status",
            text: &format!("Derived Standardized {}: {eff:.3}", pw.family.default_effect_label()),
            font_size: 12.0,
            color: tokens.accent_strong,
            z_layer: ZLayer::ModalDialog,
            tooltip: None,
            record_meta: true,
        },
    );
}

fn render_stage3_alpha_power(
    ui: &mut Ui,
    app: &UiApp,
    pw: &PowerWizardState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "3. Hypothesis Tail, Alpha & Target Power");
    let one_en = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "power.one_sided");
    if render_modal_radio(ui, tokens, meta, "power.two_tailed", "Two-tailed test", pw.two_tailed, &ControlEnablement::enabled()) {
        let mut next = pw.clone();
        next.two_tailed = true;
        let _ = next.recalculate();
        intents.push_update_power_wizard_state(next);
    }
    if render_modal_radio(ui, tokens, meta, "power.one_sided", "One-sided test", !pw.two_tailed, &one_en) {
        let mut next = pw.clone();
        next.two_tailed = false;
        let _ = next.recalculate();
        intents.push_update_power_wizard_state(next);
    }

    if !pw.two_tailed && one_en.enabled {
        ui.horizontal_wrapped(|ui| {
            if render_modal_seg_btn(ui, tokens, meta, "power.tail.greater", "H₁ > 0 (Greater)", pw.one_sided_greater, &ControlEnablement::enabled())
            {
                let mut next = pw.clone();
                next.one_sided_greater = true;
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
            if render_modal_seg_btn(ui, tokens, meta, "power.tail.less", "H₁ < 0 (Less)", !pw.one_sided_greater, &ControlEnablement::enabled()) {
                let mut next = pw.clone();
                next.one_sided_greater = false;
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        });
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, "Significance Threshold (Alpha)");
    ui.horizontal_wrapped(|ui| {
        for (k, val) in [("010", 0.10), ("005", 0.05), ("001", 0.01), ("0001", 0.001)] {
            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                &format!("power.stage3.alpha.{k}"),
                &format!("α={val}"),
                (pw.alpha - val).abs() < 1e-4,
                &ControlEnablement::enabled(),
            ) {
                let mut next = pw.clone();
                next.alpha = val;
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, "Target Power (1 - β)");
    ui.horizontal_wrapped(|ui| {
        for (k, val) in [("80", 0.80), ("85", 0.85), ("90", 0.90), ("95", 0.95)] {
            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                &format!("power.stage3.power.{k}"),
                &format!("{:.0}%", val * 100.0),
                (pw.target_power - val).abs() < 1e-3,
                &ControlEnablement::enabled(),
            ) {
                let mut next = pw.clone();
                next.target_power = val;
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
        }
    });

    if pw.family == PowerTestFamily::TwoProportions {
        ui.add_space(6.0);
        render_section_title(ui, tokens, "Proportions Variance Method");
        ui.horizontal_wrapped(|ui| {
            for (k, lbl, m) in [
                ("pooled", "Pooled Null / Unpooled Alt", TwoProportionsMethod::PooledNullUnpooledAlt),
                ("unpooled", "Unpooled Wald SE", TwoProportionsMethod::Unpooled),
                ("cohens_h", "Arcsine Cohen's h", TwoProportionsMethod::CohensH),
            ] {
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("power.prop_method.{k}"),
                    lbl,
                    pw.two_proportions_mode == m,
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = pw.clone();
                    next.two_proportions_mode = m;
                    let _ = next.recalculate();
                    intents.push_update_power_wizard_state(next);
                }
            }
        });
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "power.continuity_correction",
            "Apply continuity correction",
            pw.continuity_correction,
            &ControlEnablement::enabled(),
        ) {
            let mut next = pw.clone();
            next.continuity_correction = !pw.continuity_correction;
            let _ = next.recalculate();
            intents.push_update_power_wizard_state(next);
        }
    } else if pw.family == PowerTestFamily::Correlation {
        ui.add_space(6.0);
        render_section_title(ui, tokens, "Correlation Critical Estimator");
        ui.horizontal_wrapped(|ui| {
            for (k, lbl, m) in
                [("fisher_z", "Fisher z", CorrelationPowerMethod::FisherZ), ("exact_t", "Exact t-transform", CorrelationPowerMethod::ExactTTransform)]
            {
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("power.corr_method.{k}"),
                    lbl,
                    pw.correlation_method == m,
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = pw.clone();
                    next.correlation_method = m;
                    let _ = next.recalculate();
                    intents.push_update_power_wizard_state(next);
                }
            }
        });
    }
}

fn render_stage4_results(
    ctx: &Context,
    ui: &mut Ui,
    pw: &PowerWizardState,
    state: &mut PowerStudioEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.horizontal_wrapped(|ui| {
        for (tab, wid, lbl) in [
            (PowerResultsSubTab::NarrativeReport, "power.tab.narrative", "Narrative Report"),
            (PowerResultsSubTab::PowerCurve, "power.tab.curve", "Power Curve"),
            (PowerResultsSubTab::SensitivityMatrix, "power.tab.sensitivity", "Sensitivity Matrix"),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, state.results_tab == tab, &ControlEnablement::enabled()) {
                state.results_tab = tab;
            }
        }
    });
    ui.add_space(6.0);

    let Some(res) = &pw.result else {
        ui.label("Solving design parameters...");
        return;
    };

    match state.results_tab {
        PowerResultsSubTab::NarrativeReport => {
            let narrative = build_narrative_paragraph(pw, res);
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Vec2::new(10.0, 8.0))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Methods & Sample Size Narrative").strong().color(tokens.accent_strong));
                    ui.add_space(4.0);
                    let resp = ui.label(egui::RichText::new(&narrative).color(tokens.text_primary));
                    record_widget(
                        ui,
                        meta,
                        &resp,
                        "power.narrative.text",
                        "label",
                        &narrative,
                        None,
                        Some(json!(narrative)),
                        true,
                        None,
                        ZLayer::ModalDialog,
                        false,
                        None,
                    );
                });
            ui.add_space(6.0);
            if render_modal_action_button(
                ui,
                tokens,
                meta,
                "power.copy_narrative",
                if state.narrative_copied { "Copied to Clipboard ✓" } else { "Copy Narrative Report" },
                false,
                state.narrative_copied,
                &ControlEnablement::enabled(),
            ) {
                ctx.copy_text(narrative);
                state.narrative_copied = true;
            }
        }
        PowerResultsSubTab::PowerCurve => {
            render_power_curve_plot(ui, res, state, tokens, meta);
        }
        PowerResultsSubTab::SensitivityMatrix => {
            render_sensitivity_matrix_table(ui, pw, res, tokens, meta, intents);
        }
    }
}

fn render_power_curve_plot(ui: &mut Ui, res: &PowerAnalysisResult, state: &mut PowerStudioEguiState, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    render_section_title(ui, tokens, "Interactive Power vs. Sample Size Curve");
    let w = ui.available_width().clamp(200.0, 560.0);
    let h = 210.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(6), tokens.panel_bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 38.0, rect.top() + 14.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 24.0));
    ui.painter().line_segment([plot.left_bottom(), plot.right_bottom()], Stroke::new(1.0, tokens.text_muted));
    ui.painter().line_segment([plot.left_top(), plot.left_bottom()], Stroke::new(1.0, tokens.text_muted));

    let min_n = res.power_curve.first().map_or(2.0, |p| p.n).max(1.0);
    let max_n = res.power_curve.last().map_or((res.n_per_group as f64 * 2.0).max(20.0), |p| p.n).max(min_n + 1.0);
    let to_screen = |n: f64, pwr: f64| -> Pos2 {
        let fx = ((n - min_n) / (max_n - min_n)).clamp(0.0, 1.0) as f32;
        let fy = pwr.clamp(0.0, 1.0) as f32;
        Pos2::new(plot.left() + fx * plot.width(), plot.bottom() - fy * plot.height())
    };

    // Target power horizontal threshold
    if let Some(tp) = res.target_power {
        let y_tp = plot.bottom() - (tp.clamp(0.0, 1.0) as f32) * plot.height();
        ui.painter().line_segment([Pos2::new(plot.left(), y_tp), Pos2::new(plot.right(), y_tp)], Stroke::new(1.0, tokens.accent));
    }

    // Power curve polyline
    if res.power_curve.len() >= 2 {
        let pts: Vec<Pos2> = res.power_curve.iter().map(|pt| to_screen(pt.n, pt.power)).collect();
        ui.painter().add(egui::Shape::line(pts, Stroke::new(2.2, tokens.accent_strong)));
    }

    // Solved (n_per_group, achieved_power) focal dot
    let solved_pt = to_screen(res.n_per_group as f64, res.achieved_power);
    ui.painter().circle_filled(solved_pt, 4.5, tokens.accent_strong);

    // Interactive pointer crosshair hover
    let mut hover_desc = format!("Solved: N = {}, Power = {:.1}%", res.n_per_group, res.achieved_power * 100.0);
    if let Some(pos) = resp.hover_pos().filter(|p| plot.contains(*p)) {
        let frac = ((pos.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0);
        let n_hover = min_n + f64::from(frac) * (max_n - min_n);
        let nearest = res.power_curve.iter().min_by(|a, b| (a.n - n_hover).abs().total_cmp(&(b.n - n_hover).abs()));
        if let Some(pt) = nearest {
            let hp = to_screen(pt.n, pt.power);
            state.hover_n = Some(pt.n.round().max(1.0) as u32);
            ui.painter().line_segment([Pos2::new(hp.x, plot.top()), Pos2::new(hp.x, plot.bottom())], Stroke::new(1.0, tokens.border));
            ui.painter().circle_filled(hp, 4.0, tokens.link_blue);
            hover_desc = format!("N = {:.0}, Power = {:.1}%", pt.n, pt.power * 100.0);
        }
    }

    ui.painter().text(Pos2::new(plot.right(), plot.top() + 2.0), Align2::RIGHT_TOP, &hover_desc, FontId::proportional(10.5), tokens.text_strong);
    ui.painter().text(
        Pos2::new(plot.left(), plot.bottom() + 4.0),
        Align2::LEFT_TOP,
        format!("N={min_n:.0}"),
        FontId::proportional(10.0),
        tokens.text_muted,
    );
    ui.painter().text(
        Pos2::new(plot.right(), plot.bottom() + 4.0),
        Align2::RIGHT_TOP,
        format!("N={max_n:.0}"),
        FontId::proportional(10.0),
        tokens.text_muted,
    );

    record_widget(
        ui,
        meta,
        &resp,
        "power.curve_hover",
        "image",
        &hover_desc,
        None,
        Some(json!({ "hover_n": state.hover_n, "sensitivity_len": res.sensitivity_table.len() })),
        true,
        None,
        ZLayer::ModalDialog,
        false,
        None,
    );
}

fn render_sensitivity_matrix_table(
    ui: &mut Ui,
    pw: &PowerWizardState,
    res: &PowerAnalysisResult,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Sensitivity Matrix (α × Target Power)");
    egui::Grid::new("power_sensitivity_grid").striped(true).num_columns(5).show(ui, |ui| {
        for hdr in ["α", "Power", "N/grp", "Total N", "Select"] {
            ui.label(egui::RichText::new(hdr).strong().small().color(tokens.text_strong));
        }
        ui.end_row();
        for (idx, cell) in res.sensitivity_table.iter().enumerate() {
            ui.label(egui::RichText::new(format!("{:.3}", cell.alpha)).small());
            ui.label(egui::RichText::new(format!("{:.0}%", cell.target_power * 100.0)).small());
            ui.label(egui::RichText::new(cell.sample_size.to_string()).small());
            ui.label(egui::RichText::new(cell.total_n.to_string()).small());
            let active = (pw.alpha - cell.alpha).abs() < 1e-4 && (pw.target_power - cell.target_power).abs() < 1e-3;
            if render_modal_seg_btn(ui, tokens, meta, &format!("power.sensitivity.row.{idx}"), "Apply", active, &ControlEnablement::enabled()) {
                let mut next = pw.clone();
                next.alpha = cell.alpha;
                next.target_power = cell.target_power;
                let _ = next.recalculate();
                intents.push_update_power_wizard_state(next);
            }
            ui.end_row();
        }
    });
}

fn render_live_sidebar(
    ui: &mut Ui,
    pw: &PowerWizardState,
    state: &mut PowerStudioEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Live Sensitivity Controls");
    let alpha_out = render_scrub_unit_field(
        ui,
        pw.alpha,
        &mut state.alpha_spinner,
        ScrubUnitFieldConfig {
            widget_id: "power.sidebar.alpha",
            label: "Alpha (α)",
            command_id: None,
            min: 0.001,
            max: 0.25,
            step_per_pt: 0.001,
            decimals: 3,
            enabled: true,
            disabled_reason: None,
            allow_unit_switch: false,
            z_layer: ZLayer::ModalDialog,
        },
        tokens,
        meta,
    );
    if let Some(next_alpha) = alpha_out.committed_value {
        let mut next = pw.clone();
        next.alpha = next_alpha.clamp(0.001, 0.25);
        let _ = next.recalculate();
        intents.push_update_power_wizard_state(next);
    }

    ui.add_space(4.0);
    let pwr_enabled = pw.solve_target != PowerSolveTarget::Power;
    let pwr_out = render_scrub_unit_field(
        ui,
        pw.target_power * 100.0,
        &mut state.power_spinner,
        ScrubUnitFieldConfig {
            widget_id: "power.sidebar.power",
            label: "Target Power",
            command_id: None,
            min: 50.0,
            max: 99.5,
            step_per_pt: 0.25,
            decimals: 1,
            enabled: pwr_enabled,
            disabled_reason: (!pwr_enabled).then_some("Target power is solved automatically when solving for Power."),
            allow_unit_switch: false,
            z_layer: ZLayer::ModalDialog,
        },
        tokens,
        meta,
    );
    if let Some(next_pct) = pwr_out.committed_value {
        let mut next = pw.clone();
        next.target_power = (next_pct / 100.0).clamp(0.50, 0.995);
        let _ = next.recalculate();
        intents.push_update_power_wizard_state(next);
    }

    if pw.solve_target != PowerSolveTarget::SampleSize {
        ui.add_space(4.0);
        let step_n = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "power.sidebar.n",
                label: "Fixed N/group",
                value_text: &format!("{:.0}", pw.sample_size),
                enablement: &ControlEnablement::enabled(),
            },
        );
        if step_n != 0 {
            let mut next = pw.clone();
            next.sample_size = (next.sample_size + f64::from(step_n) * 2.0).clamp(4.0, 10_000.0);
            let _ = next.recalculate();
            intents.push_update_power_wizard_state(next);
        }
    }

    ui.add_space(8.0);
    render_section_title(ui, tokens, "Solved Readout");
    if let Some(res) = &pw.result {
        for (lbl, val) in [
            ("Required N/group", res.n_per_group.to_string()),
            ("Total Sample Size", res.total_n.to_string()),
            ("Achieved Power", format!("{:.1}%", res.achieved_power * 100.0)),
            ("Standardized Effect", format!("{:.3}", res.effect_size)),
            ("Critical Threshold", format!("{:.3}", res.critical_value)),
        ] {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(lbl).small().color(tokens.text_muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(val).small().strong().color(tokens.text_strong));
                });
            });
        }
    }
}

fn build_narrative_paragraph(pw: &PowerWizardState, res: &PowerAnalysisResult) -> String {
    let tail = if pw.two_tailed { "two-tailed" } else { "one-tailed" };
    format!(
        "For a {} {} test at α = {:.3} targeting {:.1}% statistical power with standardized effect {} = {:.3}, a required sample size of {} per group (total N = {}) achieves {:.1}% power (critical threshold = {:.3}).",
        tail,
        res.family.label(),
        res.alpha,
        pw.target_power * 100.0,
        pw.family.default_effect_label(),
        res.effect_size,
        res.n_per_group,
        res.total_n,
        res.achieved_power * 100.0,
        res.critical_value,
    )
}
