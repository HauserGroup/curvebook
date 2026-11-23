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
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens};

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
                for (wid, lbl, payload) in [
                    ("rand_groups.p2", "2 Arms (Ctrl / Tx)", "rand_groups:arms:two"),
                    ("rand_groups.p3", "3 Arms (Ctrl / Low / High)", "rand_groups:arms:three"),
                    ("rand_groups.pw", "Weighted 1:2", "rand_groups:arms:weighted"),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, false, &ControlEnablement::enabled()) {
                        intents.push_custom_view_action("dialogs_graph_misc", payload);
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
                intents.push_custom_view_action("dialogs_graph_misc", if step_n > 0 { "rand_groups:n_inc" } else { "rand_groups:n_dec" });
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
                intents.push_custom_view_action("dialogs_graph_misc", "rand_groups:scheme:balanced");
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
                intents.push_custom_view_action("dialogs_graph_misc", "rand_groups:scheme:block");
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
                    intents.push_custom_view_action("dialogs_graph_misc", if step_b > 0 { "rand_groups:block_inc" } else { "rand_groups:block_dec" });
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
                intents.push_custom_view_action("dialogs_graph_misc", if step_seed > 0 { "rand_groups:seed_inc" } else { "rand_groups:seed_dec" });
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
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
                        intents.push_custom_view_action("dialogs_graph_misc", format!("rand_sample:src:{idx}"));
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
                    intents.push_custom_view_action("dialogs_graph_misc", if step_n > 0 { "rand_sample:n_inc" } else { "rand_sample:n_dec" });
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
                    intents.push_custom_view_action("dialogs_graph_misc", if step_s > 0 { "rand_sample:seed_inc" } else { "rand_sample:seed_dec" });
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
                    intents.push_custom_view_action("dialogs_graph_misc", "rand_sample:proportional");
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
                    intents.push_custom_view_action("dialogs_graph_misc", "rand_sample:stratify:none");
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
                        intents.push_custom_view_action("dialogs_graph_misc", format!("rand_sample:stratify:{ci}"));
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
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
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
    let col_idx = app.view_shards.dialogs_graph_misc.sim_selected_col_idx.min(sd.columns.len().saturating_sub(1));

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
                intents.push_custom_view_action("dialogs_graph_misc", if step_rows > 0 { "sim_dist:rows_inc" } else { "sim_dist:rows_dec" });
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
                intents.push_custom_view_action("dialogs_graph_misc", if step_seed > 0 { "sim_dist:seed_inc" } else { "sim_dist:seed_dec" });
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
                intents.push_custom_view_action("dialogs_graph_misc", "sim_dist:multi_var");
            }
            ui.add_space(4.0);
            render_section_title(ui, tokens, "Simulated Columns");
            ui.horizontal_wrapped(|ui| {
                for (i, c) in sd.columns.iter().enumerate() {
                    if render_modal_seg_btn(ui, tokens, meta, &format!("sim_dist.col.{i}"), &c.name, i == col_idx, &ControlEnablement::enabled()) {
                        intents.push_custom_view_action("dialogs_graph_misc", format!("sim_dist:col:{i}"));
                    }
                }
                if render_modal_seg_btn(ui, tokens, meta, "sim_dist.add_col", "+ Add Column", false, &ControlEnablement::enabled()) {
                    intents.push_custom_view_action("dialogs_graph_misc", "sim_dist:add_col");
                }
            });
            if let Some(col) = sd.columns.get(col_idx) {
                ui.add_space(4.0);
                render_section_title(ui, tokens, "Distribution Family");
                ui.horizontal_wrapped(|ui| {
                    for (wid, slug, lbl, kind) in [
                        ("sim_dist.d.uni", "uniform", "Uniform", SimDistKind::Uniform),
                        ("sim_dist.d.gauss", "gaussian", "Gaussian", SimDistKind::Gaussian),
                        ("sim_dist.d.logn", "lognormal", "LogNormal", SimDistKind::LogNormal),
                        ("sim_dist.d.bin", "binomial", "Binomial", SimDistKind::Binomial),
                        ("sim_dist.d.pois", "poisson", "Poisson", SimDistKind::Poisson),
                        ("sim_dist.d.exp", "exponential", "Exponential", SimDistKind::Exponential),
                    ] {
                        if render_modal_seg_btn(ui, tokens, meta, wid, lbl, col.distribution == kind, &ControlEnablement::enabled()) {
                            intents.push_custom_view_action("dialogs_graph_misc", format!("sim_dist:dist:{slug}"));
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
                    intents.push_custom_view_action("dialogs_graph_misc", if s1 > 0 { "sim_dist:param:p1_inc" } else { "sim_dist:param:p1_dec" });
                }
                if let Some((p2_lbl, p2_val)) = p2_opt {
                    let s2 = render_stepper_row(
                        ui,
                        tokens,
                        meta,
                        StepperSpec { id_prefix: "sim_dist.p2", label: p2_lbl, value_text: &p2_val, enablement: &ControlEnablement::enabled() },
                    );
                    if s2 != 0 {
                        intents.push_custom_view_action("dialogs_graph_misc", if s2 > 0 { "sim_dist:param:p2_inc" } else { "sim_dist:param:p2_dec" });
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    }
    StatsModalFrameReport::from(capture)
}
