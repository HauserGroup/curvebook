//! `Pairwise Comparisons` modal body (`gui-layout-spec.md` §8.11).

use curvebook_doc::{BracketLabelMode, BracketStyle, CldOrdering, PairwiseDisplayMode};
use egui::Ui;

use crate::egui_shell::UiIntent;
use crate::i18n::tr;

use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, CheckboxMeta, GraphModalCtx, record_button, record_checkbox, record_checkbox_enabled};
use super::state::GraphModalEguiState;

pub(super) fn render_pairwise_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.pairwise.clone();
    let mut dirty = false;

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "pairwise.auto_add",
                label: tr(gctx.locale, "pairwise.auto_add"),
                command_id: None,
                selected: false,
                enabled: !draft.available_result_ids.is_empty(),
                disabled_reason: draft.available_result_ids.is_empty().then_some("Run pairwise statistical comparison first"),
            },
            gctx.tokens,
        )
        .clicked()
            && !draft.available_result_ids.is_empty()
        {
            draft.auto_add_from_analysis(gctx.app.document());
            dirty = true;
        }
        let mut hide_ns = draft.hide_non_significant;
        let chk = ui.checkbox(&mut hide_ns, "Hide Non-Significant");
        record_checkbox(ui, &chk, gctx.meta, "pairwise.hide_non_significant", "Hide Non-Significant", hide_ns);
        if chk.changed() {
            draft.hide_non_significant = hide_ns;
            draft.brackets_dirty = true;
            dirty = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Display Mode:");
        for (lbl, mode) in [
            ("Brackets Only", PairwiseDisplayMode::BracketsOnly),
            ("CLD Only", PairwiseDisplayMode::CompactLetterDisplayOnly),
            ("Brackets + CLD", PairwiseDisplayMode::BracketsAndCompactLetterDisplay),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("pairwise.mode.{mode:?}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.display_mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.display_mode = mode;
                draft.cld_enabled = mode != PairwiseDisplayMode::BracketsOnly;
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Bracket Style:");
        for (lbl, st) in [
            ("Square", BracketStyle::Square),
            ("Half Tick", BracketStyle::HalfTick),
            ("Flat Line", BracketStyle::FlatLine),
            ("Curly", BracketStyle::CurlyBrace),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("pairwise.bracket.{st:?}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.bracket_style == st,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.bracket_style = st;
                draft.brackets_dirty = true;
                dirty = true;
            }
        }
        ui.label("Labels:");
        for (lbl, lm) in [("Stars", BracketLabelMode::SummaryStars), ("Numeric p/q", BracketLabelMode::NumericPOrQ)] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("pairwise.label.{lm:?}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.label_mode == lm,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.label_mode = lm;
                draft.brackets_dirty = true;
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    let cld_enabled = draft.cld_enabled;
    let cld_reason = (!cld_enabled).then_some("Enable Compact Letter Display mode first");
    ui.add_enabled_ui(cld_enabled, |ui| {
        let font_cfg = ScrubUnitFieldConfig {
            step_per_pt: 0.5,
            decimals: 1,
            enabled: cld_enabled,
            disabled_reason: cld_reason,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("pairwise.cld_font_size", "CLD Font Size", 6.0, 24.0)
        };
        let f_out = render_scrub_unit_field(ui, draft.cld_font_size_pt, &mut state.cld_font_spinner, font_cfg, gctx.tokens, gctx.meta);
        if cld_enabled && let Some(v) = f_out.committed_value {
            draft.cld_font_size_pt = v.clamp(6.0, 24.0);
            dirty = true;
        }

        let off_cfg = ScrubUnitFieldConfig {
            step_per_pt: 1.0,
            decimals: 1,
            enabled: cld_enabled,
            disabled_reason: cld_reason,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("pairwise.cld_offset", "CLD Offset", 0.0, 48.0)
        };
        let o_out = render_scrub_unit_field(ui, draft.cld_offset_pt, &mut state.cld_offset_spinner, off_cfg, gctx.tokens, gctx.meta);
        if cld_enabled && let Some(v) = o_out.committed_value {
            draft.cld_offset_pt = v.clamp(0.0, 48.0);
            dirty = true;
        }

        let mut upper = draft.cld_uppercase;
        let chk = ui.checkbox(&mut upper, "Uppercase CLD Letters (A, AB)");
        record_checkbox_enabled(
            ui,
            &chk,
            gctx.meta,
            CheckboxMeta {
                widget_id: "pairwise.cld_uppercase",
                label: "Uppercase CLD Letters",
                checked: upper,
                enabled: cld_enabled,
                disabled_reason: cld_reason,
            },
        );
        if cld_enabled && chk.changed() {
            draft.cld_uppercase = upper;
            dirty = true;
        }
        let desc = draft.cld_ordering == CldOrdering::DescendingMean;
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "pairwise.cld_ordering",
                label: if desc { "Order: Descending Mean" } else { "Order: Ascending Mean" },
                command_id: None,
                selected: true,
                enabled: cld_enabled,
                disabled_reason: cld_reason,
            },
            gctx.tokens,
        )
        .clicked()
            && cld_enabled
        {
            draft.cld_ordering = match draft.cld_ordering {
                CldOrdering::DescendingMean => CldOrdering::AscendingMean,
                CldOrdering::AscendingMean => CldOrdering::DescendingMean,
            };
            dirty = true;
        }
    });
    ui.add_space(4.0);
    ui.label(format!("Active brackets: {}", draft.brackets.len()));

    if dirty {
        gctx.intents.push(UiIntent::UpdatePairwiseState(Box::new(draft)));
    }
}
