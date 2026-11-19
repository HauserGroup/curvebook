//! Native `egui` `InfoSheet` workspace (`docs/planning/gui-layout-spec.md` §8.4, `R15.3`):
//! `38%` structured Key/Value/Hook-Badge constants table + `62%` formatted laboratory notes.

use egui::Vec2;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use crate::layout::ZLayer;

/// Persistent `egui_shell` editor state for `InfoSheet` (`R15.3`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InfoViewEguiState {
    pub selected_row: Option<usize>,
    pub editing_key: Option<String>,
    pub editing_value: String,
    pub notes_draft: Option<(curvebook_doc::SheetId, String)>,
}

/// Per-frame region report returned by [`render_info_view`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InfoViewFrameReport {
    pub table_rect_pt: Option<egui::Rect>,
    pub notes_rect_pt: Option<egui::Rect>,
}

/// Render the active [`curvebook_doc::InfoSheet`] (`38%` structured constants table + `62%` notes pane).
pub fn render_info_view(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut InfoViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> InfoViewFrameReport {
    let mut report = InfoViewFrameReport::default();
    let doc = app.document();
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return report;
    };
    let Some(info) = doc.info_sheet(active_id) else {
        return report;
    };

    let avail = ui.available_rect_before_wrap();
    let table_h = (avail.height() * 0.38).clamp(90.0, avail.height().max(90.0));
    let table_rect = egui::Rect::from_min_size(avail.min, Vec2::new(avail.width(), table_h));
    let notes_rect = egui::Rect::from_min_max(egui::pos2(avail.left(), table_rect.bottom() + 4.0), avail.max);
    report.table_rect_pt = Some(table_rect);
    report.notes_rect_pt = notes_rect.is_positive().then_some(notes_rect);

    ui.scope_builder(egui::UiBuilder::new().max_rect(table_rect.shrink(8.0)), |ui| {
        ui.horizontal(|ui| {
            ui.strong(&info.title);
            let add_resp = ui.add_enabled(enabled, egui::Button::new("+ Add Constant"));
            meta.record_response(
                ui,
                &add_resp,
                FrameInspectEntryInput {
                    widget_id: "info.add_constant",
                    role: "button",
                    label: "+ Add Constant",
                    command_id: Some("info.setConstant"),
                    value: None,
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if add_resp.clicked() && enabled {
                let next_idx = info.constants.len() + 1;
                intents.push_engine("info.setConstant", json!({ "sheet_id": info.id.as_str(), "name": format!("Constant {next_idx}"), "value": "" }));
            }
        });
        ui.separator();
        for (idx, row) in info.constants.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.colored_label(tokens.text_strong, &row.name);
                ui.label(row.value.display_text());
                if let Some(units) = &row.units {
                    ui.colored_label(tokens.text_muted, units);
                }
                let _ = state.selected_row == Some(idx);
            });
        }
    });

    if notes_rect.is_positive() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(notes_rect.shrink(8.0)), |ui| {
            ui.strong("Experimental Notes");
            ui.separator();
            ui.label(&info.notes_markdown);
        });
    }

    report
}
