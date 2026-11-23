//! `Pairwise Comparisons` modal body (`gui-layout-spec.md` §8.11).

use curvebook_doc::{BracketLabelMode, BracketStyle, CldOrdering, PairwiseDisplayMode};
use egui::Ui;

use crate::i18n::tr;

use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};
use super::state::GraphModalEguiState;

pub(super) fn render_pairwise_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let pw = gctx.app.canvas_dialogs.pairwise.clone();
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
                enabled: !pw.available_result_ids.is_empty(),
                disabled_reason: pw.available_result_ids.is_empty().then_some("Run pairwise statistical comparison first"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            gctx.intents.push_dialog_widget_click("pairwise.auto_add");
        }
        let mut hide_ns = pw.hide_non_significant;
        let chk = ui.checkbox(&mut hide_ns, "Hide Non-Significant");
        record_checkbox(ui, &chk, gctx.meta, "pairwise.hide_non_significant", "Hide Non-Significant", hide_ns);
        if chk.changed() {
            let mut next = pw.clone();
            next.hide_non_significant = hide_ns;
            next.brackets_dirty = true;
            gctx.intents.push_update_pairwise_state(next);
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
                    selected: pw.display_mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pw.clone();
                next.display_mode = mode;
                next.cld_enabled = mode != PairwiseDisplayMode::BracketsOnly;
                gctx.intents.push_update_pairwise_state(next);
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
                    selected: pw.bracket_style == st,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pw.clone();
                next.bracket_style = st;
                next.brackets_dirty = true;
                gctx.intents.push_update_pairwise_state(next);
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
                    selected: pw.label_mode == lm,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pw.clone();
                next.label_mode = lm;
                next.brackets_dirty = true;
                gctx.intents.push_update_pairwise_state(next);
            }
        }
    });

    if pw.cld_enabled {
        ui.add_space(4.0);
        let font_cfg = ScrubUnitFieldConfig {
            step_per_pt: 0.5,
            decimals: 1,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("pairwise.cld_font_size", "CLD Font Size", 6.0, 24.0)
        };
        let f_out = render_scrub_unit_field(ui, pw.cld_font_size_pt, &mut state.cld_font_spinner, font_cfg, gctx.tokens, gctx.meta);
        if let Some(v) = f_out.committed_value {
            let mut next = pw.clone();
            next.cld_font_size_pt = v.clamp(6.0, 24.0);
            gctx.intents.push_update_pairwise_state(next);
        }

        let off_cfg = ScrubUnitFieldConfig {
            step_per_pt: 1.0,
            decimals: 1,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("pairwise.cld_offset", "CLD Offset", 0.0, 48.0)
        };
        let o_out = render_scrub_unit_field(ui, pw.cld_offset_pt, &mut state.cld_offset_spinner, off_cfg, gctx.tokens, gctx.meta);
        if let Some(v) = o_out.committed_value {
            let mut next = pw.clone();
            next.cld_offset_pt = v.clamp(0.0, 48.0);
            gctx.intents.push_update_pairwise_state(next);
        }

        let mut upper = pw.cld_uppercase;
        let chk = ui.checkbox(&mut upper, "Uppercase CLD Letters (A, AB)");
        record_checkbox(ui, &chk, gctx.meta, "pairwise.cld_uppercase", "Uppercase CLD Letters", upper);
        if chk.changed() {
            let mut next = pw.clone();
            next.cld_uppercase = upper;
            gctx.intents.push_update_pairwise_state(next);
        }
        let desc = pw.cld_ordering == CldOrdering::DescendingMean;
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "pairwise.cld_ordering",
                label: if desc { "Order: Descending Mean" } else { "Order: Ascending Mean" },
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            gctx.intents.push_dialog_widget_click("pairwise.cld_ordering");
        }
    }
    ui.add_space(4.0);
    ui.label(format!("Active brackets: {}", pw.brackets.len()));
}
