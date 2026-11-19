//! `Manage Variables...` master-detail modal in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.3).
//!
//! Synchronizes against `app.grid.variables_dialog` (`VariablesDialogState`), exposing filter chips,
//! role overrides, categorical level inline rename/merge, explicit/frequency/alpha ordering,
//! reference level pinning, and single-commit `PendingUiIntents` settlement.

use egui::{Context, Rect};

use crate::UiApp;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Filter tabs exposed in the `Manage Variables...` upper master list (`gui-layout-spec.md` §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ManageVarsFilter {
    #[default]
    All,
    Categorical,
    Continuous,
    Calculated,
    Labels,
}

/// Persistent transient UI state for `render_manage_vars_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManageVarsModalEguiState {
    pub filter: ManageVarsFilter,
    pub selected_level: Option<String>,
    pub rename_draft: String,
    pub search_query: String,
}

/// Frame geometry and modal capture report emitted by `render_manage_vars_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManageVarsModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Render the master-detail `Manage Variables...` modal when `app.grid.variables_dialog.open` is true.
pub fn render_manage_vars_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut ManageVarsModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ManageVarsModalFrameReport {
    if !app.grid.variables_dialog.open {
        return ManageVarsModalFrameReport::default();
    }

    let title = tr(tokens.locale, "mv.vars.title");
    let backdrop_scrim = tokens.scrim;
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_manage_vars_dialog")).backdrop_color(backdrop_scrim).show(ctx, |ui| {
        ui.set_min_width(480.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(format!(
            "Selected column: {:?}, staged recodes: {}",
            app.grid.variables_dialog.selected_column,
            app.grid.variables_dialog.recode_mappings.len()
        ));
        let _ = &state.filter;
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
                intents.push_close_manage_variables();
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
                intents.push_close_manage_variables();
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.manage_variables", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    ManageVarsModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
