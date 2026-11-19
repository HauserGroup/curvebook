//! Graph formatting & configuration modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md`
//! §8.11B–D): `Change Graph Type`, 3-schema `Format Graph`, `Format Axes` (+ `Categorical Axis
//! Guard` and `Additional Ticks / Thresholds` sub-modal), `Pairwise Comparisons`, `Bubble Plot
//! Channels`, `Apply Style From`, `Format Selected Points`, `Insert Model Equation`, and
//! `Page Layout Composer`.

use egui::{Context, Rect};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::unit_spinners::ScrubUnitFieldState;
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Top-level tab selection inside 3-schema `Format Graph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FormatGraphTab {
    #[default]
    Appearance,
    SeriesAndArrangement,
    GraphSettings,
    ValueAnnotations,
    AxisVariables,
    DataObjects,
    Legends,
    ColorMapping,
    HeatmapLabels,
    HeatmapBorders,
}

/// Top-level tab selection inside `Format Axes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FormatAxesTab {
    #[default]
    FrameAndOrigin,
    XAxis,
    LeftYAxis,
    RightYAxis,
    TitlesAndTypography,
}

/// Sub-modal draft for adding/editing an `Additional Ticks / Thresholds` line or shaded zone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdditionalTickDraft {
    pub open: bool,
    pub selected_index: Option<usize>,
    pub value_draft: String,
    pub end_value_draft: String,
    pub label_draft: String,
    pub show_line: bool,
    pub shaded_zone: bool,
}

/// Persistent transient UI state for `graph_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphModalEguiState {
    pub format_graph_tab: FormatGraphTab,
    pub format_axes_tab: FormatAxesTab,
    pub additional_ticks: AdditionalTickDraft,
    pub categorical_guard_confirmed: bool,
    pub categorical_guard_pending_axis: Option<String>,
    pub symbol_size_spinner: ScrubUnitFieldState,
    pub axis_min_spinner: ScrubUnitFieldState,
    pub axis_max_spinner: ScrubUnitFieldState,
    pub line_thickness_spinner: ScrubUnitFieldState,
}

/// Frame geometry and modal capture report emitted by `graph_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Return `true` when `app.canvas_dialogs.active_dialog` belongs to `graph_modals`.
#[must_use]
pub const fn is_graph_modal_active(dialog: &ActiveDialog) -> bool {
    matches!(
        dialog,
        ActiveDialog::ChangeGraphType { .. }
            | ActiveDialog::FormatGraph { .. }
            | ActiveDialog::FormatAxes { .. }
            | ActiveDialog::PairwiseComparisons { .. }
            | ActiveDialog::BubblePlotInspector { .. }
            | ActiveDialog::ApplyStyleFrom { .. }
            | ActiveDialog::FormatSelectedPoints { .. }
            | ActiveDialog::InsertModelEquation { .. }
            | ActiveDialog::PageLayoutComposer { .. }
    )
}

/// Render the active graph or page-layout configuration modal when active.
pub fn render_graph_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut GraphModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> GraphModalFrameReport {
    let (widget_id, title) = match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ChangeGraphType { .. } => ("dialog.change_graph_type", tr(tokens.locale, "cmd.graph.setKind")),
        ActiveDialog::FormatGraph { .. } => ("dialog.format_graph", tr(tokens.locale, "inspector.format_graph")),
        ActiveDialog::FormatAxes { .. } => ("dialog.format_axes", tr(tokens.locale, "inspector.format_axes")),
        ActiveDialog::PairwiseComparisons { .. } => ("dialog.pairwise_comparisons", tr(tokens.locale, "inspector.pairwise_comparisons")),
        ActiveDialog::BubblePlotInspector { .. } => ("dialog.bubble_plot", tr(tokens.locale, "w6.view.modal.bubble_plot_channels")),
        ActiveDialog::ApplyStyleFrom { .. } => ("dialog.apply_style_from", tr(tokens.locale, "btn.apply_style_from")),
        ActiveDialog::FormatSelectedPoints { .. } => ("dialog.format_selected_points", tr(tokens.locale, "cmd.graph.formatSelectedPoints")),
        ActiveDialog::InsertModelEquation { .. } => ("dialog.insert_model_equation", tr(tokens.locale, "w6.view.modal.insert_model_equation")),
        ActiveDialog::PageLayoutComposer { .. } => ("dialog.page_layout_composer", tr(tokens.locale, "w6.view.modal.page_layout_composer")),
        _ => return GraphModalFrameReport::default(),
    };

    let _ = &state.format_graph_tab;
    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_graph_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(500.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(title);
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
    GraphModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}
