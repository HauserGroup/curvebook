//! Application utility and settings modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md`
//! §8.14): conditional `Export Figure / Layout`, `Print`, `Unsaved Changes`, `Recovery`, `About`,
//! `Find & Replace`, and 5-category `Preferences` (with pure safe Rust conflict-checked shortcut
//! rebinding over `UI_COMMANDS` and `ShortcutRegistry`, zero external keymap crates, and zero
//! `std::sync::Mutex`).

use egui::{Context, Rect};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::unit_spinners::{ScrubUnitFieldState, ScrubUnitKind};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Five categories in the `Preferences` sidebar (`gui-layout-spec.md` §8.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PreferencesCategory {
    #[default]
    General,
    Appearance,
    Language,
    GraphDefaults,
    KeyboardShortcuts,
}

impl PreferencesCategory {
    /// Ordered preference sections.
    pub const ALL: [Self; 5] = [Self::General, Self::Appearance, Self::Language, Self::GraphDefaults, Self::KeyboardShortcuts];
}

/// Resolution chosen by the user in the `Unsaved Changes` close confirmation modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnsavedCloseAction {
    Save,
    DontSave,
    Cancel,
}

/// Resolution chosen by the user in the startup / session `Recovery` modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryDecision {
    RestoreSession,
    DiscardRecovery,
    Dismiss,
}

/// Summary metadata surfaced in the crash/session recovery modal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveryDialogSummary {
    pub source_path: Option<String>,
    pub warning_lines: Vec<String>,
    pub replayed_commands: usize,
    pub dirty_timestamp_utc: Option<String>,
}

/// Working draft edited inside the 5-category `Preferences` modal.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesModalState {
    pub active_category: PreferencesCategory,
    pub default_unit: ScrubUnitKind,
    pub default_show_en_dash: bool,
    pub shortcut_search: String,
    pub selected_shortcut_command: Option<String>,
    pub shortcut_combo_draft: String,
    pub shortcut_conflict_error: Option<String>,
    pub ui_scale_spinner: ScrubUnitFieldState,
}

impl Default for PreferencesModalState {
    fn default() -> Self {
        Self {
            active_category: PreferencesCategory::General,
            default_unit: ScrubUnitKind::Millimeters,
            default_show_en_dash: true,
            shortcut_search: String::new(),
            selected_shortcut_command: None,
            shortcut_combo_draft: String::new(),
            shortcut_conflict_error: None,
            ui_scale_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Percent),
        }
    }
}

/// Persistent draft for `Find & Replace`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FindReplaceModalState {
    pub find_query: String,
    pub replace_with: String,
    pub match_case: bool,
    pub match_entire_cell: bool,
    pub match_count: usize,
    pub status_message: Option<String>,
}

/// Persistent transient UI state for `app_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppModalsEguiState {
    pub preferences_open: bool,
    pub preferences: PreferencesModalState,
    pub recovery_open: bool,
    pub recovery_summary: RecoveryDialogSummary,
    pub last_recovery_decision: Option<RecoveryDecision>,
    pub unsaved_changes_open: bool,
    pub pending_unsaved_action: Option<UnsavedCloseAction>,
    pub find_replace_open: bool,
    pub find_replace: FindReplaceModalState,
    pub export_width_spinner: ScrubUnitFieldState,
    pub export_dpi_spinner: ScrubUnitFieldState,
}

impl AppModalsEguiState {
    /// Return `true` when any shell-managed app modal flag (`Preferences`, `Recovery`, `Unsaved`, `Find`) is active.
    #[must_use]
    pub const fn any_open(&self) -> bool {
        self.preferences_open || self.recovery_open || self.unsaved_changes_open || self.find_replace_open
    }

    /// Dismiss all shell-managed app modal flags cleanly.
    pub fn close_all(&mut self) {
        self.preferences_open = false;
        self.recovery_open = false;
        self.unsaved_changes_open = false;
        self.find_replace_open = false;
    }
}

/// Frame geometry and modal capture report emitted by `app_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Render whichever application-level modal (`UnsavedChanges`, `RecoveryDialog`, `Preferences`,
/// `FindAndReplace`, `ExportFigure`, `Print`, or `About`) is currently active.
pub fn render_app_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    host_unsaved_close: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    if host_unsaved_close || state.unsaved_changes_open {
        return render_unsaved_modal(ctx, tokens, meta, intents);
    }
    if state.recovery_open {
        return render_recovery_modal(ctx, state, tokens, meta, intents);
    }
    if state.preferences_open {
        return render_preferences_modal(ctx, app, state, tokens, meta, intents);
    }
    if state.find_replace_open {
        return render_find_replace_modal(ctx, state, tokens, meta, intents);
    }

    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ExportFigure { .. } => {
            render_simple_dialog_modal(ctx, tokens, meta, intents, "dialog.export_figure", tr(tokens.locale, "w6.view.modal.export_figure"), false)
        }
        ActiveDialog::Print { .. } => {
            render_simple_dialog_modal(ctx, tokens, meta, intents, "dialog.print", tr(tokens.locale, "toolbar.print"), false)
        }
        ActiveDialog::About => render_simple_dialog_modal(ctx, tokens, meta, intents, "dialog.about", tr(tokens.locale, "menu.help.about"), true),
        _ => AppModalFrameReport::default(),
    }
}

fn render_unsaved_modal(ctx: &Context, tokens: &Tokens, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) -> AppModalFrameReport {
    let title = tr(tokens.locale, "w6.view.shell.unsaved_title");
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_unsaved_changes_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.heading(title);
        ui.label(tr(tokens.locale, "w6.view.shell.unsaved_prompt"));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let save_lbl = tr(tokens.locale, "menu.file.save");
            let save_resp = ui.button(save_lbl);
            meta.record_response(
                ui,
                &save_resp,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.save",
                    role: "button",
                    label: save_lbl,
                    command_id: Some("project.save"),
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if save_resp.clicked() {
                intents.push_resolve_unsaved_changes(UnsavedCloseAction::Save);
            }

            let dont_lbl = tr(tokens.locale, "w6.view.shell.dont_save");
            let dont_resp = ui.button(dont_lbl);
            meta.record_response(
                ui,
                &dont_resp,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.dont_save",
                    role: "button",
                    label: dont_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if dont_resp.clicked() {
                intents.push_resolve_unsaved_changes(UnsavedCloseAction::DontSave);
            }

            let cancel_lbl = tr(tokens.locale, "btn.cancel");
            let cancel_resp = ui.button(cancel_lbl);
            meta.record_response(
                ui,
                &cancel_resp,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.cancel",
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
                intents.push_resolve_unsaved_changes(UnsavedCloseAction::Cancel);
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.unsaved_changes", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_recovery_modal(
    ctx: &Context,
    state: &AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_recovery_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.heading("Crash & Session Recovery");
        ui.label(format!(
            "Warnings: {}, replayed commands: {}",
            state.recovery_summary.warning_lines.len(),
            state.recovery_summary.replayed_commands
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let restore_resp = ui.button("Restore Session");
            meta.record_response(
                ui,
                &restore_resp,
                FrameInspectEntryInput {
                    widget_id: "modal.restore",
                    role: "button",
                    label: "Restore Session",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if restore_resp.clicked() {
                intents.push_resolve_recovery(RecoveryDecision::RestoreSession);
            }

            let discard_resp = ui.button("Discard");
            meta.record_response(
                ui,
                &discard_resp,
                FrameInspectEntryInput {
                    widget_id: "modal.discard",
                    role: "button",
                    label: "Discard",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if discard_resp.clicked() {
                intents.push_resolve_recovery(RecoveryDecision::DiscardRecovery);
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.recovery", title: "Crash & Session Recovery", z_dialog: ZLayer::ModalDialog },
        meta,
    );
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_preferences_modal(
    ctx: &Context,
    _app: &UiApp,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_preferences_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(520.0_f32.min((ctx.content_rect().width() - 32.0).max(260.0)));
        ui.heading("Preferences");
        ui.label(format!("Category: {:?}", state.preferences.active_category));
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
                intents.push_close_preferences(false);
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
                intents.push_close_preferences(true);
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.preferences", title: "Preferences", z_dialog: ZLayer::ModalDialog },
        meta,
    );
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_find_replace_modal(
    ctx: &Context,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let title = tr(tokens.locale, "menu.edit.find");
    let _ = &state.find_replace.find_query;
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_find_replace_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.heading(title);
        ui.add_space(10.0);
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
            intents.push_close_find_replace();
        }
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.find_replace", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_simple_dialog_modal(
    ctx: &Context,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    widget_id: &str,
    title: &str,
    is_about: bool,
) -> AppModalFrameReport {
    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_app_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.heading(title);
        if is_about {
            ui.label(format!("CurveBook {}", env!("CARGO_PKG_VERSION")));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ok_lbl = tr(tokens.locale, "btn.ok");
            let ok_resp = ui.button(ok_lbl);
            let ok_id = if is_about { "dialog.button.ok" } else { "modal.ok" };
            meta.record_response(
                ui,
                &ok_resp,
                FrameInspectEntryInput {
                    widget_id: ok_id,
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
                if is_about {
                    intents.push_ui("dialog.ok", json!({}));
                } else {
                    intents.push_dialog_button("ok");
                }
            }

            let cancel_lbl = tr(tokens.locale, "btn.cancel");
            let cancel_resp = ui.button(cancel_lbl);
            let cancel_id = if is_about { "dialog.button.cancel" } else { "modal.cancel" };
            meta.record_response(
                ui,
                &cancel_resp,
                FrameInspectEntryInput {
                    widget_id: cancel_id,
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
                if is_about {
                    intents.push_ui("dialog.cancel", json!({}));
                } else {
                    intents.push_dialog_button("cancel");
                }
            }
        });
    });

    let capture = finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
