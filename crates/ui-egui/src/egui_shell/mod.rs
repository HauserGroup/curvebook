//! Native desktop `egui 0.37` shell contracts, canonical layout/color tokens, single-commit
//! intent buffer, AccessKit inspection metadata, and adapted pure-`egui` widget helpers (`R15`).

use curvebook_doc::SheetId;

pub use crate::menus::ActionRank;

pub mod app_modals;
pub mod brand;
pub mod canvas_view;
pub mod center_stage;
pub mod command_palette;
pub mod data_grid;
pub(crate) mod dialog_actions;
pub mod driver;
pub mod fit_modal;
pub mod fonts;
pub mod frame_meta;
pub mod graph_modals;
pub mod import_wizard;
pub mod info_view;
pub mod inspect_bridge;
pub mod intents;
pub mod layout;
pub mod leading_dock;
pub mod manage_vars_modal;
pub mod plate_editor;
pub mod power_studio;
pub mod rasterizer;
pub mod results_view;
pub mod sheet_find_bar;
pub mod specimen;
pub mod stat_anova_modal;
pub mod stats_modals;
pub mod status_footer;
pub mod tokens;
pub mod top_bar_fit;
pub mod top_chrome;
pub mod trailing_studio;
pub mod transform_modals;
pub(crate) mod ui_kit;
pub mod unit_spinners;
pub mod welcome_hub;
pub mod widgets;

pub use frame_meta::{FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, GeometryStamp, MODAL_INSPECT_MIN_CLIP_EXTENT_PT};
pub use intents::{
    AnalyzePickerIntent, ContingencyCiTarget, DiagToggleFlag, PendingUiIntents, StatParamsIntent, UiIntent, UiIntentBatchResult, UiIntentOutcome,
};
pub use layout::{LayoutMetrics, PresetDockConfig, ResponsiveShellLayout, ShellLayoutInput, SplitOrientation, ViewportTier, WorkspacePreset};
pub use tokens::{BLUE_WHITE_VERMILION_STOPS, OKABE_ITO_SERIES_PALETTE, ScientificPaletteTokens, Tokens, color32_to_rgba8, rgba8_to_color32};

/// Persistent pure-UI `egui_shell` view states owned alongside [`UiApp`](crate::UiApp) (`R15.2+`).
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
    pub sheet_find: sheet_find_bar::SheetFindBarState,
    pub search_recents: crate::search_engine::SearchRecents,
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

    /// Prune deleted sheets cleanly, adjust cursor for removed predecessors, collapse adjacent
    /// duplicates, and record `active_sheet_id` when missing from cursor.
    pub fn sync_history_with_document(&mut self, doc: &curvebook_doc::ProjectDocument) {
        let old_cursor = self.history_cursor;
        let mut new_history = Vec::with_capacity(self.active_sheet_history.len());
        let mut new_cursor = 0usize;
        let mut cursor_mapped = false;
        for (idx, id) in self.active_sheet_history.drain(..).enumerate() {
            if doc_sheet_title(doc, &id).is_none() {
                continue;
            }
            if new_history.last() != Some(&id) {
                new_history.push(id);
            }
            if idx <= old_cursor {
                new_cursor = new_history.len().saturating_sub(1);
                cursor_mapped = true;
            }
        }
        self.active_sheet_history = new_history;
        if self.active_sheet_history.is_empty() || !cursor_mapped {
            self.history_cursor = 0;
        } else {
            self.history_cursor = new_cursor.min(self.active_sheet_history.len() - 1);
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
