//! `Format Axes` modal dispatcher, draft synchronization, Frame & Origin tab, and
//! Titles & Typography tab (`gui-layout-spec.md` §8.11D).

mod additional_ticks;
mod segments_ticks;
mod single_axis;

use curvebook_doc::{AxisSegment, FramePreset};
use egui::Ui;

use crate::UiApp;
use crate::dialogs_canvas::{AxisTarget, FormatAxesState};
use crate::egui_shell::UiIntent;
use crate::i18n::{LocalePreset, tr};

use super::helpers::{
    ButtonEntry, GraphModalCtx, LengthSpinnerSpec, format_localized_draft_f64, record_button, record_checkbox, record_input, render_length_spinner_pt,
};
use super::state::{AdditionalTickDraft, AxisDraftCache, FormatAxesTab, GraphModalEguiState, axis_target_slot};

use additional_ticks::render_additional_ticks_submodal;
use single_axis::render_axes_single_axis_tab;

pub(super) fn render_format_axes_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    sync_format_axes_local_drafts(gctx.app, state);
    let mut draft = gctx.app.canvas_dialogs.format_axes.clone();
    let mut dirty = false;

    if state.additional_ticks.open {
        render_additional_ticks_submodal(ui, state, gctx, &mut draft, &mut dirty);
    } else {
        match state.format_axes_tab {
            FormatAxesTab::FrameAndOrigin => render_axes_frame_origin_tab(ui, state, gctx, &mut draft, &mut dirty),
            FormatAxesTab::XAxis | FormatAxesTab::LeftYAxis | FormatAxesTab::RightYAxis => {
                render_axes_single_axis_tab(ui, state, gctx, &mut draft, &mut dirty);
            }
            FormatAxesTab::TitlesAndTypography => render_axes_titles_tab(ui, state, gctx, &mut draft, &mut dirty),
        }
    }

    if dirty {
        gctx.intents.push(UiIntent::UpdateFormatAxesState(Box::new(draft)));
    }
}

pub(super) fn stash_segment_drafts(state: &mut GraphModalEguiState, axis: AxisTarget, seg_idx: usize) {
    let slot = axis_target_slot(axis);
    if let Some(axis_slots) = state.format_axes_per_axis_cache.get_mut(slot)
        && let Some(cell) = axis_slots.get_mut(seg_idx)
    {
        *cell = Some(AxisDraftCache {
            min_draft: state.axis_min_draft.clone(),
            max_draft: state.axis_max_draft.clone(),
            bounds_error: state.axis_bounds_error.clone(),
            major_interval_draft: state.axis_major_interval_draft.clone(),
            major_interval_error: state.axis_major_interval_error.clone(),
            minor_ticks_draft: state.axis_minor_ticks_draft,
            minor_ticks_error: state.axis_minor_ticks_error.clone(),
            length_share_draft: state.axis_length_share_draft.clone(),
            length_share_error: state.axis_length_share_error.clone(),
        });
    }
}

pub(super) fn load_segment_drafts(
    state: &mut GraphModalEguiState,
    axis: AxisTarget,
    seg_idx: usize,
    seg: Option<&AxisSegment>,
    preset: &LocalePreset,
) {
    let slot = axis_target_slot(axis);
    let cached = state.format_axes_per_axis_cache.get_mut(slot).and_then(|s| s.get_mut(seg_idx)).and_then(Option::take);
    if let Some(c) = cached {
        state.axis_min_draft = c.min_draft;
        state.axis_max_draft = c.max_draft;
        state.axis_bounds_error = c.bounds_error;
        state.axis_major_interval_draft = c.major_interval_draft;
        state.axis_major_interval_error = c.major_interval_error;
        state.axis_minor_ticks_draft = c.minor_ticks_draft;
        state.axis_minor_ticks_error = c.minor_ticks_error;
        state.axis_length_share_draft = c.length_share_draft;
        state.axis_length_share_error = c.length_share_error;
    } else {
        state.axis_min_draft = seg.and_then(|s| s.min).map(|v| format_localized_draft_f64(v, preset)).unwrap_or_default();
        state.axis_max_draft = seg.and_then(|s| s.max).map(|v| format_localized_draft_f64(v, preset)).unwrap_or_default();
        state.axis_bounds_error = None;
        state.axis_major_interval_draft = seg.and_then(|s| s.major_tick_interval).map(|v| format_localized_draft_f64(v, preset)).unwrap_or_default();
        state.axis_major_interval_error = None;
        state.axis_minor_ticks_draft = seg.map_or(0, |s| s.minor_ticks_per_interval.min(10));
        state.axis_minor_ticks_error = None;
        let share = seg.map_or(1.0, |s| if s.length_share.is_finite() && s.length_share > 0.0 { s.length_share } else { 1.0 });
        state.axis_length_share_draft = format_localized_draft_f64(share, preset);
        state.axis_length_share_error = None;
    }
}

pub(super) fn switch_active_segment(state: &mut GraphModalEguiState, fa: &FormatAxesState, next_seg: usize, preset: &LocalePreset) {
    let max_idx = fa.axis_draft.segments.len().saturating_sub(1);
    let clamped_next = next_seg.min(max_idx);
    let prev_seg = state.format_axes_last_segment.min(max_idx);
    if clamped_next == prev_seg {
        state.format_axes_selected_segment = clamped_next;
        return;
    }
    stash_segment_drafts(state, fa.target_axis, prev_seg);
    state.format_axes_selected_segment = clamped_next;
    state.format_axes_last_segment = clamped_next;
    load_segment_drafts(state, fa.target_axis, clamped_next, fa.axis_draft.segments.get(clamped_next), preset);
}

pub(super) fn sync_format_axes_local_drafts(app: &UiApp, state: &mut GraphModalEguiState) {
    let fa = &app.canvas_dialogs.format_axes;
    let sheet_key = fa.graph_sheet_id.as_ref().map(|s| s.as_str().to_owned());
    let session_needs_sync = !state.format_axes_session_synced
        || state.format_axes_synced_generation != Some(fa.open_generation)
        || state.format_axes_last_sheet_id != sheet_key;
    let axis_changed = session_needs_sync || state.format_axes_last_target != Some(fa.target_axis);
    let max_seg = fa.axis_draft.segments.len().saturating_sub(1);
    let clamped_sel = state.format_axes_selected_segment.min(max_seg);
    state.format_axes_selected_segment = clamped_sel;
    let seg_changed = !axis_changed && clamped_sel != state.format_axes_last_segment;

    if !session_needs_sync && !axis_changed && !seg_changed {
        return;
    }
    if session_needs_sync {
        state.format_axes_session_synced = true;
        state.format_axes_synced_generation = Some(fa.open_generation);
        state.format_axes_last_sheet_id = sheet_key;
        state.format_axes_per_axis_cache = Default::default();
        state.format_axes_pending_tab = None;
        state.format_axes_frame_preset_dirty = false;
        state.format_axes_selected_segment = 0;
        state.format_axes_last_segment = 0;
        if let Some(gs) = fa.graph_sheet_id.as_ref().and_then(|id| app.document().graph_sheet(id)) {
            state.format_axes_frame_preset = gs.spec.frame_preset;
        }
        state.additional_ticks = AdditionalTickDraft::default();
        state.additional_tick_draw_gridline = true;
        state.additional_tick_status = None;
    } else if axis_changed && let Some(prev_axis) = state.format_axes_last_target {
        let prev_seg = state.format_axes_last_segment.min(2);
        stash_segment_drafts(state, prev_axis, prev_seg);
        state.additional_ticks = AdditionalTickDraft::default();
        state.additional_tick_draw_gridline = true;
        state.additional_tick_status = None;
    }

    if axis_changed {
        state.format_axes_last_target = Some(fa.target_axis);
        state.format_axes_selected_segment = 0;
        state.format_axes_last_segment = 0;
        if let Some(pending) = state.format_axes_pending_tab.take() {
            state.format_axes_tab = pending;
        } else if matches!(state.format_axes_tab, FormatAxesTab::XAxis | FormatAxesTab::LeftYAxis | FormatAxesTab::RightYAxis) {
            state.format_axes_tab = match fa.target_axis {
                AxisTarget::X => FormatAxesTab::XAxis,
                AxisTarget::Y => FormatAxesTab::LeftYAxis,
                AxisTarget::Y2 => FormatAxesTab::RightYAxis,
            };
        }
        state.format_axes_title_draft = fa.axis_draft.title.clone();
        load_segment_drafts(state, fa.target_axis, 0, fa.axis_draft.segments.first(), &app.locale_preset);
        state.axis_gap_spinner.draft_text.clear();
        state.axis_gap_spinner.editing = false;
        state.axis_gap_spinner.dragging = false;
        state.axis_stroke_spinner.draft_text.clear();
        state.axis_stroke_spinner.editing = false;
        state.axis_stroke_spinner.dragging = false;
        state.clear_categorical_guard();
    } else if seg_changed {
        let prev_seg = state.format_axes_last_segment.min(2);
        stash_segment_drafts(state, fa.target_axis, prev_seg);
        state.format_axes_last_segment = clamped_sel;
        load_segment_drafts(state, fa.target_axis, clamped_sel, fa.axis_draft.segments.get(clamped_sel), &app.locale_preset);
    }
}

fn render_axes_frame_origin_tab(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatAxesState,
    dirty: &mut bool,
) {
    ui.label(egui::RichText::new(tr(gctx.locale, "axes.frame.heading")).strong());
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
                if preset == FramePreset::DualYSecondaryRight && !draft.enable_y2 {
                    draft.enable_y2 = true;
                    *dirty = true;
                }
            }
        }
    });

    ui.add_space(8.0);
    let mut enable_y2 = draft.enable_y2;
    let y2_lbl = tr(gctx.locale, "axes.enable_y2");
    let chk = ui.checkbox(&mut enable_y2, y2_lbl);
    record_checkbox(ui, &chk, gctx.meta, "axes.enable_y2", y2_lbl, enable_y2);
    if chk.changed() {
        draft.enable_y2 = enable_y2;
        if enable_y2 {
            state.format_axes_frame_preset = FramePreset::DualYSecondaryRight;
            state.format_axes_frame_preset_dirty = true;
        }
        *dirty = true;
    }
}

fn render_axes_titles_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatAxesState, dirty: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let title_lbl = tr(gctx.locale, "axes.title_label");
        ui.label(title_lbl);
        let title_id = egui::Id::unique("axes.title.input");
        if !ui.ctx().memory(|m| m.has_focus(title_id)) && state.format_axes_title_draft != draft.axis_draft.title {
            state.format_axes_title_draft = draft.axis_draft.title.clone();
        }
        let title_edit =
            ui.add(egui::TextEdit::singleline(&mut state.format_axes_title_draft).id(title_id).desired_width((gctx.body_w - 120.0).max(120.0)));
        record_input(ui, &title_edit, gctx.meta, "axes.title", title_lbl, &state.format_axes_title_draft, true);
        if (title_edit.lost_focus() || title_edit.changed()) && draft.axis_draft.title != state.format_axes_title_draft {
            draft.axis_draft.title = state.format_axes_title_draft.clone();
            *dirty = true;
        }
    });

    ui.add_space(6.0);
    if let Some(val) = render_length_spinner_pt(
        ui,
        LengthSpinnerSpec {
            widget_id: "axes.stroke_width_pt",
            label: "Axis Spine Width",
            value_pt: draft.axis_draft.stroke_width_pt,
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
        draft.axis_draft.stroke_width_pt = val;
        *dirty = true;
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
                    selected: (draft.axis_draft.tick_angle_deg - deg).abs() < 0.1,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.axis_draft.tick_angle_deg = deg;
                *dirty = true;
            }
        }
    });
}
