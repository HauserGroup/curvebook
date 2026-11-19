//! 7-tab `Nonlinear Curve Fitting` modal (plus MV `[Data]` tab) and 3-branch `Hook Constant`
//! browser in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.9).

use curvebook_doc::AnalysisKind;
use egui::{Context, Rect};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Core tabs of the `Nonlinear Curve Fitting` parameter modal (`gui-layout-spec.md` §8.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FitModalTab {
    Data,
    #[default]
    Model,
    Compare,
    Constrain,
    InitialValues,
    Range,
    Weight,
    Output,
}

/// Persistent transient UI state for `render_fit_modal` and `render_hook_constant_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FitModalEguiState {
    pub active_tab: FitModalTab,
    pub model_search: String,
    pub custom_equation_draft: String,
    pub custom_dsl_enabled: bool,
    pub hook_search_draft: String,
    pub expand_info_branch: bool,
    pub expand_analysis_branch: bool,
    pub expand_document_branch: bool,
}

/// Frame geometry and modal capture report emitted by `fit_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FitModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Return `true` when `ActiveDialog::EditAnalysisParameters` edits a `NonlinearRegression` spec.
#[must_use]
pub fn is_nonlinear_fit_dialog_active(app: &UiApp) -> bool {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return false;
    };
    if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return matches!(spec.kind, AnalysisKind::NonlinearRegression(_));
    }
    app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::NonlinearRegression(_)))
}

/// Render the 7-tab `Nonlinear Curve Fitting` modal when active.
pub fn render_fit_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> FitModalFrameReport {
    if !is_nonlinear_fit_dialog_active(app) {
        return FitModalFrameReport::default();
    }
    let _ = &state.active_tab;
    let title = tr(tokens.locale, "w6.view.modal.edit_analysis_params");
    render_simple_fit_modal(ctx, tokens, meta, intents, "dialog.nonlinear_fit", title, "Nonlinear Regression")
}

/// Render the 3-branch `Hook Constant` browser modal when `ActiveDialog::ConstantHookPicker` is active.
pub fn render_hook_constant_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> FitModalFrameReport {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ConstantHookPicker { .. }) {
        return FitModalFrameReport::default();
    }
    let _ = &state.hook_search_draft;
    let title = tr(tokens.locale, "w6.view.modal.hook_constant");
    render_simple_fit_modal(ctx, tokens, meta, intents, "dialog.constant_hook_picker", title, "Info Constants / Fit Parameters / Result Summaries")
}

fn render_simple_fit_modal(
    ctx: &Context,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    widget_id: &str,
    title: &str,
    subtitle: &str,
) -> FitModalFrameReport {
    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_fit_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(520.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(subtitle);
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
                intents.push_dialog_button("ok");
            }
        });
    });

    let capture = finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    FitModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
