//! Reactive data transformation, baseline removal, MV filter/transform, extract & rearrange,
//! and exclusion-with-reason modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.6).

use curvebook_doc::{AnalysisKind, SheetId, TransformAnalysisSpec};
use egui::{Context, Rect};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Specific data-reshaping / transformation dialog open natively in `egui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransformModalKind {
    TransformValues,
    Normalize,
    RemoveBaseline,
    Transpose,
    PruneRows,
    MvFilterAndTransform,
    ExtractAndRearrange,
    Simulate,
    RandomSample,
}

/// Conjunctive row filter row in `MV Filter & Transform`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MvFilterPredicateRow {
    pub left_col: usize,
    pub op: String,
    pub right_value_or_col: String,
    pub compare_variable: bool,
}

/// Persistent transient UI state for `render_transform_modal` and `render_exclude_with_reason_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransformModalsEguiState {
    pub open_kind: Option<TransformModalKind>,
    pub target_sheet_id: Option<SheetId>,
    pub active_tab: usize,
    pub formula_draft: String,
    pub constant_k_draft: String,
    pub create_new_graph: bool,
    pub filter_rows: Vec<MvFilterPredicateRow>,
    pub exclude_reason_draft: String,
    pub validation_error: Option<String>,
}

/// Frame geometry and modal capture report emitted by transform modals.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransformModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Return `true` when `ActiveDialog::EditAnalysisParameters` edits a `Transform` analysis spec.
#[must_use]
pub fn is_transform_edit_dialog_active(app: &UiApp) -> bool {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return false;
    };
    if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return matches!(spec.kind, AnalysisKind::Transform(_));
    }
    app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(_)))
}

/// Render reactive Transform / Remove Baseline / MV Filter / Extract modals.
pub fn render_transform_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut TransformModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TransformModalFrameReport {
    let from_edit = is_transform_edit_dialog_active(app);
    if !from_edit && state.open_kind.is_none() {
        return TransformModalFrameReport::default();
    }

    let title = match (&app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), state.open_kind) {
        (Some(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { .. })), _) | (_, Some(TransformModalKind::RemoveBaseline)) => {
            tr(tokens.locale, "analysis.remove_baseline")
        }
        (Some(AnalysisKind::Transform(TransformAnalysisSpec::Normalize { .. })), _) | (_, Some(TransformModalKind::Normalize)) => {
            tr(tokens.locale, "cmd.analysis.run")
        }
        _ => tr(tokens.locale, "w6.view.modal.edit_analysis_params"),
    };

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_transform_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(460.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label("Reactive transform configuration preview.");
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
                if from_edit {
                    intents.push_dialog_button("cancel");
                } else {
                    intents.push_close_transform_modal();
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
                if from_edit {
                    intents.push_dialog_button("ok");
                } else {
                    intents.push_close_transform_modal();
                }
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.transform", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    TransformModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

/// Render `Exclude with Reason...` modal when `ActiveDialog::ExcludeWithReason` is active.
pub fn render_exclude_with_reason_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut TransformModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TransformModalFrameReport {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }) {
        return TransformModalFrameReport::default();
    }

    let title = tr(tokens.locale, "w6.view.modal.exclude_with_reason");
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_exclude_reason_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(400.0_f32.min((ctx.content_rect().width() - 32.0).max(220.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(format!("Reason tag: {}", app.canvas_dialogs.exclude_with_reason.reason_tag));
        let _ = &state.exclude_reason_draft;
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

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.exclude_with_reason", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    TransformModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
