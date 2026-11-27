//! Axis range bounds, discontinuous break segments (1..=3), segment length shares,
//! and tick numbering controls inside `Format Axes`.

use curvebook_doc::{BreakOrnament, GraphAxisScale, TickDirection, TickLabelFormat};
use egui::Ui;

use crate::dialogs_canvas::FormatAxesState;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{LocalePreset, tr};

use super::super::helpers::{
    ButtonEntry, GraphModalCtx, LengthSpinnerSpec, record_button, record_checkbox, record_input, record_label, render_length_spinner_pt,
};
use super::super::state::{GraphModalEguiState, axis_target_slot};
use super::load_segment_drafts;

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
    draft: &mut FormatAxesState,
    dirty: &mut bool,
    seg_idx: usize,
) {
    render_segment_bounds_row(ui, state, gctx, draft, dirty, seg_idx);
    if draft.axis_draft.segments.len() >= 2 {
        ui.add_space(4.0);
        render_segment_length_share_row(ui, state, gctx, draft, dirty, seg_idx);
    }
    ui.add_space(4.0);
    render_segment_count_and_break_controls(ui, state, gctx, draft, dirty);
}

fn render_segment_bounds_row(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
    seg_idx: usize,
) {
    let seg = draft.axis_draft.segments.get(seg_idx).cloned().unwrap_or_default();
    let min_lbl = tr(gctx.locale, "axes.min");
    let max_lbl = tr(gctx.locale, "axes.max");
    let auto_lbl = tr(gctx.locale, "axes.auto");
    let mut bounds_edited = false;

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(min_lbl);
        let min_resp = ui.add(egui::TextEdit::singleline(&mut state.axis_min_draft).hint_text(auto_lbl).desired_width(96.0));
        record_input(ui, &min_resp, gctx.meta, &format!("axes.seg.{seg_idx}.min"), min_lbl, &state.axis_min_draft, true);
        if min_resp.changed() || min_resp.lost_focus() {
            bounds_edited = true;
        }

        ui.label(max_lbl);
        let max_resp = ui.add(egui::TextEdit::singleline(&mut state.axis_max_draft).hint_text(auto_lbl).desired_width(96.0));
        record_input(ui, &max_resp, gctx.meta, &format!("axes.seg.{seg_idx}.max"), max_lbl, &state.axis_max_draft, true);
        if max_resp.changed() || max_resp.lost_focus() {
            bounds_edited = true;
        }

        let is_auto = state.axis_min_draft.trim().is_empty() && state.axis_max_draft.trim().is_empty() && seg.min.is_none() && seg.max.is_none();
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: &format!("axes.seg.{seg_idx}.auto"),
                label: auto_lbl,
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
            if draft.set_segment_bounds(seg_idx, None, None).is_ok() {
                *dirty = true;
            }
            bounds_edited = false;
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
            (Ok(lo_opt), Ok(hi_opt)) => match draft.set_segment_bounds(seg_idx, lo_opt, hi_opt) {
                Ok(()) => {
                    state.axis_bounds_error = None;
                    if seg.min != lo_opt || seg.max != hi_opt {
                        *dirty = true;
                    }
                }
                Err(err) => {
                    state.axis_bounds_error = Some(err);
                }
            },
        }
    }

    if let Some(err) = &state.axis_bounds_error {
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err).color(gctx.tokens.link_red)).wrap());
        record_label(ui, &err_resp, gctx.meta, &format!("axes.seg.{seg_idx}.bounds_error"), err);
    }
}

fn render_segment_length_share_row(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
    seg_idx: usize,
) {
    let cur_share = draft.axis_draft.segments.get(seg_idx).map_or(1.0, |s| s.length_share);
    let mut share_edited = false;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Length Share:");
        let resp = ui.add(egui::TextEdit::singleline(&mut state.axis_length_share_draft).desired_width(72.0));
        record_input(ui, &resp, gctx.meta, &format!("axes.seg.{seg_idx}.length_share"), "Length Share", &state.axis_length_share_draft, true);
        if resp.changed() || resp.lost_focus() {
            share_edited = true;
        }
    });
    if share_edited {
        let trimmed = state.axis_length_share_draft.trim();
        if let Some(val) = parse_localized_number_input(trimmed, &gctx.app.locale_preset).filter(|v| v.is_finite() && *v > 0.0) {
            state.axis_length_share_error = None;
            if (cur_share - val).abs() > 1e-12
                && let Some(seg_mut) = draft.axis_draft.segments.get_mut(seg_idx)
            {
                seg_mut.length_share = val;
                *dirty = true;
            }
        } else {
            state.axis_length_share_error = Some(format!("Invalid segment length share `{trimmed}` (must be > 0)"));
        }
    }
    if let Some(err) = &state.axis_length_share_error {
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err).color(gctx.tokens.link_red)).wrap());
        record_label(ui, &err_resp, gctx.meta, &format!("axes.seg.{seg_idx}.length_share_error"), err);
    }
}

fn render_segment_count_and_break_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Segments: {}", draft.axis_draft.segments.len()));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.segment.add",
                label: tr(gctx.locale, "axes.add_segment"),
                command_id: None,
                selected: false,
                enabled: draft.axis_draft.segments.len() < 3,
                disabled_reason: (draft.axis_draft.segments.len() >= 3).then_some("Maximum 3 axis segments supported"),
            },
            gctx.tokens,
        )
        .clicked()
            && draft.add_segment()
        {
            *dirty = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.segment.remove",
                label: tr(gctx.locale, "axes.remove_segment"),
                command_id: None,
                selected: false,
                enabled: draft.axis_draft.segments.len() > 1,
                disabled_reason: (draft.axis_draft.segments.len() <= 1).then_some("At least 1 segment required"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            let last = draft.axis_draft.segments.len().saturating_sub(1);
            if draft.remove_segment(last) {
                let slot = axis_target_slot(draft.target_axis);
                if let Some(axis_slots) = state.format_axes_per_axis_cache.get_mut(slot)
                    && let Some(cell) = axis_slots.get_mut(last)
                {
                    *cell = None;
                }
                let max_idx = draft.axis_draft.segments.len().saturating_sub(1);
                if state.format_axes_selected_segment > max_idx {
                    state.format_axes_selected_segment = max_idx;
                    state.format_axes_last_segment = max_idx;
                    load_segment_drafts(state, draft.target_axis, max_idx, draft.axis_draft.segments.get(max_idx), &gctx.app.locale_preset);
                }
                *dirty = true;
            }
        }
    });

    if draft.axis_draft.segments.len() >= 2 {
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
                        selected: draft.axis_draft.break_ornament == orn,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.axis_draft.break_ornament = orn;
                    *dirty = true;
                }
            }
        });
        if let Some(val) = render_length_spinner_pt(
            ui,
            LengthSpinnerSpec {
                widget_id: "axes.gap_width_pt",
                label: tr(gctx.locale, "axes.break_gap_width"),
                value_pt: draft.axis_draft.gap_width_pt,
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
            draft.axis_draft.gap_width_pt = val;
            *dirty = true;
        }
    }
}

pub(super) fn render_axis_ticks_section(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
    seg_idx: usize,
    is_categorical: bool,
) {
    ui.label(egui::RichText::new(tr(gctx.locale, "axes.ticks.heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, fmt) in [
            ("decimal", "Decimal", TickLabelFormat::Decimal),
            ("power10", "10^x", TickLabelFormat::PowerOfTenSuperscript),
            ("sci_e", "1e-3", TickLabelFormat::ScientificE),
            ("antilog", "Antilog", TickLabelFormat::Antilog),
            ("si", "SI Prefix", TickLabelFormat::EngineeringSi),
            ("abs_decimal", "|x| Abs", TickLabelFormat::AbsoluteDecimal),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_format.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.axis_draft.tick_format == fmt,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                if is_categorical {
                    state.categorical_guard_pending_axis = Some(format!("{:?}", draft.target_axis));
                    state.format_axes_guard_pending_scale = Some(GraphAxisScale::Linear);
                    state.format_axes_guard_pending_tick_format = Some(fmt);
                    state.categorical_guard_confirmed = false;
                } else {
                    draft.axis_draft.tick_format = fmt;
                    *dirty = true;
                }
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.axes.tick_direction"));
        for (slug, lbl_key, dir) in [
            ("outside", "r13.axes.dir_outside", TickDirection::Outside),
            ("inside", "r13.axes.dir_inside", TickDirection::Inside),
            ("both", "r13.axes.dir_both", TickDirection::Both),
            ("none", "r13.common.none", TickDirection::None),
        ] {
            let lbl = tr(gctx.locale, lbl_key);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_dir.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.axis_draft.tick_direction == dir,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.axis_draft.tick_direction = dir;
                *dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    let mut show_labels = draft.axis_draft.show_tick_labels;
    let lbl_text = tr(gctx.locale, "r13.axes.show_tick_labels");
    let show_chk = ui.checkbox(&mut show_labels, lbl_text);
    record_checkbox(ui, &show_chk, gctx.meta, "axes.show_tick_labels", lbl_text, show_labels);
    if show_chk.changed() {
        draft.axis_draft.show_tick_labels = show_labels;
        *dirty = true;
    }

    ui.add_space(4.0);
    render_major_tick_interval_row(ui, state, gctx, draft, dirty, seg_idx);

    ui.add_space(4.0);
    render_minor_ticks_stepper_row(ui, state, gctx, draft, dirty, seg_idx);

    let combined_tick_err = match (&state.axis_major_interval_error, &state.axis_minor_ticks_error) {
        (Some(a), Some(b)) => Some(format!("{a} · {b}")),
        (Some(a), None) => Some(a.clone()),
        (None, Some(b)) => Some(b.clone()),
        (None, None) => None,
    };
    if let Some(err) = combined_tick_err {
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(&err).color(gctx.tokens.link_red)).wrap());
        record_label(ui, &err_resp, gctx.meta, &format!("axes.seg.{seg_idx}.ticks_error"), &err);
    }

    ui.add_space(6.0);
    let tick_count = draft.axis_draft.additional_ticks.len();
    let thresh_count = draft.axis_draft.thresholds.len();
    let add_btn_lbl = format!("{} ({} / {})", tr(gctx.locale, "axes.add_tick.open"), tick_count, thresh_count);
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

fn render_major_tick_interval_row(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
    seg_idx: usize,
) {
    let cur_interval = draft.axis_draft.segments.get(seg_idx).and_then(|s| s.major_tick_interval);
    let mut edited = false;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Major Tick Interval:");
        let resp = ui.add(egui::TextEdit::singleline(&mut state.axis_major_interval_draft).hint_text("Auto").desired_width(96.0));
        record_input(
            ui,
            &resp,
            gctx.meta,
            &format!("axes.seg.{seg_idx}.major_tick_interval"),
            "Major Tick Interval",
            &state.axis_major_interval_draft,
            true,
        );
        if resp.changed() || resp.lost_focus() {
            edited = true;
        }
    });
    if edited {
        let trimmed = state.axis_major_interval_draft.trim();
        if trimmed.is_empty() {
            state.axis_major_interval_error = None;
            if cur_interval.is_some()
                && let Some(seg) = draft.axis_draft.segments.get_mut(seg_idx)
            {
                seg.major_tick_interval = None;
                *dirty = true;
            }
        } else if let Some(val) = parse_localized_number_input(trimmed, &gctx.app.locale_preset).filter(|v| v.is_finite() && *v > 0.0) {
            state.axis_major_interval_error = None;
            if cur_interval != Some(val)
                && let Some(seg) = draft.axis_draft.segments.get_mut(seg_idx)
            {
                seg.major_tick_interval = Some(val);
                *dirty = true;
            }
        } else {
            state.axis_major_interval_error = Some(format!("Invalid major tick interval `{trimmed}` (must be > 0)"));
        }
    }
}

fn apply_minor_ticks_update(draft: &mut FormatAxesState, dirty: &mut bool, seg_idx: usize, minor_ticks: u8) {
    if let Some(seg) = draft.axis_draft.segments.get_mut(seg_idx) {
        let next_auto = if seg.scale == GraphAxisScale::Log10 && minor_ticks == 0 { false } else { seg.auto_log_minor_ticks };
        if seg.minor_ticks_per_interval != minor_ticks || seg.auto_log_minor_ticks != next_auto {
            seg.minor_ticks_per_interval = minor_ticks;
            seg.auto_log_minor_ticks = next_auto;
            *dirty = true;
        }
    }
}

fn render_minor_ticks_stepper_row(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
    seg_idx: usize,
) {
    let buf_id = egui::Id::unique(("axes.seg.minor_ticks_buf", draft.target_axis, seg_idx, draft.open_generation));
    let (last_synced, mut buf) = ui
        .ctx()
        .data(|d| d.get_temp::<(u8, String)>(buf_id))
        .unwrap_or_else(|| (state.axis_minor_ticks_draft, state.axis_minor_ticks_draft.to_string()));
    if last_synced != state.axis_minor_ticks_draft && state.axis_minor_ticks_error.is_none() {
        buf = state.axis_minor_ticks_draft.to_string();
    }

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Minor Ticks per Interval:");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: &format!("axes.seg.{seg_idx}.minor_ticks.minus"),
                label: "-",
                command_id: None,
                selected: false,
                enabled: state.axis_minor_ticks_draft > 0,
                disabled_reason: (state.axis_minor_ticks_draft == 0).then_some("Minimum 0 minor ticks"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.axis_minor_ticks_draft = state.axis_minor_ticks_draft.saturating_sub(1);
            buf = state.axis_minor_ticks_draft.to_string();
            state.axis_minor_ticks_error = None;
            apply_minor_ticks_update(draft, dirty, seg_idx, state.axis_minor_ticks_draft);
        }

        let resp = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(44.0));
        record_input(ui, &resp, gctx.meta, &format!("axes.seg.{seg_idx}.minor_ticks_per_interval"), "Minor Ticks per Interval", &buf, true);
        if resp.changed() {
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                state.axis_minor_ticks_error = Some(String::from("Invalid minor ticks `` (enter integer 0..=10)"));
            } else if let Ok(parsed) = trimmed.parse::<u16>() {
                if parsed <= 10 {
                    let v = parsed as u8;
                    state.axis_minor_ticks_draft = v;
                    state.axis_minor_ticks_error = None;
                    apply_minor_ticks_update(draft, dirty, seg_idx, v);
                } else {
                    state.axis_minor_ticks_error = Some(format!("Minor ticks `{trimmed}` exceeds maximum 10."));
                }
            } else {
                state.axis_minor_ticks_error = Some(format!("Invalid minor ticks `{trimmed}` (must be an integer 0..=10)."));
            }
        } else if resp.lost_focus() {
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                state.axis_minor_ticks_draft = 0;
                buf = String::from("0");
                state.axis_minor_ticks_error = None;
                apply_minor_ticks_update(draft, dirty, seg_idx, 0);
            }
        }

        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: &format!("axes.seg.{seg_idx}.minor_ticks.plus"),
                label: "+",
                command_id: None,
                selected: false,
                enabled: state.axis_minor_ticks_draft < 10,
                disabled_reason: (state.axis_minor_ticks_draft >= 10).then_some("Maximum 10 minor ticks"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.axis_minor_ticks_draft = state.axis_minor_ticks_draft.saturating_add(1).min(10);
            buf = state.axis_minor_ticks_draft.to_string();
            state.axis_minor_ticks_error = None;
            apply_minor_ticks_update(draft, dirty, seg_idx, state.axis_minor_ticks_draft);
        }
    });

    ui.ctx().data_mut(|d| d.insert_temp(buf_id, (state.axis_minor_ticks_draft, buf)));
}
