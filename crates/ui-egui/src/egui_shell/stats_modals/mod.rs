//! Statistical, GLM, Cox, PCA/PCR, Clustering, Survival, and `dialogs_r9` parameter modals
//! plus `AnalyzeSheet`, `DuplicateAnalysisPrompt`, and `ChangeDataAnalyzed` in `egui 0.37`
//! (`R15.4`, `gui-layout-spec.md` §8.7–§8.8).

mod analyze_picker;
mod factorial_mv;
mod helpers;
mod model_formula;
mod nested_misc;
mod r9_modals;
mod result_viewer;
mod stat_params;
mod two_group;

#[cfg(test)]
mod tests;

use curvebook_doc::AnalysisSpec;
use egui::Rect;

use super::inspect_bridge::ModalCaptureReport;

pub use analyze_picker::{render_analyze_sheet_modal, render_change_data_analyzed_modal, render_duplicate_analysis_modal};
pub(super) use helpers::{Wave4ModalShell, render_modal_action_button, render_mv_role_badge, render_tex_equation_preview, show_wave4_modal};
pub use r9_modals::render_dialogs_r9_modal;
pub use result_viewer::render_result_viewer_modal;
pub use stat_params::{render_stat_params_modal, resolve_stat_params_banner};

/// Canonical sub-variant key for `saved_defaults` lookup & retention (`AUD-10-48`).
pub(super) fn saved_default_kind_key(kind: &curvebook_doc::AnalysisKind) -> &'static str {
    use curvebook_doc::{AnalysisKind, MultivariateSpec, SurvivalAnalysisSpec, TransformAnalysisSpec};
    match kind {
        AnalysisKind::Multivariate(mv) => match mv {
            MultivariateSpec::Pca { pcr_outcome_column: Some(_), .. } => "multivariate.pcr",
            MultivariateSpec::Pca { pcr_outcome_column: None, .. } => "multivariate.pca",
            MultivariateSpec::CorrelationMatrix { .. } => "multivariate.correlation_matrix",
            MultivariateSpec::Clustering { .. } => "multivariate.clustering",
            MultivariateSpec::MultipleImputation { .. } => "multivariate.multiple_imputation",
        },
        AnalysisKind::Survival(s) => match s {
            SurvivalAnalysisSpec::KaplanMeier { .. } => "survival.kaplan_meier",
            SurvivalAnalysisSpec::CoxProportionalHazards { .. } => "survival.cox_ph",
        },
        AnalysisKind::Transform(t) => match t {
            TransformAnalysisSpec::Math { .. } => "transform.math",
            TransformAnalysisSpec::Normalize { .. } => "transform.normalize",
            TransformAnalysisSpec::RemoveBaseline { .. } => "transform.remove_baseline",
            TransformAnalysisSpec::Transpose { .. } => "transform.transpose",
            TransformAnalysisSpec::PruneRows { .. } => "transform.prune_rows",
            TransformAnalysisSpec::SelectAndTransform { .. } => "transform.select_and_transform",
            TransformAnalysisSpec::Simulate { .. } => "transform.simulate",
            TransformAnalysisSpec::RandomSample { .. } => "transform.random_sample",
        },
        AnalysisKind::DescriptiveStats(d) if d.normality_only => "stats.normality",
        other => curvebook_doc::descriptor_for_kind(other).slug,
    }
}

/// Persistent transient UI state for `stats_modals` dialogs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatsModalsEguiState {
    pub active_tab: String,
    pub analyze_query_draft: String,
    pub analyze_focus_requested: bool,
    pub analyze_category_filter: Option<crate::dialogs_canvas::AnalysisCategory>,
    pub mv_var_search: String,
    pub show_method_reference: bool,
    pub method_saved_as_default: bool,
    pub saved_defaults: Vec<AnalysisSpec>,
    pub last_resolved_banner: String,
    pub open_session_sheet_id: Option<String>,
    pub open_generation: u64,
    pub lin_x_min_draft: String,
    pub lin_x_max_draft: String,
    pub lin_plot_min_draft: String,
    pub lin_plot_max_draft: String,
    pub lin_segments_draft: String,
    pub custom_title_draft: String,
    pub numeric_field_errors: std::collections::BTreeMap<String, String>,
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

impl From<ModalCaptureReport> for StatsModalFrameReport {
    fn from(c: ModalCaptureReport) -> Self {
        Self {
            modal_open: true,
            dialog_rect_pt: c.dialog_rect_pt,
            outside_click_captured: c.outside_click_captured,
            outside_wheel_captured: c.outside_wheel_captured,
            resolved_test_banner: String::new(),
        }
    }
}
