//! Nonlinear regression, dose-response, and curve-fit parameter modal helpers (`R10.3`).

use curvebook_doc::AnalysisKind;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::{LayoutBuilder, LayoutRectPx, WidgetId};

use super::dialogs_graph_misc::emit_descriptor_controls_grid;

/// Transient view state owned by the curve-fit parameter modal shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FitParamsViewState {
    pub model_search: String,
    pub active_fit_tab: String,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `fit_params` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut FitParamsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Return `true` when the active `edit_params.draft` targets a regression / curve-fit model.
#[must_use]
pub fn is_fit_analysis_draft(app: &UiApp) -> bool {
    app.canvas_dialogs
        .edit_params
        .draft
        .as_ref()
        .is_some_and(|d| matches!(d.kind, AnalysisKind::NonlinearRegression(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::PassingBablok(_)))
}

/// Render curve-fit parameter controls inside `rect` (delegating to the descriptor grid in scaffold).
pub fn build_fit_params_extra_section(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    controls: &[crate::inspect::UiWidgetNode],
) {
    emit_descriptor_controls_grid(builder, parent, rect, controls, 40);
}
