//! Standardized `Parameters: One-Way ANOVA` (`Cmd/Ctrl+T` / `EditAnalysisParameters`) modal
//! view in `egui 0.37` (`R15.2`, `gui-layout-spec.md` §8.8).
//!
//! Displays the live Dynamic Resolved-Test Banner, 5-tab keyboard-navigable parameter tabs,
//! clean-room design schematics, `R1`–`R4`/`R9` disabled-control visibility + unlock tooltips,
//! legacy `edit.*`/`modal.*` widget ID parity, and `100%` `ZLayer::ModalBackdrop` capture.

use egui::{Context, Rect};

use crate::UiApp;

use super::{FrameInspectMeta, PendingUiIntents, Tokens};

/// Five parameter tabs defined for the standardized One-Way ANOVA modal (`gui-layout-spec.md` §8.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnovaModalTab {
    #[default]
    ExperimentalDesign,
    AssumptionsAndTest,
    MultipleComparisons,
    Options,
    ResidualsAndPlots,
}

impl AnovaModalTab {
    /// Ordered tabs exposed in the keyboard-navigable tab strip.
    pub const ALL: [Self; 5] =
        [Self::ExperimentalDesign, Self::AssumptionsAndTest, Self::MultipleComparisons, Self::Options, Self::ResidualsAndPlots];

    /// Canonical lowercase tab slug matching `EditAnalysisParametersState::active_tab` / `stat_params`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::ExperimentalDesign => "design",
            Self::AssumptionsAndTest => "assumptions",
            Self::MultipleComparisons => "post_hoc",
            Self::Options => "options",
            Self::ResidualsAndPlots => "residuals",
        }
    }

    /// Parse a tab slug into [`AnovaModalTab`], defaulting unknown values to `None`.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "design" | "experimental_design" => Some(Self::ExperimentalDesign),
            "assumptions" | "test" | "assumptions_and_test" => Some(Self::AssumptionsAndTest),
            "post_hoc" | "multiple_comparisons" | "comparisons" => Some(Self::MultipleComparisons),
            "options" => Some(Self::Options),
            "residuals" | "plots" | "residuals_and_plots" => Some(Self::ResidualsAndPlots),
            _ => None,
        }
    }
}

/// Persistent transient UI state for [`render_one_way_anova_modal`].
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaModalEguiState {
    pub active_tab: AnovaModalTab,
    pub custom_title_draft: String,
    pub confidence_percent_draft: f64,
    pub show_method_reference: bool,
    pub last_resolved_banner: String,
    pub method_saved_as_default: bool,
}

impl Default for AnovaModalEguiState {
    fn default() -> Self {
        Self {
            active_tab: AnovaModalTab::ExperimentalDesign,
            custom_title_draft: String::new(),
            confidence_percent_draft: 95.0,
            show_method_reference: false,
            last_resolved_banner: String::new(),
            method_saved_as_default: false,
        }
    }
}

/// Frame geometry and modal capture report emitted by [`render_one_way_anova_modal`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnovaModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub resolved_test_banner: String,
}

/// Compute the live Dynamic Resolved-Test Banner text for the active One-Way ANOVA parameters.
#[must_use]
pub fn resolve_one_way_anova_banner(app: &UiApp) -> String {
    let ep = &app.canvas_dialogs.edit_params;
    match (ep.assume_gaussian, ep.paired_or_matched) {
        (true, false) if ep.welch_correction => String::from("Brown-Forsythe and Welch ANOVA + Games-Howell"),
        (true, false) => String::from("Ordinary one-way ANOVA + Tukey"),
        (true, true) => String::from("Repeated-measures one-way ANOVA + Geisser-Greenhouse"),
        (false, false) => String::from("Kruskal-Wallis test + Dunn"),
        (false, true) => String::from("Friedman test + Dunn"),
    }
}

/// Render the `One-Way ANOVA` / `EditAnalysisParameters` modal when open, buffering all changes into `intents`.
///
/// Wave 2 base stub: returns a closed report unless implemented in `r15-anova-coexist` (`R15.2`).
pub fn render_one_way_anova_modal(
    _ctx: &Context,
    app: &UiApp,
    state: &mut AnovaModalEguiState,
    _tokens: &Tokens,
    _meta: &mut FrameInspectMeta,
    _intents: &mut PendingUiIntents,
) -> AnovaModalFrameReport {
    let banner = resolve_one_way_anova_banner(app);
    state.last_resolved_banner.clone_from(&banner);
    AnovaModalFrameReport { resolved_test_banner: banner, ..AnovaModalFrameReport::default() }
}

#[cfg(test)]
mod tests {
    use super::{AnovaModalEguiState, AnovaModalTab, resolve_one_way_anova_banner};
    use crate::UiApp;

    #[test]
    fn anova_modal_tabs_and_default_banner_resolve_cleanly() {
        assert_eq!(AnovaModalTab::ALL.len(), 5);
        assert_eq!(AnovaModalTab::from_slug("assumptions"), Some(AnovaModalTab::AssumptionsAndTest));
        let state = AnovaModalEguiState::default();
        assert_eq!(state.confidence_percent_draft, 95.0);
        let app = UiApp::new();
        assert!(resolve_one_way_anova_banner(&app).contains("ANOVA"));
    }
}
