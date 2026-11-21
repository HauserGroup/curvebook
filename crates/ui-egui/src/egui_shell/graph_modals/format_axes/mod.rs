//! `Format Axes` modal dispatcher, draft synchronization, Frame & Origin tab, and
//! Titles & Typography tab (`gui-layout-spec.md` §8.11D).

mod additional_ticks;
mod segments_ticks;
mod single_axis;

use curvebook_doc::FramePreset;
use egui::Ui;

use crate::UiApp;

use super::helpers::{
    ButtonEntry, GraphModalCtx, LengthSpinnerSpec, format_localized_draft_f64, gtr, record_button, record_checkbox, record_input,
    render_length_spinner_pt,
};
use super::state::{FormatAxesTab, GraphModalEguiState};

use additional_ticks::render_additional_ticks_submodal;
use single_axis::render_axes_single_axis_tab;

pub(super) fn render_format_axes_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    sync_format_axes_local_drafts(gctx.app, state);
    if state.additional_ticks.open {
        render_additional_ticks_submodal(ui, state, gctx);
        return;
    }

    match state.format_axes_tab {
        FormatAxesTab::FrameAndOrigin => render_axes_frame_origin_tab(ui, state, gctx),
        FormatAxesTab::XAxis | FormatAxesTab::LeftYAxis | FormatAxesTab::RightYAxis => render_axes_single_axis_tab(ui, state, gctx),
        FormatAxesTab::TitlesAndTypography => render_axes_titles_tab(ui, state, gctx),
    }
}

fn sync_format_axes_local_drafts(app: &UiApp, state: &mut GraphModalEguiState) {
    let fa = &app.canvas_dialogs.format_axes;
    let sheet_key = fa.graph_sheet_id.as_ref().map(|s| s.as_str().to_owned());
    let session_needs_sync = !state.format_axes_session_synced
        || state.format_axes_synced_generation != Some(fa.open_generation)
        || state.format_axes_last_sheet_id != sheet_key;
    let axis_changed = session_needs_sync || state.format_axes_last_target != Some(fa.target_axis);
    if !session_needs_sync && !axis_changed {
        return;
    }
    if session_needs_sync {
        state.format_axes_session_synced = true;
        state.format_axes_synced_generation = Some(fa.open_generation);
        state.format_axes_last_sheet_id = sheet_key;
        state.format_axes_frame_preset_dirty = false;
        if let Some(gs) = fa.graph_sheet_id.as_ref().and_then(|id| app.document().graph_sheet(id)) {
            state.format_axes_frame_preset = gs.spec.frame_preset;
        }
        state.additional_ticks.open = false;
        state.additional_tick_status = None;
    }
    if axis_changed {
        state.format_axes_last_target = Some(fa.target_axis);
        state.format_axes_title_draft = fa.axis_draft.title.clone();
        let seg0 = fa.axis_draft.segments.first();
        state.axis_min_draft = seg0.and_then(|s| s.min).map(|v| format_localized_draft_f64(v, &app.locale_preset)).unwrap_or_default();
        state.axis_max_draft = seg0.and_then(|s| s.max).map(|v| format_localized_draft_f64(v, &app.locale_preset)).unwrap_or_default();
        state.axis_bounds_error = None;
        state.axis_major_interval_draft =
            seg0.and_then(|s| s.major_tick_interval).map(|v| format_localized_draft_f64(v, &app.locale_preset)).unwrap_or_default();
        state.axis_minor_ticks_draft = seg0.map_or(0, |s| s.minor_ticks_per_interval.min(10));
        state.axis_ticks_error = None;
        state.axis_min_spinner.draft_text.clear();
        state.axis_min_spinner.editing = false;
        state.axis_max_spinner.draft_text.clear();
        state.axis_max_spinner.editing = false;
        state.axis_gap_spinner.draft_text.clear();
        state.axis_gap_spinner.editing = false;
        state.axis_stroke_spinner.draft_text.clear();
        state.axis_stroke_spinner.editing = false;
        state.categorical_guard_confirmed = false;
        state.categorical_guard_pending_axis = None;
        state.format_axes_guard_pending_scale = None;
        state.format_axes_guard_pending_tick_format = None;
    }
}

fn render_axes_frame_origin_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.frame.heading")).strong());
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, preset) in [
            ("open_l", "Open L-Frame", FramePreset::OpenL),
            ("plain_boxed", "Plain Boxed", FramePreset::PlainBoxed),
            ("tufte", "Offset Range (Tufte)", FramePreset::OffsetRangeTufte),
            ("dual_y", "Dual Y (Left + Right Y2)", FramePreset::DualYSecondaryRight),
            ("origin", "Origin Crossing (0, 0)", FramePreset::OriginCrossing),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.frame.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: state.format_axes_frame_preset == preset,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.format_axes_frame_preset = preset;
                state.format_axes_frame_preset_dirty = true;
                if preset == FramePreset::DualYSecondaryRight && !fa.enable_y2 {
                    let mut next = fa.clone();
                    next.enable_y2 = true;
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        }
    });

    ui.add_space(8.0);
    let mut enable_y2 = fa.enable_y2;
    let y2_lbl = "Enable Secondary Right Y2 Axis";
    let chk = ui.checkbox(&mut enable_y2, y2_lbl);
    record_checkbox(ui, &chk, gctx.meta, "axes.enable_y2", y2_lbl, enable_y2);
    if chk.changed() {
        let mut next = fa;
        next.enable_y2 = enable_y2;
        if enable_y2 {
            state.format_axes_frame_preset = FramePreset::DualYSecondaryRight;
            state.format_axes_frame_preset_dirty = true;
        }
        gctx.intents.push_update_format_axes_state(next);
    }
}

fn render_axes_titles_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let title_lbl = gtr(gctx.locale, "axes.title_label");
        ui.label(title_lbl);
        let title_edit = ui.add(egui::TextEdit::singleline(&mut state.format_axes_title_draft).desired_width((gctx.body_w - 120.0).max(120.0)));
        record_input(ui, &title_edit, gctx.meta, "axes.title", title_lbl, &state.format_axes_title_draft, true);
        if title_edit.lost_focus() || title_edit.changed() {
            let mut next = fa.clone();
            next.axis_draft.title = state.format_axes_title_draft.clone();
            gctx.intents.push_update_format_axes_state(next);
        }
    });

    ui.add_space(6.0);
    if let Some(val) = render_length_spinner_pt(
        ui,
        LengthSpinnerSpec {
            widget_id: "axes.stroke_width_pt",
            label: "Axis Spine Width",
            value_pt: fa.axis_draft.stroke_width_pt,
            min_pt: 0.25,
            max_pt: 6.0,
            step_pt: 0.25,
            enabled: true,
            disabled_reason: None,
        },
        &mut state.axis_stroke_spinner,
        gctx.tokens,
        gctx.meta,
    ) {
        let mut next = fa.clone();
        next.axis_draft.stroke_width_pt = val;
        gctx.intents.push_update_format_axes_state(next);
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Tick Angle:");
        for deg in [0.0_f64, -45.0_f64, 45.0_f64, 90.0_f64] {
            let lbl = format!("{} deg", deg as i32);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_angle.{}", deg as i32),
                    label: &lbl,
                    command_id: None,
                    selected: (fa.axis_draft.tick_angle_deg - deg).abs() < 0.1,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fa.clone();
                next.axis_draft.tick_angle_deg = deg;
                gctx.intents.push_update_format_axes_state(next);
            }
        }
    });
}
