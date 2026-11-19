//! Statistical, GLM, Cox, PCA/PCR, Clustering, Survival, and `dialogs_r9` parameter modals
//! plus `AnalyzeSheet`, `DuplicateAnalysisPrompt`, and `ChangeDataAnalyzed` in `egui 0.37`
//! (`R15.4`, `gui-layout-spec.md` §8.7–§8.8).

use curvebook_doc::AnalysisKind;
use egui::{Context, Rect};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::tr;
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Persistent transient UI state for `stats_modals` dialogs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatsModalsEguiState {
    pub active_tab: String,
    pub analyze_query_draft: String,
    pub analyze_focus_requested: bool,
    pub mv_var_search: String,
    pub show_method_reference: bool,
    pub method_saved_as_default: bool,
    pub last_resolved_banner: String,
}

/// Frame geometry and modal capture report emitted by `stats_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatsModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub resolved_test_banner: String,
}

/// Render native `egui` `Analyze Data...` modal (`ActiveDialog::AnalyzeSheet`).
pub fn render_analyze_sheet_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::AnalyzeSheet { .. } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let _ = &state.analyze_query_draft;
    render_simple_stats_modal(
        ctx,
        tokens,
        meta,
        intents,
        "dialog.analyze_sheet",
        tr(tokens.locale, "w6.view.modal.analyze_data"),
        "Select an analysis family or search.",
    )
}

/// Render `DuplicateAnalysisPrompt` disambiguation modal.
pub fn render_duplicate_analysis_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::DuplicateAnalysisPrompt { .. } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let _ = &state.active_tab;
    render_simple_stats_modal(
        ctx,
        tokens,
        meta,
        intents,
        "dialog.duplicate_analysis",
        tr(tokens.locale, "w6.view.modal.analysis_already_exists"),
        "Modify existing analysis parameters or create a new result sheet.",
    )
}

/// Render `Change Data Analyzed...` retargeting modal.
pub fn render_change_data_analyzed_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::ChangeDataAnalyzed { .. } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let _ = &state.active_tab;
    render_simple_stats_modal(
        ctx,
        tokens,
        meta,
        intents,
        "dialog.change_data_analyzed",
        tr(tokens.locale, "btn.change_data_analyzed"),
        "Select a compatible source DataSheet to retarget this analysis.",
    )
}

/// Render general Statistical / GLM / Cox / PCA / PCR / Clustering / Survival parameter modals.
pub fn render_stat_params_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let banner = app
        .canvas_dialogs
        .edit_params
        .draft
        .as_ref()
        .or_else(|| app.document().result_sheet(result_sheet_id).map(|rs| &rs.analysis))
        .map(|spec| match &spec.kind {
            AnalysisKind::GeneralizedLinearModel(_) => String::from("GLM / Logistic / Poisson Parameters"),
            AnalysisKind::Survival(_) => String::from("Survival / Cox PH Parameters"),
            AnalysisKind::Multivariate(_) => String::from("PCA / PCR / Clustering / Matrix Parameters"),
            _ => spec.effective_title().to_owned(),
        })
        .unwrap_or_else(|| String::from("Analysis Parameters"));
    state.last_resolved_banner.clone_from(&banner);

    let mut report = render_simple_stats_modal(
        ctx,
        tokens,
        meta,
        intents,
        "dialog.edit_analysis_parameters",
        tr(tokens.locale, "w6.view.modal.edit_analysis_params"),
        &banner,
    );
    report.resolved_test_banner = banner;
    report
}

/// Render `dialogs_r9` generation modals (`RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`).
pub fn render_dialogs_r9_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let (widget_id, title) = match &app.canvas_dialogs.active_dialog {
        ActiveDialog::RandomizeGroups => ("dialog.randomize_groups", tr(tokens.locale, "w6.view.modal.randomize_groups")),
        ActiveDialog::RandomSampleRows { .. } => ("dialog.random_sample_rows", tr(tokens.locale, "cmd.table.randomSampleRows")),
        ActiveDialog::SimulateDistributions => ("dialog.simulate_distributions", tr(tokens.locale, "cmd.table.simulateDistributions")),
        _ => return StatsModalFrameReport::default(),
    };
    let _ = &state.active_tab;
    render_simple_stats_modal(ctx, tokens, meta, intents, widget_id, title, title)
}

/// Render `ActiveDialog::ResultViewer` when explicitly requested.
pub fn render_result_viewer_modal(
    ctx: &Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::ResultViewer { .. } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    render_simple_stats_modal(ctx, tokens, meta, intents, "dialog.result_viewer", tr(tokens.locale, "w6.view.modal.result_viewer"), "Result Viewer")
}

fn render_simple_stats_modal(
    ctx: &Context,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    widget_id: &str,
    title: &str,
    body_summary: &str,
) -> StatsModalFrameReport {
    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_stats_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(480.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0)));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(body_summary);
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
    StatsModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
        resolved_test_banner: String::new(),
    }
}
