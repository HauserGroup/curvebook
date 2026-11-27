//! `ActiveDialog::ResultViewer` modal renderer.

use egui::{Context, Key, Modifiers};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ResultSheetViewState};
use crate::i18n::{normalize_locale, tr};
use crate::modal_predicates::ControlEnablement;

use crate::egui_shell::stat_anova_modal::render_modal_seg_btn;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use super::StatsModalFrameReport;
use super::helpers::{Wave4ModalShell, render_modal_action_button, show_wave4_modal};

/// Render `ActiveDialog::ResultViewer` when explicitly requested.
pub fn render_result_viewer_modal(
    ctx: &Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::ResultViewer { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let doc = app.document();
    let rs = doc.result_sheet(result_sheet_id);
    let title = rs.map_or_else(|| tr(tokens.locale, "w6.view.modal.result_viewer").to_owned(), |r| r.title().to_owned());
    let tab_idx = app.canvas_dialogs.result_view.active_tab_index;
    let locale = normalize_locale(&app.locale);

    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.result_viewer",
            title: &title,
            subtitle_or_banner: rs.map(|r| r.analysis.effective_title()),
            banner_widget_id: "result_viewer.banner",
            max_width_pt: 780.0,
            max_height_pt: 560.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |ui, _, meta, intents| {
            if let Some(res) = rs
                && !res.tabs.is_empty()
            {
                let eff_tab_idx = tab_idx.min(res.tabs.len() - 1);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (idx, tab) in res.tabs.iter().enumerate() {
                        if render_modal_seg_btn(
                            ui,
                            tokens,
                            meta,
                            &format!("result.tab.{idx}"),
                            &tab.title,
                            idx == eff_tab_idx,
                            &ControlEnablement::enabled(),
                        ) {
                            intents.push(UiIntent::SelectResultTab { index: idx, sheet_id: None });
                        }
                    }
                });
            }
        },
        |ui, _, _w, _h, _meta, _intents| {
            if let Some(res) = rs
                && !res.tabs.is_empty()
                && let Some(tab) = res.tabs.get(tab_idx.min(res.tabs.len() - 1))
            {
                let has_row_labels = !tab.row_labels.is_empty();
                let num_cols = tab.column_headers.len().max(1) + usize::from(has_row_labels);
                egui::Grid::new("result_viewer_table").striped(true).num_columns(num_cols).show(ui, |ui| {
                    if has_row_labels {
                        ui.label(egui::RichText::new("Row").strong().small().color(tokens.text_muted));
                    }
                    for hdr in &tab.column_headers {
                        ui.label(egui::RichText::new(hdr).strong().small().color(tokens.text_strong));
                    }
                    ui.end_row();
                    for (r_idx, row) in tab.rows.iter().take(120).enumerate() {
                        if has_row_labels {
                            let row_lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
                            ui.label(egui::RichText::new(row_lbl).strong().small().color(tokens.text_primary));
                        }
                        for cell in row {
                            let s = ResultSheetViewState::format_result_cell(cell, res.analysis.p_value_format);
                            ui.label(egui::RichText::new(s).small());
                        }
                        ui.end_row();
                    }
                });
                if tab.rows.len() > 120 {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Showing first 120 of {} rows — open the full ResultSheet to inspect all rows.", tab.rows.len()))
                            .small()
                            .color(tokens.text_muted),
                    );
                }
            } else {
                ui.label(tr(locale, "results.no_tabular"));
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
