//! Single-axis (`X`, `Left Y`, `Right Y2`) tab controls, categorical tick source selectors,
//! and the `Categorical Axis Guard` confirmation banner.

use curvebook_doc::{CategoricalTickSource, FramePreset, GraphAxisScale, LogitDomain, TickLabelFormat};
use egui::{CornerRadius, Stroke, Ui};

use crate::dialogs_canvas::AxisTarget;
use crate::i18n::tr;

use super::super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_label};
use super::super::state::GraphModalEguiState;
use super::segments_ticks::{render_axis_segments_section, render_axis_ticks_section};

pub(super) fn render_axes_single_axis_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    if fa.target_axis == AxisTarget::Y2 && !fa.enable_y2 {
        ui.label(egui::RichText::new("Secondary Right Y2 axis is currently disabled.").color(gctx.tokens.text_muted));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.enable_y2",
                label: tr(gctx.locale, "axes.enable_y2"),
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = fa;
            next.enable_y2 = true;
            state.format_axes_frame_preset = FramePreset::DualYSecondaryRight;
            state.format_axes_frame_preset_dirty = true;
            gctx.intents.push_update_format_axes_state(next);
        }
        return;
    }

    let is_categorical = matches!(fa.axis_draft.tick_format, TickLabelFormat::Categorical(_));
    if is_categorical {
        render_categorical_tick_controls(ui, state, gctx, &fa);
        ui.add_space(6.0);
    }

    if state.format_axes_guard_pending_scale.is_some() || state.format_axes_guard_pending_tick_format.is_some() {
        render_categorical_guard_banner(ui, state, gctx, &fa);
        ui.add_space(6.0);
    }

    // Scale selector with Categorical Guard intercept.
    ui.label(egui::RichText::new(tr(gctx.locale, "axes.scale.heading")).strong());
    let cur_scale = fa.axis_draft.segments.first().map_or(GraphAxisScale::Linear, |s| s.scale);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, scale) in [
            ("linear", "Linear", GraphAxisScale::Linear),
            ("log10", "Log10", GraphAxisScale::Log10),
            ("log2", "Log2", GraphAxisScale::Log2),
            ("ln", "Ln", GraphAxisScale::Ln),
            ("logit", "Logit", GraphAxisScale::Logit(LogitDomain::Percent)),
            ("probit", "Probit", GraphAxisScale::Probit),
            ("reciprocal", "Reciprocal", GraphAxisScale::Reciprocal),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.scale.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: cur_scale == scale,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                if is_categorical {
                    state.categorical_guard_pending_axis = Some(format!("{:?}", fa.target_axis));
                    state.format_axes_guard_pending_scale = Some(scale);
                    state.format_axes_guard_pending_tick_format = Some(TickLabelFormat::Decimal);
                    state.categorical_guard_confirmed = false;
                } else {
                    let mut next = fa.clone();
                    let _ = next.set_segment_scale(0, scale);
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        }
    });

    ui.add_space(6.0);
    // Range & Discontinuous Segments (1..=3).
    render_axis_segments_section(ui, state, gctx, &fa);

    ui.add_space(6.0);
    // Numbering / Ticks + Additional Ticks Sub-Modal button.
    render_axis_ticks_section(ui, state, gctx, &fa, is_categorical);
}

fn render_categorical_tick_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
) {
    let cur_source = match fa.axis_draft.tick_format {
        TickLabelFormat::Categorical(src) => src,
        _ => CategoricalTickSource::RowTitles,
    };
    ui.label(egui::RichText::new(tr(gctx.locale, "axes.categorical.heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, src) in [
            ("row_titles", "Row Titles", CategoricalTickSource::RowTitles),
            ("column_titles", "Column Titles", CategoricalTickSource::ColumnTitles),
            ("row_numbers", "Row Numbers", CategoricalTickSource::RowNumbers),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_source.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: cur_source == src,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fa.clone();
                next.axis_draft.tick_format = TickLabelFormat::Categorical(src);
                gctx.intents.push_update_format_axes_state(next);
            }
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.switch_to_continuous",
                label: tr(gctx.locale, "axes.categorical.switch_btn"),
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.categorical_guard_pending_axis = Some(format!("{:?}", fa.target_axis));
            state.format_axes_guard_pending_scale = Some(GraphAxisScale::Linear);
            state.format_axes_guard_pending_tick_format = Some(TickLabelFormat::Decimal);
            state.categorical_guard_confirmed = false;
        }
    });
}

fn render_categorical_guard_banner(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
) {
    let warn_text = tr(gctx.locale, "axes.guard.warning");
    egui::Frame::group(ui.style())
        .fill(gctx.tokens.header_bg)
        .stroke(Stroke::new(1.5, gctx.tokens.accent_strong))
        .corner_radius(CornerRadius::same(6))
        .show(ui, |ui| {
            ui.set_max_width((gctx.body_w - 16.0).max(180.0));
            let warn_resp = ui.add(egui::Label::new(egui::RichText::new(warn_text).color(gctx.tokens.link_red).strong()).wrap());
            record_label(ui, &warn_resp, gctx.meta, "axes.categorical_guard.warning", warn_text);

            let confirm_chk_lbl = tr(gctx.locale, "axes.guard.confirm_chk");
            let chk = ui.checkbox(&mut state.categorical_guard_confirmed, confirm_chk_lbl);
            record_checkbox(ui, &chk, gctx.meta, "axes.categorical_guard.confirm_checkbox", confirm_chk_lbl, state.categorical_guard_confirmed);

            ui.horizontal_wrapped(|ui| {
                let can_confirm = state.categorical_guard_confirmed;
                let proceed_lbl = tr(gctx.locale, "axes.guard.confirm_btn");
                let disabled_reason = (!can_confirm).then_some("Check confirmation above to switch categorical axis to numeric scale");
                let proceed = record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "axes.categorical_guard.confirm_btn",
                        label: proceed_lbl,
                        command_id: None,
                        selected: can_confirm,
                        enabled: can_confirm,
                        disabled_reason,
                    },
                    gctx.tokens,
                );
                if proceed.clicked() && can_confirm {
                    let mut next = fa.clone();
                    next.axis_draft.tick_format = state.format_axes_guard_pending_tick_format.unwrap_or(TickLabelFormat::Decimal);
                    if let Some(scale) = state.format_axes_guard_pending_scale {
                        let _ = next.set_segment_scale(0, scale);
                    }
                    gctx.intents.push_update_format_axes_state(next);
                    state.categorical_guard_confirmed = false;
                    state.categorical_guard_pending_axis = None;
                    state.format_axes_guard_pending_scale = None;
                    state.format_axes_guard_pending_tick_format = None;
                }

                let abort_lbl = tr(gctx.locale, "axes.guard.cancel_btn");
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "axes.categorical_guard.cancel_btn",
                        label: abort_lbl,
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    state.categorical_guard_confirmed = false;
                    state.categorical_guard_pending_axis = None;
                    state.format_axes_guard_pending_scale = None;
                    state.format_axes_guard_pending_tick_format = None;
                }
            });
        });
}
