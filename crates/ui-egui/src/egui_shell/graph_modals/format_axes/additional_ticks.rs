//! `Additional Ticks / Thresholds` sub-modal body inside `Format Axes`.

use crate::egui_shell::UiIntent;
use curvebook_doc::{AdditionalTick, AxisThreshold};
use curvebook_palette::LineDashStyle;
use egui::Ui;

use crate::grid_view::parse_localized_number_input;
use crate::i18n::tr;

use super::super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_input, record_label};
use super::super::state::GraphModalEguiState;

pub(super) fn render_additional_ticks_submodal(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    ui.label(egui::RichText::new("Custom Ticks & Reference Thresholds").strong());
    ui.add_space(4.0);

    // Existing custom ticks table.
    if fa.axis_draft.additional_ticks.is_empty() {
        ui.label(egui::RichText::new("No additional ticks defined on this axis.").color(gctx.tokens.text_muted));
    } else {
        for (idx, tick) in fa.axis_draft.additional_ticks.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let label_part = tick.label.as_deref().unwrap_or("(auto)");
                let grid_part = if tick.draw_gridline { " + grid" } else { "" };
                ui.label(format!("Tick #{}: {:.4} [{}]{}", idx + 1, tick.value, label_part, grid_part));
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("axes.add_tick.delete.{idx}"),
                        label: tr(gctx.locale, "modal.btn.delete"),
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fa.clone();
                    next.axis_draft.additional_ticks.remove(idx);
                    next.stage_active_additional_ticks();
                    gctx.intents.push(UiIntent::UpdateFormatAxesState(Box::new(next)));
                }
            });
        }
    }

    if !fa.axis_draft.thresholds.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Thresholds & Shaded Zones:").strong());
        for (idx, th) in fa.axis_draft.thresholds.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let span = match th.shade_to {
                    Some(end) => format!("{:.3}..{:.3} (shaded)", th.value, end),
                    None => format!("{:.3} (line)", th.value),
                };
                ui.label(format!("Threshold #{}: {span} {}", idx + 1, th.label.as_deref().unwrap_or("")));
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("axes.threshold.delete.{idx}"),
                        label: tr(gctx.locale, "modal.btn.delete"),
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fa.clone();
                    next.axis_draft.thresholds.remove(idx);
                    next.stage_active_thresholds();
                    gctx.intents.push(UiIntent::UpdateFormatAxesState(Box::new(next)));
                }
            });
        }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Value:");
        let v_resp = ui.add(egui::TextEdit::singleline(&mut state.additional_ticks.value_draft).desired_width(86.0));
        record_input(ui, &v_resp, gctx.meta, "axes.add_tick.value", "Value", &state.additional_ticks.value_draft, true);
        ui.label("Label:");
        let l_resp = ui.add(egui::TextEdit::singleline(&mut state.additional_ticks.label_draft).desired_width(120.0));
        record_input(ui, &l_resp, gctx.meta, "axes.add_tick.label", "Label", &state.additional_ticks.label_draft, true);
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let grid_lbl = tr(gctx.locale, "axes.draw_gridline");
        let grid_chk = ui.checkbox(&mut state.additional_tick_draw_gridline, grid_lbl);
        record_checkbox(ui, &grid_chk, gctx.meta, "axes.add_tick.gridline", grid_lbl, state.additional_tick_draw_gridline);

        let line_lbl = tr(gctx.locale, "axes.show_threshold_rule");
        let line_chk = ui.checkbox(&mut state.additional_ticks.show_line, line_lbl);
        record_checkbox(ui, &line_chk, gctx.meta, "axes.add_tick.show_line", line_lbl, state.additional_ticks.show_line);

        let shade_lbl = tr(gctx.locale, "axes.shaded_zone_band");
        let shade_chk = ui.checkbox(&mut state.additional_ticks.shaded_zone, shade_lbl);
        record_checkbox(ui, &shade_chk, gctx.meta, "axes.add_tick.shaded_zone", shade_lbl, state.additional_ticks.shaded_zone);
    });

    if state.additional_ticks.shaded_zone {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("End Value (Shade To):");
            let end_resp = ui.add(egui::TextEdit::singleline(&mut state.additional_ticks.end_value_draft).desired_width(86.0));
            record_input(ui, &end_resp, gctx.meta, "axes.add_tick.end_value", "End Value", &state.additional_ticks.end_value_draft, true);
        });
    }

    if let Some(err) = &state.additional_tick_status {
        let err_resp = ui.colored_label(gctx.tokens.link_red, err);
        record_label(ui, &err_resp, gctx.meta, "axes.add_tick.error", err);
    }

    ui.add_space(6.0);
    if record_button(
        ui,
        gctx.meta,
        ButtonEntry {
            widget_id: "axes.add_tick.save",
            label: tr(gctx.locale, "axes.add_tick.save"),
            command_id: None,
            selected: true,
            enabled: true,
            disabled_reason: None,
        },
        gctx.tokens,
    )
    .clicked()
    {
        match parse_localized_number_input(&state.additional_ticks.value_draft, &gctx.app.locale_preset) {
            Some(val) => {
                let label_opt = (!state.additional_ticks.label_draft.trim().is_empty()).then(|| state.additional_ticks.label_draft.trim().to_owned());
                let mut next = fa.clone();
                if state.additional_ticks.show_line || state.additional_ticks.shaded_zone {
                    let shade_to = if state.additional_ticks.shaded_zone {
                        match parse_localized_number_input(&state.additional_ticks.end_value_draft, &gctx.app.locale_preset) {
                            Some(end) => Some(end),
                            None => {
                                state.additional_tick_status = Some(String::from("Enter a valid finite End Value for Shaded Zone"));
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    next.axis_draft.thresholds.push(AxisThreshold {
                        value: val,
                        shade_to,
                        color: curvebook_palette::Rgba8::BLACK,
                        dash_style: LineDashStyle::Dashed,
                        stroke_width_pt: 1.0,
                        label: label_opt.clone(),
                    });
                    next.stage_active_thresholds();
                }
                if !state.additional_ticks.shaded_zone {
                    next.axis_draft.additional_ticks.push(AdditionalTick {
                        value: val,
                        label: label_opt,
                        draw_gridline: state.additional_tick_draw_gridline,
                    });
                    next.stage_active_additional_ticks();
                }
                gctx.intents.push(UiIntent::UpdateFormatAxesState(Box::new(next)));
                state.additional_tick_status = None;
                state.additional_ticks.value_draft.clear();
                state.additional_ticks.end_value_draft.clear();
                state.additional_ticks.label_draft.clear();
            }
            None => {
                state.additional_tick_status = Some(String::from("Enter a valid finite numeric value"));
            }
        }
    }
}
