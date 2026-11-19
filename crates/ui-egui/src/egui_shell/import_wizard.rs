//! Five-stage `Import & Paste Special` wizard and `Convert Table Archetype...` modal in
//! `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.5).

use egui::{Context, Rect};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Five pipeline steps of the `Import & Paste Special` wizard (`gui-layout-spec.md` §8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImportWizardStep {
    #[default]
    Source,
    Preview,
    FilterAndUnstack,
    PlacementAndReshape,
    Metadata,
}

impl ImportWizardStep {
    /// Ordered stepper stages rendered across the wizard header bar.
    pub const ALL: [Self; 5] = [Self::Source, Self::Preview, Self::FilterAndUnstack, Self::PlacementAndReshape, Self::Metadata];
}

/// Persistent transient UI state for `render_import_or_convert_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportWizardEguiState {
    pub active_step: ImportWizardStep,
    pub raw_source_draft: String,
    pub missing_tokens_draft: String,
    pub plate_values_per_row_draft: String,
    pub plate_rows_per_dataset_draft: String,
    pub extract_info_metadata: bool,
    pub add_floating_note: bool,
}

/// Frame geometry and modal capture report emitted by `render_import_or_convert_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportWizardFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Render `Import & Paste Special` (`app.grid.import_wizard.open`), `ImportReshape`, or `ConvertArchetype`.
pub fn render_import_or_convert_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ImportWizardFrameReport {
    let is_wizard = app.grid.import_wizard.open;
    let is_reshape = matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ImportReshape);
    let is_convert = matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. });
    if !is_wizard && !is_reshape && !is_convert {
        return ImportWizardFrameReport::default();
    }

    let (widget_id, title) = if is_convert {
        ("dialog.convert_archetype", tr(tokens.locale, "cmd.table.convertArchetype"))
    } else if is_reshape {
        ("dialog.import_reshape", tr(tokens.locale, "w6.view.modal.import_reshape"))
    } else {
        ("dialog.import_paste_special", tr(tokens.locale, "menu.edit.paste_special"))
    };

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_import_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(540.0_f32.min((ctx.content_rect().width() - 32.0).max(260.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(format!("Step: {:?} · Live link: {}", state.active_step, app.grid.import_wizard.is_live_link));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let cancel_lbl = tr(tokens.locale, "btn.cancel");
            let cancel_resp = ui.button(cancel_lbl);
            meta.record_response(
                ui,
                &cancel_resp,
                FrameInspectEntryInput {
                    widget_id: "modal.cancel",
                    role: "button",
                    label: cancel_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if cancel_resp.clicked() {
                if is_wizard {
                    intents.push_close_import_wizard();
                } else {
                    intents.push_dialog_button("cancel");
                }
            }

            let ok_lbl = tr(tokens.locale, "btn.ok");
            let ok_resp = ui.button(ok_lbl);
            meta.record_response(
                ui,
                &ok_resp,
                FrameInspectEntryInput {
                    widget_id: "modal.ok",
                    role: "button",
                    label: ok_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if ok_resp.clicked() {
                if is_wizard {
                    intents.push_commit_import_wizard(true);
                } else {
                    intents.push_dialog_button("ok");
                }
            }
        });
    });

    let capture = finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    ImportWizardFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
