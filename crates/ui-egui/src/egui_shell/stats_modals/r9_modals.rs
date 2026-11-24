//! `dialogs_r9` data-generation modals (`RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`).

use egui::{Context, Key, Modifiers, Ui, Vec2};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::dialogs_r9::{RandomizationSchemeKind, SimDistKind};
use crate::i18n::{normalize_locale, tr};
use crate::modal_predicates::ControlEnablement;

use crate::egui_shell::stat_anova_modal::{
    StepperSpec, column_titles_for_draft, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title, render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use super::helpers::{Wave4ModalShell, render_modal_action_button, show_wave4_modal};
use super::{StatsModalFrameReport, StatsModalsEguiState};

// ============================================================================
// 3. R9 Simulation/Sampling/Randomize & ResultViewer Modals
// ============================================================================

/// Render `dialogs_r9` generation modals (`RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`).
pub fn render_dialogs_r9_modal(
    ctx: &Context,
    app: &UiApp,
    _state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let locale = normalize_locale(&app.locale);
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::RandomizeGroups => render_randomize_groups_modal(ctx, app, locale, tokens, meta, intents),
        ActiveDialog::RandomSampleRows { .. } => render_random_sample_modal(ctx, app, locale, tokens, meta, intents),
        ActiveDialog::SimulateDistributions => render_simulate_dist_modal(ctx, app, locale, tokens, meta, intents),
        _ => StatsModalFrameReport::default(),
    }
}

fn render_randomize_groups_modal(
    ctx: &Context,
    app: &UiApp,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let rg = &app.canvas_dialogs.randomize_groups;
    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.randomize_groups",
            title: tr(locale, "w6.view.modal.randomize_groups"),
            subtitle_or_banner: Some("Generate balanced or permuted-block treatment group assignments."),
            banner_widget_id: "rand_groups.banner",
            max_width_pt: 580.0,
            max_height_pt: 460.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _, _w, _h, meta, intents| {
            render_section_title(ui, tokens, "Treatment Arms Presets");
            ui.horizontal_wrapped(|ui| {
                for (wid, lbl, raw) in [
                    ("rand_groups.p2", "2 Arms (Ctrl / Tx)", "Control, Treatment"),
                    ("rand_groups.p3", "3 Arms (Ctrl / Low / High)", "Control, Low Dose, High Dose"),
                    ("rand_groups.pw", "Weighted 1:2", "Control:1, Treatment:2"),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, false, &ControlEnablement::enabled()) {
                        let mut next = rg.clone();
                        let _ = next.set_groups_from_str(raw);
                        intents.push(UiIntent::UpdateRandomizeGroupsState(Box::new(next)));
                    }
                }
            });
            let step_n = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "rand_groups.n",
                    label: "Total Subjects (N)",
                    value_text: &rg.subject_count.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_n != 0 {
                let mut next = rg.clone();
                next.subject_count = if step_n > 0 { (rg.subject_count + 4).min(500) } else { rg.subject_count.saturating_sub(4).max(2) };
                intents.push(UiIntent::UpdateRandomizeGroupsState(Box::new(next)));
            }
            ui.add_space(4.0);
            render_section_title(ui, tokens, "Randomization Scheme");
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "rand_groups.sch.balanced",
                "Complete balanced allocation",
                rg.scheme == RandomizationSchemeKind::CompleteBalanced,
                &ControlEnablement::enabled(),
            ) {
                let mut next = rg.clone();
                next.scheme = RandomizationSchemeKind::CompleteBalanced;
                intents.push(UiIntent::UpdateRandomizeGroupsState(Box::new(next)));
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "rand_groups.sch.block",
                "Permuted block randomization",
                rg.scheme == RandomizationSchemeKind::PermutedBlock,
                &ControlEnablement::enabled(),
            ) {
                let mut next = rg.clone();
                next.scheme = RandomizationSchemeKind::PermutedBlock;
                intents.push(UiIntent::UpdateRandomizeGroupsState(Box::new(next)));
            }
            if rg.scheme == RandomizationSchemeKind::PermutedBlock {
                let blk = rg.block_sizes.first().copied().unwrap_or(4);
                let step_b = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        id_prefix: "rand_groups.block",
                        label: "Block Size",
                        value_text: &blk.to_string(),
                        enablement: &ControlEnablement::enabled(),
                    },
                );
                if step_b != 0 {
                    let mut next = rg.clone();
                    let next_blk = if step_b > 0 { (blk + 2).min(64) } else { blk.saturating_sub(2).max(2) };
                    next.block_sizes = vec![next_blk];
                    intents.push(UiIntent::UpdateRandomizeGroupsState(Box::new(next)));
                }
            }
            let step_seed = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "rand_groups.seed",
                    label: "PRNG Seed",
                    value_text: &rg.seed.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_seed != 0 {
                let mut next = rg.clone();
                next.seed = if step_seed > 0 { rg.seed + 1 } else { rg.seed.saturating_sub(1).max(1) };
                intents.push(UiIntent::UpdateRandomizeGroupsState(Box::new(next)));
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                }
            });
        },
    );
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    } else if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    StatsModalFrameReport::from(capture)
}

fn render_random_sample_modal(
    ctx: &Context,
    app: &UiApp,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let rs = &app.canvas_dialogs.random_sample_rows;
    let doc = app.document();
    let prop_en = if rs.stratify_column.is_none() {
        ControlEnablement::disabled("Proportional allocation requires selecting a stratification column.")
    } else {
        crate::modal_predicates::evaluate_dialog_widget(&app.canvas_dialogs, doc, "sample.proportional_allocation")
    };

    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.random_sample_rows",
            title: tr(locale, "cmd.table.randomSampleRows"),
            subtitle_or_banner: Some("Draw a reproducible simple or stratified random sample of rows."),
            banner_widget_id: "rand_sample.banner",
            max_width_pt: 660.0,
            max_height_pt: 480.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _, avail_w, _h, meta, intents| {
            let titles = column_titles_for_draft(doc, rs.source_sheet_id.as_ref());
            let render_left = |ui: &mut Ui, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents| {
                render_section_title(ui, tokens, "Source DataSheet");
                for (idx, ds) in doc.data_sheets.iter().enumerate() {
                    let selected = rs.source_sheet_id.as_ref() == Some(&ds.id);
                    if render_modal_radio(ui, tokens, meta, &format!("rand_sample.src.{idx}"), &ds.title, selected, &ControlEnablement::enabled()) {
                        let mut next = rs.clone();
                        next.source_sheet_id = Some(ds.id.clone());
                        intents.push(UiIntent::UpdateRandomSampleRowsState(Box::new(next)));
                    }
                }
                ui.add_space(4.0);
                let step_n = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        id_prefix: "rand_sample.n",
                        label: "Sample Size (rows)",
                        value_text: &rs.sample_size.to_string(),
                        enablement: &ControlEnablement::enabled(),
                    },
                );
                if step_n != 0 {
                    let mut next = rs.clone();
                    next.sample_size = if step_n > 0 { (rs.sample_size + 5).min(5000) } else { rs.sample_size.saturating_sub(5).max(1) };
                    intents.push(UiIntent::UpdateRandomSampleRowsState(Box::new(next)));
                }
                let step_s = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        id_prefix: "rand_sample.seed",
                        label: "PRNG Seed",
                        value_text: &rs.seed.to_string(),
                        enablement: &ControlEnablement::enabled(),
                    },
                );
                if step_s != 0 {
                    let mut next = rs.clone();
                    next.seed = if step_s > 0 { rs.seed + 1 } else { rs.seed.saturating_sub(1).max(1) };
                    intents.push(UiIntent::UpdateRandomSampleRowsState(Box::new(next)));
                }
            };
            let render_right = |ui: &mut Ui, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents| {
                render_section_title(ui, tokens, "Stratification");
                if render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    "sample.proportional_allocation",
                    "Proportional allocation across strata",
                    rs.proportional_allocation,
                    &prop_en,
                ) {
                    let mut next = rs.clone();
                    next.proportional_allocation = !rs.proportional_allocation;
                    intents.push(UiIntent::UpdateRandomSampleRowsState(Box::new(next)));
                }
                if render_modal_radio(
                    ui,
                    tokens,
                    meta,
                    "rand_sample.strat.none",
                    "Unstratified simple random sample",
                    rs.stratify_column.is_none(),
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = rs.clone();
                    next.stratify_column = None;
                    intents.push(UiIntent::UpdateRandomSampleRowsState(Box::new(next)));
                }
                for (ci, t) in titles.iter().enumerate() {
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("rand_sample.strat.{ci}"),
                        &format!("Stratify by: {t}"),
                        rs.stratify_column == Some(ci),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = rs.clone();
                        next.stratify_column = Some(ci);
                        intents.push(UiIntent::UpdateRandomSampleRowsState(Box::new(next)));
                    }
                }
            };
            if avail_w >= 480.0 {
                let left_w = (avail_w * 0.48).max(200.0);
                let right_w = (avail_w - left_w - 10.0).max(200.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(left_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left_w);
                        render_left(ui, meta, intents);
                    });
                    ui.add_space(10.0);
                    ui.allocate_ui_with_layout(Vec2::new(right_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(right_w);
                        render_right(ui, meta, intents);
                    });
                });
            } else {
                render_left(ui, meta, intents);
                ui.add_space(4.0);
                render_right(ui, meta, intents);
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                }
            });
        },
    );
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    } else if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    StatsModalFrameReport::from(capture)
}

fn render_simulate_dist_modal(
    ctx: &Context,
    app: &UiApp,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let sd = &app.canvas_dialogs.simulate_distributions;
    let col_idx = sd.selected_col_idx.min(sd.columns.len().saturating_sub(1));

    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.simulate_distributions",
            title: tr(locale, "cmd.table.simulateDistributions"),
            subtitle_or_banner: Some("Simulate synthetic probability columns for power/Monte Carlo benchmarks."),
            banner_widget_id: "sim_dist.banner",
            max_width_pt: 600.0,
            max_height_pt: 480.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _, _w, _h, meta, intents| {
            let step_rows = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "sim_dist.rows",
                    label: "Number of Rows",
                    value_text: &sd.n_rows.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_rows != 0 {
                let mut next = sd.clone();
                next.n_rows = if step_rows > 0 { (sd.n_rows + 25).min(10_000) } else { sd.n_rows.saturating_sub(25).max(5) };
                intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
            }
            let step_seed = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "sim_dist.seed",
                    label: "PRNG Seed",
                    value_text: &sd.seed.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_seed != 0 {
                let mut next = sd.clone();
                next.seed = if step_seed > 0 { sd.seed + 1 } else { sd.seed.saturating_sub(1).max(1) };
                intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "simulate.use_multiple_variables",
                "Create MultipleVariables table",
                sd.use_multiple_variables,
                &ControlEnablement::enabled(),
            ) {
                let mut next = sd.clone();
                next.use_multiple_variables = !sd.use_multiple_variables;
                intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
            }
            ui.add_space(4.0);
            render_section_title(ui, tokens, "Simulated Columns");
            ui.horizontal_wrapped(|ui| {
                for (i, c) in sd.columns.iter().enumerate() {
                    if render_modal_seg_btn(ui, tokens, meta, &format!("sim_dist.col.{i}"), &c.name, i == col_idx, &ControlEnablement::enabled()) {
                        let mut next = sd.clone();
                        next.selected_col_idx = i.min(next.columns.len().saturating_sub(1));
                        intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
                    }
                }
                if render_modal_seg_btn(ui, tokens, meta, "sim_dist.add_col", "+ Add Column", false, &ControlEnablement::enabled()) {
                    let mut next = sd.clone();
                    let idx = next.columns.len();
                    if idx < 12 {
                        next.columns.push(crate::dialogs_r9::SimulatedDistColumnDraft { name: format!("Var {}", idx + 1), ..Default::default() });
                        next.selected_col_idx = idx;
                    }
                    intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
                }
            });
            if let Some(col) = sd.columns.get(col_idx) {
                ui.add_space(4.0);
                render_section_title(ui, tokens, "Distribution Family");
                ui.horizontal_wrapped(|ui| {
                    for (wid, lbl, kind) in [
                        ("sim_dist.d.uni", "Uniform", SimDistKind::Uniform),
                        ("sim_dist.d.gauss", "Gaussian", SimDistKind::Gaussian),
                        ("sim_dist.d.logn", "LogNormal", SimDistKind::LogNormal),
                        ("sim_dist.d.bin", "Binomial", SimDistKind::Binomial),
                        ("sim_dist.d.pois", "Poisson", SimDistKind::Poisson),
                        ("sim_dist.d.exp", "Exponential", SimDistKind::Exponential),
                    ] {
                        if render_modal_seg_btn(ui, tokens, meta, wid, lbl, col.distribution == kind, &ControlEnablement::enabled()) {
                            let mut next = sd.clone();
                            if let Some(c) = next.columns.get_mut(col_idx) {
                                c.distribution = kind;
                            }
                            intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
                        }
                    }
                });
                let (p1_lbl, p1_val, p2_opt) = match col.distribution {
                    SimDistKind::Uniform => ("Minimum", format!("{:.2}", col.min), Some(("Maximum", format!("{:.2}", col.max)))),
                    SimDistKind::Gaussian => ("Mean (μ)", format!("{:.2}", col.mean), Some(("SD (σ)", format!("{:.2}", col.sd)))),
                    SimDistKind::LogNormal => ("Log Mean", format!("{:.2}", col.mean_log), Some(("Log SD", format!("{:.2}", col.sd_log)))),
                    SimDistKind::Binomial => ("Trials (n)", col.trials.to_string(), Some(("Prob (p)", format!("{:.2}", col.p)))),
                    SimDistKind::Poisson => ("Lambda (λ)", format!("{:.2}", col.lambda), None),
                    SimDistKind::Exponential => ("Rate (λ)", format!("{:.2}", col.rate), None),
                };
                let s1 = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec { id_prefix: "sim_dist.p1", label: p1_lbl, value_text: &p1_val, enablement: &ControlEnablement::enabled() },
                );
                if s1 != 0 {
                    let mut next = sd.clone();
                    let s = if s1 > 0 { 1.0 } else { -1.0 };
                    if let Some(c) = next.columns.get_mut(col_idx) {
                        match c.distribution {
                            SimDistKind::Uniform => c.min = (c.min + s).min(c.max - 0.5),
                            SimDistKind::Gaussian => c.mean += s,
                            SimDistKind::LogNormal => c.mean_log += s * 0.25,
                            SimDistKind::Binomial => {
                                let t = i64::try_from(c.trials).unwrap_or(10) + if s > 0.0 { 1 } else { -1 };
                                c.trials = u64::try_from(t.clamp(1, 500)).unwrap_or(10);
                            }
                            SimDistKind::Poisson => c.lambda = (c.lambda + s).clamp(0.5, 500.0),
                            SimDistKind::Exponential => c.rate = (c.rate + s * 0.25).clamp(0.05, 100.0),
                        }
                    }
                    intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
                }
                if let Some((p2_lbl, p2_val)) = p2_opt {
                    let s2 = render_stepper_row(
                        ui,
                        tokens,
                        meta,
                        StepperSpec { id_prefix: "sim_dist.p2", label: p2_lbl, value_text: &p2_val, enablement: &ControlEnablement::enabled() },
                    );
                    if s2 != 0 {
                        let mut next = sd.clone();
                        let s = if s2 > 0 { 1.0 } else { -1.0 };
                        if let Some(c) = next.columns.get_mut(col_idx) {
                            match c.distribution {
                                SimDistKind::Uniform => c.max = (c.max + s).max(c.min + 0.5),
                                SimDistKind::Gaussian => c.sd = (c.sd + s * 0.5).clamp(0.1, 1000.0),
                                SimDistKind::LogNormal => c.sd_log = (c.sd_log + s * 0.1).clamp(0.05, 10.0),
                                SimDistKind::Binomial => c.p = (c.p + s * 0.05).clamp(0.05, 0.95),
                                SimDistKind::Poisson | SimDistKind::Exponential => {}
                            }
                        }
                        intents.push(UiIntent::UpdateSimulateDistributionsState(Box::new(next)));
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                }
            });
        },
    );
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    } else if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    StatsModalFrameReport::from(capture)
}
