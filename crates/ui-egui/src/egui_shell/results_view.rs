//! Native `egui` multi-tab reactive `ResultSheet` viewer (`docs/planning/gui-layout-spec.md` §8.10, `R15.3`).

use curvebook_doc::ResultCell;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use crate::layout::ZLayer;
use crate::tr;

/// Persistent `egui_shell` state for the multi-tab `ResultSheet` viewer (`R15.3`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultsViewEguiState {
    pub scroll_offset_y: f32,
    pub hovered_row: Option<usize>,
}

/// Per-frame region & active-tab report returned by [`render_results_view`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultsViewFrameReport {
    pub header_rect_pt: Option<egui::Rect>,
    pub body_rect_pt: Option<egui::Rect>,
    pub active_tab_index: usize,
}

fn format_result_cell(cell: &ResultCell) -> String {
    match cell {
        ResultCell::Empty => String::new(),
        ResultCell::Number { value, prefix, units } => {
            let pfx = prefix.as_deref().unwrap_or("");
            match units {
                Some(u) if !u.is_empty() => format!("{pfx}{value:.4} {u}"),
                _ => format!("{pfx}{value:.4}"),
            }
        }
        ResultCell::ConfidenceInterval { lower, upper } => format!("{lower:?} to {upper:?}"),
        ResultCell::PValue { formatted, stars, .. } => {
            if stars.is_empty() {
                formatted.clone()
            } else {
                format!("{formatted} ({stars})")
            }
        }
        ResultCell::Text(s) => s.clone(),
        ResultCell::Boolean(b) => String::from(if *b { "Yes" } else { "No" }),
    }
}

/// Render the active [`curvebook_doc::ResultSheet`] with top header bar, sub-tab strip, and table body.
pub fn render_results_view(
    ui: &mut egui::Ui,
    app: &UiApp,
    _state: &mut ResultsViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ResultsViewFrameReport {
    let mut report = ResultsViewFrameReport::default();
    let doc = app.document();
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return report;
    };
    let Some(rs) = doc.result_sheet(active_id) else {
        return report;
    };

    let tab_count = rs.tabs.len().max(1);
    let active_idx = app.canvas_dialogs.result_view.active_tab_index.min(tab_count.saturating_sub(1));
    report.active_tab_index = active_idx;

    let avail = ui.available_rect_before_wrap();
    let hdr_rect = egui::Rect::from_min_size(avail.min, egui::Vec2::new(avail.width(), 32.0));
    report.header_rect_pt = Some(hdr_rect);

    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.strong(rs.title());
        let edit_lbl = tr(&app.locale, "toolbar.parameters");
        let edit_resp = ui.add_enabled(enabled, egui::Button::new(edit_lbl));
        meta.record_response(
            ui,
            &edit_resp,
            FrameInspectEntryInput {
                widget_id: "result.edit_params",
                role: "button",
                label: edit_lbl,
                command_id: Some("ui.editParams.open"),
                value: None,
                enabled,
                disabled_reason: (!enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if edit_resp.clicked() && enabled {
            intents.push_ui("ui.editParams.open", json!({ "sheet_id": rs.id.as_str() }));
        }

        ui.separator();
        for (idx, tab) in rs.tabs.iter().enumerate() {
            let selected = idx == active_idx;
            let resp = ui.add_enabled(enabled, egui::Button::new(&tab.title).selected(selected));
            let wid = format!("result.tab.{}", tab.id);
            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "tab",
                    label: &tab.title,
                    command_id: None,
                    value: Some(json!({ "selected": selected, "index": idx, "tab_id": tab.id })),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() && enabled && !selected {
                intents.push_select_result_tab(idx);
            }
        }
    });

    ui.separator();
    let body_rect = ui.available_rect_before_wrap();
    report.body_rect_pt = body_rect.is_positive().then_some(body_rect);

    if let Some(tab) = rs.tabs.get(active_idx) {
        egui::ScrollArea::vertical().id_salt("egui_shell.results.scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.colored_label(tokens.text_muted, &tab.title);
            for (row_idx, cells) in tab.rows.iter().take(200).enumerate() {
                ui.horizontal(|ui| {
                    let row_lbl = tab.row_labels.get(row_idx).map_or("", String::as_str);
                    if !row_lbl.is_empty() {
                        ui.strong(row_lbl);
                    }
                    for cell in cells {
                        ui.label(format_result_cell(cell));
                    }
                });
            }
        });
    }

    report
}
