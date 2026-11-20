//! `ActiveDialog::ResultViewer` modal renderer.

use egui::Context;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ResultSheetViewState};
use crate::i18n::{normalize_locale, tr};
use crate::modal_predicates::ControlEnablement;

use crate::egui_shell::stat_anova_modal::render_modal_seg_btn;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens};

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
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (idx, tab) in res.tabs.iter().enumerate() {
                        if render_modal_seg_btn(
                            ui,
                            tokens,
                            meta,
                            &format!("result.tab.{idx}"),
                            &tab.title,
                            idx == tab_idx.min(res.tabs.len() - 1),
                            &ControlEnablement::enabled(),
                        ) {
                            intents.push_select_result_tab(idx);
                        }
                    }
                });
            }
        },
        |ui, _, _w, _h, _meta, _intents| {
            if let Some(res) = rs
                && let Some(tab) = res.tabs.get(tab_idx).or_else(|| res.tabs.first())
            {
                egui::Grid::new("result_viewer_table").striped(true).num_columns(tab.column_headers.len().max(1)).show(ui, |ui| {
                    for hdr in &tab.column_headers {
                        ui.label(egui::RichText::new(hdr).strong().small().color(tokens.text_strong));
                    }
                    ui.end_row();
                    for row in tab.rows.iter().take(120) {
                        for cell in row {
                            let s = ResultSheetViewState::format_result_cell(cell, res.analysis.p_value_format);
                            ui.label(egui::RichText::new(s).small());
                        }
                        ui.end_row();
                    }
                });
            } else {
                ui.label("No tabular results available.");
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
    StatsModalFrameReport::from(capture)
}
