//! Native desktop `egui 0.37` shell contracts, canonical layout/color tokens, single-commit
//! intent buffer, AccessKit inspection metadata, and adapted pure-`egui` widget helpers (`R15`).

use curvebook_doc::{CellCoord, ConfidenceLevel, ErrorCode, LevelOrderSpec, SheetCellRange, SheetId, SheetSection};
use curvebook_engine::{CommandOutcome, EngineError, Session};
use curvebook_palette::{CIVIDIS_STOPS, CanvasPreviewFilter, OKABE_ITO, Rgba8, VIRIDIS_STOPS};
use egui::{Color32, CornerRadius, FontId, Rect, Stroke, TextStyle, Visuals};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::control_io::dispatch_ui_command_as_ui;
use crate::dialogs_canvas::{
    ActiveDialog, ActiveInspector, AnalyzeDuplicateChoice, ApplyStyleFromState, BubblePlotInspectorState, ChangeGraphTypeState,
    CreateGraphDialogState, EqualizeScalingDialogState, ExportFigureState, FormatAxesState, FormatEmbeddedTableDialogState, FormatGraphState,
    FormatImageDialogState, PageLayoutComposerState, PairwiseComparisonsState, PrintDialogState,
};
use crate::grid_view::{ImportOriginKind, ImportPasteWizardState};
use crate::i18n;
use crate::inspect::ZLayer;
use crate::os_integration::ClipboardBundle;
use crate::power_wizard::PowerWizardState;
use crate::theme::{UiThemeMode, UiThemeSpec};
use crate::workspace_nav::{ActiveWorkspacePane, NavigatorNestingMode};

pub mod app_modals;
pub mod brand;
pub mod canvas_view;
pub mod center_stage;
pub mod command_palette;
pub mod data_grid;
pub mod driver;
pub mod fit_modal;
pub mod fonts;
pub mod graph_modals;
pub mod import_wizard;
pub mod info_view;
pub mod inspect_bridge;
pub mod leading_dock;
pub mod manage_vars_modal;
pub mod power_studio;
pub mod rasterizer;
pub mod results_view;
pub mod stat_anova_modal;
pub mod stats_modals;
pub mod status_footer;
pub mod top_bar_fit;
pub mod top_chrome;
pub mod trailing_studio;
pub mod transform_modals;
pub mod unit_spinners;
pub mod welcome_hub;
pub mod widgets;

/// Pre-configured dock visibility & width profiles surfaced by Tier 1's Workspace Preset Switcher (`R15.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WorkspacePreset {
    #[default]
    Default,
    DataPrep,
    Modeling,
    Publication,
    Compact,
}

/// Dock configuration implied by applying a [`WorkspacePreset`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresetDockConfig {
    pub navigator_open: bool,
    pub family_open: bool,
    pub inspector_open: bool,
    pub leading_collapsed_rail: bool,
    pub trailing_collapsed_rail: bool,
    pub leading_width_pt: f32,
    pub trailing_width_pt: f32,
}

impl WorkspacePreset {
    /// Ordered list of workspace presets exposed in Tier 1 switcher controls.
    pub const ALL: [Self; 5] = [Self::Default, Self::DataPrep, Self::Modeling, Self::Publication, Self::Compact];

    /// Canonical identifier string for telemetry, intents, and persistence.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::DataPrep => "data_prep",
            Self::Modeling => "modeling",
            Self::Publication => "publication",
            Self::Compact => "compact",
        }
    }

    /// Parse a serialized preset slug case-insensitively.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "default" => Some(Self::Default),
            "data_prep" | "dataprep" | "data prep" => Some(Self::DataPrep),
            "modeling" => Some(Self::Modeling),
            "publication" => Some(Self::Publication),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }

    /// Concise localized-ready display label for Tier 1 pill UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::DataPrep => "Data Prep",
            Self::Modeling => "Modeling",
            Self::Publication => "Publication",
            Self::Compact => "Compact",
        }
    }

    /// Default dock visibility and width profile for this workspace preset.
    #[must_use]
    pub const fn default_dock_config(self) -> PresetDockConfig {
        match self {
            Self::Default => PresetDockConfig {
                navigator_open: true,
                family_open: true,
                inspector_open: true,
                leading_collapsed_rail: false,
                trailing_collapsed_rail: false,
                leading_width_pt: 240.0,
                trailing_width_pt: 280.0,
            },
            Self::DataPrep => PresetDockConfig {
                navigator_open: true,
                family_open: false,
                inspector_open: true,
                leading_collapsed_rail: false,
                trailing_collapsed_rail: true,
                leading_width_pt: 220.0,
                trailing_width_pt: 280.0,
            },
            Self::Modeling => PresetDockConfig {
                navigator_open: true,
                family_open: true,
                inspector_open: true,
                leading_collapsed_rail: false,
                trailing_collapsed_rail: false,
                leading_width_pt: 210.0,
                trailing_width_pt: 320.0,
            },
            Self::Publication => PresetDockConfig {
                navigator_open: true,
                family_open: false,
                inspector_open: true,
                leading_collapsed_rail: true,
                trailing_collapsed_rail: false,
                leading_width_pt: 240.0,
                trailing_width_pt: 300.0,
            },
            Self::Compact => PresetDockConfig {
                navigator_open: true,
                family_open: false,
                inspector_open: true,
                leading_collapsed_rail: true,
                trailing_collapsed_rail: true,
                leading_width_pt: 210.0,
                trailing_width_pt: 260.0,
            },
        }
    }
}

/// Persistent pure-UI `egui_shell` view states owned alongside [`UiApp`] (`R15.2+`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EguiShellViewState {
    pub column_grid: data_grid::ColumnGridEguiState,
    pub anova_modal: stat_anova_modal::AnovaModalEguiState,
    pub manage_vars_modal: manage_vars_modal::ManageVarsModalEguiState,
    pub import_wizard: import_wizard::ImportWizardEguiState,
    pub transform_modals: transform_modals::TransformModalsEguiState,
    pub stats_modals: stats_modals::StatsModalsEguiState,
    pub fit_modal: fit_modal::FitModalEguiState,
    pub power_studio: power_studio::PowerStudioEguiState,
    pub graph_modals: graph_modals::GraphModalEguiState,
    pub app_modals: app_modals::AppModalsEguiState,
    pub workspace_preset: WorkspacePreset,
    pub top_chrome: top_chrome::TopChromeEguiState,
    pub command_palette: command_palette::CommandPaletteEguiState,
    pub welcome_hub: welcome_hub::WelcomeHubEguiState,
    pub leading_dock: leading_dock::LeadingDockEguiState,
    pub center_stage: center_stage::CenterStageEguiState,
    pub info_view: info_view::InfoViewEguiState,
    pub results_view: results_view::ResultsViewEguiState,
    pub canvas_view: canvas_view::CanvasViewEguiState,
    pub trailing_studio: trailing_studio::TrailingStudioEguiState,
    pub status_footer: status_footer::StatusFooterEguiState,
    pub launcher_open: bool,
    pub leading_width_pt: Option<f32>,
    pub trailing_width_pt: Option<f32>,
    pub leading_collapsed_rail: bool,
    pub trailing_collapsed_rail: bool,
    pub leading_flyout_open: bool,
    pub trailing_flyout_open: bool,
    pub active_sheet_history: Vec<SheetId>,
    pub history_cursor: usize,
    pub quick_stats_cache: status_footer::QuickStatsCache,
    pub progress_modal: Option<center_stage::ProgressModalState>,
    pub last_shell_layout: Option<ResponsiveShellLayout>,
}

/// Resolve any sheet's display title by [`SheetId`] across all five folders.
#[must_use]
pub fn doc_sheet_title<'a>(doc: &'a curvebook_doc::ProjectDocument, id: &SheetId) -> Option<&'a str> {
    if let Some(ds) = doc.data_sheet(id) {
        return Some(ds.title.as_str());
    }
    if let Some(info) = doc.info_sheet(id) {
        return Some(info.title.as_str());
    }
    if let Some(rs) = doc.result_sheet(id) {
        return Some(rs.title());
    }
    if let Some(gs) = doc.graph_sheet(id) {
        return Some(gs.title.as_str());
    }
    doc.layout_sheet(id).map(|ls| ls.title.as_str())
}

impl EguiShellViewState {
    /// Record an active sheet visit onto `< >` navigation history unless already at the cursor.
    pub fn record_active_sheet_visit(&mut self, sheet_id: &SheetId) {
        if let Some(current) = self.active_sheet_history.get(self.history_cursor)
            && current == sheet_id
        {
            return;
        }
        if self.history_cursor + 1 < self.active_sheet_history.len() {
            self.active_sheet_history.truncate(self.history_cursor + 1);
        }
        self.active_sheet_history.push(sheet_id.clone());
        self.history_cursor = self.active_sheet_history.len().saturating_sub(1);
    }

    /// Prune deleted sheets cleanly and record `active_sheet_id` when missing from cursor.
    pub fn sync_history_with_document(&mut self, doc: &curvebook_doc::ProjectDocument) {
        self.active_sheet_history.retain(|id| doc_sheet_title(doc, id).is_some());
        if self.active_sheet_history.is_empty() {
            self.history_cursor = 0;
        } else if self.history_cursor >= self.active_sheet_history.len() {
            self.history_cursor = self.active_sheet_history.len() - 1;
        }
        if let Some(active) = doc.active_sheet_id.as_ref() {
            self.record_active_sheet_visit(active);
        }
    }

    /// Return `true` when stepping Back (`<`) moves to an earlier visited sheet.
    #[must_use]
    pub const fn can_go_back(&self) -> bool {
        self.history_cursor > 0 && !self.active_sheet_history.is_empty()
    }

    /// Return `true` when stepping Forward (`>`) moves to a newer visited sheet.
    #[must_use]
    pub const fn can_go_forward(&self) -> bool {
        self.history_cursor + 1 < self.active_sheet_history.len()
    }
}

/// Active desktop UI presentation backend (`--ui-backend=egui`, sole desktop presenter post-R15.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiBackend {
    #[default]
    Egui,
}

impl UiBackend {
    /// Parse a backend selector flag or environment value (`"egui"`).
    ///
    /// Legacy backend (`"legacy"`) has been retired and returns `None`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "egui" => Some(Self::Egui),
            _ => None,
        }
    }

    /// Canonical lowercase CLI/environment string identifier for this backend.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Egui => "egui",
        }
    }

    /// Resolve backend selection from optional `CURVEBOOK_UI_BACKEND` env and `--ui-backend` CLI flag.
    ///
    /// CLI flag takes precedence over environment variable; unset inputs resolve to [`Self::Egui`].
    ///
    /// # Errors
    /// Returns an actionable message when `"legacy"` or any non-empty unrecognized value is supplied.
    pub fn from_env_and_args(env_val: Option<&str>, cli_val: Option<&str>) -> Result<Self, String> {
        if let Some(cli) = cli_val.map(str::trim).filter(|s| !s.is_empty()) {
            if cli.eq_ignore_ascii_case("legacy") {
                return Err(String::from("invalid --ui-backend value `legacy`: legacy UI backend has been retired; use `egui`"));
            }
            return Self::parse(cli).ok_or_else(|| format!("invalid --ui-backend value `{cli}`; expected `egui`"));
        }
        if let Some(env_raw) = env_val.map(str::trim).filter(|s| !s.is_empty()) {
            if env_raw.eq_ignore_ascii_case("legacy") {
                return Err(String::from("invalid CURVEBOOK_UI_BACKEND value `legacy`: legacy UI backend has been retired; use `egui`"));
            }
            return Self::parse(env_raw).ok_or_else(|| format!("invalid CURVEBOOK_UI_BACKEND value `{env_raw}`; expected `egui`"));
        }
        Ok(Self::Egui)
    }

    /// Resolve backend selection from `CURVEBOOK_UI_BACKEND`, defaulting to [`Self::Egui`].
    #[must_use]
    pub fn from_process_env() -> Self {
        std::env::var("CURVEBOOK_UI_BACKEND").ok().and_then(|v| Self::parse(&v)).unwrap_or_default()
    }
}

/// Canonical `1.0x` logical-point geometry and typography tokens (`docs/planning/gui-layout-spec.md` §9.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutMetrics {
    pub app_bar_h: f32,
    pub action_strip_h: f32,
    pub status_bar_h: f32,
    pub navigator_default_w: f32,
    pub navigator_min_w: f32,
    pub navigator_max_w: f32,
    pub navigator_collapse_breakpoint_w: f32,
    pub inspector_default_w: f32,
    pub inspector_min_w: f32,
    pub inspector_max_w: f32,
    pub inspector_collapse_breakpoint_w: f32,
    pub dock_collapsed_rail_w: f32,
    pub splitter_visual_w: f32,
    pub splitter_hit_slop_w: f32,
    pub center_stage_min_w: f32,
    pub split_vertical_breakpoint_w: f32,
    pub grid_row_h: f32,
    pub grid_header_tier_heights: [f32; 3],
    pub min_hit_target_pt: f32,
    pub min_toolbar_target_pt: f32,
    pub scroll_bar_thickness: f32,
    pub scroll_bar_hover_thickness: f32,
    pub font_ui_body_pt: f32,
    pub font_ui_small_pt: f32,
    pub font_grid_numeral_pt: f32,
    pub min_window_w_pt: f32,
    pub min_window_h_pt: f32,
    pub default_window_w_pt: f32,
    pub default_window_h_pt: f32,
    pub wide_desktop_breakpoint_w: f32,
}

impl LayoutMetrics {
    /// Canonical `1.0x` logical-point metrics from `docs/planning/gui-layout-spec.md` §9.1.
    pub const CANONICAL: Self = Self {
        app_bar_h: 32.0,
        action_strip_h: 38.0,
        status_bar_h: 26.0,
        navigator_default_w: 240.0,
        navigator_min_w: 180.0,
        navigator_max_w: 380.0,
        navigator_collapse_breakpoint_w: 760.0,
        inspector_default_w: 280.0,
        inspector_min_w: 220.0,
        inspector_max_w: 420.0,
        inspector_collapse_breakpoint_w: 960.0,
        dock_collapsed_rail_w: 36.0,
        splitter_visual_w: 4.0,
        splitter_hit_slop_w: 8.0,
        center_stage_min_w: 360.0,
        split_vertical_breakpoint_w: 720.0,
        grid_row_h: 24.0,
        grid_header_tier_heights: [20.0, 24.0, 20.0],
        min_hit_target_pt: 24.0,
        min_toolbar_target_pt: 28.0,
        scroll_bar_thickness: 10.0,
        scroll_bar_hover_thickness: 12.0,
        font_ui_body_pt: 13.0,
        font_ui_small_pt: 11.5,
        font_grid_numeral_pt: 12.5,
        min_window_w_pt: 640.0,
        min_window_h_pt: 420.0,
        default_window_w_pt: 1280.0,
        default_window_h_pt: 800.0,
        wide_desktop_breakpoint_w: 1440.0,
    };

    /// Return the canonical `1.0x` layout metrics.
    #[must_use]
    pub const fn canonical() -> Self {
        Self::CANONICAL
    }

    /// Snap a logical `pt` value to rounded device pixels under `scale_factor`.
    #[must_use]
    pub fn scaled_px(pt: f32, scale_factor: f32) -> f32 {
        let safe_pt = if pt.is_finite() && pt > 0.0 { pt } else { 0.0 };
        let safe_scale = if scale_factor.is_finite() { scale_factor.clamp(0.5, 8.0) } else { 1.0 };
        (safe_pt * safe_scale).round()
    }

    /// Total top chrome height (`70 pt` normally, `0 pt` in Presentation Mode).
    #[must_use]
    pub const fn top_chrome_height(self, presentation_mode: bool) -> f32 {
        if presentation_mode { 0.0 } else { self.app_bar_h + self.action_strip_h }
    }

    /// Bottom status footer height (`26 pt` normally, `0 pt` in Presentation Mode).
    #[must_use]
    pub const fn status_bar_height(self, presentation_mode: bool) -> f32 {
        if presentation_mode { 0.0 } else { self.status_bar_h }
    }

    /// Combined 3-tier frozen column-grid header height (`64 pt`).
    #[must_use]
    pub const fn grid_header_total_height(self) -> f32 {
        self.grid_header_tier_heights[0] + self.grid_header_tier_heights[1] + self.grid_header_tier_heights[2]
    }

    /// Solve responsive dock collapse, splitter widths, and Center Stage budget (`gui-layout-spec.md` §2.3).
    #[must_use]
    pub fn solve_shell_layout(self, input: ShellLayoutInput) -> ResponsiveShellLayout {
        let w = sanitize_nonneg(input.viewport_w_pt);
        let h = sanitize_nonneg(input.viewport_h_pt);
        let tier = if w >= self.wide_desktop_breakpoint_w {
            ViewportTier::WideDesktop
        } else if w >= self.inspector_collapse_breakpoint_w {
            ViewportTier::StandardLaptop
        } else if w >= self.min_window_w_pt {
            ViewportTier::CompactTablet
        } else {
            ViewportTier::NarrowHeadless
        };
        let split_orientation = if w >= self.split_vertical_breakpoint_w { SplitOrientation::Horizontal } else { SplitOrientation::Vertical };

        if input.presentation_mode {
            return ResponsiveShellLayout {
                tier,
                top_chrome_h_pt: 0.0,
                tier1_app_bar_h_pt: 0.0,
                tier2_action_strip_h_pt: 0.0,
                status_bar_h_pt: 0.0,
                leading_dock_w_pt: 0.0,
                leading_collapsed_to_rail: false,
                leading_visible: false,
                leading_splitter_w_pt: 0.0,
                family_visible: false,
                trailing_dock_w_pt: 0.0,
                trailing_collapsed_to_rail: false,
                trailing_visible: false,
                trailing_splitter_w_pt: 0.0,
                center_stage_w_pt: w,
                center_stage_h_pt: h,
                split_orientation,
                rtl_mirrored: false,
            };
        }

        let tier1_app_bar_h_pt = self.app_bar_h.min(h * 0.45).max(0.0);
        let tier2_action_strip_h_pt = self.action_strip_h.min((h - tier1_app_bar_h_pt).max(0.0) * 0.65).max(0.0);
        let top_chrome_h_pt = tier1_app_bar_h_pt + tier2_action_strip_h_pt;
        let status_bar_h_pt = self.status_bar_h.min((h - top_chrome_h_pt).max(0.0)).max(0.0);
        let center_stage_h_pt = (h - top_chrome_h_pt - status_bar_h_pt).max(0.0);

        if w < 420.0 {
            return ResponsiveShellLayout {
                tier,
                top_chrome_h_pt,
                tier1_app_bar_h_pt,
                tier2_action_strip_h_pt,
                status_bar_h_pt,
                leading_dock_w_pt: 0.0,
                leading_collapsed_to_rail: false,
                leading_visible: false,
                leading_splitter_w_pt: 0.0,
                family_visible: false,
                trailing_dock_w_pt: 0.0,
                trailing_collapsed_to_rail: false,
                trailing_visible: false,
                trailing_splitter_w_pt: 0.0,
                center_stage_w_pt: w,
                center_stage_h_pt,
                split_orientation,
                rtl_mirrored: false,
            };
        }

        if input.empty_workspace {
            let show_rails = w >= self.min_window_w_pt;
            let lead_w = if show_rails && input.navigator_open { self.dock_collapsed_rail_w } else { 0.0 };
            let trail_w = if show_rails && input.inspector_open { self.dock_collapsed_rail_w } else { 0.0 };
            return ResponsiveShellLayout {
                tier,
                top_chrome_h_pt,
                tier1_app_bar_h_pt,
                tier2_action_strip_h_pt,
                status_bar_h_pt,
                leading_dock_w_pt: lead_w,
                leading_collapsed_to_rail: lead_w > 0.0,
                leading_visible: lead_w > 0.0,
                leading_splitter_w_pt: 0.0,
                family_visible: false,
                trailing_dock_w_pt: trail_w,
                trailing_collapsed_to_rail: trail_w > 0.0,
                trailing_visible: trail_w > 0.0,
                trailing_splitter_w_pt: 0.0,
                center_stage_w_pt: (w - lead_w - trail_w).max(0.0),
                center_stage_h_pt,
                split_orientation,
                rtl_mirrored: false,
            };
        }

        let rail = self.dock_collapsed_rail_w;
        let split = self.splitter_visual_w;
        let mut lead_collapsed = !input.navigator_open || input.leading_user_collapsed || w < self.navigator_collapse_breakpoint_w;
        let mut trail_collapsed = !input.inspector_open || input.trailing_user_collapsed || w < self.inspector_collapse_breakpoint_w;

        let mut lead_w = if lead_collapsed {
            rail
        } else {
            input.leading_width_override_pt.unwrap_or(self.navigator_default_w).clamp(self.navigator_min_w, self.navigator_max_w)
        };
        let mut trail_w = if trail_collapsed {
            rail
        } else {
            input.trailing_width_override_pt.unwrap_or(self.inspector_default_w).clamp(self.inspector_min_w, self.inspector_max_w)
        };
        let mut lead_split = if lead_collapsed { 0.0 } else { split };
        let mut trail_split = if trail_collapsed { 0.0 } else { split };

        let target_center_min =
            if !lead_collapsed && !trail_collapsed && w >= self.inspector_collapse_breakpoint_w { 480.0 } else { self.center_stage_min_w };

        let mut center_w = w - lead_w - lead_split - trail_w - trail_split;
        if center_w < target_center_min {
            let deficit = target_center_min - center_w;
            if !trail_collapsed && !lead_collapsed {
                let lead_slack = (lead_w - self.navigator_min_w).max(0.0);
                let trail_slack = (trail_w - self.inspector_min_w).max(0.0);
                let total_slack = lead_slack + trail_slack;
                if total_slack > 0.0 {
                    let take = deficit.min(total_slack);
                    lead_w -= take * (lead_slack / total_slack);
                    trail_w -= take * (trail_slack / total_slack);
                }
            } else if !lead_collapsed {
                let lead_slack = (lead_w - self.navigator_min_w).max(0.0);
                lead_w -= deficit.min(lead_slack);
            } else if !trail_collapsed {
                let trail_slack = (trail_w - self.inspector_min_w).max(0.0);
                trail_w -= deficit.min(trail_slack);
            }
            center_w = w - lead_w - lead_split - trail_w - trail_split;
        }

        if center_w < self.center_stage_min_w && !trail_collapsed {
            trail_collapsed = true;
            trail_w = rail;
            trail_split = 0.0;
            center_w = w - lead_w - lead_split - trail_w - trail_split;
        }
        if center_w < self.center_stage_min_w && !lead_collapsed {
            lead_collapsed = true;
            lead_w = rail;
            lead_split = 0.0;
            center_w = w - lead_w - lead_split - trail_w - trail_split;
        }

        ResponsiveShellLayout {
            tier,
            top_chrome_h_pt,
            tier1_app_bar_h_pt,
            tier2_action_strip_h_pt,
            status_bar_h_pt,
            leading_dock_w_pt: lead_w,
            leading_collapsed_to_rail: lead_collapsed,
            leading_visible: lead_w > 0.0,
            leading_splitter_w_pt: lead_split,
            family_visible: !lead_collapsed && input.family_open,
            trailing_dock_w_pt: trail_w,
            trailing_collapsed_to_rail: trail_collapsed,
            trailing_visible: trail_w > 0.0,
            trailing_splitter_w_pt: trail_split,
            center_stage_w_pt: center_w.max(0.0),
            center_stage_h_pt,
            split_orientation,
            rtl_mirrored: false,
        }
    }
}

const fn sanitize_nonneg(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 { value } else { 0.0 }
}

/// Responsive viewport width tier (`gui-layout-spec.md` §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ViewportTier {
    WideDesktop,
    #[default]
    StandardLaptop,
    CompactTablet,
    NarrowHeadless,
}

/// Center split orientation derived from window width (`< 720 pt` switches to Vertical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SplitOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Pure input parameters evaluated by [`LayoutMetrics::solve_shell_layout`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellLayoutInput {
    pub viewport_w_pt: f32,
    pub viewport_h_pt: f32,
    pub presentation_mode: bool,
    pub empty_workspace: bool,
    pub navigator_open: bool,
    pub inspector_open: bool,
    pub family_open: bool,
    pub leading_user_collapsed: bool,
    pub trailing_user_collapsed: bool,
    pub leading_width_override_pt: Option<f32>,
    pub trailing_width_override_pt: Option<f32>,
    pub host_draws_menu_bar: bool,
}

/// Deterministic panel dimensions and collapse state computed once per frame for `egui_shell`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsiveShellLayout {
    pub tier: ViewportTier,
    pub top_chrome_h_pt: f32,
    pub tier1_app_bar_h_pt: f32,
    pub tier2_action_strip_h_pt: f32,
    pub status_bar_h_pt: f32,
    pub leading_dock_w_pt: f32,
    pub leading_collapsed_to_rail: bool,
    pub leading_visible: bool,
    pub leading_splitter_w_pt: f32,
    pub family_visible: bool,
    pub trailing_dock_w_pt: f32,
    pub trailing_collapsed_to_rail: bool,
    pub trailing_visible: bool,
    pub trailing_splitter_w_pt: f32,
    pub center_stage_w_pt: f32,
    pub center_stage_h_pt: f32,
    pub split_orientation: SplitOrientation,
    pub rtl_mirrored: bool,
}

impl Default for LayoutMetrics {
    fn default() -> Self {
        Self::CANONICAL
    }
}

/// Convert [`Rgba8`] into an [`egui::Color32`].
#[must_use]
pub const fn rgba8_to_color32(color: Rgba8) -> Color32 {
    Color32::from_rgba_premultiplied(color.r, color.g, color.b, color.a)
}

/// Convert an unpremultiplied/opaque [`egui::Color32`] to [`Rgba8`].
#[must_use]
pub const fn color32_to_rgba8(color: Color32) -> Rgba8 {
    Rgba8::rgba(color.r(), color.g(), color.b(), color.a())
}

/// Canonical 8-color Okabe-Ito categorical series palette (`gui-layout-spec.md` §9.2).
pub const OKABE_ITO_SERIES_PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x00, 0x00, 0x00),
    Color32::from_rgb(0xE6, 0x9F, 0x00),
    Color32::from_rgb(0x56, 0xB4, 0xE9),
    Color32::from_rgb(0x00, 0x9E, 0x73),
    Color32::from_rgb(0xF0, 0xE4, 0x42),
    Color32::from_rgb(0x00, 0x72, 0xB2),
    Color32::from_rgb(0xD5, 0x5E, 0x00),
    Color32::from_rgb(0xCC, 0x79, 0xA7),
];

/// Diverging Blue-White-Vermilion colorblind-safe ramp stops (`-1 -> 0 -> +1`).
pub const BLUE_WHITE_VERMILION_STOPS: [Color32; 3] =
    [Color32::from_rgb(0x00, 0x72, 0xB2), Color32::from_rgb(0xF7, 0xF7, 0xF7), Color32::from_rgb(0xD5, 0x5E, 0x00)];

/// Scientific categorical and perceptual colormap accessors for `egui` views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScientificPaletteTokens;

impl ScientificPaletteTokens {
    /// Okabe-Ito 8-color series palette (`gui-layout-spec.md` §9.2).
    #[must_use]
    pub const fn okabe_ito_series() -> [Color32; 8] {
        OKABE_ITO_SERIES_PALETTE
    }

    /// Okabe-Ito swatches in `curvebook-palette` default ordering.
    #[must_use]
    pub fn okabe_ito_palette_order() -> Vec<Color32> {
        OKABE_ITO.iter().copied().map(rgba8_to_color32).collect()
    }

    /// 17-stop Viridis perceptual colormap (`t = 0/16 .. 16/16`).
    #[must_use]
    pub fn viridis_stops() -> [Color32; 17] {
        VIRIDIS_STOPS.map(rgba8_to_color32)
    }

    /// 17-stop Cividis CVD-optimized colormap (`t = 0/16 .. 16/16`).
    #[must_use]
    pub fn cividis_stops() -> [Color32; 17] {
        CIVIDIS_STOPS.map(rgba8_to_color32)
    }

    /// Diverging Blue-White-Vermilion stops (`-1 -> 0 -> +1`).
    #[must_use]
    pub const fn blue_white_vermilion_stops() -> [Color32; 3] {
        BLUE_WHITE_VERMILION_STOPS
    }
}

/// Complete WCAG 2.1 AA theme, semantic role, provenance, and geometry tokens for the `egui` shell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub mode: UiThemeMode,
    pub dark: bool,
    pub locale: &'static str,
    pub ui_scale: f32,
    pub metrics: LayoutMetrics,
    pub app_bar: Color32,
    pub panel_bg: Color32,
    pub surface_bg: Color32,
    pub canvas_mat: Color32,
    pub header_bg: Color32,
    pub border: Color32,
    pub divider: Color32,
    pub hover_bg: Color32,
    pub active_bg: Color32,
    pub focus_ring: Color32,
    pub input_bg: Color32,
    pub input_border: Color32,
    pub button_bg: Color32,
    pub button_border: Color32,
    pub text_primary: Color32,
    pub text_strong: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    pub icon: Color32,
    pub accent: Color32,
    pub accent_strong: Color32,
    pub paper_white: Color32,
    pub scrim: Color32,
    pub role_num: Color32,
    pub role_cat: Color32,
    pub role_lbl: Color32,
    pub role_fx: Color32,
    pub link_blue: Color32,
    pub link_green: Color32,
    pub link_red: Color32,
    pub excluded_cell_text: Color32,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::for_mode(UiThemeMode::Light)
    }
}

impl Tokens {
    /// Construct canonical [`Tokens`] for `mode` at `1.0x` scale and `"en"` locale.
    #[must_use]
    pub fn for_mode(mode: UiThemeMode) -> Self {
        Self::from_theme_spec(&UiThemeSpec::for_mode(mode), "en")
    }

    /// Construct canonical [`Tokens`] from a [`UiThemeSpec`] and normalized BCP-47 `locale`.
    #[must_use]
    pub fn from_theme_spec(spec: &UiThemeSpec, locale: &str) -> Self {
        let dark = !matches!(spec.mode, UiThemeMode::Light);
        let ui_scale = if spec.ui_scale.is_finite() { spec.ui_scale.clamp(0.75, 3.0) } else { 1.0 };
        let canonical_locale = i18n::normalize_locale(locale);
        let panel_bg = rgba8_to_color32(spec.panel_bg);
        let surface_bg = rgba8_to_color32(spec.surface_bg);
        let text_primary = rgba8_to_color32(spec.text_primary);
        let text_muted = rgba8_to_color32(spec.text_muted);
        let accent = rgba8_to_color32(spec.accent);
        let border = rgba8_to_color32(spec.border);

        match spec.mode {
            UiThemeMode::Light => Self {
                mode: spec.mode,
                dark,
                locale: canonical_locale,
                ui_scale,
                metrics: LayoutMetrics::CANONICAL,
                app_bar: Color32::from_rgb(234, 237, 242),
                panel_bg,
                surface_bg,
                canvas_mat: Color32::from_rgb(237, 240, 245),
                header_bg: Color32::from_rgb(232, 236, 242),
                border,
                divider: Color32::from_rgb(214, 219, 228),
                hover_bg: Color32::from_rgb(228, 234, 244),
                active_bg: Color32::from_rgb(212, 224, 245),
                focus_ring: Color32::from_rgb(29, 78, 216),
                input_bg: Color32::WHITE,
                input_border: Color32::from_rgb(132, 140, 154),
                button_bg: Color32::from_rgb(232, 236, 242),
                button_border: Color32::from_rgb(120, 129, 145),
                text_primary,
                text_strong: Color32::from_rgb(15, 19, 26),
                text_muted,
                text_disabled: Color32::from_rgb(125, 134, 148),
                icon: Color32::from_rgb(38, 45, 58),
                accent,
                accent_strong: Color32::from_rgb(24, 64, 184),
                paper_white: rgba8_to_color32(spec.paper_white),
                scrim: Color32::from_rgba_unmultiplied(15, 23, 42, 128),
                role_num: Color32::from_rgb(29, 78, 216),
                role_cat: Color32::from_rgb(180, 62, 0),
                role_lbl: Color32::from_rgb(71, 85, 105),
                role_fx: Color32::from_rgb(4, 120, 87),
                link_blue: rgba8_to_color32(spec.link_blue),
                link_green: rgba8_to_color32(spec.link_green),
                link_red: rgba8_to_color32(spec.link_red),
                excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
            },
            UiThemeMode::Dark => Self {
                mode: spec.mode,
                dark,
                locale: canonical_locale,
                ui_scale,
                metrics: LayoutMetrics::CANONICAL,
                app_bar: Color32::from_rgb(20, 23, 29),
                panel_bg,
                surface_bg,
                canvas_mat: Color32::from_rgb(21, 24, 30),
                header_bg: Color32::from_rgb(38, 43, 53),
                border,
                divider: Color32::from_rgb(56, 63, 76),
                hover_bg: Color32::from_rgb(45, 52, 66),
                active_bg: Color32::from_rgb(52, 66, 92),
                focus_ring: Color32::from_rgb(96, 165, 250),
                input_bg: Color32::from_rgb(20, 23, 29),
                input_border: border,
                button_bg: Color32::from_rgb(43, 49, 60),
                button_border: Color32::from_rgb(112, 123, 140),
                text_primary,
                text_strong: Color32::from_rgb(248, 250, 252),
                text_muted,
                text_disabled: Color32::from_rgb(128, 138, 152),
                icon: Color32::from_rgb(212, 220, 232),
                accent,
                accent_strong: Color32::from_rgb(37, 99, 235),
                paper_white: rgba8_to_color32(spec.paper_white),
                scrim: Color32::from_rgba_unmultiplied(15, 23, 42, 128),
                role_num: Color32::from_rgb(96, 165, 250),
                role_cat: Color32::from_rgb(251, 146, 60),
                role_lbl: Color32::from_rgb(148, 163, 184),
                role_fx: Color32::from_rgb(52, 211, 153),
                link_blue: rgba8_to_color32(spec.link_blue),
                link_green: rgba8_to_color32(spec.link_green),
                link_red: rgba8_to_color32(spec.link_red),
                excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
            },
            UiThemeMode::HighContrast => Self {
                mode: spec.mode,
                dark,
                locale: canonical_locale,
                ui_scale,
                metrics: LayoutMetrics::CANONICAL,
                app_bar: Color32::BLACK,
                panel_bg,
                surface_bg,
                canvas_mat: Color32::from_rgb(8, 10, 14),
                header_bg: Color32::from_rgb(22, 26, 34),
                border,
                divider: Color32::from_rgb(160, 170, 188),
                hover_bg: Color32::from_rgb(32, 38, 52),
                active_bg: Color32::from_rgb(38, 60, 96),
                focus_ring: Color32::from_rgb(125, 211, 252),
                input_bg: Color32::BLACK,
                input_border: border,
                button_bg: Color32::from_rgb(24, 28, 36),
                button_border: Color32::from_rgb(210, 218, 230),
                text_primary,
                text_strong: Color32::WHITE,
                text_muted,
                text_disabled: Color32::from_rgb(165, 174, 188),
                icon: Color32::WHITE,
                accent,
                accent_strong: Color32::from_rgb(14, 116, 144),
                paper_white: rgba8_to_color32(spec.paper_white),
                scrim: Color32::from_rgba_unmultiplied(15, 23, 42, 128),
                role_num: Color32::from_rgb(125, 211, 252),
                role_cat: Color32::from_rgb(253, 186, 116),
                role_lbl: Color32::from_rgb(203, 213, 225),
                role_fx: Color32::from_rgb(110, 231, 183),
                link_blue: rgba8_to_color32(spec.link_blue),
                link_green: rgba8_to_color32(spec.link_green),
                link_red: rgba8_to_color32(spec.link_red),
                excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
            },
        }
    }

    /// Retrieve active [`Tokens`] cached in `ctx`, defaulting to [`UiThemeMode::Light`].
    #[must_use]
    pub fn get(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<Self>(egui::Id::NULL)).unwrap_or_default()
    }

    /// Build [`egui::Visuals`] aligned with these WCAG 2.1 AA theme tokens.
    #[must_use]
    pub fn to_visuals(&self) -> Visuals {
        let mut visuals = if self.dark { Visuals::dark() } else { Visuals::light() };
        visuals.dark_mode = self.dark;
        visuals.override_text_color = Some(self.text_primary);
        visuals.weak_text_color = Some(self.text_muted);
        visuals.panel_fill = self.panel_bg;
        visuals.window_fill = self.surface_bg;
        visuals.extreme_bg_color = self.input_bg;
        visuals.faint_bg_color = self.header_bg;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(6);
        visuals.selection.bg_fill = self.active_bg;
        visuals.selection.stroke = Stroke::new(2.0, self.focus_ring);
        visuals.widgets.noninteractive.bg_fill = self.panel_bg;
        visuals.widgets.noninteractive.weak_bg_fill = self.panel_bg;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text_primary);
        visuals.widgets.inactive.bg_fill = self.button_bg;
        visuals.widgets.inactive.weak_bg_fill = self.button_bg;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.button_border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.text_primary);
        visuals.widgets.hovered.bg_fill = self.hover_bg;
        visuals.widgets.hovered.weak_bg_fill = self.hover_bg;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.focus_ring);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.text_strong);
        visuals.widgets.active.bg_fill = self.active_bg;
        visuals.widgets.active.weak_bg_fill = self.active_bg;
        visuals.widgets.active.bg_stroke = Stroke::new(2.0, self.focus_ring);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.text_strong);
        visuals.widgets.open.bg_fill = self.hover_bg;
        visuals.widgets.open.weak_bg_fill = self.hover_bg;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, self.text_strong);
        visuals
    }

    /// Apply theme visuals, canonical typography sizes, and non-overlapping scrollbar metrics to `style`.
    pub fn configure_style(&self, style: &mut egui::Style) {
        style.visuals = self.to_visuals();
        style.text_styles.insert(TextStyle::Body, FontId::proportional(self.metrics.font_ui_body_pt));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(self.metrics.font_ui_body_pt));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(self.metrics.font_ui_small_pt));
        style.text_styles.insert(TextStyle::Monospace, FontId::monospace(self.metrics.font_grid_numeral_pt));
        style.text_styles.insert(TextStyle::Heading, FontId::proportional(15.0));

        style.spacing.scroll.floating = false;
        style.spacing.scroll.bar_width = self.metrics.scroll_bar_thickness;
        style.spacing.scroll.bar_inner_margin = 2.0;
        style.spacing.scroll.bar_outer_margin = 2.0;
        style.spacing.interact_size.y = self.metrics.min_hit_target_pt;
    }

    /// Synchronize the currently executing [`egui::Ui`] style immediately (including pass-0 frames).
    pub fn apply_to_ui(&self, ui: &mut egui::Ui) {
        self.configure_style(ui.style_mut());
    }

    /// Install these tokens, visuals, canonical typography sizes, and non-overlapping scrollbar
    /// gutter once per frame on `ctx` without changing `zoom_factor`.
    pub fn apply_theme_to_ctx(&self, ctx: &egui::Context) {
        let egui_theme = if self.dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, *self));
        ctx.set_theme(egui_theme);
        ctx.set_visuals(self.to_visuals());
        ctx.all_styles_mut(|style| {
            self.configure_style(style);
        });
    }

    /// Install these tokens, visuals, canonical typography sizes, non-overlapping scrollbar gutter,
    /// and `ui_scale` once per frame on `ctx`.
    pub fn apply_to_ctx(&self, ctx: &egui::Context) {
        self.apply_theme_to_ctx(ctx);
        ctx.set_zoom_factor(self.ui_scale.clamp(0.75, 3.0));
    }

    /// Verify all primary text and interactive UI boundary pairs satisfy WCAG 2.1 AA contrast.
    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        let text_pairs = [
            (self.text_primary, self.surface_bg),
            (self.text_primary, self.panel_bg),
            (self.text_primary, self.app_bar),
            (self.text_primary, self.header_bg),
            (self.text_muted, self.surface_bg),
            (self.text_muted, self.panel_bg),
            (self.text_muted, self.app_bar),
        ];
        let ui_pairs = [
            (self.border, self.surface_bg),
            (self.border, self.panel_bg),
            (self.input_border, self.surface_bg),
            (self.button_border, self.panel_bg),
            (self.focus_ring, self.surface_bg),
            (self.role_num, self.surface_bg),
            (self.role_cat, self.surface_bg),
            (self.role_lbl, self.surface_bg),
            (self.role_fx, self.surface_bg),
        ];
        text_pairs.into_iter().all(|(fg, bg)| color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg)) >= 4.5)
            && ui_pairs.into_iter().all(|(fg, bg)| color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg)) >= 3.0)
    }
}

/// User intent captured during an `egui` pass and deferred until `!ctx.will_discard()` settles.
#[derive(Debug, Clone, PartialEq)]
pub enum UiIntent {
    EngineCommand { command_id: String, params: serde_json::Value },
    UiCommand { command_id: String, params: serde_json::Value },
    SetTheme(UiThemeMode),
    SetLocale(String),
    SetUiScale(f32),
    SelectGridCell { coord: curvebook_tools::GridCellCoord },
    ExtendGridSelection { coord: curvebook_tools::GridCellCoord },
    SetRowTitlesCollapsed { collapsed: bool },
    DialogWidgetClick { widget_id: String },
    DialogButtonAction { action: String },
    SetTextFieldBuffer { widget_id: String, text: String },
    CustomViewAction { domain: String, payload: String },
    SelectResultTab { index: usize },
    SetActiveInspector { inspector: ActiveInspector },
    SetFocusPane { pane: ActiveWorkspacePane },
    SetCanvasZoom { zoom: f64 },
    ToggleNavigatorSection { section: SheetSection },
    SetNavigatorSearch { query: String },
    SetNavigatorNesting { mode: NavigatorNestingMode },
    SetSplitRatio { ratio: f64 },
    SetPreviewFilter { filter: CanvasPreviewFilter },
    SetPaletteOpen { open: bool },
    SetLauncherOpen { open: bool },
    ApplyWorkspacePreset { preset: WorkspacePreset },
    SyncLeadingWidth { width_pt: f32 },
    SyncTrailingWidth { width_pt: f32 },
    ToggleLeadingCollapsedRail,
    ToggleTrailingCollapsedRail,
    HistoryBack,
    HistoryForward,
    DismissDiagnosticBanner { sheet_id: SheetId },
    OpenRetargetDialog { result_sheet_id: SheetId, target_data_sheet_id: Option<SheetId> },
    BrushPoint { sheet_id: SheetId, coord: curvebook_tools::GridCellCoord },
    ClearBrush,
    PatchActiveDialog { fields: serde_json::Map<String, serde_json::Value> },
    CommitDialogAction { action: String },
    CloseAllModals,
    OpenManageVariables { sheet_id: SheetId, column: Option<usize> },
    SelectManageVarColumn { sheet_id: SheetId, column: usize },
    StageManageVarRecode { from: String, to: String },
    ClearManageVarRecodes,
    ApplyManageVarRecodes { sheet_id: SheetId },
    ApplyManageVarOrder { sheet_id: SheetId, column: usize, order: LevelOrderSpec, reference_level: Option<String> },
    CloseManageVariables,
    OpenImportWizard { origin: ImportOriginKind, is_live_link: bool, target_sheet_id: Option<SheetId> },
    UpdateImportWizard(Box<ImportPasteWizardState>),
    CommitImportWizard { close_after: bool },
    CloseImportWizard,
    OpenTransformModal { sheet_id: SheetId, kind: transform_modals::TransformModalKind },
    CloseTransformModal,
    GridCopySelection,
    GridCutSelection,
    GridPasteFromClipboard { start_coord: Option<curvebook_tools::GridCellCoord> },
    WorkspaceCopy { dpi: u16 },
    SyncResultSelection { sheet_id: SheetId, tab_index: usize, anchor: (usize, usize), focus: (usize, usize) },
    CanvasPaste { target_sheet_id: SheetId, anchor_pt: Option<[f64; 2]> },
    PasteImageFromClipboard { target_sheet_id: SheetId, anchor_pt: Option<[f64; 2]> },
    SetAnalyzeQuery { query: String },
    SelectAnalyzeEntry { entry_id: String, clear_duplicate_prompt: bool },
    ToggleAnalyzeDataset { col_index: usize },
    SetAnalyzeSelectedDatasets { cols: Vec<usize> },
    SetEditParamsDraft { spec: Box<curvebook_doc::AnalysisSpec> },
    SetEditParamsActiveTab { tab: String },
    SetEditParamsPairedOrMatched { paired: bool },
    SetEditParamsConfidence { level: ConfidenceLevel },
    SaveEditParamsAsDefault,
    SelectRetargetSource { target_data_sheet_id: SheetId },
    ResolveDuplicateAnalysis { modify_existing_id: Option<SheetId> },
    SelectHookConstant { index: Option<usize> },
    UnlinkHookConstant,
    UpdatePowerWizardState(Box<PowerWizardState>),
    CommitPowerWizard { action: String },
    UpdateChangeGraphTypeState(Box<ChangeGraphTypeState>),
    UpdateFormatGraphState(Box<FormatGraphState>),
    UpdateFormatAxesState(Box<FormatAxesState>),
    UpdateBubblePlotState(Box<BubblePlotInspectorState>),
    UpdatePairwiseState(Box<PairwiseComparisonsState>),
    UpdatePageLayoutState(Box<PageLayoutComposerState>),
    UpdateExportFigureState(Box<ExportFigureState>),
    UpdatePrintDialogState(Box<PrintDialogState>),
    UpdateCreateGraphState(Box<CreateGraphDialogState>),
    UpdateEqualizeScalingState(Box<EqualizeScalingDialogState>),
    UpdateFormatEmbeddedTableState(Box<FormatEmbeddedTableDialogState>),
    UpdateFormatImageState(Box<FormatImageDialogState>),
    UpdateApplyStyleState(Box<ApplyStyleFromState>),
    SetGallerySelection { sheet_ids: Vec<SheetId> },
    OpenPreferences { category: Option<app_modals::PreferencesCategory> },
    ClosePreferences { save: bool },
    SetShortcutBinding { command_id: String, combo: String },
    ResetShortcutBinding { command_id: Option<String> },
    UpdatePreferencesDraft(Box<app_modals::PreferencesModalState>),
    OpenRecoveryDialog { summary: Box<app_modals::RecoveryDialogSummary> },
    ResolveRecoveryDialog { decision: app_modals::RecoveryDecision },
    ResolveUnsavedChanges { action: app_modals::UnsavedCloseAction },
    OpenFindReplace { initial_query: Option<String> },
    CloseFindReplace,
    ExecuteFindReplace { replace_all: bool, find_query: String, replace_with: String, match_case: bool, match_entire_cell: bool },
    UpdateFeedbackDraft(Box<crate::feedback::FeedbackDraft>),
    OpenFeedbackOnGitHub,
    CopyFeedbackMarkdown,
    ResetFeedbackDraft,
}

/// Outcome produced when committing a buffered [`UiIntent`].
#[derive(Debug, Clone, PartialEq)]
pub enum UiIntentOutcome {
    Engine { command_id: String, outcome: CommandOutcome },
    Ui { command_id: String, result: Option<serde_json::Value> },
    ShellUpdated,
}

/// Single-commit FIFO queue buffering GUI intents across discardable `egui` layout passes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingUiIntents {
    queue: Vec<UiIntent>,
}

impl PendingUiIntents {
    /// Construct an empty intent queue.
    #[must_use]
    pub const fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// Discard all buffered intents without executing them.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Number of queued intents awaiting frame settlement.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns `true` when no intents are buffered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Inspect queued intents in FIFO order.
    #[must_use]
    pub fn intents(&self) -> &[UiIntent] {
        &self.queue
    }

    /// Enqueue a typed [`UiIntent`].
    pub fn push(&mut self, intent: UiIntent) {
        self.queue.push(intent);
    }

    /// Enqueue an engine command to execute once via `Caller::Ui` after layout settles.
    pub fn push_engine(&mut self, command_id: impl Into<String>, params: serde_json::Value) {
        self.queue.push(UiIntent::EngineCommand { command_id: command_id.into(), params });
    }

    /// Enqueue a front-end `ui.*` / `app.*` shell command to execute once after layout settles.
    pub fn push_ui(&mut self, command_id: impl Into<String>, params: serde_json::Value) {
        self.queue.push(UiIntent::UiCommand { command_id: command_id.into(), params });
    }

    /// Enqueue single-cell selection sync for `coord` without mutating `Session`.
    pub fn push_select_grid_cell(&mut self, coord: curvebook_tools::GridCellCoord) {
        self.queue.push(UiIntent::SelectGridCell { coord });
    }

    /// Enqueue O(1) rectangular selection extension to `coord` (`Shift+Click` / drag).
    pub fn push_extend_grid_selection(&mut self, coord: curvebook_tools::GridCellCoord) {
        self.queue.push(UiIntent::ExtendGridSelection { coord });
    }

    /// Enqueue Row Titles column collapse/expand state update.
    pub fn push_set_row_titles_collapsed(&mut self, collapsed: bool) {
        self.queue.push(UiIntent::SetRowTitlesCollapsed { collapsed });
    }

    /// Enqueue a dialog control click (`edit.assume_gaussian`, `edit.post_hoc.tukey`, etc.).
    pub fn push_dialog_widget_click(&mut self, widget_id: impl Into<String>) {
        self.queue.push(UiIntent::DialogWidgetClick { widget_id: widget_id.into() });
    }

    /// Enqueue a dialog footer button action (`"ok"`, `"cancel"`, `"apply"`, `"create"`).
    pub fn push_dialog_button(&mut self, action: impl Into<String>) {
        self.queue.push(UiIntent::DialogButtonAction { action: action.into() });
    }

    /// Synchronize a modal/view text input buffer (`widget_id`) into `app.interaction.text_field_buffers`.
    pub fn push_set_text_field_buffer(&mut self, widget_id: impl Into<String>, text: impl Into<String>) {
        self.queue.push(UiIntent::SetTextFieldBuffer { widget_id: widget_id.into(), text: text.into() });
    }

    /// Enqueue a view-shard custom action (`"stat_params"`, `"grid_core"`, etc.).
    pub fn push_custom_view_action(&mut self, domain: impl Into<String>, payload: impl Into<String>) {
        self.queue.push(UiIntent::CustomViewAction { domain: domain.into(), payload: payload.into() });
    }

    /// Select a `ResultSheet` tab index cleanly on settlement.
    pub fn push_select_result_tab(&mut self, index: usize) {
        self.queue.push(UiIntent::SelectResultTab { index });
    }

    /// Activate a specific [`ActiveInspector`] tab inside Trailing Studio Dock.
    pub fn push_set_active_inspector(&mut self, inspector: ActiveInspector) {
        self.queue.push(UiIntent::SetActiveInspector { inspector });
    }

    /// Update focused top-level workspace pane (`F6` / `Shift+F6` / click).
    pub fn push_set_focus_pane(&mut self, pane: ActiveWorkspacePane) {
        self.queue.push(UiIntent::SetFocusPane { pane });
    }

    /// Update canvas zoom percentage cleanly (`0.1 ..= 8.0`).
    pub fn push_set_canvas_zoom(&mut self, zoom: f64) {
        self.queue.push(UiIntent::SetCanvasZoom { zoom });
    }

    /// Toggle collapse/expand state of a Navigator folder section.
    pub fn push_toggle_navigator_section(&mut self, section: SheetSection) {
        self.queue.push(UiIntent::ToggleNavigatorSection { section });
    }

    /// Update Navigator instant substring filter query.
    pub fn push_set_navigator_search(&mut self, query: impl Into<String>) {
        self.queue.push(UiIntent::SetNavigatorSearch { query: query.into() });
    }

    /// Switch Navigator organization mode (`SeparateFolders` vs `NestedWithResults`).
    pub fn push_set_navigator_nesting(&mut self, mode: NavigatorNestingMode) {
        self.queue.push(UiIntent::SetNavigatorNesting { mode });
    }

    /// Persist Center Stage split divider ratio (`0.15 ..= 0.85`).
    pub fn push_set_split_ratio(&mut self, ratio: f64) {
        self.queue.push(UiIntent::SetSplitRatio { ratio });
    }

    /// Switch [`CanvasPreviewFilter`] simulation (`Normal` / CVD / `GrayscalePrint`).
    pub fn push_set_preview_filter(&mut self, filter: CanvasPreviewFilter) {
        self.queue.push(UiIntent::SetPreviewFilter { filter });
    }

    /// Open or close the `Cmd/Ctrl+K` Command Palette overlay.
    pub fn push_set_palette_open(&mut self, open: bool) {
        self.queue.push(UiIntent::SetPaletteOpen { open });
    }

    /// Open or dismiss the 3-pane Welcome & Workspace Hub launcher overlay.
    pub fn push_set_launcher_open(&mut self, open: bool) {
        self.queue.push(UiIntent::SetLauncherOpen { open });
    }

    /// Apply a saved [`WorkspacePreset`] dock configuration.
    pub fn push_apply_workspace_preset(&mut self, preset: WorkspacePreset) {
        self.queue.push(UiIntent::ApplyWorkspacePreset { preset });
    }

    /// Persist dragged Leading Dock width in logical `pt`.
    pub fn push_sync_leading_width(&mut self, width_pt: f32) {
        self.queue.push(UiIntent::SyncLeadingWidth { width_pt });
    }

    /// Persist dragged Trailing Studio width in logical `pt`.
    pub fn push_sync_trailing_width(&mut self, width_pt: f32) {
        self.queue.push(UiIntent::SyncTrailingWidth { width_pt });
    }

    /// Toggle Leading Dock between `240 pt` expanded and `36 pt` collapsed rail.
    pub fn push_toggle_leading_collapsed_rail(&mut self) {
        self.queue.push(UiIntent::ToggleLeadingCollapsedRail);
    }

    /// Toggle Trailing Studio between `280 pt` expanded and `36 pt` collapsed rail.
    pub fn push_toggle_trailing_collapsed_rail(&mut self) {
        self.queue.push(UiIntent::ToggleTrailingCollapsedRail);
    }

    /// Navigate Back (`<`) in sheet visit history.
    pub fn push_history_back(&mut self) {
        self.queue.push(UiIntent::HistoryBack);
    }

    /// Navigate Forward (`>`) in sheet visit history.
    pub fn push_history_forward(&mut self) {
        self.queue.push(UiIntent::HistoryForward);
    }

    /// Dismiss the non-blocking bottom diagnostic banner for `sheet_id`.
    pub fn push_dismiss_diagnostic_banner(&mut self, sheet_id: SheetId) {
        self.queue.push(UiIntent::DismissDiagnosticBanner { sheet_id });
    }

    /// Open `Change Data Analyzed...` retargeting `result_sheet_id` toward `target_data_sheet_id`.
    pub fn push_open_retarget_dialog(&mut self, result_sheet_id: SheetId, target_data_sheet_id: Option<SheetId>) {
        self.queue.push(UiIntent::OpenRetargetDialog { result_sheet_id, target_data_sheet_id });
    }

    /// Brush a point/cell coordinate for two-way Split View & Data Point Drill-Back.
    pub fn push_brush_point(&mut self, sheet_id: SheetId, coord: curvebook_tools::GridCellCoord) {
        self.queue.push(UiIntent::BrushPoint { sheet_id, coord });
    }

    /// Clear any active single-point or row brushing highlight.
    pub fn push_clear_brush(&mut self) {
        self.queue.push(UiIntent::ClearBrush);
    }

    /// Patch fields on the active modal dialog after the settled frame pass.
    pub fn push_patch_active_dialog(&mut self, fields: serde_json::Map<String, serde_json::Value>) {
        self.queue.push(UiIntent::PatchActiveDialog { fields });
    }

    /// Execute a modal footer button action (`"ok"`, `"cancel"`, `"apply"`, `"create"`, etc.).
    pub fn push_commit_dialog_action(&mut self, action: impl Into<String>) {
        self.queue.push(UiIntent::CommitDialogAction { action: action.into() });
    }

    /// Close all open `ActiveDialog` and shell modal surfaces cleanly.
    pub fn push_close_all_modals(&mut self) {
        self.queue.push(UiIntent::CloseAllModals);
    }

    /// Open `Manage Variables...` synced to `sheet_id` and optional `column`.
    pub fn push_open_manage_variables(&mut self, sheet_id: SheetId, column: Option<usize>) {
        self.queue.push(UiIntent::OpenManageVariables { sheet_id, column });
    }

    /// Select column `column` inside `Manage Variables...`.
    pub fn push_select_manage_var_column(&mut self, sheet_id: SheetId, column: usize) {
        self.queue.push(UiIntent::SelectManageVarColumn { sheet_id, column });
    }

    /// Stage a categorical level rename/merge (`from -> to`) inside `Manage Variables...`.
    pub fn push_stage_manage_var_recode(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.queue.push(UiIntent::StageManageVarRecode { from: from.into(), to: to.into() });
    }

    /// Clear all unstaged categorical recode mappings.
    pub fn push_clear_manage_var_recodes(&mut self) {
        self.queue.push(UiIntent::ClearManageVarRecodes);
    }

    /// Commit staged categorical level recodes via `table.recodeLevels` (`Caller::Ui`).
    pub fn push_apply_manage_var_recodes(&mut self, sheet_id: SheetId) {
        self.queue.push(UiIntent::ApplyManageVarRecodes { sheet_id });
    }

    /// Commit categorical level order and optional reference level via `table.setCategoricalOrder`.
    pub fn push_apply_manage_var_order(&mut self, sheet_id: SheetId, column: usize, order: LevelOrderSpec, reference_level: Option<String>) {
        self.queue.push(UiIntent::ApplyManageVarOrder { sheet_id, column, order, reference_level });
    }

    /// Close `Manage Variables...` without additional mutations.
    pub fn push_close_manage_variables(&mut self) {
        self.queue.push(UiIntent::CloseManageVariables);
    }

    /// Open the 5-stage `Import & Paste Special` wizard.
    pub fn push_open_import_wizard(&mut self, origin: ImportOriginKind, is_live_link: bool, target_sheet_id: Option<SheetId>) {
        self.queue.push(UiIntent::OpenImportWizard { origin, is_live_link, target_sheet_id });
    }

    /// Persist a settled snapshot update to `app.grid.import_wizard`.
    pub fn push_update_import_wizard(&mut self, wizard: ImportPasteWizardState) {
        self.queue.push(UiIntent::UpdateImportWizard(Box::new(wizard)));
    }

    /// Commit the configured `Import & Paste Special` wizard via `Caller::Ui`.
    pub fn push_commit_import_wizard(&mut self, close_after: bool) {
        self.queue.push(UiIntent::CommitImportWizard { close_after });
    }

    /// Close `Import & Paste Special`.
    pub fn push_close_import_wizard(&mut self) {
        self.queue.push(UiIntent::CloseImportWizard);
    }

    /// Open a native Transform / Baseline / MV Filter / Extract modal for `sheet_id`.
    pub fn push_open_transform_modal(&mut self, sheet_id: SheetId, kind: transform_modals::TransformModalKind) {
        self.queue.push(UiIntent::OpenTransformModal { sheet_id, kind });
    }

    /// Close the shell-hosted transform modal.
    pub fn push_close_transform_modal(&mut self) {
        self.queue.push(UiIntent::CloseTransformModal);
    }

    /// Copy the active grid selection to TSV clipboard.
    pub fn push_grid_copy_selection(&mut self) {
        self.queue.push(UiIntent::GridCopySelection);
    }

    /// Cut the active grid selection (copy + single-commit `table.clearCells`).
    pub fn push_grid_cut_selection(&mut self) {
        self.queue.push(UiIntent::GridCutSelection);
    }

    /// Paste TSV clipboard contents at `start_coord` (or current active cell) via `table.paste`.
    pub fn push_grid_paste_from_clipboard(&mut self, start_coord: Option<curvebook_tools::GridCellCoord>) {
        self.queue.push(UiIntent::GridPasteFromClipboard { start_coord });
    }

    /// Copy selection or active Graph/Layout vector [`ClipboardBundle`] cleanly on commit.
    pub fn push_workspace_copy(&mut self, dpi: u16) {
        self.queue.push(UiIntent::WorkspaceCopy { dpi });
    }

    /// Synchronize active `ResultSheet` rectangular cell-range selection onto `app.view_shards.results`.
    pub fn push_sync_result_selection(&mut self, sheet_id: SheetId, tab_index: usize, anchor: (usize, usize), focus: (usize, usize)) {
        self.queue.push(UiIntent::SyncResultSelection { sheet_id, tab_index, anchor, focus });
    }

    /// Paste clipboard image or copied `ResultSheet` table onto `target_sheet_id`.
    pub fn push_canvas_paste(&mut self, target_sheet_id: SheetId, anchor_pt: Option<[f64; 2]>) {
        self.queue.push(UiIntent::CanvasPaste { target_sheet_id, anchor_pt });
    }

    /// Paste clipboard image bytes onto `target_sheet_id` via `edit.pasteImage`.
    pub fn push_paste_image_from_clipboard(&mut self, target_sheet_id: SheetId, anchor_pt: Option<[f64; 2]>) {
        self.queue.push(UiIntent::PasteImageFromClipboard { target_sheet_id, anchor_pt });
    }

    /// Update the live `Analyze...` instant search query.
    pub fn push_set_analyze_query(&mut self, query: impl Into<String>) {
        self.queue.push(UiIntent::SetAnalyzeQuery { query: query.into() });
    }

    /// Select an analysis catalog entry ID in `Analyze...`.
    pub fn push_select_analyze_entry(&mut self, entry_id: impl Into<String>, clear_duplicate_prompt: bool) {
        self.queue.push(UiIntent::SelectAnalyzeEntry { entry_id: entry_id.into(), clear_duplicate_prompt });
    }

    /// Toggle inclusion of dataset `col_index` in `Analyze...`.
    pub fn push_toggle_analyze_dataset(&mut self, col_index: usize) {
        self.queue.push(UiIntent::ToggleAnalyzeDataset { col_index });
    }

    /// Replace the selected dataset column indices in `Analyze...`.
    pub fn push_set_analyze_selected_datasets(&mut self, cols: Vec<usize>) {
        self.queue.push(UiIntent::SetAnalyzeSelectedDatasets { cols });
    }

    /// Commit an updated `AnalysisSpec` draft onto `edit_params`.
    pub fn push_set_edit_params_draft(&mut self, spec: curvebook_doc::AnalysisSpec) {
        self.queue.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
    }

    /// Persist the active tab slug on `edit_params`.
    pub fn push_set_edit_params_active_tab(&mut self, tab: impl Into<String>) {
        self.queue.push(UiIntent::SetEditParamsActiveTab { tab: tab.into() });
    }

    /// Toggle repeated-measures/paired state on `edit_params`.
    pub fn push_set_edit_params_paired_or_matched(&mut self, paired: bool) {
        self.queue.push(UiIntent::SetEditParamsPairedOrMatched { paired });
    }

    /// Update confidence interval level on `edit_params`.
    pub fn push_set_edit_params_confidence(&mut self, level: ConfidenceLevel) {
        self.queue.push(UiIntent::SetEditParamsConfidence { level });
    }

    /// Persist the current `edit_params` draft configuration as family default preset.
    pub fn push_save_edit_params_as_default(&mut self) {
        self.queue.push(UiIntent::SaveEditParamsAsDefault);
    }

    /// Select target source table inside `ChangeDataAnalyzed`.
    pub fn push_select_retarget_source(&mut self, target_data_sheet_id: SheetId) {
        self.queue.push(UiIntent::SelectRetargetSource { target_data_sheet_id });
    }

    /// Resolve `DuplicateAnalysisPrompt` (`Some(existing)` modifies existing; `None` creates new).
    pub fn push_resolve_duplicate_analysis(&mut self, modify_existing_id: Option<SheetId>) {
        self.queue.push(UiIntent::ResolveDuplicateAnalysis { modify_existing_id });
    }

    /// Select a hook option index inside `ConstantHookPicker`.
    pub fn push_select_hook_constant(&mut self, index: Option<usize>) {
        self.queue.push(UiIntent::SelectHookConstant { index });
    }

    /// Clear hook selection on the active target.
    pub fn push_unlink_hook_constant(&mut self) {
        self.queue.push(UiIntent::UnlinkHookConstant);
    }

    /// Persist updated `PowerWizardState` draft cleanly on settlement.
    pub fn push_update_power_wizard_state(&mut self, state: PowerWizardState) {
        self.queue.push(UiIntent::UpdatePowerWizardState(Box::new(state)));
    }

    /// Commit `PowerWizard` action (`"next"`, `"back"`, `"create"`, `"ok"`, `"cancel"`).
    pub fn push_commit_power_wizard(&mut self, action: impl Into<String>) {
        self.queue.push(UiIntent::CommitPowerWizard { action: action.into() });
    }

    /// Persist updated `ChangeGraphTypeState` draft cleanly on settlement.
    pub fn push_update_change_graph_type_state(&mut self, state: ChangeGraphTypeState) {
        self.queue.push(UiIntent::UpdateChangeGraphTypeState(Box::new(state)));
    }

    /// Persist updated `FormatGraphState` draft cleanly on settlement.
    pub fn push_update_format_graph_state(&mut self, state: FormatGraphState) {
        self.queue.push(UiIntent::UpdateFormatGraphState(Box::new(state)));
    }

    /// Persist updated `FormatAxesState` draft cleanly on settlement.
    pub fn push_update_format_axes_state(&mut self, state: FormatAxesState) {
        self.queue.push(UiIntent::UpdateFormatAxesState(Box::new(state)));
    }

    /// Persist updated `BubblePlotInspectorState` draft cleanly on settlement.
    pub fn push_update_bubble_plot_state(&mut self, state: BubblePlotInspectorState) {
        self.queue.push(UiIntent::UpdateBubblePlotState(Box::new(state)));
    }

    /// Persist updated `PairwiseComparisonsState` draft cleanly on settlement.
    pub fn push_update_pairwise_state(&mut self, state: PairwiseComparisonsState) {
        self.queue.push(UiIntent::UpdatePairwiseState(Box::new(state)));
    }

    /// Persist updated `PageLayoutComposerState` draft cleanly on settlement.
    pub fn push_update_page_layout_state(&mut self, state: PageLayoutComposerState) {
        self.queue.push(UiIntent::UpdatePageLayoutState(Box::new(state)));
    }

    /// Persist updated `ExportFigureState` draft cleanly on settlement.
    pub fn push_update_export_figure_state(&mut self, state: ExportFigureState) {
        self.queue.push(UiIntent::UpdateExportFigureState(Box::new(state)));
    }

    /// Persist updated `PrintDialogState` draft cleanly on settlement.
    pub fn push_update_print_dialog_state(&mut self, state: PrintDialogState) {
        self.queue.push(UiIntent::UpdatePrintDialogState(Box::new(state)));
    }

    /// Persist updated `CreateGraphDialogState` draft cleanly on settlement.
    pub fn push_update_create_graph_state(&mut self, state: CreateGraphDialogState) {
        self.queue.push(UiIntent::UpdateCreateGraphState(Box::new(state)));
    }

    /// Persist updated `EqualizeScalingDialogState` draft cleanly on settlement.
    pub fn push_update_equalize_scaling_state(&mut self, state: EqualizeScalingDialogState) {
        self.queue.push(UiIntent::UpdateEqualizeScalingState(Box::new(state)));
    }

    /// Persist updated `FormatEmbeddedTableDialogState` draft cleanly on settlement.
    pub fn push_update_format_embedded_table_state(&mut self, state: FormatEmbeddedTableDialogState) {
        self.queue.push(UiIntent::UpdateFormatEmbeddedTableState(Box::new(state)));
    }

    /// Persist updated `FormatImageDialogState` draft cleanly on settlement.
    pub fn push_update_format_image_state(&mut self, state: FormatImageDialogState) {
        self.queue.push(UiIntent::UpdateFormatImageState(Box::new(state)));
    }

    /// Persist updated `ApplyStyleFromState` draft cleanly on settlement.
    pub fn push_update_apply_style_state(&mut self, state: ApplyStyleFromState) {
        self.queue.push(UiIntent::UpdateApplyStyleState(Box::new(state)));
    }

    /// Replace Portfolio Gallery multi-selection set cleanly on settlement.
    pub fn push_set_gallery_selection(&mut self, sheet_ids: Vec<SheetId>) {
        self.queue.push(UiIntent::SetGallerySelection { sheet_ids });
    }

    /// Open `Preferences` modal on optional category.
    pub fn push_open_preferences(&mut self, category: Option<app_modals::PreferencesCategory>) {
        self.queue.push(UiIntent::OpenPreferences { category });
    }

    /// Close `Preferences` modal (`save == true` commits active draft preferences).
    pub fn push_close_preferences(&mut self, save: bool) {
        self.queue.push(UiIntent::ClosePreferences { save });
    }

    /// Rebind `command_id` to `combo` with conflict validation via `ShortcutRegistry`.
    pub fn push_set_shortcut_binding(&mut self, command_id: impl Into<String>, combo: impl Into<String>) {
        self.queue.push(UiIntent::SetShortcutBinding { command_id: command_id.into(), combo: combo.into() });
    }

    /// Reset a single shortcut binding or all shortcuts (`None`) to defaults.
    pub fn push_reset_shortcut_binding(&mut self, command_id: Option<String>) {
        self.queue.push(UiIntent::ResetShortcutBinding { command_id });
    }

    /// Persist updated `PreferencesModalState` draft cleanly on settlement.
    pub fn push_update_preferences_draft(&mut self, draft: app_modals::PreferencesModalState) {
        self.queue.push(UiIntent::UpdatePreferencesDraft(Box::new(draft)));
    }

    /// Open the crash/session recovery prompt with `summary`.
    pub fn push_open_recovery_dialog(&mut self, summary: app_modals::RecoveryDialogSummary) {
        self.queue.push(UiIntent::OpenRecoveryDialog { summary: Box::new(summary) });
    }

    /// Record the user's decision on `RecoveryDialog` (`RestoreSession` / `DiscardRecovery` / `Dismiss`).
    pub fn push_resolve_recovery(&mut self, decision: app_modals::RecoveryDecision) {
        self.queue.push(UiIntent::ResolveRecoveryDialog { decision });
    }

    /// Record the user's decision on `UnsavedChanges` (`Save` / `DontSave` / `Cancel`).
    pub fn push_resolve_unsaved_changes(&mut self, action: app_modals::UnsavedCloseAction) {
        self.queue.push(UiIntent::ResolveUnsavedChanges { action });
    }

    /// Open `Find & Replace` with optional initial query.
    pub fn push_open_find_replace(&mut self, initial_query: Option<String>) {
        self.queue.push(UiIntent::OpenFindReplace { initial_query });
    }

    /// Close `Find & Replace`.
    pub fn push_close_find_replace(&mut self) {
        self.queue.push(UiIntent::CloseFindReplace);
    }

    /// Execute `Find & Replace` cell match or batch single-commit replacement via `table.setCells`.
    pub fn push_execute_find_replace(
        &mut self,
        replace_all: bool,
        find_query: impl Into<String>,
        replace_with: impl Into<String>,
        match_case: bool,
        match_entire_cell: bool,
    ) {
        self.queue.push(UiIntent::ExecuteFindReplace {
            replace_all,
            find_query: find_query.into(),
            replace_with: replace_with.into(),
            match_case,
            match_entire_cell,
        });
    }

    /// Persist updated `FeedbackDraft` fields cleanly on settlement.
    pub fn push_update_feedback_draft(&mut self, draft: crate::feedback::FeedbackDraft) {
        self.queue.push(UiIntent::UpdateFeedbackDraft(Box::new(draft)));
    }

    /// Open the prefilled GitHub new-issue URL for the current feedback draft.
    pub fn push_open_feedback_on_github(&mut self) {
        self.queue.push(UiIntent::OpenFeedbackOnGitHub);
    }

    /// Copy the untruncated Markdown feedback report to the configured clipboard adapter.
    pub fn push_copy_feedback_markdown(&mut self) {
        self.queue.push(UiIntent::CopyFeedbackMarkdown);
    }

    /// Clear user-entered feedback form fields while keeping category and diagnostics preference.
    pub fn push_reset_feedback_draft(&mut self) {
        self.queue.push(UiIntent::ResetFeedbackDraft);
    }

    /// Commit buffered intents once via `Session::execute_as_ui` (`Caller::Ui`) when `!will_discard`.
    ///
    /// When `will_discard` is `true`, clears the buffer without executing any mutations so
    /// multi-pass `ctx.request_discard()` layouts never duplicate undo steps or journal entries.
    ///
    /// # Errors
    /// Returns [`EngineError`] if any queued engine or UI command fails validation or preconditions.
    pub fn commit_if_settled(&mut self, app: &mut UiApp, will_discard: bool) -> Result<Vec<UiIntentOutcome>, EngineError> {
        let pending = std::mem::take(&mut self.queue);
        if will_discard || pending.is_empty() {
            return Ok(Vec::new());
        }
        let mut outcomes = Vec::with_capacity(pending.len());
        for intent in pending {
            outcomes.push(apply_single_ui_intent(app, intent)?);
        }
        app.sync_views();
        app.egui_views.sync_history_with_document(app.session.document());
        Ok(outcomes)
    }

    /// Commit only [`UiIntent::EngineCommand`] items directly on `session` (`Caller::Ui`) when `!will_discard`.
    ///
    /// # Errors
    /// Returns [`EngineError`] if any queued command fails.
    pub fn commit_engine_intents_on_session(&mut self, session: &mut Session, will_discard: bool) -> Result<Vec<CommandOutcome>, EngineError> {
        let pending = std::mem::take(&mut self.queue);
        if will_discard || pending.is_empty() {
            return Ok(Vec::new());
        }
        let mut outcomes = Vec::new();
        for intent in pending {
            if let UiIntent::EngineCommand { command_id, params } = intent {
                outcomes.push(session.execute_as_ui(&command_id, params)?);
            }
        }
        Ok(outcomes)
    }
}

fn apply_single_ui_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::EngineCommand { command_id, mut params } => {
            if matches!(command_id.as_str(), "sheet.duplicate" | "sheet.duplicateFamily" | "sheet.delete" | "sheet.freeze" | "sheet.unfreeze")
                && params.get("sheet_id").is_none()
                && let Some(active_id) = app.session.document().active_sheet_id.as_ref()
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("sheet_id"), serde_json::Value::String(active_id.as_str().to_owned()));
            }
            let outcome = app.execute_session_ui_cmd(&command_id, params)?;
            if matches!(command_id.as_str(), "edit.undo" | "edit.redo" | "sheet.select") {
                crate::interaction::clear_inspector_axis_buffers(app);
            }
            Ok(UiIntentOutcome::Engine { command_id, outcome })
        }
        UiIntent::UiCommand { command_id, params } => {
            let result = dispatch_ui_command_as_ui(app, &command_id, &params)?;
            Ok(UiIntentOutcome::Ui { command_id, result })
        }
        UiIntent::SetTheme(mode) => {
            let prev_scale = app.theme.ui_scale;
            app.set_theme_mode(mode);
            app.theme.ui_scale = prev_scale;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetLocale(locale) => {
            let _ = app.set_locale(&locale);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetUiScale(scale) => {
            let clamped = if scale.is_finite() { scale.clamp(0.75, 3.0) } else { 1.0 };
            app.theme.ui_scale = clamped;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SelectGridCell { coord } => {
            app.grid.select_single_sync(coord);
            app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ExtendGridSelection { coord } => {
            let mut grid = std::mem::take(&mut app.grid);
            if let Some(active_id) = app.document().active_sheet_id.as_ref()
                && let Some(ds) = app.document().data_sheet(active_id)
            {
                grid.extend_shift_range(ds, coord);
            }
            app.grid = grid;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetRowTitlesCollapsed { collapsed } => {
            app.grid.row_titles_collapsed_override = Some(collapsed);
            app.view_shards.grid_core.row_titles_collapsed = collapsed;
            app.egui_views.column_grid.row_titles_collapsed = collapsed;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::DialogWidgetClick { widget_id } => {
            let result = crate::gui_actions::apply_dialog_widget_click(app, &widget_id)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.widget:{widget_id}"), result })
        }
        UiIntent::DialogButtonAction { action } => {
            let result = crate::gui_actions::execute_dialog_button_action(app, &action)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.button:{action}"), result: Some(result) })
        }
        UiIntent::SetTextFieldBuffer { widget_id, text } => {
            app.interaction.text_field_buffers.insert(widget_id.clone(), crate::interaction::TextFieldEditState::new(text, false));
            app.focus_widget_id = Some(widget_id.clone());
            Ok(UiIntentOutcome::Ui { command_id: format!("textbox.sync:{widget_id}"), result: None })
        }
        UiIntent::CustomViewAction { domain, payload } => {
            let mut shards = std::mem::take(&mut app.view_shards);
            let res = crate::views::dispatch_custom_view_action(app, &mut shards, &domain, &payload);
            app.view_shards = shards;
            let outcome = res.map_err(|msg| EngineError::with_code(curvebook_doc::ErrorCode::Conflict, msg))?;
            Ok(UiIntentOutcome::Ui {
                command_id: outcome.command_id.unwrap_or_else(|| format!("{domain}:{payload}")),
                result: outcome.command_result,
            })
        }
        other => apply_shell_workspace_intent(app, other),
    }
}

fn apply_shell_workspace_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::SelectResultTab { index } => {
            let max_idx = app
                .session
                .document()
                .active_sheet_id
                .as_ref()
                .and_then(|id| app.session.document().result_sheet(id))
                .map_or(0, |rs| rs.tabs.len().saturating_sub(1));
            app.canvas_dialogs.result_view.active_tab_index = index.min(max_idx);
        }
        UiIntent::SetActiveInspector { inspector } => {
            app.canvas_dialogs.active_inspector = inspector;
            if inspector != ActiveInspector::None {
                app.nav.inspector_open = true;
                app.egui_views.trailing_collapsed_rail = false;
            }
        }
        UiIntent::SetFocusPane { pane } => {
            app.nav.active_pane = pane;
        }
        UiIntent::SetCanvasZoom { zoom } => {
            app.canvas_dialogs.canvas.zoom = if zoom.is_finite() { zoom.clamp(0.1, 8.0) } else { 1.0 };
        }
        UiIntent::ToggleNavigatorSection { section } => {
            if let Some(pos) = app.nav.collapsed_sections.iter().position(|s| *s == section) {
                app.nav.collapsed_sections.remove(pos);
            } else {
                app.nav.collapsed_sections.push(section);
            }
        }
        UiIntent::SetNavigatorSearch { query } => {
            app.nav.search_query = query;
        }
        UiIntent::SetNavigatorNesting { mode } => {
            app.nav.nesting_mode = mode;
        }
        UiIntent::SetSplitRatio { ratio } => {
            app.split.set_ratio(ratio);
        }
        UiIntent::SetPreviewFilter { filter } => {
            app.preview_filter = filter;
        }
        UiIntent::SetPaletteOpen { open } => {
            if open {
                app.canvas_dialogs.open_command_palette();
                app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
                app.egui_views.command_palette.just_opened = true;
            } else if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.active_dialog = ActiveDialog::None;
            }
        }
        UiIntent::SetLauncherOpen { open } => {
            app.egui_views.launcher_open = open;
            app.nav.welcome.open = open;
        }
        UiIntent::ApplyWorkspacePreset { preset } => {
            let cfg = preset.default_dock_config();
            app.nav.navigator_open = cfg.navigator_open;
            app.family_open = cfg.family_open;
            app.nav.inspector_open = cfg.inspector_open;
            app.egui_views.workspace_preset = preset;
            app.egui_views.leading_collapsed_rail = cfg.leading_collapsed_rail;
            app.egui_views.trailing_collapsed_rail = cfg.trailing_collapsed_rail;
            app.egui_views.leading_width_pt = Some(cfg.leading_width_pt);
            app.egui_views.trailing_width_pt = Some(cfg.trailing_width_pt);
        }
        UiIntent::SyncLeadingWidth { width_pt } => {
            let clamped = width_pt.clamp(LayoutMetrics::CANONICAL.navigator_min_w, LayoutMetrics::CANONICAL.navigator_max_w);
            app.egui_views.leading_width_pt = Some(clamped);
        }
        UiIntent::SyncTrailingWidth { width_pt } => {
            let clamped = width_pt.clamp(LayoutMetrics::CANONICAL.inspector_min_w, LayoutMetrics::CANONICAL.inspector_max_w);
            app.egui_views.trailing_width_pt = Some(clamped);
        }
        UiIntent::ToggleLeadingCollapsedRail => {
            app.egui_views.leading_collapsed_rail = !app.egui_views.leading_collapsed_rail;
            if !app.egui_views.leading_collapsed_rail {
                app.nav.navigator_open = true;
            }
        }
        UiIntent::ToggleTrailingCollapsedRail => {
            app.egui_views.trailing_collapsed_rail = !app.egui_views.trailing_collapsed_rail;
            if !app.egui_views.trailing_collapsed_rail {
                app.nav.inspector_open = true;
            }
        }
        UiIntent::HistoryBack => return step_history_intent(app, false),
        UiIntent::HistoryForward => return step_history_intent(app, true),
        UiIntent::DismissDiagnosticBanner { sheet_id } => {
            if !app.egui_views.center_stage.dismissed_banner_sheets.contains(&sheet_id) {
                app.egui_views.center_stage.dismissed_banner_sheets.push(sheet_id);
            }
        }
        UiIntent::OpenRetargetDialog { result_sheet_id, target_data_sheet_id } => {
            let doc = app.session.document();
            if let Some(rs) = doc.result_sheet(&result_sheet_id) {
                app.canvas_dialogs.change_data.open_for_result(rs, doc);
                if let Some(target) = target_data_sheet_id {
                    app.canvas_dialogs.change_data.selected_target_id = Some(target);
                }
                app.canvas_dialogs.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id };
            }
        }
        UiIntent::BrushPoint { sheet_id, coord } => {
            app.brush_from_grid_cell(&sheet_id, coord);
        }
        UiIntent::ClearBrush => {
            app.clear_brush();
        }
        other => return apply_wave4_intent(app, other),
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn apply_wave4_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::PatchActiveDialog { fields } => {
            crate::gui_actions::patch_active_dialog(app, &fields)?;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::CommitDialogAction { action } => {
            let result = crate::gui_actions::execute_dialog_button_action(app, &action)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.button:{action}"), result: Some(result) })
        }
        UiIntent::CloseAllModals => {
            app.canvas_dialogs.close_dialog();
            app.grid.variables_dialog.open = false;
            app.grid.import_wizard.open = false;
            app.egui_views.transform_modals.open_kind = None;
            app.egui_views.app_modals.close_all();
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::OpenManageVariables { .. }
        | UiIntent::SelectManageVarColumn { .. }
        | UiIntent::StageManageVarRecode { .. }
        | UiIntent::ClearManageVarRecodes
        | UiIntent::ApplyManageVarRecodes { .. }
        | UiIntent::ApplyManageVarOrder { .. }
        | UiIntent::CloseManageVariables
        | UiIntent::OpenImportWizard { .. }
        | UiIntent::UpdateImportWizard(_)
        | UiIntent::CommitImportWizard { .. }
        | UiIntent::CloseImportWizard
        | UiIntent::OpenTransformModal { .. }
        | UiIntent::CloseTransformModal
        | UiIntent::GridCopySelection
        | UiIntent::GridCutSelection
        | UiIntent::GridPasteFromClipboard { .. }
        | UiIntent::WorkspaceCopy { .. }
        | UiIntent::CanvasPaste { .. }
        | UiIntent::PasteImageFromClipboard { .. } => apply_wave4_grid_wizard_intent(app, intent),
        UiIntent::SetAnalyzeQuery { .. }
        | UiIntent::SelectAnalyzeEntry { .. }
        | UiIntent::ToggleAnalyzeDataset { .. }
        | UiIntent::SetAnalyzeSelectedDatasets { .. }
        | UiIntent::SetEditParamsDraft { .. }
        | UiIntent::SetEditParamsActiveTab { .. }
        | UiIntent::SetEditParamsPairedOrMatched { .. }
        | UiIntent::SetEditParamsConfidence { .. }
        | UiIntent::SaveEditParamsAsDefault
        | UiIntent::SelectRetargetSource { .. }
        | UiIntent::ResolveDuplicateAnalysis { .. }
        | UiIntent::SelectHookConstant { .. }
        | UiIntent::UnlinkHookConstant
        | UiIntent::UpdatePowerWizardState(_)
        | UiIntent::CommitPowerWizard { .. }
        | UiIntent::UpdateChangeGraphTypeState(_)
        | UiIntent::UpdateFormatGraphState(_)
        | UiIntent::UpdateFormatAxesState(_)
        | UiIntent::UpdateBubblePlotState(_)
        | UiIntent::UpdatePairwiseState(_)
        | UiIntent::UpdatePageLayoutState(_)
        | UiIntent::UpdateExportFigureState(_)
        | UiIntent::UpdatePrintDialogState(_)
        | UiIntent::UpdateCreateGraphState(_)
        | UiIntent::UpdateEqualizeScalingState(_)
        | UiIntent::UpdateFormatEmbeddedTableState(_)
        | UiIntent::UpdateFormatImageState(_)
        | UiIntent::UpdateApplyStyleState(_)
        | UiIntent::SyncResultSelection { .. }
        | UiIntent::SetGallerySelection { .. } => apply_wave4_modal_state_intent(app, intent),
        UiIntent::OpenPreferences { .. }
        | UiIntent::ClosePreferences { .. }
        | UiIntent::SetShortcutBinding { .. }
        | UiIntent::ResetShortcutBinding { .. }
        | UiIntent::UpdatePreferencesDraft(_)
        | UiIntent::OpenRecoveryDialog { .. }
        | UiIntent::ResolveRecoveryDialog { .. }
        | UiIntent::ResolveUnsavedChanges { .. }
        | UiIntent::OpenFindReplace { .. }
        | UiIntent::CloseFindReplace
        | UiIntent::ExecuteFindReplace { .. }
        | UiIntent::UpdateFeedbackDraft(_)
        | UiIntent::OpenFeedbackOnGitHub
        | UiIntent::CopyFeedbackMarkdown
        | UiIntent::ResetFeedbackDraft => apply_wave4_app_modal_intent(app, intent),
        _ => Ok(UiIntentOutcome::ShellUpdated),
    }
}

fn apply_wave4_grid_wizard_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::OpenManageVariables { sheet_id, column } => {
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                let col = column.unwrap_or(0).min(ds.archetype.column_count().saturating_sub(1));
                app.grid.variables_dialog.open_for_column(&ds, col);
            } else {
                app.grid.variables_dialog.open = true;
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SelectManageVarColumn { sheet_id, column } => {
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                let col = column.min(ds.archetype.column_count().saturating_sub(1));
                app.grid.variables_dialog.open_for_column(&ds, col);
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::StageManageVarRecode { from, to } => {
            app.grid.variables_dialog.add_recode_mapping(from, to);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ClearManageVarRecodes => {
            app.grid.variables_dialog.recode_mappings.clear();
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ApplyManageVarRecodes { sheet_id } => {
            if let Some(tool_intent) = app.grid.variables_dialog.build_recode_command(&sheet_id) {
                let outcome = app.execute_session_ui_cmd(&tool_intent.command_id, tool_intent.params)?;
                app.grid.variables_dialog.recode_mappings.clear();
                if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                    app.grid.variables_dialog.sync_from_sheet(&ds);
                }
                return Ok(UiIntentOutcome::Engine { command_id: tool_intent.command_id, outcome });
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ApplyManageVarOrder { sheet_id, column, order, reference_level } => {
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                app.grid.variables_dialog.set_level_order(order.clone(), Some(&ds));
            }
            let params = serde_json::json!({
                "sheet_id": sheet_id.as_str(),
                "column": column,
                "order_spec": order,
                "reference_level": reference_level,
            });
            let outcome = app.execute_session_ui_cmd("table.setCategoricalOrder", params)?;
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                app.grid.variables_dialog.sync_from_sheet(&ds);
            }
            Ok(UiIntentOutcome::Engine { command_id: String::from("table.setCategoricalOrder"), outcome })
        }
        UiIntent::CloseManageVariables => {
            app.grid.variables_dialog.open = false;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::OpenImportWizard { origin, is_live_link, target_sheet_id } => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.origin_kind = origin;
            app.grid.import_wizard.is_live_link = is_live_link;
            app.grid.import_wizard.source_sheet_id = target_sheet_id
                .or_else(|| app.session.document().active_sheet_id.as_ref().filter(|id| app.session.document().data_sheet(id).is_some()).cloned());
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::UpdateImportWizard(wizard) => {
            app.grid.import_wizard = *wizard;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::CommitImportWizard { close_after } => {
            let target_id = app
                .session
                .document()
                .active_sheet_id
                .as_ref()
                .filter(|id| app.session.document().data_sheet(id).is_some())
                .cloned()
                .or_else(|| app.grid.import_wizard.source_sheet_id.clone())
                .or_else(|| app.session.document().data_sheets.first().map(|s| s.id.clone()))
                .ok_or_else(|| EngineError::with_code(ErrorCode::Conflict, "no data sheet available for import wizard"))?;
            let tool_intent = app.grid.import_wizard.build_command_intent(&target_id, Some(app.session.document()));
            let outcome = app.execute_session_ui_cmd(&tool_intent.command_id, tool_intent.params)?;
            if close_after {
                app.grid.import_wizard.open = false;
            }
            Ok(UiIntentOutcome::Engine { command_id: tool_intent.command_id, outcome })
        }
        UiIntent::CloseImportWizard => {
            app.grid.import_wizard.open = false;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::OpenTransformModal { sheet_id, kind } => {
            app.egui_views.transform_modals.open_kind = Some(kind);
            app.egui_views.transform_modals.target_sheet_id = Some(sheet_id);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::CloseTransformModal => {
            app.egui_views.transform_modals.open_kind = None;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::GridCopySelection => execute_grid_clipboard_copy(app, false),
        UiIntent::GridCutSelection => execute_grid_clipboard_copy(app, true),
        UiIntent::GridPasteFromClipboard { start_coord } => execute_grid_clipboard_paste(app, start_coord),
        UiIntent::WorkspaceCopy { dpi } => execute_workspace_copy(app, dpi),
        UiIntent::CanvasPaste { target_sheet_id, anchor_pt } => execute_canvas_paste(app, &target_sheet_id, anchor_pt),
        UiIntent::PasteImageFromClipboard { target_sheet_id, anchor_pt } => execute_paste_image_from_clipboard(app, &target_sheet_id, anchor_pt),
        _ => Ok(UiIntentOutcome::ShellUpdated),
    }
}

fn encode_base64_bytes(data: &[u8]) -> String {
    const LUT: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(LUT[((n >> 18) & 0x3F) as usize] as char);
        out.push(LUT[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { LUT[((n >> 6) & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { LUT[(n & 0x3F) as usize] as char } else { '=' });
    }
    out
}

fn build_canvas_paste_bounds(anchor_pt: Option<[f64; 2]>, w: f64, h: f64) -> Option<Vec<f64>> {
    anchor_pt.map(|[x, y]| vec![x.max(4.0), y.max(4.0), w, h])
}

fn execute_paste_image_from_clipboard(
    app: &mut UiApp,
    target_sheet_id: &SheetId,
    anchor_pt: Option<[f64; 2]>,
) -> Result<UiIntentOutcome, EngineError> {
    let img_bytes = app
        .read_clipboard_image()
        .map_err(|msg| EngineError::with_code(ErrorCode::Conflict, msg))?
        .or_else(|| app.interaction.last_clipboard_image_png.clone());
    let Some(bytes) = img_bytes else {
        return Err(EngineError::with_code(ErrorCode::Conflict, "No PNG, JPEG, or TIFF image found on clipboard"));
    };
    let mut params = serde_json::Map::new();
    params.insert(String::from("target_sheet_id"), serde_json::json!(target_sheet_id.as_str()));
    params.insert(String::from("bytes_base64"), serde_json::json!(encode_base64_bytes(&bytes)));
    if let Some(bounds) = build_canvas_paste_bounds(anchor_pt, 160.0, 120.0) {
        params.insert(String::from("bounds_pt"), serde_json::json!(bounds));
    }
    let outcome = app.execute_command("edit.pasteImage", serde_json::Value::Object(params))?;
    app.interaction.last_clipboard_image_png = Some(bytes);
    app.sync_views();
    Ok(UiIntentOutcome::Engine { command_id: String::from("edit.pasteImage"), outcome })
}

fn execute_canvas_paste(app: &mut UiApp, target_sheet_id: &SheetId, anchor_pt: Option<[f64; 2]>) -> Result<UiIntentOutcome, EngineError> {
    if let Ok(Some(bytes)) = app.read_clipboard_image() {
        app.interaction.last_clipboard_image_png = Some(bytes);
        return execute_paste_image_from_clipboard(app, target_sheet_id, anchor_pt);
    }
    if let Some(copied) = app.interaction.last_copied_result_table.clone() {
        let mut params = serde_json::Map::new();
        params.insert(String::from("source_sheet_id"), serde_json::json!(copied.source_sheet_id.as_str()));
        params.insert(String::from("target_sheet_id"), serde_json::json!(target_sheet_id.as_str()));
        if let Some(tab_id) = copied.tab_id {
            params.insert(String::from("tab_id"), serde_json::json!(tab_id));
        }
        if let Some(rows) = copied.selected_rows {
            params.insert(String::from("selected_rows"), serde_json::json!(rows));
        }
        if let Some(cols) = copied.selected_cols {
            params.insert(String::from("selected_cols"), serde_json::json!(cols));
        }
        if let Some(bounds) = build_canvas_paste_bounds(anchor_pt, 180.0, 90.0) {
            params.insert(String::from("bounds_pt"), serde_json::json!(bounds));
        }
        let outcome = app.execute_command("edit.pasteResultTable", serde_json::Value::Object(params))?;
        app.sync_views();
        return Ok(UiIntentOutcome::Engine { command_id: String::from("edit.pasteResultTable"), outcome });
    }
    if app.interaction.last_clipboard_image_png.is_some() {
        return execute_paste_image_from_clipboard(app, target_sheet_id, anchor_pt);
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_workspace_copy(app: &mut UiApp, dpi: u16) -> Result<UiIntentOutcome, EngineError> {
    if let Some(rs_id) = crate::interaction::resolve_result_sheet_for_shortcut(app, false) {
        let mut results_state = std::mem::take(&mut app.view_shards.results);
        let out = crate::views::results::handle_custom_action(app, &mut results_state, &format!("copy_selection:{}", rs_id.as_str()));
        app.view_shards.results = results_state;
        if let Ok(outcome) = out
            && outcome.handled
        {
            return Ok(UiIntentOutcome::Ui {
                command_id: outcome.command_id.unwrap_or_else(|| String::from("results.copy_selection")),
                result: outcome.command_result,
            });
        }
    }
    if !app.egui_views.launcher_open
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && let Some(fig_id) = crate::interaction::resolve_figure_sheet_for_shortcut(app)
    {
        let outcome =
            crate::interaction::copy_figure_sheet_bundle_dpi(app, &fig_id, dpi).map_err(|msg| EngineError::with_code(ErrorCode::Conflict, msg))?;
        return Ok(UiIntentOutcome::Ui { command_id: outcome.command_id.unwrap_or(outcome.action), result: outcome.command_result });
    }
    if app.session.document().active_sheet_id.as_ref().is_some_and(|id| app.session.document().data_sheet(id).is_some()) {
        return execute_grid_clipboard_copy(app, false);
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_grid_clipboard_copy(app: &mut UiApp, cut: bool) -> Result<UiIntentOutcome, EngineError> {
    let doc = app.session.document();
    let Some(ds_id) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let Some(ds) = doc.data_sheet(&ds_id) else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let coords = app.grid.all_selected_coords(ds);
    let range = if let Some(first) = coords.first() {
        let mut min_r = first.row;
        let mut max_r = first.row;
        let mut min_c = first.col;
        let mut max_c = first.col;
        let mut min_s = first.subcol;
        let mut max_s = first.subcol;
        for c in &coords {
            min_r = min_r.min(c.row);
            max_r = max_r.max(c.row);
            min_c = min_c.min(c.col);
            max_c = max_c.max(c.col);
            min_s = min_s.min(c.subcol);
            max_s = max_s.max(c.subcol);
        }
        let _ = (min_s, max_s);
        Some(SheetCellRange::new(CellCoord::new(min_r, min_c, min_s), CellCoord::new(max_r, max_c, max_s)))
    } else {
        None
    };
    let tsv = match range {
        Some(r) => ClipboardBundle::tsv_from_datasheet_range(ds, r, app.locale_preset.decimal_separator),
        None => ClipboardBundle::tsv_from_datasheet(ds, app.locale_preset.decimal_separator).unwrap_or_default(),
    };
    app.interaction.last_clipboard_text = Some(tsv.clone());
    let _ = app.write_clipboard_text(&tsv);
    if cut
        && !ds.frozen
        && let Some(r) = range
    {
        let outcome = app.execute_session_ui_cmd("table.clearCells", serde_json::json!({ "sheet_id": ds_id.as_str(), "range": r }))?;
        return Ok(UiIntentOutcome::Engine { command_id: String::from("table.clearCells"), outcome });
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_grid_clipboard_paste(app: &mut UiApp, start_coord: Option<curvebook_tools::GridCellCoord>) -> Result<UiIntentOutcome, EngineError> {
    let Some(ds_id) = app.session.document().active_sheet_id.as_ref().filter(|id| app.session.document().data_sheet(id).is_some()).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let clip = app
        .read_clipboard_text()
        .ok()
        .filter(|s| s.available)
        .and_then(|s| s.text)
        .filter(|t| !t.is_empty())
        .or_else(|| app.interaction.last_clipboard_text.clone())
        .unwrap_or_default();
    if clip.is_empty() {
        return Ok(UiIntentOutcome::ShellUpdated);
    }
    let anchor = start_coord.or(app.grid.tool.active).unwrap_or(curvebook_tools::GridCellCoord::new(0, 0, 0));
    let outcome = app.execute_session_ui_cmd(
        "table.paste",
        serde_json::json!({
            "sheet_id": ds_id.as_str(),
            "text": clip,
            "start_row": anchor.row,
            "start_col": anchor.col,
            "start_subcol": anchor.subcol,
            "propagate_title_header": false,
        }),
    )?;
    Ok(UiIntentOutcome::Engine { command_id: String::from("table.paste"), outcome })
}

fn apply_wave4_modal_state_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::SetAnalyzeQuery { query } => app.canvas_dialogs.analyze.search_query = query,
        UiIntent::SelectAnalyzeEntry { entry_id, clear_duplicate_prompt } => {
            app.canvas_dialogs.analyze.selected_entry_id = Some(entry_id);
            if clear_duplicate_prompt && let ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, .. } = app.canvas_dialogs.active_dialog.clone()
            {
                app.canvas_dialogs.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id };
            }
        }
        UiIntent::ToggleAnalyzeDataset { col_index } => {
            let sel = &mut app.canvas_dialogs.analyze.selected_data_sets;
            if let Some(pos) = sel.iter().position(|&c| c == col_index) {
                sel.remove(pos);
            } else {
                sel.push(col_index);
            }
        }
        UiIntent::SetAnalyzeSelectedDatasets { cols } => {
            app.canvas_dialogs.analyze.selected_data_sets = cols;
        }
        UiIntent::SetEditParamsDraft { spec } => {
            app.canvas_dialogs.edit_params.draft = Some(*spec);
            app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
        }
        UiIntent::SetEditParamsActiveTab { tab } => {
            app.canvas_dialogs.edit_params.active_tab = tab;
        }
        UiIntent::SetEditParamsPairedOrMatched { paired } => {
            app.canvas_dialogs.edit_params.paired_or_matched = paired;
        }
        UiIntent::SetEditParamsConfidence { level } => {
            if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
                spec.confidence_level = level;
            }
        }
        UiIntent::SaveEditParamsAsDefault => {
            app.egui_views.anova_modal.method_saved_as_default = true;
            app.egui_views.stats_modals.method_saved_as_default = true;
        }
        UiIntent::SelectRetargetSource { target_data_sheet_id } => {
            app.canvas_dialogs.change_data.selected_target_id = Some(target_data_sheet_id);
        }
        UiIntent::ResolveDuplicateAnalysis { modify_existing_id } => {
            let choice = match modify_existing_id {
                Some(id) => AnalyzeDuplicateChoice::ModifyExisting(id),
                None => AnalyzeDuplicateChoice::CreateNew,
            };
            if let Some(tool_intent) = app.canvas_dialogs.analyze.resolve_duplicate_choice(&choice) {
                app.canvas_dialogs.close_dialog();
                let outcome = app.execute_session_ui_cmd(&tool_intent.command_id, tool_intent.params)?;
                return Ok(UiIntentOutcome::Engine { command_id: tool_intent.command_id, outcome });
            }
        }
        UiIntent::SelectHookConstant { index } => {
            app.canvas_dialogs.hook_picker.selected_index = index;
        }
        UiIntent::UnlinkHookConstant => {
            app.canvas_dialogs.hook_picker.selected_index = None;
        }
        UiIntent::UpdatePowerWizardState(state) => {
            app.canvas_dialogs.power_wizard = *state;
            let _ = app.canvas_dialogs.power_wizard.recalculate();
        }
        UiIntent::CommitPowerWizard { action } => {
            let result = crate::gui_actions::execute_dialog_button_action(app, &action)?;
            return Ok(UiIntentOutcome::Ui { command_id: format!("power_wizard:{action}"), result: Some(result) });
        }
        UiIntent::UpdateChangeGraphTypeState(state) => app.canvas_dialogs.change_graph_type = *state,
        UiIntent::UpdateFormatGraphState(state) => app.canvas_dialogs.format_graph = *state,
        UiIntent::UpdateFormatAxesState(state) => app.canvas_dialogs.format_axes = *state,
        UiIntent::UpdateBubblePlotState(state) => app.canvas_dialogs.bubble_plot = *state,
        UiIntent::UpdatePairwiseState(state) => app.canvas_dialogs.pairwise = *state,
        UiIntent::UpdatePageLayoutState(state) => app.canvas_dialogs.page_layout = *state,
        UiIntent::UpdateExportFigureState(state) => app.canvas_dialogs.export_figure = *state,
        UiIntent::UpdatePrintDialogState(state) => app.canvas_dialogs.print = *state,
        UiIntent::UpdateCreateGraphState(state) => app.canvas_dialogs.create_graph = *state,
        UiIntent::UpdateEqualizeScalingState(state) => app.canvas_dialogs.equalize_scaling = *state,
        UiIntent::UpdateFormatEmbeddedTableState(state) => app.canvas_dialogs.format_embedded_table = *state,
        UiIntent::UpdateFormatImageState(state) => app.canvas_dialogs.format_image = *state,
        UiIntent::UpdateApplyStyleState(state) => app.canvas_dialogs.apply_style = *state,
        UiIntent::SyncResultSelection { sheet_id, tab_index, anchor, focus } => {
            app.view_shards.results.bound_sheet_id = Some(sheet_id.as_str().to_owned());
            app.view_shards.results.bound_tab_idx = Some(tab_index);
            app.view_shards.results.selected_anchor = Some(anchor);
            app.view_shards.results.selected_focus = Some(focus);
        }
        UiIntent::SetGallerySelection { sheet_ids } => {
            app.nav.gallery.selected_ids = sheet_ids.into_iter().collect();
            for card in &mut app.nav.gallery.cards {
                card.selected = app.nav.gallery.selected_ids.contains(&card.sheet_id);
            }
        }
        _ => {}
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn apply_wave4_app_modal_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::OpenPreferences { category } => {
            if let Some(cat) = category {
                app.egui_views.app_modals.preferences.active_category = cat;
            }
            app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
            app.egui_views.app_modals.preferences_open = true;
        }
        UiIntent::ClosePreferences { save } => {
            if save {
                let scale_val = app.egui_views.app_modals.preferences.ui_scale_spinner.last_committed_value;
                if let Some(pct) = scale_val {
                    let scale = (pct / 100.0).clamp(0.75, 3.0) as f32;
                    app.theme.ui_scale = scale;
                }
            }
            app.egui_views.app_modals.preferences_open = false;
        }
        UiIntent::SetShortcutBinding { command_id, combo } => match app.shortcuts.set_shortcut(&command_id, &combo) {
            Ok(()) => {
                app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
            }
            Err(err) => {
                app.egui_views.app_modals.preferences.shortcut_conflict_error = Some(err.clone());
                return Err(EngineError::with_code(ErrorCode::Conflict, err));
            }
        },
        UiIntent::ResetShortcutBinding { command_id } => match command_id {
            Some(id) => {
                if let Err(err) = app.shortcuts.reset_shortcut(&id) {
                    app.egui_views.app_modals.preferences.shortcut_conflict_error = Some(err.clone());
                    return Err(EngineError::with_code(ErrorCode::Conflict, err));
                }
                app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
            }
            None => {
                app.shortcuts.reset_all();
                app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
            }
        },
        UiIntent::UpdatePreferencesDraft(draft) => {
            app.egui_views.app_modals.preferences = *draft;
        }
        UiIntent::OpenRecoveryDialog { summary } => {
            app.egui_views.app_modals.recovery_summary = *summary;
            app.egui_views.app_modals.recovery_open = true;
        }
        UiIntent::ResolveRecoveryDialog { decision } => {
            app.egui_views.app_modals.last_recovery_decision = Some(decision);
            app.egui_views.app_modals.recovery_open = false;
        }
        UiIntent::ResolveUnsavedChanges { action } => {
            app.egui_views.app_modals.pending_unsaved_action = Some(action);
            app.egui_views.app_modals.unsaved_changes_open = false;
        }
        UiIntent::OpenFindReplace { initial_query } => {
            if let Some(q) = initial_query {
                app.egui_views.app_modals.find_replace.find_query = q;
            }
            app.egui_views.app_modals.find_replace_open = true;
        }
        UiIntent::CloseFindReplace => {
            app.egui_views.app_modals.find_replace_open = false;
        }
        UiIntent::ExecuteFindReplace { replace_all, find_query, replace_with, match_case, match_entire_cell } => {
            return apply_find_replace_on_active_sheet(app, replace_all, &find_query, &replace_with, match_case, match_entire_cell);
        }
        UiIntent::UpdateFeedbackDraft(draft) => {
            app.canvas_dialogs.feedback = *draft;
        }
        UiIntent::OpenFeedbackOnGitHub => {
            if let Err(err) = app.open_feedback_on_github() {
                app.canvas_dialogs.feedback.status_message = Some(format!("Could not open GitHub issue form: {err}"));
            }
        }
        UiIntent::CopyFeedbackMarkdown => {
            if let Err(err) = app.copy_feedback_markdown() {
                app.canvas_dialogs.feedback.status_message = Some(format!("Could not copy Markdown report: {err}"));
            }
        }
        UiIntent::ResetFeedbackDraft => {
            app.canvas_dialogs.feedback.clear_fields();
        }
        _ => {}
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn apply_find_replace_on_active_sheet(
    app: &mut UiApp,
    replace_all: bool,
    find_query: &str,
    replace_with: &str,
    match_case: bool,
    match_entire_cell: bool,
) -> Result<UiIntentOutcome, EngineError> {
    if find_query.is_empty() {
        return Ok(UiIntentOutcome::ShellUpdated);
    }
    let Some(ds_id) = app.session.document().active_sheet_id.as_ref().filter(|id| app.session.document().data_sheet(id).is_some()).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let Some(ds) = app.session.document().data_sheet(&ds_id).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };

    let mut edits = Vec::new();
    let mut first_match: Option<curvebook_tools::GridCellCoord> = None;
    let mut match_count = 0usize;
    let query_cmp = if match_case { find_query.to_owned() } else { find_query.to_ascii_lowercase() };

    let rows = ds.archetype.row_count();
    let cols = ds.archetype.column_count();
    for r in 0..rows {
        for c in 0..cols {
            let subcols = crate::grid_view::data_set_subcolumn_count(&ds.archetype, c).max(1);
            for s in 0..subcols {
                let Some(cell) = ds.archetype.data_cell(CellCoord::new(r, c, s)) else {
                    continue;
                };
                let raw = match &cell.value {
                    curvebook_doc::CellValue::Empty => continue,
                    curvebook_doc::CellValue::Number(v) => v.to_string(),
                    curvebook_doc::CellValue::Text(t) => t.clone(),
                };
                let hay = if match_case { raw.clone() } else { raw.to_ascii_lowercase() };
                let matched = if match_entire_cell { hay == query_cmp } else { hay.contains(&query_cmp) };
                if !matched {
                    continue;
                }
                match_count += 1;
                if first_match.is_none() {
                    first_match = Some(curvebook_tools::GridCellCoord::new(r, c, s));
                }
                if replace_all {
                    let next_text = if match_entire_cell {
                        replace_with.to_owned()
                    } else if match_case {
                        raw.replace(find_query, replace_with)
                    } else {
                        let mut out = String::new();
                        let lower_raw = raw.to_ascii_lowercase();
                        let mut cursor = 0usize;
                        while let Some(rel) = lower_raw.get(cursor..).and_then(|slice| slice.find(&query_cmp)) {
                            let start = cursor + rel;
                            let end = start + query_cmp.len();
                            if let Some(prefix) = raw.get(cursor..start) {
                                out.push_str(prefix);
                            }
                            out.push_str(replace_with);
                            cursor = end;
                        }
                        if let Some(tail) = raw.get(cursor..) {
                            out.push_str(tail);
                        }
                        out
                    };
                    edits.push(serde_json::json!({ "row": r, "col": c, "subcol": s, "value": next_text }));
                }
            }
        }
    }

    app.egui_views.app_modals.find_replace.match_count = match_count;
    if let Some(coord) = first_match {
        app.grid.select_single_sync(coord);
        app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
    }
    if replace_all && !edits.is_empty() {
        let outcome = app.execute_session_ui_cmd("table.setCells", serde_json::json!({ "sheet_id": ds_id.as_str(), "edits": edits }))?;
        return Ok(UiIntentOutcome::Engine { command_id: String::from("table.setCells"), outcome });
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn step_history_intent(app: &mut UiApp, forward: bool) -> Result<UiIntentOutcome, EngineError> {
    let target = if forward {
        if app.egui_views.can_go_forward() {
            app.egui_views.history_cursor += 1;
            app.egui_views.active_sheet_history.get(app.egui_views.history_cursor).cloned()
        } else {
            None
        }
    } else if app.egui_views.can_go_back() {
        app.egui_views.history_cursor -= 1;
        app.egui_views.active_sheet_history.get(app.egui_views.history_cursor).cloned()
    } else {
        None
    };

    if let Some(sheet_id) = target {
        let outcome = app.execute_session_ui_cmd("sheet.select", serde_json::json!({ "sheet_id": sheet_id.as_str() }))?;
        return Ok(UiIntentOutcome::Engine { command_id: String::from("sheet.select"), outcome });
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

/// Metadata recorded for a single rendered `egui` widget during a settled frame pass.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInspectNodeMeta {
    pub egui_id: egui::Id,
    pub accesskit_id: egui::accesskit::NodeId,
    pub widget_id: String,
    pub role: String,
    pub label: String,
    pub command_id: Option<String>,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    /// Interaction rectangle clipped against the active UI clip rect (logical `pt`).
    pub clipped_rect: Rect,
    pub z_layer: ZLayer,
    pub ellipsized: bool,
    pub full_label: Option<String>,
}

/// Parameter bundle for [`FrameInspectMeta::record_response`].
#[derive(Debug, Clone)]
pub struct FrameInspectEntryInput<'a> {
    pub widget_id: &'a str,
    pub role: &'a str,
    pub label: &'a str,
    pub command_id: Option<&'a str>,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub z_layer: ZLayer,
    pub ellipsized: bool,
    pub full_label: Option<&'a str>,
}

/// Settled-pass (`!ctx.will_discard()`) side-table mapping `egui`/AccessKit nodes to CurveBook
/// `widget_id`, `command_id`, `disabled_reason`, clipped interaction `Rect`, and ellipsis metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInspectMeta {
    pub pass_nr: u64,
    pub scale_factor: f32,
    pub viewport_rect: Rect,
    pub entries: Vec<FrameInspectNodeMeta>,
}

impl Default for FrameInspectMeta {
    fn default() -> Self {
        Self { pass_nr: 0, scale_factor: 1.0, viewport_rect: Rect::NOTHING, entries: Vec::new() }
    }
}

impl FrameInspectMeta {
    /// Construct an empty settled-frame side-table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset side-table entries for a fresh frame pass.
    pub fn begin_pass(&mut self, ctx: &egui::Context) {
        self.pass_nr = ctx.cumulative_pass_nr();
        self.scale_factor = ctx.pixels_per_point();
        self.viewport_rect = ctx.content_rect();
        self.entries.clear();
    }

    /// Clear all recorded entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of recorded widget entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when no widget entries are recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a pre-built [`FrameInspectNodeMeta`] entry.
    pub fn record(&mut self, entry: FrameInspectNodeMeta) {
        self.entries.push(entry);
    }

    /// Record inspection metadata from an `egui::Response`, clipping its interaction rect to `ui.clip_rect()`.
    pub fn record_response(&mut self, ui: &egui::Ui, resp: &egui::Response, input: FrameInspectEntryInput<'_>) {
        let clipped_rect = resp.interact_rect.intersect(ui.clip_rect());
        self.entries.push(FrameInspectNodeMeta {
            egui_id: resp.id,
            accesskit_id: resp.id.accesskit_id(),
            widget_id: input.widget_id.to_owned(),
            role: input.role.to_owned(),
            label: input.label.to_owned(),
            command_id: input.command_id.map(str::to_owned),
            value: input.value,
            enabled: input.enabled,
            disabled_reason: input.disabled_reason.map(str::to_owned),
            clipped_rect,
            z_layer: input.z_layer,
            ellipsized: input.ellipsized,
            full_label: input.full_label.map(str::to_owned),
        });
    }

    /// Look up the first recorded entry matching `widget_id`.
    #[must_use]
    pub fn find_by_widget_id(&self, widget_id: &str) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.widget_id == widget_id)
    }

    /// Look up the first recorded entry matching `egui_id`.
    #[must_use]
    pub fn find_by_egui_id(&self, egui_id: egui::Id) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.egui_id == egui_id)
    }

    /// Look up the first recorded entry matching `accesskit_id`.
    #[must_use]
    pub fn find_by_accesskit_id(&self, accesskit_id: egui::accesskit::NodeId) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.accesskit_id == accesskit_id)
    }
}

/// Geometry and DPI synchronization epoch used to prevent stale pre-resize `ui.screenshot` captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct GeometryStamp {
    pub frame_seq: u64,
    pub pass_nr: u64,
    pub physical_width_px: u32,
    pub physical_height_px: u32,
    /// Scale factor quantized in basis points (`round(pixels_per_point * 10_000.0)`).
    pub scale_bps: u32,
    pub settled: bool,
}

impl GeometryStamp {
    /// Quantize a `pixels_per_point` float into integer basis points (`10_000` == `1.0x`).
    #[must_use]
    pub fn quantize_scale(pixels_per_point: f32) -> u32 {
        let clamped = if pixels_per_point.is_finite() { pixels_per_point.clamp(0.5, 8.0) } else { 1.0 };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bps = (f64::from(clamped) * 10_000.0).round() as u32;
        bps
    }

    /// Construct a [`GeometryStamp`] from a settled/unsettled `egui::Context` frame pass.
    #[must_use]
    pub fn from_context(frame_seq: u64, ctx: &egui::Context, physical_size_px: [u32; 2], settled: bool) -> Self {
        Self {
            frame_seq,
            pass_nr: ctx.cumulative_pass_nr(),
            physical_width_px: physical_size_px[0],
            physical_height_px: physical_size_px[1],
            scale_bps: Self::quantize_scale(ctx.pixels_per_point()),
            settled,
        }
    }

    /// Return `true` when this stamp is settled (`!ctx.will_discard()`) and matches the requested
    /// framebuffer pixel dimensions and scale factor.
    #[must_use]
    pub fn matches_target(&self, width_px: u32, height_px: u32, pixels_per_point: f32) -> bool {
        self.settled
            && self.physical_width_px == width_px
            && self.physical_height_px == height_px
            && self.scale_bps == Self::quantize_scale(pixels_per_point)
    }

    /// Recover the floating-point `pixels_per_point` scale factor from [`Self::scale_bps`].
    #[must_use]
    pub fn scale_factor(self) -> f32 {
        (f64::from(self.scale_bps.max(5_000)) / 10_000.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrameInspectEntryInput, FrameInspectMeta, GeometryStamp, LayoutMetrics, PendingUiIntents, ScientificPaletteTokens, Tokens, UiBackend,
        color32_to_rgba8,
    };
    use crate::UiApp;
    use crate::inspect::ZLayer;
    use crate::theme::UiThemeMode;
    use serde_json::json;

    #[test]
    fn backend_switch_parses_env_and_cli_with_egui_default_and_rejects_legacy() {
        assert_eq!(UiBackend::from_env_and_args(None, None), Ok(UiBackend::Egui));
        assert_eq!(UiBackend::from_env_and_args(Some("egui"), None), Ok(UiBackend::Egui));
        assert_eq!(UiBackend::from_env_and_args(Some("unknown"), Some("EGUI")), Ok(UiBackend::Egui));
        assert!(UiBackend::from_env_and_args(None, Some("legacy")).is_err());
        assert!(UiBackend::from_env_and_args(Some("legacy"), None).is_err());
        assert!(UiBackend::from_env_and_args(None, Some("unknown")).is_err());
    }

    #[test]
    fn canonical_tokens_meet_wcag_aa_and_preserve_scientific_palettes() {
        for mode in UiThemeMode::ALL {
            let tokens = Tokens::for_mode(mode);
            assert!(tokens.meets_wcag_aa(), "WCAG 2.1 AA failed for {mode:?}");
            let visuals = tokens.to_visuals();
            assert_eq!(visuals.widgets.inactive.weak_bg_fill, tokens.button_bg, "weak_bg_fill must match button_bg for {mode:?}");
            assert_eq!(visuals.widgets.hovered.weak_bg_fill, tokens.hover_bg);
            assert_eq!(visuals.widgets.active.weak_bg_fill, tokens.active_bg);
            assert_eq!(visuals.widgets.open.weak_bg_fill, tokens.hover_bg);
        }
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height(false), 70.0);
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height(true), 0.0);
        assert_eq!(LayoutMetrics::CANONICAL.status_bar_height(false), 26.0);
        assert_eq!(LayoutMetrics::CANONICAL.grid_header_total_height(), 64.0);
        assert_eq!(ScientificPaletteTokens::okabe_ito_series().len(), 8);
        assert_eq!(ScientificPaletteTokens::viridis_stops().len(), 17);
        assert_eq!(ScientificPaletteTokens::cividis_stops().len(), 17);
        assert_eq!(ScientificPaletteTokens::blue_white_vermilion_stops().len(), 3);
        let first = color32_to_rgba8(ScientificPaletteTokens::okabe_ito_series()[0]);
        assert_eq!(first, curvebook_palette::Rgba8::BLACK);
    }

    #[test]
    fn pending_intents_discard_on_multi_pass_and_commit_once_when_settled() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        intents.push_engine("sheet.createData", json!({"sheet_id": "d-1", "title": "Discard Test", "kind": "column", "auto_graph": false}));
        let discarded = intents.commit_if_settled(&mut app, true)?;
        assert!(discarded.is_empty());
        assert!(intents.is_empty());
        assert_eq!(app.document().total_sheet_count(), 0);

        intents.push_engine("sheet.createData", json!({"sheet_id": "d-1", "title": "Committed Test", "kind": "column", "auto_graph": false}));
        let committed = intents.commit_if_settled(&mut app, false)?;
        assert_eq!(committed.len(), 1);
        assert_eq!(app.document().total_sheet_count(), 1);
        assert_eq!(app.session().undo_depth(), 1);
        Ok(())
    }

    #[test]
    fn frame_inspect_meta_clips_rects_and_geometry_stamp_validates_target() {
        let ctx = egui::Context::default();
        let mut meta = FrameInspectMeta::new();
        let mut captured_id = None;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
            |ui| {
                meta.begin_pass(ui.ctx());
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(120.0, 28.0), egui::Sense::click());
                captured_id = Some(resp.id);
                meta.record_response(
                    ui,
                    &resp,
                    FrameInspectEntryInput {
                        widget_id: "toolbar.analyze",
                        role: "Button",
                        label: "Analyze...",
                        command_id: Some("ui.analyze.open"),
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                assert!(rect.width() >= 28.0);
            },
        );
        output.textures_delta.clear();
        let hit = meta.find_by_widget_id("toolbar.analyze").expect("entry recorded");
        assert_eq!(hit.command_id.as_deref(), Some("ui.analyze.open"));
        assert_eq!(Some(hit.egui_id), captured_id);
        assert!(hit.clipped_rect.width() > 0.0);

        let stale = GeometryStamp::from_context(1, &ctx, [640, 420], false);
        assert!(!stale.matches_target(640, 420, 1.0));
        let settled = GeometryStamp::from_context(2, &ctx, [1280, 840], true);
        assert!(settled.matches_target(1280, 840, 1.0));
        assert!(!settled.matches_target(640, 420, 1.0));
    }

    #[test]
    fn pending_intents_route_grid_and_dialog_actions_once_settled() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        intents.push_select_grid_cell(curvebook_tools::GridCellCoord::new(2, 1, 0));
        intents.push_set_row_titles_collapsed(true);
        let outcomes = intents.commit_if_settled(&mut app, false)?;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(2, 1, 0)));
        assert_eq!(app.focus_widget_id.as_deref(), Some("grid.cell.2.1.0"));
        assert_eq!(app.grid.row_titles_collapsed_override, Some(true));
        Ok(())
    }

    #[test]
    fn responsive_shell_layout_enforces_spec_breakpoints_and_presentation_mode() {
        use super::{ShellLayoutInput, SplitOrientation, ViewportTier, WorkspacePreset};

        let metrics = LayoutMetrics::CANONICAL;
        let base_in = ShellLayoutInput {
            viewport_w_pt: 1440.0,
            viewport_h_pt: 900.0,
            presentation_mode: false,
            empty_workspace: false,
            navigator_open: true,
            inspector_open: true,
            family_open: true,
            leading_user_collapsed: false,
            trailing_user_collapsed: false,
            leading_width_override_pt: None,
            trailing_width_override_pt: None,
            host_draws_menu_bar: false,
        };

        let wide = metrics.solve_shell_layout(base_in);
        assert_eq!(wide.tier, ViewportTier::WideDesktop);
        assert!(!wide.leading_collapsed_to_rail);
        assert!(!wide.trailing_collapsed_to_rail);
        assert!(wide.center_stage_w_pt >= 900.0);
        assert_eq!(wide.split_orientation, SplitOrientation::Horizontal);

        let laptop = metrics.solve_shell_layout(ShellLayoutInput { viewport_w_pt: 960.0, ..base_in });
        assert_eq!(laptop.tier, ViewportTier::StandardLaptop);
        assert!(!laptop.leading_collapsed_to_rail);
        assert!(!laptop.trailing_collapsed_to_rail);
        assert!(laptop.center_stage_w_pt >= 480.0);

        let tablet_900 = metrics.solve_shell_layout(ShellLayoutInput { viewport_w_pt: 900.0, ..base_in });
        assert_eq!(tablet_900.tier, ViewportTier::CompactTablet);
        assert!(!tablet_900.leading_collapsed_to_rail);
        assert!(tablet_900.trailing_collapsed_to_rail);
        assert_eq!(tablet_900.trailing_dock_w_pt, 36.0);
        assert!(tablet_900.center_stage_w_pt >= 360.0);

        let tablet_640 = metrics.solve_shell_layout(ShellLayoutInput { viewport_w_pt: 640.0, ..base_in });
        assert!(tablet_640.leading_collapsed_to_rail);
        assert!(tablet_640.trailing_collapsed_to_rail);
        assert_eq!(tablet_640.split_orientation, SplitOrientation::Vertical);
        assert!(tablet_640.center_stage_w_pt >= 360.0);

        let present = metrics.solve_shell_layout(ShellLayoutInput { presentation_mode: true, ..base_in });
        assert_eq!(present.top_chrome_h_pt, 0.0);
        assert_eq!(present.status_bar_h_pt, 0.0);
        assert!(!present.leading_visible);
        assert!(!present.trailing_visible);
        assert_eq!(present.center_stage_w_pt, 1440.0);
        assert_eq!(present.center_stage_h_pt, 900.0);

        for preset in WorkspacePreset::ALL {
            assert_eq!(WorkspacePreset::parse(preset.as_str()), Some(preset));
        }
    }
}
