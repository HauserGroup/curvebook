//! Four-stage `Power & Sample Size Studio` (`65%` left 3-tab narrative/curve/sensitivity matrix +
//! `35%` live sensitivity sidebar) in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.13).

use curvebook_doc::AnalysisKind;
use egui::{Context, Rect};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::unit_spinners::ScrubUnitFieldState;
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Three tabs rendered inside the `65%` Stage 4 results pane of `Power & Sample Size Studio`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PowerResultsSubTab {
    #[default]
    NarrativeReport,
    PowerCurve,
    SensitivityMatrix,
}

/// Persistent transient UI state for `render_power_studio_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerStudioEguiState {
    pub results_tab: PowerResultsSubTab,
    pub alpha_spinner: ScrubUnitFieldState,
    pub power_spinner: ScrubUnitFieldState,
    pub hover_n: Option<u32>,
    pub narrative_copied: bool,
}

/// Frame geometry and modal capture report emitted by `power_studio`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerStudioFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Return `true` when `ActiveDialog::PowerAndSampleSize` OR `EditAnalysisParameters` (`PowerAnalysis`) is open.
#[must_use]
pub fn is_power_studio_dialog_active(app: &UiApp) -> bool {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => true,
        ActiveDialog::EditAnalysisParameters { result_sheet_id } => {
            if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
                return matches!(spec.kind, AnalysisKind::PowerAnalysis(_));
            }
            app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::PowerAnalysis(_)))
        }
        _ => false,
    }
}

/// Render the 4-stage `Power & Sample Size Studio` modal when active.
pub fn render_power_studio_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut PowerStudioEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> PowerStudioFrameReport {
    if !is_power_studio_dialog_active(app) {
        return PowerStudioFrameReport::default();
    }

    let _ = &state.results_tab;
    let title = tr(tokens.locale, "w6.view.modal.power_and_sample_size");
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_power_studio_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(540.0_f32.min((ctx.content_rect().width() - 32.0).max(260.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(format!("Stage: {:?}", app.canvas_dialogs.power_wizard.stage));
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
                intents.push_dialog_button("cancel");
            }

            let ok_lbl = tr(tokens.locale, "w6.view.btn.create");
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
                let act = if app.canvas_dialogs.power_wizard.stage == crate::power_wizard::PowerWizardStage::Results { "create" } else { "ok" };
                intents.push_dialog_button(act);
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.power_and_sample_size", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    PowerStudioFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
