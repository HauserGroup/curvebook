//! Axis range bounds, discontinuous break segments (1..=3), and tick numbering controls
//! inside `Format Axes`.

use curvebook_doc::{BreakOrnament, GraphAxisScale, TickLabelFormat};
use egui::Ui;

use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;

use super::super::helpers::{
    ButtonEntry, GraphModalCtx, LengthSpinnerSpec, gtr, record_button, record_input, record_label, render_length_spinner_pt,
};
use super::super::state::GraphModalEguiState;

fn parse_optional_bound_draft(raw: &str, label: &str, preset: &LocalePreset) -> Result<Option<f64>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_localized_number_input(trimmed, preset).map(Some).ok_or_else(|| format!("Invalid {label} `{trimmed}`"))
}

pub(super) fn render_axis_segments_section(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
) {
    let seg0 = fa.axis_draft.segments.first().cloned().unwrap_or_default();
    let min_lbl = gtr(gctx.locale, "axes.min");
    let max_lbl = gtr(gctx.locale, "axes.max");
    let mut bounds_edited = false;

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(min_lbl);
        let min_resp = ui.add(egui::TextEdit::singleline(&mut state.axis_min_draft).hint_text("Auto").desired_width(96.0));
        record_input(ui, &min_resp, gctx.meta, "axes.seg.0.min", min_lbl, &state.axis_min_draft, true);
        if min_resp.changed() || min_resp.lost_focus() {
            bounds_edited = true;
        }

        ui.label(max_lbl);
        let max_resp = ui.add(egui::TextEdit::singleline(&mut state.axis_max_draft).hint_text("Auto").desired_width(96.0));
        record_input(ui, &max_resp, gctx.meta, "axes.seg.0.max", max_lbl, &state.axis_max_draft, true);
        if max_resp.changed() || max_resp.lost_focus() {
            bounds_edited = true;
        }

        let is_auto = state.axis_min_draft.trim().is_empty() && state.axis_max_draft.trim().is_empty() && seg0.min.is_none() && seg0.max.is_none();
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.seg.0.auto",
                label: "Auto",
                command_id: None,
                selected: is_auto,
                enabled: !is_auto,
                disabled_reason: is_auto.then_some("Axis range is already set to Auto"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.axis_min_draft.clear();
            state.axis_max_draft.clear();
            state.axis_bounds_error = None;
            let mut next = fa.clone();
            if next.set_segment_bounds(0, None, None).is_ok() {
                gctx.intents.push_update_format_axes_state(next);
            }
        }
    });

    if bounds_edited {
        match (
            parse_optional_bound_draft(&state.axis_min_draft, "axis minimum", &gctx.app.locale_preset),
            parse_optional_bound_draft(&state.axis_max_draft, "axis maximum", &gctx.app.locale_preset),
        ) {
            (Err(err), _) | (_, Err(err)) => {
                state.axis_bounds_error = Some(err);
            }
            (Ok(lo_opt), Ok(hi_opt)) => {
                let mut next = fa.clone();
                match next.set_segment_bounds(0, lo_opt, hi_opt) {
                    Ok(()) => {
                        state.axis_bounds_error = None;
                        if seg0.min != lo_opt || seg0.max != hi_opt {
                            gctx.intents.push_update_format_axes_state(next);
                        }
                    }
                    Err(err) => {
                        state.axis_bounds_error = Some(err);
                    }
                }
            }
        }
    }

    if let Some(err) = &state.axis_bounds_error {
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err).color(gctx.tokens.link_red)).wrap());
        record_label(ui, &err_resp, gctx.meta, "axes.seg.0.bounds_error", err);
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Segments: {}", fa.axis_draft.segments.len()));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.segment.add",
                label: "+ Add Break Segment",
                command_id: None,
                selected: false,
                enabled: fa.axis_draft.segments.len() < 3,
                disabled_reason: (fa.axis_draft.segments.len() >= 3).then_some("Maximum 3 axis segments supported"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = fa.clone();
            if next.add_segment() {
                gctx.intents.push_update_format_axes_state(next);
            }
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.segment.remove",
                label: "- Remove Segment",
                command_id: None,
                selected: false,
                enabled: fa.axis_draft.segments.len() > 1,
                disabled_reason: (fa.axis_draft.segments.len() <= 1).then_some("At least 1 segment required"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = fa.clone();
            let last = next.axis_draft.segments.len().saturating_sub(1);
            if next.remove_segment(last) {
                gctx.intents.push_update_format_axes_state(next);
            }
        }
    });

    if fa.axis_draft.segments.len() >= 2 {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Break Style:");
            for (slug, lbl, orn) in [
                ("slash", "// Double Slash", BreakOrnament::DoubleSlash),
                ("zigzag", "Zigzag", BreakOrnament::Zigzag),
                ("blank", "Blank Gap", BreakOrnament::BlankGap),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("axes.break_ornament.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fa.axis_draft.break_ornament == orn,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fa.clone();
                    next.axis_draft.break_ornament = orn;
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        });
        if let Some(val) = render_length_spinner_pt(
            ui,
            LengthSpinnerSpec {
                widget_id: "axes.gap_width_pt",
                label: "Break Gap Width",
                value_pt: fa.axis_draft.gap_width_pt,
                min_pt: 2.0,
                max_pt: 48.0,
                step_pt: 0.5,
                enabled: true,
                disabled_reason: None,
            },
            &mut state.axis_gap_spinner,
            gctx.tokens,
            gctx.meta,
        ) {
            let mut next = fa.clone();
            next.axis_draft.gap_width_pt = val;
            gctx.intents.push_update_format_axes_state(next);
        }
    }
}

pub(super) fn render_axis_ticks_section(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
    is_categorical: bool,
) {
    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.ticks.heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, fmt) in [
            ("decimal", "Decimal", TickLabelFormat::Decimal),
            ("power10", "10^x", TickLabelFormat::PowerOfTenSuperscript),
            ("sci_e", "1e-3", TickLabelFormat::ScientificE),
            ("antilog", "Antilog", TickLabelFormat::Antilog),
            ("si", "SI Prefix", TickLabelFormat::EngineeringSi),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_format.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fa.axis_draft.tick_format == fmt,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                if is_categorical {
                    state.categorical_guard_pending_axis = Some(format!("{:?}", fa.target_axis));
                    state.format_axes_guard_pending_scale = Some(GraphAxisScale::Linear);
                    state.format_axes_guard_pending_tick_format = Some(fmt);
                    state.categorical_guard_confirmed = false;
                } else {
                    let mut next = fa.clone();
                    next.axis_draft.tick_format = fmt;
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        }
    });

    ui.add_space(6.0);
    let tick_count = fa.axis_draft.additional_ticks.len();
    let thresh_count = fa.axis_draft.thresholds.len();
    let add_btn_lbl = format!("{} ({} / {})", gtr(gctx.locale, "axes.add_tick.open"), tick_count, thresh_count);
    if record_button(
        ui,
        gctx.meta,
        ButtonEntry {
            widget_id: "axes.additional_ticks.open",
            label: &add_btn_lbl,
            command_id: None,
            selected: false,
            enabled: true,
            disabled_reason: None,
        },
        gctx.tokens,
    )
    .clicked()
    {
        state.additional_ticks.open = true;
        state.additional_tick_status = None;
    }
}
