//! 5-mode `CenterStage` orchestrator (`Welcome`, `ColumnGrid`, `InfoSheet`, `ResultSheet`,
//! `CanvasViewport`, `PortfolioGallery`, Split Comparison, Clamped `FloatingNote` overlays,
//! `QOL-7` bottom diagnostic/clipping/linked-region banners, and `chrono`-free Progress Modal;
//! `docs/planning/gui-layout-spec.md` §5, `R15.3`).

use std::collections::BTreeMap;

use curvebook_chart::preflight_check_scene;
use curvebook_doc::{FloatingNote, GraphSheet, InfoSheet, LayoutSheet, ProjectDocument, ResultSheet, SheetFolderKind, SheetId, TableArchetype};
use curvebook_text::expand_template_hooks;
use curvebook_tools::GridCellCoord;
use egui::{Color32, CornerRadius, FontId, Id, Key, Order, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::AssumptionDiagnosticBanner;
use crate::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view_for_sheet};
use crate::egui_shell::data_grid::{ColumnGridEguiState, ColumnGridRenderContext, DataGridScrollTarget, render_data_grid_for_sheet};
use crate::egui_shell::info_view::{InfoViewEguiState, render_info_view_for_sheet};
use crate::egui_shell::inspect_bridge::HybridCoexistenceDecision;
use crate::egui_shell::results_view::{
    ResultsViewEguiState, render_horizontal_line_comparison_popup, render_results_view_for_sheet_with_options, trigger_horizontal_line_comparison,
};
use crate::egui_shell::sheet_find_bar::{
    DataSheetFindMatch, DataSheetFindOverlay, ResultSheetFindOverlay, find_matches_in_data_sheet_cached, find_matches_in_result_sheet_cached,
    render_sheet_find_bar, set_active_find_locale,
};
use crate::egui_shell::welcome_hub::render_welcome_hub;
use crate::egui_shell::widgets::dropdown;
use crate::egui_shell::{
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, LayoutMetrics, PendingUiIntents, ResponsiveShellLayout, SplitOrientation, Tokens,
    UiIntent, color32_to_rgba8, doc_sheet_title, rgba8_to_color32,
};
use crate::i18n::tr_or_fallback;
use crate::inspect::ZLayer;
use crate::workspace_nav::ActiveWorkspacePane;

use super::portfolio_gallery::GALLERY_DEFAULT_CARD_W_PT;
pub(super) use super::portfolio_gallery::compute_gallery_fit_zoom_pt;
pub use super::portfolio_gallery::{GalleryContextMenuState, GalleryInlineRenameState};

const MIN_SPLIT_RATIO: f32 = 0.15;
const MAX_SPLIT_RATIO: f32 = 0.85;
const DEFAULT_SPLIT_RATIO: f32 = 0.50;
const DATA_RESULTS_MIN_SPLIT_RATIO: f64 = 0.35;
const DATA_RESULTS_MAX_SPLIT_RATIO: f64 = 0.72;
const DATA_RESULTS_PADDING_PT: f32 = 16.0;
const SPLITTER_VISUAL_PT: f32 = 4.0;
const BOTTOM_BANNER_HEIGHT_PT: f32 = 32.0;
const FLOATING_NOTE_HEADER_H_PT: f32 = 26.0;
const FLOATING_NOTE_MIN_W_PT: f32 = 228.0;
const FLOATING_NOTE_MIN_H_PT: f32 = 76.0;
const FLOATING_NOTE_EDIT_MIN_H_PT: f32 = 140.0;
const FLOATING_NOTE_MARGIN_PT: f32 = 6.0;
/// Resolved presentation mode for the primary `CenterStage` pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CenterStageMode {
    /// No active project sheet (or launcher-only empty state): centered 3-pane `Welcome & Workspace Hub`.
    #[default]
    Welcome,
    /// Native virtualized `egui` `Column` table sheet (`R15.2`).
    ColumnGrid,
    /// Native virtualized `egui` grid for `XY`, `Grouped`, `Contingency`, `Survival`,
    /// `PartsOfWhole`, `MultipleVariables`, and `Nested` tables (`R15.4`).
    DataGrid,
    /// Native structured `38% / 62%` metadata constants and project notes editor (`§8.4`).
    InfoSheet,
    /// Native multi-tab reactive `ResultSheet` analysis report viewer (`§8.10`).
    ResultSheet,
    /// Native async `ChartScene` (`GraphSheet`) or `LayoutScene` (`LayoutSheet`) viewport (`§8.11A`, §8.12).
    CanvasViewport,
    /// Multi-card `Portfolio Gallery` thumbnail & filter grid (`§5`).
    Gallery,
}

/// Active split-view pane slot for `F6` focus cycling (`§5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SplitPaneSlot {
    /// Left (or top `<720 pt`) split pane.
    #[default]
    Primary,
    /// Right (or bottom `<720 pt`) split pane.
    Secondary,
}

/// Non-blocking or blocking background operation progress overlay (`chrono`-free; §5).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressModalState {
    /// Concise title describing the active operation (e.g., `"Opening Project"`, `"Exporting SVG"`).
    pub operation_label: String,
    /// Secondary detail line (filename, sheet count, or step status).
    pub detail: String,
    /// Determinate `0.0..=1.0` completion fraction, or `None` for indeterminate animated sweep.
    pub progress_0_to_1: Option<f32>,
    /// Whether the user may request cancellation via the `Cancel` button.
    pub cancellable: bool,
    /// Set to `true` when the user clicks `Cancel` (`modal.progress.cancel`).
    pub cancel_requested: bool,
}

/// Inline markdown edit draft for an active [`FloatingNote`] card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloatingNoteEditDraft {
    pub sheet_id: SheetId,
    pub note_id: String,
    pub draft_text: String,
}

/// Persistent `egui_shell` workspace state owned by `CenterStage` (`R15.3`).
#[derive(Debug, Clone, PartialEq)]
pub struct CenterStageEguiState {
    /// `true` while the user is actively dragging the Split Comparison divider bar.
    pub split_dragging: bool,
    /// Sheets whose bottom non-blocking diagnostic/clipping/linked banner was explicitly dismissed.
    pub dismissed_banner_sheets: Vec<SheetId>,
    /// Active drag state for a `FloatingNote` header move gesture.
    pub active_floating_note_drag: Option<(SheetId, usize)>,
    /// Most recent resolved [`CenterStageMode`] across settled frames.
    pub last_active_mode: CenterStageMode,
    /// Focused half inside Split Comparison (`F6` / click).
    pub focused_split_pane: SplitPaneSlot,
    /// Active floating-note ID carrying keyboard/F6 focus.
    pub focused_floating_note_id: Option<String>,
    /// Split fraction clamped strictly to Spec §5 `[0.15, 0.85]`.
    pub split_ratio_override: Option<f32>,
    /// Snapshot of `app.split.ratio` used to detect external `ui.split.toggle` changes.
    pub last_seen_app_split_ratio: Option<f64>,
    /// Instant case-insensitive substring query filtering `Portfolio Gallery` cards.
    pub gallery_search: String,
    /// Desired thumbnail card width inside `Portfolio Gallery` (`120..=260 pt`).
    pub gallery_zoom_pt: f32,
    /// Repositioned `FloatingNote` top-left stage-relative coordinates keyed by `(sheet_id, note_id)`.
    pub note_positions_pt: BTreeMap<(SheetId, String), [f32; 2]>,
    /// Resized `FloatingNote` card dimensions keyed by `(sheet_id, note_id)`.
    pub note_sizes_pt: BTreeMap<(SheetId, String), [f32; 2]>,
    /// Local collapse state override keyed by `(sheet_id, note_id)`.
    pub note_collapsed: BTreeMap<(SheetId, String), bool>,
    /// Viewport-pinned state keyed by `(sheet_id, note_id)`.
    pub note_pinned: BTreeMap<(SheetId, String), bool>,
    /// Active resize corner gesture for a `FloatingNote` card.
    pub active_floating_note_resize: Option<(SheetId, usize)>,
    /// Active inline markdown edit draft for a `FloatingNote` card.
    pub editing_floating_note: Option<FloatingNoteEditDraft>,
    /// Pending initial keyboard focus request when a `FloatingNote` card enters inline edit mode.
    pub pending_floating_note_edit_focus: Option<(SheetId, String)>,
    /// Last `app.mutation_seq()` reconciled against local `FloatingNote` drag/resize/collapse caches.
    pub last_note_sync_mutation_seq: Option<u64>,
    /// Independent `DataGrid` view state when the secondary split pane displays a `DataSheet` (`R20.3`).
    pub secondary_column_grid: ColumnGridEguiState,
    /// Independent `InfoSheet` view state when the secondary split pane displays an `InfoSheet`.
    pub secondary_info_view: InfoViewEguiState,
    /// Independent `ResultSheet` view state when the secondary split pane displays a `ResultSheet`.
    pub secondary_results_view: ResultsViewEguiState,
    /// Independent viewport state when the secondary split pane displays a `GraphSheet`/`LayoutSheet`.
    pub secondary_canvas_view: CanvasViewEguiState,
    /// Sheet whose bottom banner `[Why?]` details drawer is expanded.
    pub banner_details_open: Option<SheetId>,
    /// Last active grid coordinate mirrored into two-way point brushing.
    pub last_synced_grid_coord: Option<GridCellCoord>,
    /// Anchor sheet ID for contiguous `Shift+Click` selection inside `Portfolio Gallery`.
    pub gallery_anchor_id: Option<SheetId>,
    /// Tracks whether `app.grid.formula_bar.active` was open on the previous pass (for initial focus).
    pub last_formula_bar_active: bool,
    /// Last `(primary, secondary)` sheet pair rendered in Split Comparison (`R20.13` initial ratio).
    pub last_split_pair: Option<(SheetId, SheetId)>,
    /// Split pair awaiting content-aware initial `Data || Results` split ratio computation.
    pub auto_ratio_pending_pair: Option<(SheetId, SheetId)>,
    /// Split pair whose ratio was manually adjusted by the user (preserved against auto-sizing).
    pub manual_ratio_pair: Option<(SheetId, SheetId)>,
    /// Most recent `DataSheet` ID active in the workspace (disambiguates `app.grid` header coordinate ownership).
    pub last_active_data_sheet_id: Option<SheetId>,
    /// Last synced gallery search string used to reconcile direct test/intent mutations cleanly.
    pub last_synced_gallery_search: String,
    /// Folders collapsed inside `Portfolio Gallery` `All` view (`NavigatorOrder` / `Type` sorts).
    pub gallery_collapsed_sections: Vec<SheetFolderKind>,
    /// Keyboard-focused card sheet ID inside `Portfolio Gallery`.
    pub gallery_focused_id: Option<SheetId>,
    /// Last measured stage width in points for footer `Fit` zoom calculations.
    pub gallery_last_stage_w_pt: f32,
    /// Open right-click card context menu inside `Portfolio Gallery`.
    pub gallery_context_menu: Option<GalleryContextMenuState>,
    /// Active inline rename editor for a gallery card.
    pub gallery_renaming: Option<GalleryInlineRenameState>,
}

impl Default for CenterStageEguiState {
    fn default() -> Self {
        Self {
            split_dragging: false,
            dismissed_banner_sheets: Vec::new(),
            active_floating_note_drag: None,
            last_active_mode: CenterStageMode::default(),
            focused_split_pane: SplitPaneSlot::Primary,
            focused_floating_note_id: None,
            split_ratio_override: None,
            last_seen_app_split_ratio: None,
            gallery_search: String::new(),
            gallery_zoom_pt: GALLERY_DEFAULT_CARD_W_PT,
            note_positions_pt: BTreeMap::new(),
            note_sizes_pt: BTreeMap::new(),
            note_collapsed: BTreeMap::new(),
            note_pinned: BTreeMap::new(),
            active_floating_note_resize: None,
            editing_floating_note: None,
            pending_floating_note_edit_focus: None,
            last_note_sync_mutation_seq: None,
            secondary_column_grid: ColumnGridEguiState::default(),
            secondary_info_view: InfoViewEguiState::default(),
            secondary_results_view: ResultsViewEguiState::default(),
            secondary_canvas_view: CanvasViewEguiState::default(),
            banner_details_open: None,
            last_synced_grid_coord: None,
            gallery_anchor_id: None,
            last_formula_bar_active: false,
            last_split_pair: None,
            auto_ratio_pending_pair: None,
            manual_ratio_pair: None,
            last_active_data_sheet_id: None,
            last_synced_gallery_search: String::new(),
            gallery_collapsed_sections: Vec::new(),
            gallery_focused_id: None,
            gallery_last_stage_w_pt: 720.0,
            gallery_context_menu: None,
            gallery_renaming: None,
        }
    }
}

/// Per-frame environment context passed into [`render_center_stage`].
#[derive(Debug, Clone, Copy)]
pub struct CenterStageContext<'a> {
    /// Active semantic theme tokens & spacing metrics.
    pub tokens: &'a Tokens,
    /// Hybrid coexistence decision governing legacy vs native pane routing.
    pub decision: &'a HybridCoexistenceDecision,
    /// Responsive shell geometry computed by `LayoutMetrics::solve_shell_layout`.
    pub layout: ResponsiveShellLayout,
    /// Outer workspace focus pane from `UiApp::nav.active_pane`.
    pub focused_pane: ActiveWorkspacePane,
    /// `false` when a top-level modal backdrop blocks underlying interactions.
    pub enabled: bool,
}

/// Per-frame geometry & interaction summary returned by [`render_center_stage`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CenterStageFrameReport {
    /// Bounds of the primary (or sole) stage pane in logical points.
    pub primary_rect_pt: Option<egui::Rect>,
    /// Bounds of the secondary split pane in logical points, if split comparison is active.
    pub secondary_rect_pt: Option<egui::Rect>,
    /// Active scrollbar rect reported for non-overlap verification.
    pub scrollbar_rect_pt: Option<egui::Rect>,
    /// Bounds of the bottom-edge `28–36 pt` `QOL-7` banner when visible.
    pub diagnostic_banner_rect_pt: Option<egui::Rect>,
    /// `true` when the active CenterStage mode rendered natively in `egui` (rather than deferred).
    pub handled_natively: bool,
    /// `true` when the non-exported Canvas Grid alignment overlay was painted on `paper_rect`.
    pub grid_overlay_shown: bool,
}

fn record_region_meta(meta: &mut FrameInspectMeta, ui: &egui::Ui, resp: &egui::Response, input: FrameInspectEntryInput<'_>) {
    meta.record_response(ui, resp, input);
}

pub(super) fn inspect_input<'a>(
    widget_id: &'a str,
    role: &'a str,
    label: &'a str,
    command_id: Option<&'a str>,
    value: Option<serde_json::Value>,
    enabled: bool,
    z_layer: ZLayer,
) -> FrameInspectEntryInput<'a> {
    FrameInspectEntryInput {
        widget_id,
        role,
        label,
        command_id,
        value,
        enabled,
        disabled_reason: (!enabled).then_some("Blocked by modal"),
        z_layer,
        ellipsized: false,
        full_label: None,
    }
}

/// Resolve which [`CenterStageMode`] owns the primary stage pane for `(app, views)`.
#[must_use]
pub fn resolve_center_stage_mode(app: &UiApp, views: &EguiShellViewState) -> CenterStageMode {
    let doc = app.document();
    if app.nav.gallery.open {
        return CenterStageMode::Gallery;
    }
    if views.launcher_open || app.nav.welcome.open || doc.total_sheet_count() == 0 {
        return CenterStageMode::Welcome;
    }
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return CenterStageMode::Welcome;
    };
    if let Some(ds) = doc.data_sheet(active_id) {
        if matches!(ds.archetype, TableArchetype::Column(_)) {
            return CenterStageMode::ColumnGrid;
        }
        return CenterStageMode::DataGrid;
    }
    if doc.info_sheet(active_id).is_some() {
        return CenterStageMode::InfoSheet;
    }
    if doc.result_sheet(active_id).is_some() {
        return CenterStageMode::ResultSheet;
    }
    if doc.graph_sheet(active_id).is_some() || doc.layout_sheet(active_id).is_some() {
        return CenterStageMode::CanvasViewport;
    }
    CenterStageMode::Welcome
}

fn mode_for_sheet(doc: &ProjectDocument, sheet_id: &SheetId) -> CenterStageMode {
    if let Some(ds) = doc.data_sheet(sheet_id) {
        if matches!(ds.archetype, TableArchetype::Column(_)) { CenterStageMode::ColumnGrid } else { CenterStageMode::DataGrid }
    } else if doc.info_sheet(sheet_id).is_some() {
        CenterStageMode::InfoSheet
    } else if doc.result_sheet(sheet_id).is_some() {
        CenterStageMode::ResultSheet
    } else if doc.graph_sheet(sheet_id).is_some() || doc.layout_sheet(sheet_id).is_some() {
        CenterStageMode::CanvasViewport
    } else {
        CenterStageMode::Welcome
    }
}

/// Render `CenterStage`: single or split modes, `PortfolioGallery`, `F6` pane focus ring,
/// clamped `FloatingNote` overlays, bottom `28–36 pt` banners, and `chrono`-free progress modal.
pub fn render_center_stage(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: CenterStageContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> CenterStageFrameReport {
    let mut report = CenterStageFrameReport::default();
    let stage_rect = ui.available_rect_before_wrap();
    if !stage_rect.is_positive() {
        render_progress_modal_if_open(ui, app, views, ctx_in.tokens, meta);
        return report;
    }

    ensure_solid_stage_scrollbars(ui, ctx_in.tokens);
    let mode = resolve_center_stage_mode(app, views);
    views.center_stage.last_active_mode = mode;

    let doc = app.document();
    let active_sheet_id = doc.active_sheet_id.clone();
    if let Some(aid) = active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        views.center_stage.last_active_data_sheet_id = Some(aid.clone());
    } else if let Some(prev_ds) = views.center_stage.last_active_data_sheet_id.as_ref()
        && doc.data_sheet(prev_ds).is_none()
    {
        views.center_stage.last_active_data_sheet_id = None;
    }

    let split_active = app.split.enabled && !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery);
    if !split_active {
        views.center_stage.last_split_pair = None;
        views.center_stage.auto_ratio_pending_pair = None;
        views.center_stage.manual_ratio_pair = None;
    }

    let modal_blocked = !ctx_in.enabled || views.progress_modal.is_some();
    let stage_enabled = !modal_blocked;

    // Reserve bottom 32 pt for QOL-7 banner when visible so scrollbars and footer stay unoccluded.
    // In split view, the primary pane's sheet owns the singleton bottom banner.
    let banner_sheet_id = if split_active { app.split.resolve_sheets(doc).0.or_else(|| active_sheet_id.clone()) } else { active_sheet_id.clone() };
    let banner_kind = if !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery) {
        detect_bottom_banner(ui.ctx(), app, views, banner_sheet_id.as_ref())
    } else {
        None
    };

    let (work_rect, banner_rect) = if banner_kind.is_some() && stage_rect.height() >= 120.0 {
        let banner_h = BOTTOM_BANNER_HEIGHT_PT.clamp(28.0, 36.0);
        let top = Rect::from_min_max(stage_rect.min, pos2(stage_rect.right(), (stage_rect.bottom() - banner_h).max(stage_rect.top())));
        let bot = Rect::from_min_max(pos2(stage_rect.left(), top.bottom()), stage_rect.max);
        (top, Some(bot))
    } else {
        (stage_rect, None)
    };

    let split_orientation = resolve_split_orientation(work_rect, &ctx_in.tokens.metrics);
    handle_f6_split_and_note_focus(ui, app, views, &ctx_in, split_active, split_orientation, active_sheet_id.as_ref(), intents);
    let sheet_find_sync = sync_sheet_find_before_stage(app, views, mode, active_sheet_id.as_ref(), intents);

    if mode == CenterStageMode::Gallery {
        report.primary_rect_pt = Some(work_rect);
        report.handled_natively = true;
        let mut gal_io = super::portfolio_gallery::GalleryFrameIo { views, meta, intents };
        report.scrollbar_rect_pt = super::portfolio_gallery::render_portfolio_gallery(ui, app, work_rect, ctx_in.tokens, stage_enabled, &mut gal_io);
        draw_focus_ring_if_active(ui, work_rect, ctx_in.tokens, ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid);
        render_progress_modal_if_open(ui, app, views, ctx_in.tokens, meta);
        return report;
    }

    if split_active {
        let mut io = StageFrameIo { views, meta, intents, report: &mut report };
        render_split_stage(ui, app, &ctx_in, work_rect, stage_enabled, &mut io);
    } else {
        let single_click_in_pane = stage_enabled
            && mode != CenterStageMode::Welcome
            && ui
                .input(|i| (i.pointer.primary_pressed() || i.pointer.primary_clicked()).then(|| i.pointer.interact_pos()).flatten())
                .is_some_and(|p| work_rect.contains(p) && ui.ctx().layer_id_at(p).is_none_or(|l| l.order <= ui.layer_id().order));
        if single_click_in_pane && ctx_in.focused_pane != ActiveWorkspacePane::MainCanvasOrGrid {
            intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::MainCanvasOrGrid });
        }
        let pane_focused = stage_enabled && (ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid || single_click_in_pane);
        report.primary_rect_pt = Some(work_rect);
        let mut pane_ui = ui.new_child(egui::UiBuilder::new().max_rect(work_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        pane_ui.set_clip_rect(work_rect);
        let mut io = StageFrameIo { views, meta, intents, report: &mut report };
        render_single_pane_mode(
            &mut pane_ui,
            app,
            SinglePaneTarget { mode, sheet_id: active_sheet_id.as_ref(), slot: SplitPaneSlot::Primary, enabled: stage_enabled, pane_focused },
            &ctx_in,
            &mut io,
        );
        draw_focus_ring_if_active(
            ui,
            work_rect,
            ctx_in.tokens,
            (ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid || single_click_in_pane) && mode != CenterStageMode::Welcome,
        );
    }

    if let (Some(banner), Some(b_rect)) = (banner_kind, banner_rect) {
        report.diagnostic_banner_rect_pt = Some(b_rect);
        render_bottom_banner(
            ui,
            app,
            views,
            BottomBannerSpec { kind: &banner, rect: b_rect, tokens: ctx_in.tokens, enabled: stage_enabled },
            meta,
            intents,
        );
    }

    if mode != CenterStageMode::CanvasViewport {
        views.canvas_view.last_active_paper_sheet_id = None;
        views.canvas_view.last_paper_rect_pt = None;
        views.canvas_view.last_scene_size_pt = None;
    }

    if !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery) {
        let notes = collect_visible_floating_notes(doc, active_sheet_id.as_ref(), &views.center_stage.note_pinned);
        if !notes.is_empty() {
            render_floating_notes_overlay(
                ui,
                app,
                views,
                FloatingNotesSpec {
                    notes,
                    stage_bounds: work_rect,
                    tokens: ctx_in.tokens,
                    enabled: stage_enabled,
                    notes_pane_focused: ctx_in.focused_pane == ActiveWorkspacePane::FloatingNotes,
                },
                meta,
                intents,
            );
        }
    }

    if stage_enabled && let Some((is_data, is_result, match_count)) = sheet_find_sync {
        let anchor_rect = report.primary_rect_pt.unwrap_or(work_rect);
        let still_open =
            render_sheet_find_bar(ui.ctx(), anchor_rect, is_data, is_result, match_count, &mut views.sheet_find, ctx_in.tokens, meta, intents);
        if !still_open {
            views.column_grid.active_find_matches = None;
            views.results_view.active_find_matches = None;
        }
    }

    render_progress_modal_if_open(ui, app, views, ctx_in.tokens, meta);
    report
}

fn sync_sheet_find_before_stage(
    app: &UiApp,
    views: &mut EguiShellViewState,
    mode: CenterStageMode,
    active_sheet_id: Option<&SheetId>,
    intents: &mut PendingUiIntents,
) -> Option<(bool, bool, usize)> {
    let is_data = matches!(mode, CenterStageMode::ColumnGrid | CenterStageMode::DataGrid);
    let is_result = mode == CenterStageMode::ResultSheet;
    if !views.sheet_find.open || (!is_data && !is_result) {
        views.column_grid.active_find_matches = None;
        views.results_view.active_find_matches = None;
        return None;
    }

    let sheet_id = active_sheet_id?;
    if views.sheet_find.last_sheet_id.as_ref() != Some(sheet_id) {
        views.sheet_find.last_sheet_id = Some(sheet_id.clone());
        views.sheet_find.active_match_index = 0;
        views.sheet_find.pending_scroll_to_active = true;
    }

    let doc = app.document();
    if is_data && let Some(ds) = doc.data_sheet(sheet_id) {
        let _ = set_active_find_locale(sheet_id, &app.locale_preset);
        let matches = find_matches_in_data_sheet_cached(
            app.mutation_seq(),
            ds,
            &views.sheet_find.query,
            views.sheet_find.match_case,
            views.sheet_find.match_entire_cell,
            &app.locale_preset,
        );
        views.sheet_find.sync_match_count(matches.len());
        let overlay = DataSheetFindOverlay::from_matches(&matches, views.sheet_find.active_match_index);
        views.column_grid.active_find_matches = (!overlay.is_empty()).then(|| overlay.to_grid_highlight());
        views.results_view.active_find_matches = None;
        if views.sheet_find.pending_scroll_to_active {
            if let Some(active) = matches.get(views.sheet_find.active_match_index) {
                let target = match active {
                    DataSheetFindMatch::Cell(coord) => DataGridScrollTarget::Cell(*coord),
                    DataSheetFindMatch::ColumnTitle { col } => DataGridScrollTarget::ColumnTitle { col: *col },
                    DataSheetFindMatch::RowTitle { row } => DataGridScrollTarget::RowTitle { row: *row },
                };
                views.column_grid.scroll_to_target(target);
            }
            views.sheet_find.pending_scroll_to_active = false;
        }
        return Some((true, false, matches.len()));
    }

    if is_result && let Some(rs) = doc.result_sheet(sheet_id) {
        let active_tab_idx = views.results_view.active_tab_for_sheet(sheet_id, rs.tabs.len(), app.canvas_dialogs.result_view.active_tab_index);
        let matches = find_matches_in_result_sheet_cached(
            app.mutation_seq(),
            rs,
            active_tab_idx,
            views.sheet_find.all_result_tabs,
            &views.sheet_find.query,
            views.sheet_find.match_case,
            views.sheet_find.match_entire_cell,
        );
        views.sheet_find.sync_match_count(matches.len());
        let overlay = ResultSheetFindOverlay::from_matches(&matches, views.sheet_find.active_match_index);
        views.results_view.active_find_matches = (!overlay.is_empty()).then(|| overlay.to_results_highlight(active_tab_idx));
        views.column_grid.active_find_matches = None;
        if views.sheet_find.pending_scroll_to_active {
            if let Some(active) = matches.get(views.sheet_find.active_match_index) {
                if active.tab_idx != active_tab_idx {
                    views.results_view.per_sheet_active_tab.insert(sheet_id.clone(), active.tab_idx);
                    intents.push(UiIntent::SelectResultTab { index: active.tab_idx, sheet_id: Some(sheet_id.clone()) });
                }
                views.results_view.scroll_to_match(sheet_id.clone(), active.tab_idx, active.row_idx, active.col_idx);
            }
            views.sheet_find.pending_scroll_to_active = false;
        }
        return Some((false, true, matches.len()));
    }

    views.column_grid.active_find_matches = None;
    views.results_view.active_find_matches = None;
    None
}

fn ensure_solid_stage_scrollbars(ui: &mut egui::Ui, tokens: &Tokens) {
    let style = ui.style_mut();
    style.spacing.scroll.floating = false;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.visuals.widgets.inactive.bg_fill = tokens.panel_bg;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, tokens.text_muted);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, tokens.text_primary);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, tokens.accent);
}

fn draw_focus_ring_if_active(ui: &egui::Ui, rect: Rect, tokens: &Tokens, focused: bool) {
    if focused && rect.is_positive() {
        ui.painter().rect_stroke(rect.shrink(1.0), CornerRadius::same(2), Stroke::new(2.0, tokens.focus_ring), StrokeKind::Inside);
    }
}

fn resolve_split_orientation(work_rect: Rect, metrics: &LayoutMetrics) -> SplitOrientation {
    if work_rect.width() >= metrics.split_vertical_breakpoint_w {
        SplitOrientation::Horizontal
    } else if work_rect.height() >= metrics.split_vertical_min_h {
        SplitOrientation::Vertical
    } else {
        SplitOrientation::CompactSinglePane
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_f6_split_and_note_focus(
    ui: &egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: &CenterStageContext<'_>,
    split_active: bool,
    split_orientation: SplitOrientation,
    active_sheet_id: Option<&SheetId>,
    intents: &mut PendingUiIntents,
) {
    if !ctx_in.enabled {
        return;
    }

    let doc = app.document();
    let (primary_id, secondary_resolved) = app.split.resolve_sheets(doc);
    let secondary_id = secondary_resolved.or_else(|| pick_fallback_secondary_sheet(doc, primary_id.as_ref())).or_else(|| primary_id.clone());
    let sync_active_sheet_for_slot = |slot: SplitPaneSlot, intents: &mut PendingUiIntents| {
        let target = match slot {
            SplitPaneSlot::Primary => primary_id.as_ref(),
            SplitPaneSlot::Secondary => secondary_id.as_ref(),
        };
        if let Some(sid) = target
            && doc.active_sheet_id.as_ref() != Some(sid)
        {
            intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str(), "mode": "family" }));
        }
    };
    let both_panes_visible = split_active && split_orientation != SplitOrientation::CompactSinglePane;

    let mut cycle_reverse: Option<bool> = None;
    let mut found_cycle = false;
    for intent in intents.intents() {
        if let UiIntent::UiCommand { command_id, params } = intent
            && command_id == "ui.pane.cycle"
        {
            found_cycle = true;
            cycle_reverse = Some(params.get("reverse").and_then(serde_json::Value::as_bool).unwrap_or(false));
        }
    }

    if found_cycle && split_active {
        if !both_panes_visible {
            return;
        }
        let reverse = cycle_reverse.unwrap_or(false);
        if ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid {
            let consume_internal = match (views.center_stage.focused_split_pane, reverse) {
                (SplitPaneSlot::Primary, false) => {
                    views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
                    sync_active_sheet_for_slot(SplitPaneSlot::Secondary, intents);
                    ui.ctx().request_repaint();
                    true
                }
                (SplitPaneSlot::Secondary, true) => {
                    views.center_stage.focused_split_pane = SplitPaneSlot::Primary;
                    sync_active_sheet_for_slot(SplitPaneSlot::Primary, intents);
                    ui.ctx().request_repaint();
                    true
                }
                _ => false,
            };
            if consume_internal {
                strip_pane_cycle_intent(intents);
            }
        } else if reverse && ctx_in.focused_pane == ActiveWorkspacePane::Inspector {
            views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
            sync_active_sheet_for_slot(SplitPaneSlot::Secondary, intents);
            ui.ctx().request_repaint();
        } else if !reverse
            && (ctx_in.focused_pane == ActiveWorkspacePane::FamilyStrip
                || (ctx_in.focused_pane == ActiveWorkspacePane::Navigator && !app.is_workspace_pane_visible(ActiveWorkspacePane::FamilyStrip)))
        {
            views.center_stage.focused_split_pane = SplitPaneSlot::Primary;
            sync_active_sheet_for_slot(SplitPaneSlot::Primary, intents);
            ui.ctx().request_repaint();
        }
        return;
    }

    // Standalone `render_center_stage` fallback when outer `inspect_bridge` shortcut handler wasn't run.
    let (f6_pressed, shift_held) = ui.ctx().input(|i| (i.key_pressed(Key::F6), i.modifiers.shift));
    if !f6_pressed || found_cycle {
        return;
    }

    let has_notes = active_sheet_id.is_some_and(|id| !active_sheet_floating_notes(doc, id).is_empty());
    if ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid {
        if both_panes_visible {
            if !shift_held && views.center_stage.focused_split_pane == SplitPaneSlot::Primary {
                views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
                sync_active_sheet_for_slot(SplitPaneSlot::Secondary, intents);
                ui.ctx().request_repaint();
            } else if shift_held && views.center_stage.focused_split_pane == SplitPaneSlot::Secondary {
                views.center_stage.focused_split_pane = SplitPaneSlot::Primary;
                sync_active_sheet_for_slot(SplitPaneSlot::Primary, intents);
                ui.ctx().request_repaint();
            } else if !shift_held && has_notes {
                intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::FloatingNotes });
            } else {
                intents.push_ui("ui.pane.cycle", json!({ "reverse": shift_held }));
            }
        } else if !shift_held && has_notes {
            intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::FloatingNotes });
        } else if !shift_held {
            intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::Inspector });
        } else {
            intents.push_ui("ui.pane.cycle", json!({ "reverse": true }));
        }
    } else if ctx_in.focused_pane == ActiveWorkspacePane::FloatingNotes {
        if both_panes_visible {
            let target = if shift_held { SplitPaneSlot::Secondary } else { SplitPaneSlot::Primary };
            views.center_stage.focused_split_pane = target;
            sync_active_sheet_for_slot(target, intents);
            ui.ctx().request_repaint();
        }
        intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::MainCanvasOrGrid });
    }
}

fn strip_pane_cycle_intent(intents: &mut PendingUiIntents) {
    let preserved: Vec<UiIntent> = intents
        .intents()
        .iter()
        .filter(|intent| !matches!(intent, UiIntent::UiCommand { command_id, .. } if command_id == "ui.pane.cycle"))
        .cloned()
        .collect();
    intents.clear();
    for intent in preserved {
        intents.push(intent);
    }
}

fn resolve_effective_split_ratio(app: &UiApp, state: &mut CenterStageEguiState, current_pair: Option<&(SheetId, SheetId)>) -> f32 {
    if state.last_seen_app_split_ratio != Some(app.split.ratio) {
        if state.last_seen_app_split_ratio.is_some() && state.auto_ratio_pending_pair.as_ref() != current_pair {
            state.manual_ratio_pair = current_pair.cloned();
            state.auto_ratio_pending_pair = None;
        }
        state.last_seen_app_split_ratio = Some(app.split.ratio);
        state.split_ratio_override = Some((app.split.ratio as f32).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO));
    }
    state.split_ratio_override.unwrap_or(app.split.ratio as f32).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
}

fn resolve_data_results_pair(doc: &ProjectDocument, primary_id: Option<&SheetId>, secondary_id: Option<&SheetId>) -> Option<(SheetId, bool)> {
    let (Some(p), Some(s)) = (primary_id, secondary_id) else {
        return None;
    };
    if doc.data_sheet(p).is_some() && doc.result_sheet(s).is_some() {
        Some((s.clone(), false))
    } else if doc.result_sheet(p).is_some() && doc.data_sheet(s).is_some() {
        Some((p.clone(), true))
    } else {
        None
    }
}

fn compute_data_results_initial_split_ratio(
    natural_table_width_pt: f32,
    work_width_pt: f32,
    scrollbar_w_pt: f32,
    padding_pt: f32,
    results_is_primary: bool,
) -> f64 {
    let avail_w = f64::from((work_width_pt - SPLITTER_VISUAL_PT).max(1.0));
    let needed_results_w = f64::from((natural_table_width_pt + scrollbar_w_pt + padding_pt).max(0.0));
    let results_frac = needed_results_w / avail_w;
    let raw_primary = if results_is_primary { results_frac } else { 1.0 - results_frac };
    raw_primary.clamp(DATA_RESULTS_MIN_SPLIT_RATIO, DATA_RESULTS_MAX_SPLIT_RATIO)
}

struct StageFrameIo<'a> {
    views: &'a mut EguiShellViewState,
    meta: &'a mut FrameInspectMeta,
    intents: &'a mut PendingUiIntents,
    report: &'a mut CenterStageFrameReport,
}

struct SplitPaneSpec<'a> {
    slot: SplitPaneSlot,
    pane_rect: Rect,
    sheet_id: Option<&'a SheetId>,
    other_sheet_id: Option<&'a SheetId>,
    last_active_data_sheet_id: Option<&'a SheetId>,
    locale: &'a str,
    enabled: bool,
    focused: bool,
}

struct SplitPaneHeaderSpec<'a> {
    slot: SplitPaneSlot,
    header_rect: Rect,
    sheet_id: Option<&'a SheetId>,
    other_sheet_id: Option<&'a SheetId>,
    last_active_data_sheet_id: Option<&'a SheetId>,
    locale: &'a str,
    tokens: &'a Tokens,
    enabled: bool,
    focused: bool,
}

struct SinglePaneTarget<'a> {
    mode: CenterStageMode,
    sheet_id: Option<&'a SheetId>,
    slot: SplitPaneSlot,
    enabled: bool,
    pane_focused: bool,
}

#[allow(clippy::too_many_arguments)]
fn render_compact_split_stage(
    ui: &mut egui::Ui,
    app: &UiApp,
    ctx_in: &CenterStageContext<'_>,
    work_rect: Rect,
    enabled: bool,
    primary_id: Option<&SheetId>,
    secondary_id: Option<&SheetId>,
    io: &mut StageFrameIo<'_>,
) {
    let doc = app.document();
    let show_secondary = if doc.active_sheet_id.as_ref() == secondary_id && primary_id != secondary_id {
        true
    } else if doc.active_sheet_id.as_ref() == primary_id {
        false
    } else {
        io.views.center_stage.focused_split_pane == SplitPaneSlot::Secondary
    };
    let visible_slot = if show_secondary { SplitPaneSlot::Secondary } else { SplitPaneSlot::Primary };
    let visible_sheet = if show_secondary { secondary_id } else { primary_id };
    io.views.center_stage.focused_split_pane = visible_slot;
    io.report.primary_rect_pt = Some(work_rect);
    io.report.secondary_rect_pt = None;

    let header_h = ctx_in.tokens.metrics.panel_header_h.min(work_rect.height());
    let header_rect = Rect::from_min_max(work_rect.min, pos2(work_rect.right(), work_rect.top() + header_h));
    let body_rect = Rect::from_min_max(pos2(work_rect.left(), header_rect.bottom()), work_rect.max);
    let compact_click_in_pane = enabled
        && ui
            .input(|i| (i.pointer.primary_pressed() || i.pointer.primary_clicked()).then(|| i.pointer.interact_pos()).flatten())
            .is_some_and(|p| body_rect.contains(p) && ui.ctx().layer_id_at(p).is_none_or(|l| l.order <= ui.layer_id().order));
    if compact_click_in_pane && ctx_in.focused_pane != ActiveWorkspacePane::MainCanvasOrGrid {
        io.intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::MainCanvasOrGrid });
    }
    let compact_pane_focused = enabled && (ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid || compact_click_in_pane);
    render_compact_split_flip_header(
        ui,
        doc,
        header_rect,
        primary_id,
        secondary_id,
        show_secondary,
        ctx_in.tokens,
        enabled,
        compact_pane_focused,
        io.meta,
        io.intents,
    );
    let pane_mode = visible_sheet.map_or(CenterStageMode::Welcome, |id| mode_for_sheet(doc, id));
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    body_ui.set_clip_rect(body_rect);
    render_single_pane_mode(
        &mut body_ui,
        app,
        SinglePaneTarget { mode: pane_mode, sheet_id: visible_sheet, slot: visible_slot, enabled, pane_focused: compact_pane_focused },
        ctx_in,
        io,
    );
    sync_two_way_split_brushing(app, io.views, primary_id, secondary_id, io.intents);
}

#[allow(clippy::too_many_arguments)]
fn try_apply_data_results_initial_ratio(
    ui: &egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    intents: &mut PendingUiIntents,
    work_width_pt: f32,
    current_pair: Option<&(SheetId, SheetId)>,
    data_results_config: Option<&(SheetId, bool)>,
    pre_render: bool,
) -> Option<f32> {
    let pair = current_pair?;
    let (rs_id, results_is_primary) = data_results_config?;
    if views.center_stage.auto_ratio_pending_pair.as_ref() != Some(pair) || views.center_stage.manual_ratio_pair.as_ref() == Some(pair) {
        return None;
    }
    let doc = app.document();
    let res_state = if *results_is_primary { &views.results_view } else { &views.center_stage.secondary_results_view };
    let active_tab_idx =
        doc.result_sheet(rs_id).map_or(0, |rs| res_state.active_tab_for_sheet(rs_id, rs.tabs.len(), app.canvas_dialogs.result_view.active_tab_index));
    let nat_w = if pre_render {
        res_state
            .natural_table_width_for_sheet(rs_id, active_tab_idx)
            .or_else(|| views.results_view.natural_table_width_for_sheet(rs_id, active_tab_idx))
            .or_else(|| views.center_stage.secondary_results_view.natural_table_width_for_sheet(rs_id, active_tab_idx))
    } else {
        res_state.natural_table_width_for_sheet(rs_id, active_tab_idx)
    };
    let Some(nat_w) = nat_w.filter(|w| w.is_finite() && *w > 0.0) else {
        if !pre_render {
            views.center_stage.auto_ratio_pending_pair = None;
        }
        return None;
    };
    let scroll_w = ui.spacing().scroll.allocated_width().max(14.0);
    let init_ratio = compute_data_results_initial_split_ratio(nat_w, work_width_pt, scroll_w, DATA_RESULTS_PADDING_PT, *results_is_primary);
    views.center_stage.split_ratio_override = Some(init_ratio as f32);
    views.center_stage.last_seen_app_split_ratio = Some(init_ratio);
    views.center_stage.auto_ratio_pending_pair = None;
    if (app.split.ratio - init_ratio).abs() > 1e-4 {
        intents.push(UiIntent::SetSplitRatio { ratio: init_ratio });
    }
    if !pre_render {
        ui.ctx().request_repaint();
    }
    Some(init_ratio as f32)
}

#[allow(clippy::too_many_arguments)]
fn sync_split_pane_click_focus(
    ui: &egui::Ui,
    doc: &ProjectDocument,
    ctx_in: &CenterStageContext<'_>,
    views: &mut CenterStageEguiState,
    intents: &mut PendingUiIntents,
    primary_id: Option<&SheetId>,
    secondary_id: Option<&SheetId>,
    primary_rect: Rect,
    secondary_rect: Rect,
    divider_hit_rect: Rect,
    enabled: bool,
) -> Option<SplitPaneSlot> {
    if !enabled {
        return None;
    }
    let panel_h = ctx_in.tokens.metrics.panel_header_h;
    let body_of = |r: Rect| Rect::from_min_max(pos2(r.left(), r.top() + panel_h.min(r.height())), r.max);
    let click_pos = ui
        .input(|i| (i.pointer.primary_pressed() || i.pointer.primary_clicked()).then(|| i.pointer.interact_pos()).flatten())
        .filter(|p| !divider_hit_rect.contains(*p) && ui.ctx().layer_id_at(*p).is_none_or(|l| l.order <= ui.layer_id().order))?;
    let slot = if body_of(primary_rect).contains(click_pos) {
        SplitPaneSlot::Primary
    } else if body_of(secondary_rect).contains(click_pos) {
        SplitPaneSlot::Secondary
    } else {
        return None;
    };
    let slot_changed = views.focused_split_pane != slot;
    views.focused_split_pane = slot;
    if ctx_in.focused_pane != ActiveWorkspacePane::MainCanvasOrGrid {
        intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::MainCanvasOrGrid });
    }
    let target_sid = match slot {
        SplitPaneSlot::Primary => primary_id,
        SplitPaneSlot::Secondary => secondary_id,
    };
    if let Some(sid) = target_sid
        && doc.active_sheet_id.as_ref() != Some(sid)
    {
        intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str(), "mode": "family" }));
    }
    if slot_changed {
        ui.ctx().request_repaint();
    }
    Some(slot)
}

fn render_split_stage(ui: &mut egui::Ui, app: &UiApp, ctx_in: &CenterStageContext<'_>, work_rect: Rect, enabled: bool, io: &mut StageFrameIo<'_>) {
    let doc = app.document();
    let orientation = resolve_split_orientation(work_rect, &ctx_in.tokens.metrics);
    let (primary_id, secondary_resolved) = app.split.resolve_sheets(doc);
    let secondary_id = secondary_resolved.or_else(|| pick_fallback_secondary_sheet(doc, primary_id.as_ref())).or_else(|| primary_id.clone());

    if orientation == SplitOrientation::CompactSinglePane {
        render_compact_split_stage(ui, app, ctx_in, work_rect, enabled, primary_id.as_ref(), secondary_id.as_ref(), io);
        return;
    }

    let StageFrameIo { views, meta, intents, report } = io;
    let current_pair = primary_id.clone().zip(secondary_id.clone());
    let data_results_config = resolve_data_results_pair(doc, primary_id.as_ref(), secondary_id.as_ref());
    if views.center_stage.last_split_pair != current_pair {
        views.center_stage.last_split_pair = current_pair.clone();
        views.center_stage.manual_ratio_pair = None;
        views.center_stage.auto_ratio_pending_pair =
            if orientation == SplitOrientation::Horizontal && data_results_config.is_some() { current_pair.clone() } else { None };
    }

    let mut ratio = resolve_effective_split_ratio(app, &mut views.center_stage, current_pair.as_ref());
    if orientation == SplitOrientation::Horizontal
        && let Some(init_ratio) = try_apply_data_results_initial_ratio(
            ui,
            app,
            views,
            intents,
            work_rect.width(),
            current_pair.as_ref(),
            data_results_config.as_ref(),
            true,
        )
    {
        ratio = init_ratio;
    }

    let (mut primary_rect, mut divider_visual_rect, divider_hit_rect, mut secondary_rect) =
        compute_split_geometries(work_rect, orientation, ratio, ctx_in.tokens.metrics.splitter_hit_slop_w);

    let divider_resp =
        ui.interact(divider_hit_rect, Id::unique("center_stage.split.divider"), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    divider_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, enabled, "Split Comparison Divider"));
    if enabled {
        let cursor = match orientation {
            SplitOrientation::Horizontal => egui::CursorIcon::ResizeHorizontal,
            SplitOrientation::Vertical | SplitOrientation::CompactSinglePane => egui::CursorIcon::ResizeVertical,
        };
        if divider_resp.hovered() || divider_resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if divider_resp.double_clicked() {
            let is_data_graph = primary_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some())
                && secondary_id.as_ref().is_some_and(|id| doc.graph_sheet(id).is_some());
            ratio = if is_data_graph { 0.40 } else { DEFAULT_SPLIT_RATIO };
            views.center_stage.manual_ratio_pair = current_pair.clone();
            views.center_stage.auto_ratio_pending_pair = None;
            views.center_stage.split_ratio_override = Some(ratio);
            views.center_stage.last_seen_app_split_ratio = Some(f64::from(ratio).clamp(0.15, 0.85));
            intents.push(UiIntent::SetSplitRatio { ratio: f64::from(ratio) });
            (primary_rect, divider_visual_rect, _, secondary_rect) =
                compute_split_geometries(work_rect, orientation, ratio, ctx_in.tokens.metrics.splitter_hit_slop_w);
        } else if divider_resp.dragged() {
            views.center_stage.split_dragging = true;
            views.center_stage.manual_ratio_pair = current_pair.clone();
            views.center_stage.auto_ratio_pending_pair = None;
            if let Some(pointer) = divider_resp.interact_pointer_pos() {
                let raw = match orientation {
                    SplitOrientation::Horizontal => (pointer.x - work_rect.left()) / work_rect.width().max(1.0),
                    SplitOrientation::Vertical | SplitOrientation::CompactSinglePane => (pointer.y - work_rect.top()) / work_rect.height().max(1.0),
                };
                ratio = raw.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
                views.center_stage.split_ratio_override = Some(ratio);
                views.center_stage.last_seen_app_split_ratio = Some(f64::from(ratio).clamp(0.15, 0.85));
                intents.push(UiIntent::SetSplitRatio { ratio: f64::from(ratio) });
                (primary_rect, divider_visual_rect, _, secondary_rect) =
                    compute_split_geometries(work_rect, orientation, ratio, ctx_in.tokens.metrics.splitter_hit_slop_w);
            }
        } else {
            views.center_stage.split_dragging = false;
        }
    }

    let divider_color = if divider_resp.hovered() || divider_resp.dragged() { ctx_in.tokens.accent } else { ctx_in.tokens.divider };
    ui.painter().rect_filled(divider_visual_rect, CornerRadius::ZERO, ctx_in.tokens.panel_bg);
    match orientation {
        SplitOrientation::Horizontal => {
            ui.painter().vline(divider_visual_rect.center().x, divider_visual_rect.y_range(), Stroke::new(1.0, divider_color));
        }
        SplitOrientation::Vertical | SplitOrientation::CompactSinglePane => {
            ui.painter().hline(divider_visual_rect.x_range(), divider_visual_rect.center().y, Stroke::new(1.0, divider_color));
        }
    }
    meta.record_response(
        ui,
        &divider_resp,
        FrameInspectEntryInput {
            widget_id: "split.divider",
            role: "separator",
            label: "Split Comparison Divider",
            command_id: None,
            value: Some(json!({ "ratio": ratio })),
            enabled,
            disabled_reason: (!enabled).then_some("Blocked by modal"),
            z_layer: ZLayer::SplitHandle,
            ellipsized: false,
            full_label: None,
        },
    );

    report.primary_rect_pt = Some(primary_rect);
    report.secondary_rect_pt = Some(secondary_rect);

    let clicked_slot = sync_split_pane_click_focus(
        ui,
        doc,
        ctx_in,
        &mut views.center_stage,
        intents,
        primary_id.as_ref(),
        secondary_id.as_ref(),
        primary_rect,
        secondary_rect,
        divider_hit_rect,
        enabled,
    );

    let stage_has_focus = ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid || clicked_slot.is_some();
    let locale = app.locale.as_str();
    let last_ds_id = views.center_stage.last_active_data_sheet_id.clone();

    {
        let primary_focused = stage_has_focus && views.center_stage.focused_split_pane == SplitPaneSlot::Primary;
        let mut pane_io = StageFrameIo { views, meta, intents, report };
        render_split_pane_container(
            ui,
            app,
            ctx_in,
            SplitPaneSpec {
                slot: SplitPaneSlot::Primary,
                pane_rect: primary_rect,
                sheet_id: primary_id.as_ref(),
                other_sheet_id: secondary_id.as_ref(),
                last_active_data_sheet_id: last_ds_id.as_ref(),
                locale,
                enabled,
                focused: primary_focused,
            },
            &mut pane_io,
        );

        let secondary_focused = stage_has_focus && pane_io.views.center_stage.focused_split_pane == SplitPaneSlot::Secondary;
        render_split_pane_container(
            ui,
            app,
            ctx_in,
            SplitPaneSpec {
                slot: SplitPaneSlot::Secondary,
                pane_rect: secondary_rect,
                sheet_id: secondary_id.as_ref(),
                other_sheet_id: primary_id.as_ref(),
                last_active_data_sheet_id: last_ds_id.as_ref(),
                locale,
                enabled,
                focused: secondary_focused,
            },
            &mut pane_io,
        );
    }

    if orientation == SplitOrientation::Horizontal {
        let _ = try_apply_data_results_initial_ratio(
            ui,
            app,
            views,
            intents,
            work_rect.width(),
            current_pair.as_ref(),
            data_results_config.as_ref(),
            false,
        );
    }

    sync_two_way_split_brushing(app, views, primary_id.as_ref(), secondary_id.as_ref(), intents);
}

#[allow(clippy::too_many_arguments)]
fn render_compact_split_flip_header(
    ui: &mut egui::Ui,
    doc: &ProjectDocument,
    header_rect: Rect,
    primary_id: Option<&SheetId>,
    secondary_id: Option<&SheetId>,
    show_secondary: bool,
    tokens: &Tokens,
    enabled: bool,
    focused: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.painter().rect_filled(header_rect, CornerRadius::ZERO, tokens.panel_bg);
    ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, tokens.divider));
    if focused {
        ui.painter().hline(header_rect.x_range(), header_rect.top() + 1.0, Stroke::new(tokens.metrics.stroke_focus_pt, tokens.accent));
    }

    let p_title = primary_id.and_then(|id| doc_sheet_title(doc, id)).unwrap_or("Sheet 1");
    let s_title = secondary_id.and_then(|id| doc_sheet_title(doc, id)).unwrap_or("Sheet 2");

    let mut hdr_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(vec2(tokens.metrics.space_md, tokens.metrics.space_xxs)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    hdr_ui.set_clip_rect(header_rect);

    let close_w = 48.0;
    let seg_w = (hdr_ui.available_width() - close_w - tokens.metrics.space_md).max(120.0);
    hdr_ui.allocate_ui(vec2(seg_w, tokens.metrics.min_hit_target_pt + tokens.metrics.space_xs), |seg_ui| {
        let opts = [
            crate::egui_shell::ui_kit::SegmentedOption { widget_id: "split.compact.primary", label: p_title, tooltip: Some(p_title), value: None },
            crate::egui_shell::ui_kit::SegmentedOption { widget_id: "split.compact.secondary", label: s_title, tooltip: Some(s_title), value: None },
        ];
        if let Some(chosen) =
            crate::egui_shell::ui_kit::segmented_control(seg_ui, tokens, meta, &opts, usize::from(show_secondary), enabled, ZLayer::Workspace)
        {
            let target = if chosen == 1 { secondary_id } else { primary_id };
            if let Some(sid) = target {
                intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str(), "mode": "family" }));
            }
        }
    });

    hdr_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |right_ui| {
        let (_, clicked) = crate::egui_shell::ui_kit::ghost_action_button(
            right_ui,
            tokens,
            meta,
            "split.close",
            "Close",
            Some("ui.split.toggle"),
            false,
            enabled,
            ZLayer::Workspace,
        );
        if clicked {
            intents.push_ui("ui.split.toggle", json!({ "enabled": false }));
        }
    });
}

fn render_split_pane_container(ui: &mut egui::Ui, app: &UiApp, ctx_in: &CenterStageContext<'_>, pane: SplitPaneSpec<'_>, io: &mut StageFrameIo<'_>) {
    let SplitPaneSpec { slot, pane_rect, sheet_id, other_sheet_id, last_active_data_sheet_id, locale, enabled, focused } = pane;
    let doc = app.document();
    let header_h = ctx_in.tokens.metrics.panel_header_h.min(pane_rect.height());
    let header_rect = Rect::from_min_max(pane_rect.min, pos2(pane_rect.right(), pane_rect.top() + header_h));
    let body_rect = Rect::from_min_max(pos2(pane_rect.left(), header_rect.bottom()), pane_rect.max);

    render_split_pane_header(
        ui,
        app,
        SplitPaneHeaderSpec {
            slot,
            header_rect,
            sheet_id,
            other_sheet_id,
            last_active_data_sheet_id,
            locale,
            tokens: ctx_in.tokens,
            enabled,
            focused,
        },
        io.meta,
        io.intents,
    );

    let pane_mode = sheet_id.map_or(CenterStageMode::Welcome, |id| mode_for_sheet(doc, id));
    let mut body_ui =
        ui.new_child(egui::UiBuilder::new().id_salt(("split_pane_body", slot)).max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    body_ui.set_clip_rect(body_rect);

    render_single_pane_mode(&mut body_ui, app, SinglePaneTarget { mode: pane_mode, sheet_id, slot, enabled, pane_focused: focused }, ctx_in, io);

    let meta = &mut *io.meta;

    let pane_wid = match slot {
        SplitPaneSlot::Primary => "center.pane.primary",
        SplitPaneSlot::Secondary => "center.pane.secondary",
    };
    let pane_lbl = match slot {
        SplitPaneSlot::Primary => "Primary Split Pane",
        SplitPaneSlot::Secondary => "Secondary Split Pane",
    };
    let pane_a11y = ui.interact(pane_rect, Id::unique(("center_stage_pane_a11y", pane_wid)), Sense::hover());
    pane_a11y.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Region, enabled, pane_lbl));
    record_region_meta(meta, ui, &pane_a11y, inspect_input(pane_wid, "region", pane_lbl, None, None, enabled, ZLayer::Workspace));
}

enum SplitHeaderBadge {
    DataCoord { label: String },
    LinkedBrush { label: String, clear_widget_id: &'static str },
}

fn resolve_split_header_badge(
    doc: &ProjectDocument,
    app: &UiApp,
    slot: SplitPaneSlot,
    sheet_id: Option<&SheetId>,
    other_sheet_id: Option<&SheetId>,
    last_active_data_sheet_id: Option<&SheetId>,
) -> Option<SplitHeaderBadge> {
    let sid = sheet_id?;
    if doc.data_sheet(sid).is_some() {
        let data_sheet_owns_grid = doc.active_sheet_id.as_ref() == Some(sid)
            || (!doc.active_sheet_id.as_ref().is_some_and(|aid| doc.data_sheet(aid).is_some())
                && (last_active_data_sheet_id == Some(sid)
                    || (last_active_data_sheet_id.is_none() && !other_sheet_id.is_some_and(|oid| doc.data_sheet(oid).is_some()))));
        let coord = if data_sheet_owns_grid {
            if let Some((xr, xs)) = app.grid.active_x_cell {
                Some(format!("R{}X{}", xr + 1, xs + 1))
            } else {
                app.grid.tool.active.map(|c| format!("R{}C{}", c.row + 1, c.col + 1))
            }
        } else {
            app.brushed_cell.as_ref().filter(|b| &b.sheet_id == sid).map(|b| format!("R{}C{}", b.row + 1, b.col + 1))
        };
        return coord.map(|label| SplitHeaderBadge::DataCoord { label });
    }
    if doc.graph_sheet(sid).is_some()
        && let Some(brushed) = app.brushed_cell.as_ref()
        && doc.upstream_dependencies(sid).contains(&brushed.sheet_id)
    {
        let other_also_linked =
            other_sheet_id.is_some_and(|oid| doc.graph_sheet(oid).is_some() && doc.upstream_dependencies(oid).contains(&brushed.sheet_id));
        let clear_widget_id = if slot == SplitPaneSlot::Primary && other_also_linked { "split.clear_brush.primary" } else { "split.clear_brush" };
        return Some(SplitHeaderBadge::LinkedBrush { label: format!("Brushed: R{}C{}", brushed.row + 1, brushed.col + 1), clear_widget_id });
    }
    None
}

fn render_split_pane_header(
    ui: &mut egui::Ui,
    app: &UiApp,
    spec: SplitPaneHeaderSpec<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let SplitPaneHeaderSpec { slot, header_rect, sheet_id, other_sheet_id, last_active_data_sheet_id, locale, tokens, enabled, focused } = spec;
    let doc = app.document();
    ui.painter().rect_filled(header_rect, CornerRadius::ZERO, tokens.panel_bg);
    ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, tokens.divider));
    if focused {
        ui.painter().hline(header_rect.x_range(), header_rect.top() + 1.0, Stroke::new(tokens.metrics.stroke_focus_pt, tokens.accent));
    }

    let header_wid = match slot {
        SplitPaneSlot::Primary => "split.header.primary",
        SplitPaneSlot::Secondary => "split.header.secondary",
    };
    let hdr_region = ui.interact(header_rect, Id::unique(("split_pane_header_region", header_wid)), Sense::hover());
    hdr_region.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Region, enabled, "Split Pane Header"));
    meta.record_response(ui, &hdr_region, inspect_input(header_wid, "region", "Split Pane Header", None, None, enabled, ZLayer::Workspace));

    let mut hdr_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(vec2(tokens.metrics.space_md, tokens.metrics.space_xs)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    hdr_ui.set_clip_rect(header_rect);

    let disambig = crate::workspace_nav::compute_sheet_disambiguation(doc);
    let current_display: String = sheet_id
        .and_then(|id| doc_sheet_title(doc, id).map(|t| format_split_selector_label(id, t, &disambig)))
        .unwrap_or_else(|| String::from("(None)"));
    let current_title = current_display.as_str();
    let sel_wid = match slot {
        SplitPaneSlot::Primary => "split.primary_selector",
        SplitPaneSlot::Secondary => "split.secondary_selector",
    };
    let header_badge = resolve_split_header_badge(doc, app, slot, sheet_id, other_sheet_id, last_active_data_sheet_id);
    let small_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
    let badge_needed_w = match &header_badge {
        Some(SplitHeaderBadge::DataCoord { label }) => {
            hdr_ui.painter().layout_no_wrap(label.clone(), small_font.clone(), tokens.text_primary).size().x + 6.0
        }
        Some(SplitHeaderBadge::LinkedBrush { label, .. }) => {
            let text_w = hdr_ui.painter().layout_no_wrap(label.clone(), small_font.clone(), tokens.text_primary).size().x;
            text_w + 14.0 + tokens.metrics.min_hit_target_pt + tokens.metrics.space_xs
        }
        None => 0.0,
    };
    let right_reserve = if slot == SplitPaneSlot::Secondary { 104.0 } else { 0.0 };
    let badge_reserve = if badge_needed_w > 0.0 { badge_needed_w + tokens.metrics.space_sm } else { 0.0 };
    let combo_cap_w = (hdr_ui.available_width() - right_reserve - badge_reserve - tokens.metrics.space_xs).max(88.0);
    let combo_w = (header_rect.width() * 0.52).clamp(88.0, 240.0).min(combo_cap_w);
    let title_color = if focused { tokens.text_strong } else { tokens.text_muted };
    let all_sheets = collect_all_sheet_ids(doc, &disambig);
    let combo = dropdown(("split_pane_sheet", slot), RichText::new(current_title).size(tokens.metrics.font_ui_body_pt).color(title_color))
        .width(combo_w)
        .enabled(enabled)
        .show_ui(&mut hdr_ui, |combo_ui| {
            for (cand_id, cand_title, kind_badge) in &all_sheets {
                let selected = sheet_id == Some(cand_id);
                if combo_ui.selectable_label(selected, format!("[{kind_badge}] {cand_title}")).clicked() && enabled {
                    let param_key = match slot {
                        SplitPaneSlot::Primary => "primary_sheet_id",
                        SplitPaneSlot::Secondary => "secondary_sheet_id",
                    };
                    intents.push_ui("ui.split.toggle", json!({ "enabled": true, param_key: cand_id.as_str() }));
                    if doc.active_sheet_id.as_ref() == sheet_id {
                        intents.push_engine("sheet.select", json!({ "sheet_id": cand_id.as_str(), "mode": "family" }));
                    }
                }
            }
        });
    let mut inspect = inspect_input(sel_wid, "combobox", current_title, Some("ui.split.toggle"), None, enabled, ZLayer::Workspace);
    inspect.ellipsized = combo.ellipsized;
    inspect.full_label = combo.ellipsized.then_some(current_title);
    meta.record_response(&hdr_ui, &combo.response, inspect);

    if let Some(badge) = header_badge
        && (hdr_ui.available_width() - right_reserve) >= badge_needed_w - 1.0
    {
        match badge {
            SplitHeaderBadge::DataCoord { label } => {
                let coord_wid = match slot {
                    SplitPaneSlot::Primary => "split.header.coord.primary",
                    SplitPaneSlot::Secondary => "split.header.coord.secondary",
                };
                let mut rt = RichText::new(&label).small();
                rt = if focused { rt.strong().color(tokens.text_strong) } else { rt.color(tokens.text_muted) };
                let resp = hdr_ui.label(rt);
                meta.record_response(&hdr_ui, &resp, inspect_input(coord_wid, "label", &label, None, None, true, ZLayer::Workspace));
            }
            SplitHeaderBadge::LinkedBrush { label, clear_widget_id } => {
                let brush_wid = match slot {
                    SplitPaneSlot::Primary => "split.header.brush.primary",
                    SplitPaneSlot::Secondary => "split.header.brush.secondary",
                };
                let pill = egui::Frame::NONE
                    .fill(tokens.surface_recessed)
                    .stroke(Stroke::new(1.0, tokens.divider))
                    .corner_radius(CornerRadius::same(tokens.metrics.radius_sm))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(&mut hdr_ui, |pill_ui| pill_ui.label(RichText::new(&label).small().color(tokens.text_primary)));
                meta.record_response(&hdr_ui, &pill.inner, inspect_input(brush_wid, "label", &label, None, None, true, ZLayer::Workspace));
                let (_, clicked) = crate::egui_shell::ui_kit::ghost_action_button(
                    &mut hdr_ui,
                    tokens,
                    meta,
                    clear_widget_id,
                    "×",
                    None,
                    false,
                    enabled,
                    ZLayer::Workspace,
                );
                if clicked {
                    intents.push(UiIntent::ClearBrush);
                }
            }
        }
    }

    if slot == SplitPaneSlot::Secondary {
        hdr_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |right_ui| {
            right_ui.spacing_mut().item_spacing.x = tokens.metrics.space_xs;
            let close_lbl = tr_or_fallback(locale, "w5.split.close", "Close");
            let (_, close_clicked) = crate::egui_shell::ui_kit::ghost_action_button(
                right_ui,
                tokens,
                meta,
                "split.close",
                close_lbl,
                Some("ui.split.toggle"),
                false,
                enabled,
                ZLayer::Workspace,
            );
            if close_clicked {
                intents.push_ui("ui.split.toggle", json!({ "enabled": false }));
            }

            let swap_lbl = tr_or_fallback(locale, "w5.split.swap", "Swap");
            let swap_enabled = enabled && sheet_id.is_some() && other_sheet_id.is_some();
            let (_, swap_clicked) = crate::egui_shell::ui_kit::ghost_action_button(
                right_ui,
                tokens,
                meta,
                "split.swap",
                swap_lbl,
                Some("ui.split.toggle"),
                false,
                swap_enabled,
                ZLayer::Workspace,
            );
            if swap_clicked && let (Some(sec), Some(prim)) = (sheet_id, other_sheet_id) {
                intents.push_ui(
                    "ui.split.toggle",
                    json!({
                        "enabled": true,
                        "primary_sheet_id": sec.as_str(),
                        "secondary_sheet_id": prim.as_str(),
                    }),
                );
            }
        });
    }
}

fn compute_split_geometries(work_rect: Rect, orientation: SplitOrientation, ratio: f32, hit_slop: f32) -> (Rect, Rect, Rect, Rect) {
    let half_vis = SPLITTER_VISUAL_PT * 0.5;
    let half_hit = hit_slop.max(SPLITTER_VISUAL_PT) * 0.5;
    match orientation {
        SplitOrientation::Horizontal => {
            let split_x = (work_rect.left() + work_rect.width() * ratio).clamp(work_rect.left() + 48.0, work_rect.right() - 48.0);
            let left = Rect::from_min_max(work_rect.min, pos2(split_x - half_vis, work_rect.bottom()));
            let right = Rect::from_min_max(pos2(split_x + half_vis, work_rect.top()), work_rect.max);
            let vis = Rect::from_min_max(pos2(split_x - half_vis, work_rect.top()), pos2(split_x + half_vis, work_rect.bottom()));
            let hit = Rect::from_min_max(pos2(split_x - half_hit, work_rect.top()), pos2(split_x + half_hit, work_rect.bottom()));
            (left, vis, hit, right)
        }
        SplitOrientation::Vertical | SplitOrientation::CompactSinglePane => {
            let split_y = (work_rect.top() + work_rect.height() * ratio).clamp(work_rect.top() + 48.0, work_rect.bottom() - 48.0);
            let top = Rect::from_min_max(work_rect.min, pos2(work_rect.right(), split_y - half_vis));
            let bottom = Rect::from_min_max(pos2(work_rect.left(), split_y + half_vis), work_rect.max);
            let vis = Rect::from_min_max(pos2(work_rect.left(), split_y - half_vis), pos2(work_rect.right(), split_y + half_vis));
            let hit = Rect::from_min_max(pos2(work_rect.left(), split_y - half_hit), pos2(work_rect.right(), split_y + half_hit));
            (top, vis, hit, bottom)
        }
    }
}

fn render_single_pane_mode(ui: &mut egui::Ui, app: &UiApp, target: SinglePaneTarget<'_>, ctx_in: &CenterStageContext<'_>, io: &mut StageFrameIo<'_>) {
    let SinglePaneTarget { mode, sheet_id, slot, enabled, pane_focused } = target;
    let StageFrameIo { views, meta, intents, report } = io;
    match mode {
        CenterStageMode::Welcome => {
            report.handled_natively = true;
            let _ = render_welcome_hub(ui, app, &mut views.welcome_hub, ctx_in.tokens, enabled, meta, intents);
        }
        CenterStageMode::ColumnGrid | CenterStageMode::DataGrid => {
            report.handled_natively = true;
            if slot == SplitPaneSlot::Primary {
                render_mv_formula_bar_if_active(ui, app, views, ctx_in.tokens, enabled, meta, intents);
            }
            let (grid_state, pane_salt) = match slot {
                SplitPaneSlot::Primary => (&mut views.column_grid, "primary"),
                SplitPaneSlot::Secondary => (&mut views.center_stage.secondary_column_grid, "secondary"),
            };
            let grid_rep = render_data_grid_for_sheet(
                ui,
                app,
                sheet_id,
                grid_state,
                ColumnGridRenderContext::new(ctx_in.tokens, !enabled || ctx_in.decision.blocks_egui_grid)
                    .with_pane_salt(pane_salt)
                    .with_pane_focused(pane_focused),
                meta,
                intents,
            );
            report.scrollbar_rect_pt = grid_rep.vertical_scrollbar_rect_pt.or(grid_rep.horizontal_scrollbar_rect_pt).or(report.scrollbar_rect_pt);
        }
        CenterStageMode::InfoSheet => {
            report.handled_natively = true;
            let info_state = match slot {
                SplitPaneSlot::Primary => &mut views.info_view,
                SplitPaneSlot::Secondary => &mut views.center_stage.secondary_info_view,
            };
            let _ = render_info_view_for_sheet(ui, app, sheet_id, info_state, ctx_in.tokens, enabled, meta, intents);
        }
        CenterStageMode::ResultSheet => {
            report.handled_natively = true;
            let res_state = match slot {
                SplitPaneSlot::Primary => &mut views.results_view,
                SplitPaneSlot::Secondary => &mut views.center_stage.secondary_results_view,
            };
            let res_rep =
                render_results_view_for_sheet_with_options(ui, app, sheet_id, res_state, ctx_in.tokens, enabled, pane_focused, true, meta, intents);
            if report.scrollbar_rect_pt.is_none() {
                report.scrollbar_rect_pt = res_rep.scrollbar_rect_pt;
            }
        }
        CenterStageMode::CanvasViewport => {
            report.handled_natively = true;
            let canvas_state = match slot {
                SplitPaneSlot::Primary => &mut views.canvas_view,
                SplitPaneSlot::Secondary => &mut views.center_stage.secondary_canvas_view,
            };
            let c_rep = render_canvas_view_for_sheet(
                ui,
                app,
                sheet_id,
                canvas_state,
                CanvasViewContext { tokens: ctx_in.tokens, enabled, is_secondary_split: slot == SplitPaneSlot::Secondary },
                meta,
                intents,
            );
            report.grid_overlay_shown |= c_rep.grid_overlay_shown;
        }
        CenterStageMode::Gallery => {
            report.handled_natively = true;
        }
    }
}

fn render_mv_formula_bar_if_active(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let ds_opt = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
    let ds_frozen = ds_opt.is_some_and(|ds| ds.frozen);
    let fb = &app.grid.formula_bar;
    if ds_opt.is_none() || !fb.active {
        views.center_stage.last_formula_bar_active = false;
        return;
    }

    let input_enabled = enabled && !ds_frozen;
    let has_target = fb.target_column.is_some() || !fb.target_title.trim().is_empty();
    let commit_enabled = input_enabled && has_target && fb.is_valid && fb.parse_error.is_none();
    let commit_reason = if !enabled {
        Some("modal dialog is active")
    } else if ds_frozen {
        Some("Data sheet is frozen (read-only).")
    } else if !has_target {
        Some("Formula bar is not active.")
    } else if fb.parse_error.is_some() {
        fb.parse_error.as_deref()
    } else if !fb.is_valid {
        Some("Formula cannot be empty or invalid.")
    } else {
        None
    };

    let te_id = Id::unique("formula_bar.input");
    let was_focused = ui.ctx().memory(|m| m.focused() == Some(te_id));
    let enter_pressed = was_focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
    let esc_pressed = (was_focused || app.focus_widget_id.as_deref() == Some("formula_bar.input"))
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let tab_pressed = was_focused && !fb.completions.is_empty() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Tab));

    egui::Frame::NONE.fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.border)).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |bar| {
        bar.horizontal(|row| {
            let target_lbl = if fb.target_title.trim().is_empty() { "fx" } else { fb.target_title.as_str() };
            row.label(RichText::new(format!("fx ({target_lbl}) =")).small().strong().color(tokens.text_strong));

            let mut draft = fb.input.clone();
            let edit_w = (row.available_width() - 150.0).max(120.0);
            let mut edit =
                egui::TextEdit::singleline(&mut draft).id(te_id).desired_width(edit_w).hint_text("= [Variable A] + [Variable B]").lock_focus(true);
            if !input_enabled {
                edit = edit.interactive(false);
            }
            let edit_resp = row.add(edit);
            if (!views.center_stage.last_formula_bar_active || app.focus_widget_id.as_deref() == Some("formula_bar.input"))
                && input_enabled
                && row.ctx().memory(|m| m.focused().is_none())
            {
                edit_resp.request_focus();
            }
            meta.record_response(
                row,
                &edit_resp,
                FrameInspectEntryInput {
                    widget_id: "formula_bar.input",
                    role: "textbox",
                    label: "Formula bar",
                    command_id: None,
                    value: Some(json!({
                        "active": fb.active,
                        "target_column": fb.target_column,
                        "target_title": fb.target_title,
                        "input": draft,
                        "is_valid": fb.is_valid,
                        "parse_error": fb.parse_error,
                    })),
                    enabled: input_enabled,
                    disabled_reason: if !enabled {
                        Some("modal dialog is active")
                    } else if ds_frozen {
                        Some("Data sheet is frozen (read-only).")
                    } else {
                        None
                    },
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if draft != fb.input && input_enabled {
                intents.push(UiIntent::SetFormulaBarInput { text: draft });
            }
            if enter_pressed && input_enabled {
                intents.push(UiIntent::CommitFormulaBar);
            } else if esc_pressed && input_enabled {
                intents.push(UiIntent::CancelFormulaBar);
            } else if tab_pressed && input_enabled {
                intents.push(UiIntent::AcceptFormulaBarCompletion);
            }

            let apply_resp = row.add_enabled(commit_enabled, egui::Button::new(RichText::new("Apply").small()));
            meta.record_response(
                row,
                &apply_resp,
                FrameInspectEntryInput {
                    widget_id: "formula_bar.commit",
                    role: "button",
                    label: "Apply formula",
                    command_id: Some("table.setCalculatedVariable"),
                    value: Some(json!({
                        "target_column": fb.target_column,
                        "is_valid": fb.is_valid,
                        "parse_error": fb.parse_error,
                    })),
                    enabled: commit_enabled,
                    disabled_reason: commit_reason,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if apply_resp.clicked() && commit_enabled {
                intents.push(UiIntent::CommitFormulaBar);
            }

            let cancel_resp = row.add_enabled(enabled, egui::Button::new(RichText::new("Cancel").small()));
            meta.record_response(
                row,
                &cancel_resp,
                FrameInspectEntryInput {
                    widget_id: "formula_bar.cancel",
                    role: "button",
                    label: "Cancel formula",
                    command_id: None,
                    value: Some(json!({ "active": fb.active })),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if cancel_resp.clicked() && enabled {
                intents.push(UiIntent::CancelFormulaBar);
            }
        });
    });

    views.center_stage.last_formula_bar_active = true;
}

fn sync_two_way_split_brushing(
    app: &UiApp,
    views: &mut EguiShellViewState,
    primary_id: Option<&SheetId>,
    secondary_id: Option<&SheetId>,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let grid_sheet_id = doc
        .active_sheet_id
        .as_ref()
        .filter(|id| doc.data_sheet(id).is_some())
        .cloned()
        .or_else(|| [primary_id, secondary_id].into_iter().flatten().find(|id| doc.data_sheet(id).is_some()).cloned());
    let Some(grid_id) = grid_sheet_id else {
        return;
    };

    let mut has_brush_intent = false;
    let mut selected_coord: Option<(Option<SheetId>, GridCellCoord)> = None;
    for intent in intents.intents() {
        match intent {
            UiIntent::BrushPoint { .. } | UiIntent::ClearBrush => has_brush_intent = true,
            UiIntent::SelectGridCell { sheet_id, coord } | UiIntent::ExtendGridSelection { sheet_id, coord } => {
                selected_coord = Some((sheet_id.clone(), *coord));
            }
            _ => {}
        }
    }

    if let Some((sel_sheet_id, coord)) = selected_coord {
        let brush_sheet = sel_sheet_id.filter(|id| doc.data_sheet(id).is_some()).unwrap_or(grid_id);
        views.center_stage.last_synced_grid_coord = Some(coord);
        if !has_brush_intent {
            intents.push(UiIntent::BrushPoint { sheet_id: brush_sheet, coord });
        }
        return;
    }

    if !has_brush_intent
        && doc.active_sheet_id.as_ref() == Some(&grid_id)
        && app.grid.selection_belongs_to(&grid_id)
        && app.grid.tool.active != views.center_stage.last_synced_grid_coord
    {
        views.center_stage.last_synced_grid_coord = app.grid.tool.active;
        if let Some(coord) = app.grid.tool.active {
            intents.push(UiIntent::BrushPoint { sheet_id: grid_id, coord });
        }
    }
}

fn pick_fallback_secondary_sheet(doc: &ProjectDocument, primary_id: Option<&SheetId>) -> Option<SheetId> {
    let disambig = BTreeMap::new();
    let all = collect_all_sheet_ids(doc, &disambig);
    all.into_iter().map(|(id, _, _)| id).find(|id| Some(id) != primary_id)
}

fn format_split_selector_label(id: &SheetId, title: &str, disambig: &BTreeMap<SheetId, crate::workspace_nav::SheetDisambiguation>) -> String {
    match disambig.get(id) {
        Some(info) if info.has_duplicate_title => match info.subtitle.as_deref() {
            Some(sub) => format!("{title} \u{2014} {sub}"),
            None => title.to_owned(),
        },
        _ => title.to_owned(),
    }
}

fn collect_all_sheet_ids(
    doc: &ProjectDocument,
    disambig: &BTreeMap<SheetId, crate::workspace_nav::SheetDisambiguation>,
) -> Vec<(SheetId, String, &'static str)> {
    let mut out = Vec::with_capacity(doc.total_sheet_count());
    for s in &doc.data_sheets {
        out.push((s.id.clone(), format_split_selector_label(&s.id, &s.title, disambig), "Data"));
    }
    for s in &doc.info_sheets {
        out.push((s.id.clone(), format_split_selector_label(&s.id, &s.title, disambig), "Info"));
    }
    for s in &doc.result_sheets {
        out.push((s.id.clone(), format_split_selector_label(&s.id, s.title(), disambig), "Result"));
    }
    for s in &doc.graph_sheets {
        out.push((s.id.clone(), format_split_selector_label(&s.id, &s.title, disambig), "Graph"));
    }
    for s in &doc.layout_sheets {
        out.push((s.id.clone(), format_split_selector_label(&s.id, &s.title, disambig), "Layout"));
    }
    out
}

#[derive(Debug, Clone)]
enum BottomBannerKind {
    ResultDiagnostic { sheet_id: SheetId, banner: AssumptionDiagnosticBanner },
    GraphClipping { sheet_id: SheetId, count: usize, details: Vec<String> },
    DataLinkedRegion { sheet_id: SheetId, region_count: usize, upstream_id: Option<SheetId> },
}

fn detect_bottom_banner(ctx: &egui::Context, app: &UiApp, views: &EguiShellViewState, active_sheet_id: Option<&SheetId>) -> Option<BottomBannerKind> {
    let sheet_id = active_sheet_id?;
    if views.center_stage.dismissed_banner_sheets.contains(sheet_id) || app.egui_views.center_stage.dismissed_banner_sheets.contains(sheet_id) {
        return None;
    }
    let doc = app.document();

    if let Some(rs) = doc.result_sheet(sheet_id)
        && let Some(banner) = AssumptionDiagnosticBanner::from_result_localized(rs, app.locale.as_str())
    {
        return Some(BottomBannerKind::ResultDiagnostic { sheet_id: sheet_id.clone(), banner });
    }

    if let Some(gs) = doc.graph_sheet(sheet_id) {
        let (count, details) = collect_graph_clipping_warnings(ctx, app, gs);
        if count > 0 {
            return Some(BottomBannerKind::GraphClipping { sheet_id: sheet_id.clone(), count, details });
        }
    }

    if let Some(ds) = doc.data_sheet(sheet_id)
        && !ds.linked_regions.is_empty()
    {
        let upstream = ds.linked_regions.iter().find_map(|r| r.spec.source.upstream_sheet_id().cloned());
        return Some(BottomBannerKind::DataLinkedRegion { sheet_id: sheet_id.clone(), region_count: ds.linked_regions.len(), upstream_id: upstream });
    }

    None
}

fn collect_graph_clipping_warnings(ctx: &egui::Context, app: &UiApp, gs: &GraphSheet) -> (usize, Vec<String>) {
    let mut details = Vec::new();
    if let Some(scene) = super::canvas_view::resolve_cached_scene(ctx, app, &gs.id, "canvas") {
        for w in &scene.warnings {
            details.push(format!("{}: {}", w.code, w.message));
        }
        for pf in preflight_check_scene(&scene) {
            if !pf.severity.eq_ignore_ascii_case("info") {
                details.push(format!("{}: {}", pf.code, pf.message));
            }
        }
    }
    if details.is_empty() && gs.spec.x_axis.segments.iter().any(|s| s.min.is_some() && s.max.is_some() && s.min >= s.max) {
        details.push(String::from("Invalid axis segment bounds clip plotted data."));
    }
    (details.len(), details)
}

struct BottomBannerSpec<'a> {
    kind: &'a BottomBannerKind,
    rect: Rect,
    tokens: &'a Tokens,
    enabled: bool,
}

fn render_bottom_banner(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    spec: BottomBannerSpec<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let BottomBannerSpec { kind, rect, tokens, enabled } = spec;
    let locale = app.locale.as_str();
    let warn_bg = rgba8_to_color32(curvebook_palette::lerp_oklab(color32_to_rgba8(tokens.surface_bg), color32_to_rgba8(tokens.warning), 0.14));
    let (bg, border, text_color) = match kind {
        BottomBannerKind::ResultDiagnostic { banner, .. } => {
            let warn = matches!(banner.severity, curvebook_doc::ChecklistStatus::Violated | curvebook_doc::ChecklistStatus::Warn);
            if warn { (warn_bg, tokens.warning, tokens.text_primary) } else { (tokens.active_bg, tokens.accent, tokens.text_primary) }
        }
        BottomBannerKind::GraphClipping { .. } => (warn_bg, tokens.warning, tokens.text_primary),
        BottomBannerKind::DataLinkedRegion { .. } => (tokens.active_bg, tokens.accent, tokens.text_primary),
    };

    ui.painter().rect(rect, CornerRadius::ZERO, bg, Stroke::new(1.0, border), StrokeKind::Inside);

    let (sheet_id, wid_prefix, summary, why_text): (&SheetId, &str, String, Option<String>) = match kind {
        BottomBannerKind::ResultDiagnostic { sheet_id, banner } => {
            let why = if banner.limitation_note.is_empty() { banner.diagnostic_metric.clone() } else { Some(banner.limitation_note.clone()) };
            (sheet_id, "result.diagnostic_banner", banner.summary.clone(), why)
        }
        BottomBannerKind::GraphClipping { sheet_id, count, details } => {
            (sheet_id, "canvas.clipping_banner", format!("{count} chart clipping / preflight notice(s)"), details.first().cloned())
        }
        BottomBannerKind::DataLinkedRegion { sheet_id, region_count, .. } => (
            sheet_id,
            "canvas.linked_banner",
            format!("{region_count} live linked region(s) — read-only"),
            Some(String::from("Cells in a linked region mirror an upstream sheet or external link. Unlink or navigate to source to edit.")),
        ),
    };

    let inner = rect.shrink2(vec2(8.0, 4.0));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bar_ui.set_clip_rect(rect);

    let is_nl_result = match kind {
        BottomBannerKind::ResultDiagnostic { sheet_id, .. } => {
            app.document().result_sheet(sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, curvebook_doc::AnalysisKind::NonlinearRegression(_)))
        }
        _ => false,
    };
    let text_reserve = if is_nl_result { 410.0 } else { 250.0 };
    let text_budget = (inner.width() - text_reserve).max(90.0);
    let lbl_resp = bar_ui
        .add_sized([text_budget, inner.height()], egui::Label::new(RichText::new(&summary).small().color(text_color)).truncate())
        .on_hover_text(&summary);
    let banner_cmd = (wid_prefix == "result.diagnostic_banner").then_some("ui.result.inspectDiagnostics");
    meta.record_response(&bar_ui, &lbl_resp, inspect_input(wid_prefix, "status", &summary, banner_cmd, None, enabled, ZLayer::FloatingOverlay));

    let btn_min_h = 22.0_f32;
    bar_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btns| {
        let dismiss_lbl = if is_nl_result {
            tr_or_fallback(locale, "results.banner.dismiss", "Dismiss")
        } else {
            tr_or_fallback(locale, "w5.banner.dismiss", "Dismiss")
        };
        let dismiss_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new(dismiss_lbl).small()).min_size(vec2(54.0_f32, btn_min_h)));
        let dismiss_wid = format!("{wid_prefix}.dismiss");
        meta.record_response(btns, &dismiss_resp, inspect_input(&dismiss_wid, "button", dismiss_lbl, None, None, enabled, ZLayer::FloatingOverlay));
        if dismiss_resp.clicked() && enabled {
            if views.center_stage.banner_details_open.as_ref() == Some(sheet_id) {
                views.center_stage.banner_details_open = None;
            }
            if views.results_view.compare_horizontal_open_sheet.as_ref() == Some(sheet_id) {
                views.results_view.compare_horizontal_open_sheet = None;
            }
            if !views.center_stage.dismissed_banner_sheets.contains(sheet_id) {
                views.center_stage.dismissed_banner_sheets.push(sheet_id.clone());
            }
            intents.push(UiIntent::DismissDiagnosticBanner { sheet_id: sheet_id.clone() });
        }

        if why_text.is_some() {
            let why_open = views.center_stage.banner_details_open.as_ref() == Some(sheet_id);
            let why_lbl = tr_or_fallback(locale, "results.banner.why", "Why?");
            let why_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new(why_lbl).small()).min_size(vec2(42.0_f32, btn_min_h)));
            let why_wid = format!("{wid_prefix}.why");
            meta.record_response(btns, &why_resp, inspect_input(&why_wid, "button", why_lbl, None, None, enabled, ZLayer::FloatingOverlay));
            if why_resp.clicked() && enabled {
                if views.results_view.compare_horizontal_open_sheet.as_ref() == Some(sheet_id) {
                    views.results_view.compare_horizontal_open_sheet = None;
                }
                views.center_stage.banner_details_open = if why_open { None } else { Some(sheet_id.clone()) };
            }
        }

        match kind {
            BottomBannerKind::ResultDiagnostic { sheet_id, banner } => {
                if is_nl_result {
                    let cmp_lbl = tr_or_fallback(locale, "results.banner.compare_horizontal", "Compare vs. Horizontal Line");
                    let cmp_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new(cmp_lbl).small()).min_size(vec2(148.0_f32, btn_min_h)));
                    meta.record_response(
                        btns,
                        &cmp_resp,
                        inspect_input("result.diagnostic_banner.compare_horizontal", "button", cmp_lbl, None, None, enabled, ZLayer::FloatingOverlay),
                    );
                    if cmp_resp.clicked() && enabled {
                        if views.center_stage.banner_details_open.as_ref() == Some(sheet_id) {
                            views.center_stage.banner_details_open = None;
                        }
                        trigger_horizontal_line_comparison(app.document(), app.mutation_seq(), sheet_id, &mut views.results_view, locale);
                    }

                    let constraints_lbl = tr_or_fallback(locale, "results.banner.constraints", "Constraints");
                    let constraints_resp =
                        btns.add_enabled(enabled, egui::Button::new(RichText::new(constraints_lbl).small()).min_size(vec2(74.0_f32, btn_min_h)));
                    meta.record_ranked_response(
                        btns,
                        &constraints_resp,
                        inspect_input(
                            "result.diagnostic_banner.constraints",
                            "button",
                            constraints_lbl,
                            Some("ui.editParams.open"),
                            None,
                            enabled,
                            ZLayer::FloatingOverlay,
                        ),
                        crate::menus::ActionRank::Secondary,
                    );
                    if constraints_resp.clicked() && enabled {
                        intents.push_ui("ui.editParams.open", json!({ "sheet_id": sheet_id.as_str() }));
                    }
                } else {
                    let insp_lbl = tr_or_fallback(locale, "w5.banner.inspect", "Inspect Details");
                    let insp_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new(insp_lbl).small()).min_size(vec2(86.0_f32, btn_min_h)));
                    meta.record_response(
                        btns,
                        &insp_resp,
                        inspect_input(
                            "result.diagnostic_banner.inspect",
                            "button",
                            insp_lbl,
                            Some("ui.result.inspectDiagnostics"),
                            None,
                            enabled,
                            ZLayer::FloatingOverlay,
                        ),
                    );
                    if insp_resp.clicked() && enabled {
                        let mut params = json!({ "result_sheet_id": sheet_id.as_str() });
                        if let Some(tab_id) = banner.detail_tab_ids.first() {
                            params["tab_id"] = json!(tab_id);
                        }
                        intents.push_ui("ui.result.inspectDiagnostics", params);
                    }
                }
            }
            BottomBannerKind::GraphClipping { sheet_id, .. } => {
                let fit_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new("Fit Range").small()).min_size(vec2(66.0, 22.0)));
                meta.record_response(
                    btns,
                    &fit_resp,
                    inspect_input(
                        "canvas.clipping_banner.fit_range",
                        "button",
                        "Fit Range",
                        Some("graph.setAxes"),
                        None,
                        enabled,
                        ZLayer::FloatingOverlay,
                    ),
                );
                if fit_resp.clicked() && enabled {
                    intents.push_engine(
                        "graph.setAxes",
                        json!({
                            "sheet_id": sheet_id.as_str(),
                            "x_axis": { "min": null, "max": null },
                            "y_axis": { "min": null, "max": null },
                        }),
                    );
                }
            }
            BottomBannerKind::DataLinkedRegion { sheet_id, upstream_id, .. } => {
                let unlink_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new("Unlink to Edit").small()).min_size(vec2(86.0, 22.0)));
                meta.record_response(
                    btns,
                    &unlink_resp,
                    inspect_input(
                        "canvas.linked_banner.unlink",
                        "button",
                        "Unlink to Edit",
                        Some("table.clearCells"),
                        None,
                        enabled,
                        ZLayer::FloatingOverlay,
                    ),
                );
                if unlink_resp.clicked() && enabled {
                    intents.push_engine("table.clearCells", json!({ "sheet_id": sheet_id.as_str(), "cells": [], "unlink": true }));
                }

                if let Some(up_id) = upstream_id {
                    let goto_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new("Go to Source").small()).min_size(vec2(82.0, 22.0)));
                    meta.record_response(
                        btns,
                        &goto_resp,
                        inspect_input(
                            "canvas.linked_banner.goto_source",
                            "button",
                            "Go to Source",
                            Some("sheet.select"),
                            None,
                            enabled,
                            ZLayer::FloatingOverlay,
                        ),
                    );
                    if goto_resp.clicked() && enabled {
                        intents.push_engine("sheet.select", json!({ "sheet_id": up_id.as_str() }));
                    }
                }
            }
        }
    });

    if views.center_stage.banner_details_open.as_ref() == Some(sheet_id)
        && let Some(detail) = why_text
    {
        let pop_rect = Rect::from_min_max(
            pos2(rect.left() + 8.0, (rect.top() - 48.0).max(0.0)),
            pos2((rect.right() - 8.0).max(rect.left() + 180.0), rect.top() - 2.0),
        );
        if enabled {
            egui::Area::new(Id::unique(("center_stage_banner_why", sheet_id.as_str()))).order(Order::Middle).fixed_pos(pop_rect.min).show(
                ui.ctx(),
                |pop_ui| {
                    egui::Frame::popup(pop_ui.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.0, border)).show(pop_ui, |inner_ui| {
                        inner_ui.set_max_width(pop_rect.width());
                        inner_ui.label(RichText::new(detail).small().color(tokens.text_primary));
                    });
                },
            );
        }
    }
    if is_nl_result {
        render_horizontal_line_comparison_popup(ui, tokens, rect, true, sheet_id, &mut views.results_view, enabled, meta);
    }
}

struct VisibleFloatingNote<'a> {
    sheet_id: SheetId,
    note: &'a FloatingNote,
    pinned: bool,
}

fn collect_visible_floating_notes<'a>(
    doc: &'a curvebook_doc::ProjectDocument,
    active_sheet_id: Option<&SheetId>,
    note_pinned: &BTreeMap<(SheetId, String), bool>,
) -> Vec<VisibleFloatingNote<'a>> {
    let mut visible = Vec::new();
    if let Some(active_id) = active_sheet_id {
        for note in active_sheet_floating_notes(doc, active_id) {
            let key = (active_id.clone(), note.id.as_str().to_owned());
            let pinned = note_pinned.get(&key).copied().unwrap_or(false);
            visible.push(VisibleFloatingNote { sheet_id: active_id.clone(), note, pinned });
        }
    }
    let all_sheets = doc
        .data_sheets
        .iter()
        .map(|s| (&s.id, s.floating_notes.as_slice()))
        .chain(doc.info_sheets.iter().map(|s| (&s.id, s.floating_notes.as_slice())))
        .chain(doc.result_sheets.iter().map(|s| (&s.id, s.floating_notes.as_slice())))
        .chain(doc.graph_sheets.iter().map(|s| (&s.id, s.floating_notes.as_slice())))
        .chain(doc.layout_sheets.iter().map(|s| (&s.id, s.floating_notes.as_slice())));
    for (owner_id, notes) in all_sheets {
        if active_sheet_id == Some(owner_id) {
            continue;
        }
        for note in notes {
            let key = (owner_id.clone(), note.id.as_str().to_owned());
            if note_pinned.get(&key).copied().unwrap_or(false) {
                visible.push(VisibleFloatingNote { sheet_id: owner_id.clone(), note, pinned: true });
            }
        }
    }
    // Render unpinned notes first so pinned notes stack cleanly above them inside Order::Middle.
    visible.sort_by_key(|item| item.pinned);
    visible
}

struct FloatingNotesSpec<'a> {
    notes: Vec<VisibleFloatingNote<'a>>,
    stage_bounds: Rect,
    tokens: &'a Tokens,
    enabled: bool,
    notes_pane_focused: bool,
}

fn next_floating_note_color(current: curvebook_palette::Rgba8) -> &'static str {
    let rgb = (current.r, current.g, current.b);
    if current.a == 0 || rgb == (FloatingNote::DEFAULT_FILL.r, FloatingNote::DEFAULT_FILL.g, FloatingNote::DEFAULT_FILL.b) {
        "blue"
    } else if rgb == (FloatingNote::BLUE_FILL.r, FloatingNote::BLUE_FILL.g, FloatingNote::BLUE_FILL.b) {
        "green"
    } else if rgb == (FloatingNote::GREEN_FILL.r, FloatingNote::GREEN_FILL.g, FloatingNote::GREEN_FILL.b) {
        "red"
    } else if rgb == (FloatingNote::RED_FILL.r, FloatingNote::RED_FILL.g, FloatingNote::RED_FILL.b) {
        "gray"
    } else {
        "yellow"
    }
}

const fn floating_note_preset_rgba(color_name: &str) -> curvebook_palette::Rgba8 {
    if color_name.eq_ignore_ascii_case("blue") {
        FloatingNote::BLUE_FILL
    } else if color_name.eq_ignore_ascii_case("green") {
        FloatingNote::GREEN_FILL
    } else if color_name.eq_ignore_ascii_case("red") {
        FloatingNote::RED_FILL
    } else if color_name.eq_ignore_ascii_case("gray") {
        FloatingNote::GRAY_FILL
    } else {
        FloatingNote::DEFAULT_FILL
    }
}

fn attach_note_button_tooltip(resp: &egui::Response, tokens: &Tokens, title: &str, enabled: bool, disabled_reason: Option<&str>) {
    let mut spec = super::ui_kit::TooltipSpec::new(title);
    if !enabled && let Some(reason) = disabled_reason {
        spec = spec.with_disabled_reason(reason);
    }
    let _ = super::ui_kit::show_tooltip(resp, tokens, spec);
}

#[allow(clippy::too_many_arguments)]
fn render_floating_note_inline_editor(
    area_ui: &mut egui::Ui,
    views: &mut EguiShellViewState,
    sheet_id: &SheetId,
    note_id: &str,
    body_rect: Rect,
    locale: &str,
    tokens: &Tokens,
    enabled: bool,
    can_mutate: bool,
    mutate_disabled_reason: Option<&str>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let edit_label = tr_or_fallback(locale, "floating_note.edit", "Edit Note");
    let save_label = tr_or_fallback(locale, "floating_note.save", "Save");
    let cancel_label = tr_or_fallback(locale, "floating_note.cancel", "Cancel");

    let actions_h = tokens.metrics.min_hit_target_pt;
    let gap = 4.0_f32;
    let actions_rect = Rect::from_min_max(pos2(body_rect.left(), (body_rect.bottom() - actions_h).max(body_rect.top())), body_rect.max);
    let editor_rect = Rect::from_min_max(body_rect.min, pos2(body_rect.right(), (actions_rect.top() - gap).max(body_rect.top() + 24.0)));

    let editor_id = Id::unique(("floating_note_editor", sheet_id.as_str(), note_id));
    let editor_focused = area_ui.ctx().memory(|m| m.focused() == Some(editor_id));
    let save_shortcut = can_mutate
        && editor_focused
        && area_ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Enter) || i.consume_key(egui::Modifiers::CTRL, Key::Enter));
    let cancel_shortcut = enabled && area_ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));

    let mut draft_text = views
        .center_stage
        .editing_floating_note
        .as_ref()
        .filter(|e| &e.sheet_id == sheet_id && e.note_id == note_id)
        .map(|e| e.draft_text.clone())
        .unwrap_or_default();

    let mut editor_ui = area_ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("floating_note_editor_wrap", sheet_id.as_str(), note_id))
            .max_rect(editor_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    editor_ui.set_clip_rect(editor_rect);
    let scroll_out = egui::ScrollArea::vertical()
        .id_salt(("floating_note_editor_scroll", sheet_id.as_str(), note_id))
        .max_height(editor_rect.height())
        .auto_shrink([false, false])
        .show(&mut editor_ui, |scroll_ui| {
            let te_w = (editor_rect.width() - 12.0).max(40.0);
            let te_h = (editor_rect.height() - 4.0).max(28.0);
            let mut te = egui::TextEdit::multiline(&mut draft_text).id(editor_id).desired_width(te_w).desired_rows(3).lock_focus(true);
            if !can_mutate {
                te = te.interactive(false);
            }
            scroll_ui.add_sized(vec2(te_w, te_h), te)
        });
    let editor_resp = scroll_out.inner;
    let editor_ak = edit_label.to_owned();
    editor_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, can_mutate, editor_ak.clone()));
    if views.center_stage.pending_floating_note_edit_focus.as_ref().is_some_and(|(sid, nid)| sid == sheet_id && nid == note_id) {
        if can_mutate {
            editor_resp.request_focus();
        }
        views.center_stage.pending_floating_note_edit_focus = None;
    }
    let editor_wid = format!("floating_note.{}.{}.editor", sheet_id.as_str(), note_id);
    meta.record_response(
        &editor_ui,
        &editor_resp,
        FrameInspectEntryInput {
            widget_id: &editor_wid,
            role: "textbox",
            label: edit_label,
            command_id: None,
            value: Some(json!(draft_text)),
            enabled: can_mutate,
            disabled_reason: mutate_disabled_reason,
            z_layer: ZLayer::FloatingOverlay,
            ellipsized: false,
            full_label: None,
        },
    );

    let mut actions_ui = area_ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("floating_note_actions", sheet_id.as_str(), note_id))
            .max_rect(actions_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    actions_ui.set_clip_rect(actions_rect);
    actions_ui.spacing_mut().item_spacing.x = tokens.metrics.space_sm;

    let save_wid = format!("floating_note.{}.{}.save", sheet_id.as_str(), note_id);
    let (_, save_clicked) = super::ui_kit::styled_button(
        &mut actions_ui,
        tokens,
        meta,
        super::ui_kit::StyledButtonSpec {
            widget_id: &save_wid,
            role: "button",
            a11y_role: egui::Role::Button,
            a11y_selectable: false,
            label: save_label,
            tooltip: None,
            command_id: Some("sheet.updateFloatingNote"),
            width: super::ui_kit::ButtonWidth::FitText { min_w: 52.0, max_w: 98.0, pad_x: 12.0 },
            height: tokens.metrics.min_hit_target_pt,
            font_size: tokens.metrics.font_ui_small_pt,
            radius: tokens.metrics.radius_sm,
            text_align: super::ui_kit::ButtonTextAlign::Center,
            break_anywhere: false,
            primary: true,
            selected: false,
            enabled: can_mutate,
            accent_text_when_selected: false,
            border_override: None,
            value: None,
            disabled_reason: mutate_disabled_reason,
            z_layer: ZLayer::FloatingOverlay,
            skip_offscreen: false,
            clip_interact_to_viewport: false,
            tooltip_mode: super::ui_kit::TooltipMode::Standard,
        },
    );

    let cancel_wid = format!("floating_note.{}.{}.cancel", sheet_id.as_str(), note_id);
    let (_, cancel_clicked) = super::ui_kit::styled_button(
        &mut actions_ui,
        tokens,
        meta,
        super::ui_kit::StyledButtonSpec {
            widget_id: &cancel_wid,
            role: "button",
            a11y_role: egui::Role::Button,
            a11y_selectable: false,
            label: cancel_label,
            tooltip: None,
            command_id: None,
            width: super::ui_kit::ButtonWidth::FitText { min_w: 56.0, max_w: 98.0, pad_x: 12.0 },
            height: tokens.metrics.min_hit_target_pt,
            font_size: tokens.metrics.font_ui_small_pt,
            radius: tokens.metrics.radius_sm,
            text_align: super::ui_kit::ButtonTextAlign::Center,
            break_anywhere: false,
            primary: false,
            selected: false,
            enabled,
            accent_text_when_selected: false,
            border_override: None,
            value: None,
            disabled_reason: (!enabled).then_some("Blocked by modal"),
            z_layer: ZLayer::FloatingOverlay,
            skip_offscreen: false,
            clip_interact_to_viewport: false,
            tooltip_mode: super::ui_kit::TooltipMode::Standard,
        },
    );

    if (save_clicked || save_shortcut) && can_mutate {
        intents.push_engine(
            "sheet.updateFloatingNote",
            json!({
                "sheet_id": sheet_id.as_str(),
                "note_id": note_id,
                "body_markdown": draft_text,
            }),
        );
        views.center_stage.editing_floating_note = None;
        views.center_stage.pending_floating_note_edit_focus = None;
        area_ui.ctx().request_repaint();
    } else if (cancel_clicked || cancel_shortcut) && enabled {
        views.center_stage.editing_floating_note = None;
        views.center_stage.pending_floating_note_edit_focus = None;
        area_ui.ctx().request_repaint();
    } else if let Some(edit) = views.center_stage.editing_floating_note.as_mut()
        && &edit.sheet_id == sheet_id
        && edit.note_id == note_id
    {
        edit.draft_text = draft_text;
    }
}

struct FloatingNoteCardCtx<'a> {
    doc: &'a ProjectDocument,
    sheet_id: &'a SheetId,
    note: &'a FloatingNote,
    note_id: &'a str,
    idx: usize,
    key: &'a (SheetId, String),
    sheet_frozen: bool,
    locale: &'a str,
    tokens: &'a Tokens,
    enabled: bool,
    can_mutate: bool,
    mutate_disabled_reason: Option<&'a str>,
    is_editing: bool,
    collapsed: bool,
    pinned: bool,
    pos: egui::Pos2,
    card_w: f32,
    card_h: f32,
    expanded_h: f32,
    min_h: f32,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    stage_bounds: Rect,
    note_ink: Color32,
}

fn render_floating_note_drag_header(
    area_ui: &mut egui::Ui,
    views: &mut EguiShellViewState,
    cx: &FloatingNoteCardCtx<'_>,
    drag_rect: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let drag_resp = area_ui.interact(
        drag_rect,
        Id::unique(("floating_note_drag", cx.sheet_id.as_str(), cx.note_id)),
        if cx.enabled { Sense::click_and_drag() } else { Sense::hover() },
    );
    let enabled = cx.enabled;
    drag_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Move Floating Note"));
    if drag_resp.clicked() && cx.enabled {
        views.center_stage.focused_floating_note_id = Some(cx.note_id.to_owned());
        intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::FloatingNotes });
    }
    if drag_resp.drag_started() && cx.enabled {
        views.center_stage.active_floating_note_drag = Some((cx.sheet_id.clone(), cx.idx));
    }
    if drag_resp.dragged() && cx.enabled {
        let delta = drag_resp.drag_delta();
        let next_x = (cx.pos.x + delta.x).clamp(cx.min_x, cx.max_x) - cx.stage_bounds.left();
        let next_y = (cx.pos.y + delta.y).clamp(cx.min_y, cx.max_y) - cx.stage_bounds.top();
        views.center_stage.note_positions_pt.insert(cx.key.clone(), [next_x, next_y]);
    }
    if drag_resp.drag_stopped() {
        views.center_stage.active_floating_note_drag = None;
        if cx.enabled
            && let Some([nx, ny]) = views.center_stage.note_positions_pt.get(cx.key).copied()
        {
            if cx.sheet_frozen {
                intents.push(UiIntent::SetFloatingNotePosition {
                    sheet_id: cx.sheet_id.clone(),
                    note_id: cx.note_id.to_owned(),
                    pos_pt: [f64::from(nx), f64::from(ny)],
                });
            } else {
                intents.push_engine(
                    "sheet.updateFloatingNote",
                    json!({
                        "sheet_id": cx.sheet_id.as_str(),
                        "note_id": cx.note_id,
                        "bounds_pt": {
                            "x": f64::from(nx),
                            "y": f64::from(ny),
                            "width": f64::from(cx.card_w),
                            "height": f64::from(cx.expanded_h),
                        },
                    }),
                );
            }
        }
    }

    let title_text = format!("Note · {}", cx.note_id);
    let title_max_w = (drag_rect.width() - cx.tokens.metrics.space_sm - 2.0).max(12.0);
    let (title_galley, _) = super::ui_kit::elided_galley(
        area_ui.painter(),
        &title_text,
        FontId::proportional(cx.tokens.metrics.font_ui_small_pt),
        cx.note_ink,
        title_max_w,
        false,
    );
    let title_pos = pos2(drag_rect.left() + cx.tokens.metrics.space_sm, drag_rect.center().y - title_galley.size().y * 0.5);
    area_ui.painter().with_clip_rect(drag_rect).galley(title_pos, title_galley, cx.note_ink);

    let drag_wid = format!("floating_note.{}.{}.drag_handle", cx.sheet_id.as_str(), cx.note_id);
    meta.record_response(
        area_ui,
        &drag_resp,
        inspect_input(&drag_wid, "button", "Move Floating Note", None, None, cx.enabled, ZLayer::FloatingOverlay),
    );
}

struct NoteHeaderBtnSpec<'a> {
    salt: &'static str,
    rect: Rect,
    widget_suffix: &'static str,
    label: &'a str,
    glyph: Option<&'a str>,
    swatch_fill: Option<Color32>,
    command_id: Option<&'a str>,
    value: Option<serde_json::Value>,
    active: bool,
    enabled: bool,
    disabled_reason: Option<&'a str>,
    ink: Color32,
}

fn render_note_header_button(area_ui: &mut egui::Ui, cx: &FloatingNoteCardCtx<'_>, spec: NoteHeaderBtnSpec<'_>, meta: &mut FrameInspectMeta) -> bool {
    let resp = area_ui.interact(
        spec.rect,
        Id::unique((spec.salt, cx.sheet_id.as_str(), cx.note_id)),
        if spec.enabled { Sense::click() } else { Sense::hover() },
    );
    let ak_label = spec.label.to_owned();
    let enabled = spec.enabled;
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_label.clone()));

    let bg = if spec.active {
        cx.tokens.active_bg
    } else if resp.hovered() && spec.enabled {
        cx.tokens.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        area_ui.painter().rect_filled(spec.rect, CornerRadius::same(cx.tokens.metrics.radius_sm), bg);
    }
    if let Some(swatch) = spec.swatch_fill {
        let swatch_rect = Rect::from_center_size(spec.rect.center(), Vec2::splat(10.0));
        area_ui.painter().rect(swatch_rect, CornerRadius::same(2), swatch, Stroke::new(1.2, spec.ink), StrokeKind::Inside);
    } else if let Some(glyph) = spec.glyph {
        area_ui.painter().text(
            spec.rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(cx.tokens.metrics.font_ui_small_pt),
            spec.ink,
        );
    }
    attach_note_button_tooltip(&resp, cx.tokens, spec.label, spec.enabled, spec.disabled_reason);

    let wid = format!("floating_note.{}.{}.{}", cx.sheet_id.as_str(), cx.note_id, spec.widget_suffix);
    meta.record_response(
        area_ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: &wid,
            role: "button",
            label: spec.label,
            command_id: spec.command_id,
            value: spec.value,
            enabled: spec.enabled,
            disabled_reason: (!spec.enabled).then_some(spec.disabled_reason).flatten(),
            z_layer: ZLayer::FloatingOverlay,
            ellipsized: false,
            full_label: None,
        },
    );
    resp.clicked() && spec.enabled
}

fn render_floating_note_header(
    area_ui: &mut egui::Ui,
    views: &mut EguiShellViewState,
    cx: &FloatingNoteCardCtx<'_>,
    hdr_rect: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let btn_side = cx.tokens.metrics.min_hit_target_pt;
    let btn_gap = 2.0_f32;
    let right_pad = 2.0_f32;
    let btn_top = hdr_rect.top() + ((FLOATING_NOTE_HEADER_H_PT - btn_side) * 0.5).max(0.0);

    let delete_rect = Rect::from_min_size(pos2(hdr_rect.right() - right_pad - btn_side, btn_top), Vec2::splat(btn_side));
    let col_rect = Rect::from_min_size(pos2(delete_rect.left() - btn_gap - btn_side, btn_top), Vec2::splat(btn_side));
    let pin_rect = Rect::from_min_size(pos2(col_rect.left() - btn_gap - btn_side, btn_top), Vec2::splat(btn_side));
    let color_rect = Rect::from_min_size(pos2(pin_rect.left() - btn_gap - btn_side, btn_top), Vec2::splat(btn_side));
    let edit_rect = Rect::from_min_size(pos2(color_rect.left() - btn_gap - btn_side, btn_top), Vec2::splat(btn_side));
    let drag_rect = Rect::from_min_max(hdr_rect.min, pos2((edit_rect.left() - btn_gap).max(hdr_rect.left() + 24.0), hdr_rect.bottom()));

    render_floating_note_drag_header(area_ui, views, cx, drag_rect, meta, intents);

    let btn_ink = if cx.can_mutate { cx.note_ink } else { cx.tokens.text_disabled };
    let modal_reason = (!cx.enabled).then_some("Blocked by modal");

    let edit_label = tr_or_fallback(cx.locale, "floating_note.edit", "Edit Note");
    if render_note_header_button(
        area_ui,
        cx,
        NoteHeaderBtnSpec {
            salt: "floating_note_edit",
            rect: edit_rect,
            widget_suffix: "edit",
            label: edit_label,
            glyph: Some("E"),
            swatch_fill: None,
            command_id: Some("sheet.updateFloatingNote"),
            value: Some(json!({ "editing": cx.is_editing })),
            active: cx.is_editing,
            enabled: cx.can_mutate,
            disabled_reason: cx.mutate_disabled_reason,
            ink: btn_ink,
        },
        meta,
    ) {
        views.center_stage.editing_floating_note =
            Some(FloatingNoteEditDraft { sheet_id: cx.sheet_id.clone(), note_id: cx.note_id.to_owned(), draft_text: cx.note.body_markdown.clone() });
        views.center_stage.pending_floating_note_edit_focus = Some(cx.key.clone());
        views.center_stage.note_collapsed.insert(cx.key.clone(), false);
        views.center_stage.focused_floating_note_id = Some(cx.note_id.to_owned());
        area_ui.ctx().request_repaint();
    }

    let color_label = tr_or_fallback(cx.locale, "floating_note.color", "Cycle Note Color");
    let next_color = next_floating_note_color(cx.note.fill_color);
    let swatch_fill = if cx.can_mutate { rgba8_to_color32(floating_note_preset_rgba(next_color)) } else { cx.tokens.panel_bg };
    if render_note_header_button(
        area_ui,
        cx,
        NoteHeaderBtnSpec {
            salt: "floating_note_color",
            rect: color_rect,
            widget_suffix: "color",
            label: color_label,
            glyph: None,
            swatch_fill: Some(swatch_fill),
            command_id: Some("sheet.updateFloatingNote"),
            value: Some(json!({ "next_color": next_color })),
            active: false,
            enabled: cx.can_mutate,
            disabled_reason: cx.mutate_disabled_reason,
            ink: btn_ink,
        },
        meta,
    ) {
        intents.push_engine(
            "sheet.updateFloatingNote",
            json!({
                "sheet_id": cx.sheet_id.as_str(),
                "note_id": cx.note_id,
                "fill_color": next_color,
            }),
        );
    }

    if render_note_header_button(
        area_ui,
        cx,
        NoteHeaderBtnSpec {
            salt: "floating_note_pin",
            rect: pin_rect,
            widget_suffix: "pin",
            label: "Pin Floating Note",
            glyph: Some(if cx.pinned { "P*" } else { "P" }),
            swatch_fill: None,
            command_id: None,
            value: None,
            active: cx.pinned,
            enabled: cx.enabled,
            disabled_reason: modal_reason,
            ink: cx.note_ink,
        },
        meta,
    ) {
        views.center_stage.note_pinned.insert(cx.key.clone(), !cx.pinned);
    }

    let col_label = if cx.collapsed { "Expand Note" } else { "Collapse Note" };
    if render_note_header_button(
        area_ui,
        cx,
        NoteHeaderBtnSpec {
            salt: "floating_note_col",
            rect: col_rect,
            widget_suffix: "collapse",
            label: col_label,
            glyph: Some(if cx.collapsed { "+" } else { "−" }),
            swatch_fill: None,
            command_id: (!cx.sheet_frozen).then_some("sheet.updateFloatingNote"),
            value: None,
            active: false,
            enabled: cx.enabled,
            disabled_reason: modal_reason,
            ink: cx.note_ink,
        },
        meta,
    ) {
        let next_collapsed = !cx.collapsed;
        views.center_stage.note_collapsed.insert(cx.key.clone(), next_collapsed);
        if next_collapsed && cx.is_editing {
            views.center_stage.editing_floating_note = None;
            views.center_stage.pending_floating_note_edit_focus = None;
        }
        if cx.sheet_frozen {
            intents.push(UiIntent::ToggleFloatingNoteCollapsed { sheet_id: cx.sheet_id.clone(), note_id: cx.note_id.to_owned() });
        } else {
            intents.push_engine(
                "sheet.updateFloatingNote",
                json!({
                    "sheet_id": cx.sheet_id.as_str(),
                    "note_id": cx.note_id,
                    "minimized": next_collapsed,
                }),
            );
        }
    }

    let delete_label = tr_or_fallback(cx.locale, "floating_note.delete", "Delete Note");
    if render_note_header_button(
        area_ui,
        cx,
        NoteHeaderBtnSpec {
            salt: "floating_note_delete",
            rect: delete_rect,
            widget_suffix: "delete",
            label: delete_label,
            glyph: Some("×"),
            swatch_fill: None,
            command_id: Some("sheet.deleteFloatingNote"),
            value: None,
            active: false,
            enabled: cx.can_mutate,
            disabled_reason: cx.mutate_disabled_reason,
            ink: btn_ink,
        },
        meta,
    ) {
        if cx.is_editing {
            views.center_stage.editing_floating_note = None;
            views.center_stage.pending_floating_note_edit_focus = None;
        }
        intents.push_engine(
            "sheet.deleteFloatingNote",
            json!({
                "sheet_id": cx.sheet_id.as_str(),
                "note_id": cx.note_id,
            }),
        );
    }
}

fn render_floating_note_body_and_resize(
    area_ui: &mut egui::Ui,
    views: &mut EguiShellViewState,
    cx: &FloatingNoteCardCtx<'_>,
    frame_rect: Rect,
    hdr_rect: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let body_rect = Rect::from_min_max(
        pos2(frame_rect.left() + cx.tokens.metrics.space_md, hdr_rect.bottom() + cx.tokens.metrics.space_xs),
        pos2(frame_rect.right() - cx.tokens.metrics.space_md, frame_rect.bottom() - 14.0),
    );
    if cx.is_editing {
        render_floating_note_inline_editor(
            area_ui,
            views,
            cx.sheet_id,
            cx.note_id,
            body_rect,
            cx.locale,
            cx.tokens,
            cx.enabled,
            cx.can_mutate,
            cx.mutate_disabled_reason,
            meta,
            intents,
        );
    } else {
        let expanded = expand_template_hooks(&cx.note.body_markdown, |tok| resolve_stage_hook(cx.doc, tok));
        let body_galley =
            area_ui.painter().layout(expanded, FontId::proportional(cx.tokens.metrics.font_ui_small_pt), cx.note_ink, body_rect.width().max(40.0));
        area_ui.painter().with_clip_rect(body_rect).galley(body_rect.min, body_galley, cx.note_ink);
    }

    let resize_rect = Rect::from_min_max(pos2(frame_rect.right() - 14.0, frame_rect.bottom() - 14.0), frame_rect.max);
    let resize_resp = area_ui.interact(
        resize_rect,
        Id::unique(("floating_note_resize", cx.sheet_id.as_str(), cx.note_id)),
        if cx.enabled { Sense::drag() } else { Sense::hover() },
    );
    let enabled = cx.enabled;
    resize_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Resize Note"));
    area_ui.painter().line_segment(
        [pos2(resize_rect.left() + 3.0, resize_rect.bottom() - 3.0), pos2(resize_rect.right() - 3.0, resize_rect.top() + 3.0)],
        Stroke::new(1.2, cx.tokens.text_muted),
    );
    if resize_resp.dragged() && cx.enabled {
        let d = resize_resp.drag_delta();
        let avail_max_w = (cx.stage_bounds.right() - cx.pos.x - FLOATING_NOTE_MARGIN_PT).max(FLOATING_NOTE_MIN_W_PT);
        let avail_max_h = (cx.stage_bounds.bottom() - cx.pos.y - FLOATING_NOTE_MARGIN_PT).max(cx.min_h);
        let next_w = (cx.card_w + d.x).clamp(FLOATING_NOTE_MIN_W_PT.min(avail_max_w), avail_max_w);
        let next_h = (cx.card_h + d.y).clamp(cx.min_h.min(avail_max_h), avail_max_h);
        views.center_stage.note_sizes_pt.insert(cx.key.clone(), [next_w, next_h]);
        views.center_stage.active_floating_note_resize = Some((cx.sheet_id.clone(), cx.idx));
    }
    if resize_resp.drag_stopped() {
        views.center_stage.active_floating_note_resize = None;
        if cx.enabled && !cx.sheet_frozen {
            let [nx, ny] = views
                .center_stage
                .note_positions_pt
                .get(cx.key)
                .copied()
                .unwrap_or([cx.pos.x - cx.stage_bounds.left(), cx.pos.y - cx.stage_bounds.top()]);
            let [nw, nh] = views.center_stage.note_sizes_pt.get(cx.key).copied().unwrap_or([cx.card_w, cx.card_h]);
            intents.push_engine(
                "sheet.updateFloatingNote",
                json!({
                    "sheet_id": cx.sheet_id.as_str(),
                    "note_id": cx.note_id,
                    "bounds_pt": {
                        "x": f64::from(nx),
                        "y": f64::from(ny),
                        "width": f64::from(nw),
                        "height": f64::from(nh),
                    },
                }),
            );
        }
    }
    let resize_wid = format!("floating_note.{}.{}.resize_handle", cx.sheet_id.as_str(), cx.note_id);
    meta.record_response(area_ui, &resize_resp, inspect_input(&resize_wid, "button", "Resize Note", None, None, cx.enabled, ZLayer::FloatingOverlay));
}

fn render_floating_notes_overlay(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    spec: FloatingNotesSpec<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let FloatingNotesSpec { notes, stage_bounds, tokens, enabled, notes_pane_focused } = spec;
    if stage_bounds.width() < 180.0 || stage_bounds.height() < 60.0 {
        return;
    }
    let doc = app.document();
    let locale = app.locale.as_str();

    let mutation_seq = app.mutation_seq();
    if let Some(prev_seq) = views.center_stage.last_note_sync_mutation_seq {
        if prev_seq != mutation_seq {
            views.center_stage.last_note_sync_mutation_seq = Some(mutation_seq);
            if views.center_stage.active_floating_note_drag.is_none() {
                views.center_stage.note_positions_pt.retain(|(sid, _), _| doc.data_sheet(sid).is_some_and(|ds| ds.frozen));
            }
            if views.center_stage.active_floating_note_resize.is_none() {
                views.center_stage.note_sizes_pt.retain(|(sid, _), _| doc.data_sheet(sid).is_some_and(|ds| ds.frozen));
            }
            views.center_stage.note_collapsed.retain(|(sid, _), _| doc.data_sheet(sid).is_some_and(|ds| ds.frozen));
        }
    } else {
        views.center_stage.last_note_sync_mutation_seq = Some(mutation_seq);
    }

    if let Some(edit) = views.center_stage.editing_floating_note.as_ref() {
        let edit_sheet_frozen = doc.data_sheet(&edit.sheet_id).is_some_and(|ds| ds.frozen);
        let edit_still_visible = notes.iter().any(|item| item.sheet_id == edit.sheet_id && item.note.id.as_str() == edit.note_id);
        if edit_sheet_frozen || !edit_still_visible {
            views.center_stage.editing_floating_note = None;
            views.center_stage.pending_floating_note_edit_focus = None;
        }
    }

    let first_id = notes.first().map(|n| n.note.id.as_str().to_owned());
    if views.center_stage.focused_floating_note_id.is_none() {
        views.center_stage.focused_floating_note_id = first_id;
    }

    let note_ink = rgba8_to_color32(crate::theme::UiThemeSpec::light().text_primary);
    for (idx, item) in notes.iter().enumerate() {
        let sheet_id = &item.sheet_id;
        let note = item.note;
        let note_id = note.id.as_str().to_owned();
        let key = (sheet_id.clone(), note_id.clone());
        let sheet_frozen = doc.data_sheet(sheet_id).is_some_and(|ds| ds.frozen);
        let can_mutate = enabled && !sheet_frozen;
        let mutate_disabled_reason = if !enabled {
            Some("Blocked by modal")
        } else if sheet_frozen {
            Some("Data sheet is frozen (read-only)")
        } else {
            None
        };

        let is_editing = views.center_stage.editing_floating_note.as_ref().is_some_and(|e| &e.sheet_id == sheet_id && e.note_id == note_id);
        let collapsed = if is_editing {
            false
        } else if sheet_frozen {
            views
                .center_stage
                .note_collapsed
                .get(&key)
                .copied()
                .or_else(|| app.nav.floating_notes_collapsed.get(&note_id).copied())
                .unwrap_or(note.minimized)
        } else {
            views.center_stage.note_collapsed.get(&key).copied().unwrap_or(note.minimized)
        };
        let pinned = item.pinned;

        let [raw_w, raw_h] =
            views.center_stage.note_sizes_pt.get(&key).copied().unwrap_or([note.bounds_pt.width as f32, note.bounds_pt.height as f32]);
        let min_h = if is_editing { FLOATING_NOTE_EDIT_MIN_H_PT } else { FLOATING_NOTE_MIN_H_PT };
        let max_w = (stage_bounds.width() - FLOATING_NOTE_MARGIN_PT * 2.0).max(FLOATING_NOTE_MIN_W_PT);
        let max_h = (stage_bounds.height() - FLOATING_NOTE_MARGIN_PT * 2.0).max(min_h);
        let card_w = raw_w.clamp(FLOATING_NOTE_MIN_W_PT.min(max_w), max_w);
        let expanded_h = raw_h.max(FLOATING_NOTE_MIN_H_PT).clamp(FLOATING_NOTE_MIN_H_PT.min(max_h), max_h);
        let card_h = if collapsed { FLOATING_NOTE_HEADER_H_PT } else { raw_h.max(min_h).clamp(min_h.min(max_h), max_h) };

        let [rel_x, rel_y] = views.center_stage.note_positions_pt.get(&key).copied().unwrap_or([note.bounds_pt.x as f32, note.bounds_pt.y as f32]);

        let min_x = stage_bounds.left() + FLOATING_NOTE_MARGIN_PT;
        let max_x = (stage_bounds.right() - card_w - FLOATING_NOTE_MARGIN_PT).max(min_x);
        let min_y = stage_bounds.top() + FLOATING_NOTE_MARGIN_PT;
        let max_y = (stage_bounds.bottom() - card_h - FLOATING_NOTE_MARGIN_PT).max(min_y);

        let pos = pos2((stage_bounds.left() + rel_x).clamp(min_x, max_x), (stage_bounds.top() + rel_y).clamp(min_y, max_y));
        views.center_stage.note_positions_pt.insert(key.clone(), [pos.x - stage_bounds.left(), pos.y - stage_bounds.top()]);

        let card_rect = Rect::from_min_size(pos, vec2(card_w, card_h));
        let fill = if note.fill_color.a == 0 { rgba8_to_color32(FloatingNote::DEFAULT_FILL) } else { rgba8_to_color32(note.fill_color) };
        let is_focused = notes_pane_focused && views.center_stage.focused_floating_note_id.as_deref() == Some(note_id.as_str());

        let cx = FloatingNoteCardCtx {
            doc,
            sheet_id,
            note,
            note_id: &note_id,
            idx,
            key: &key,
            sheet_frozen,
            locale,
            tokens,
            enabled,
            can_mutate,
            mutate_disabled_reason,
            is_editing,
            collapsed,
            pinned,
            pos,
            card_w,
            card_h,
            expanded_h,
            min_h,
            min_x,
            max_x,
            min_y,
            max_y,
            stage_bounds,
            note_ink,
        };

        let order = if enabled { Order::Middle } else { Order::Background };
        let area_resp = egui::Area::new(Id::unique(("center_stage_floating_note", sheet_id.as_str(), note_id.as_str())))
            .order(order)
            .fixed_pos(card_rect.min)
            .interactable(enabled)
            .constrain_to(stage_bounds)
            .show(ui.ctx(), |area_ui| {
                area_ui.set_min_size(card_rect.size());
                area_ui.set_max_size(card_rect.size());

                let (frame_rect, frame_resp) = area_ui.allocate_exact_size(card_rect.size(), Sense::hover());
                let card_ak = format!("Floating Note {note_id}");
                frame_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Region, enabled, card_ak.clone()));
                area_ui.painter().rect(
                    frame_rect,
                    CornerRadius::same(4),
                    fill,
                    Stroke::new(if is_focused { 2.0 } else { 1.0 }, if is_focused { tokens.focus_ring } else { tokens.border }),
                    StrokeKind::Inside,
                );

                let hdr_rect = Rect::from_min_max(frame_rect.min, pos2(frame_rect.right(), frame_rect.top() + FLOATING_NOTE_HEADER_H_PT));
                render_floating_note_header(area_ui, views, &cx, hdr_rect, meta, intents);
                if !collapsed {
                    render_floating_note_body_and_resize(area_ui, views, &cx, frame_rect, hdr_rect, meta, intents);
                }

                let card_wid = format!("floating_note.{}.{}.card", sheet_id.as_str(), note_id);
                record_region_meta(
                    meta,
                    area_ui,
                    &frame_resp,
                    inspect_input(&card_wid, "region", &note_id, None, None, enabled, ZLayer::FloatingOverlay),
                );
            });
        if enabled && pinned {
            ui.ctx().move_to_top(area_resp.response.layer_id);
        }
    }
}

fn active_sheet_floating_notes<'a>(doc: &'a ProjectDocument, id: &SheetId) -> &'a [FloatingNote] {
    if let Some(ds) = doc.data_sheet(id) {
        &ds.floating_notes
    } else if let Some(info) = doc.info_sheet(id) {
        &info.floating_notes
    } else if let Some(rs) = doc.result_sheet(id) {
        &rs.floating_notes
    } else if let Some(gs) = doc.graph_sheet(id) {
        &gs.floating_notes
    } else if let Some(ls) = doc.layout_sheet(id) {
        &ls.floating_notes
    } else {
        &[]
    }
}

fn resolve_stage_hook(doc: &ProjectDocument, token: &str) -> Option<String> {
    let trimmed = token.trim();
    if let Some(rest) = trimmed.strip_prefix("info:") {
        let mut parts = rest.splitn(2, ':');
        let first = parts.next()?.trim();
        if let Some(second) = parts.next() {
            return doc.info_sheet(&SheetId::new(first)).and_then(|s| s.get_constant(second)).map(|c| c.value.display_text());
        }
        return doc.info_sheets.first().and_then(|s| s.get_constant(first)).map(|c| c.value.display_text());
    }
    if let Some(rest) = trimmed.strip_prefix("result:") {
        let parts: Vec<&str> = rest.split(':').map(str::trim).collect();
        if let [sid, key] = parts.as_slice() {
            return doc.result_sheet(&SheetId::new(*sid)).and_then(|rs| rs.lookup_scalar(key, None)).map(|v| format!("{v:.4}"));
        }
        if let [sid, ds, key] = parts.as_slice()
            && let Ok(idx) = ds.parse::<usize>()
        {
            return doc.result_sheet(&SheetId::new(*sid)).and_then(|rs| rs.lookup_scalar(key, Some(idx))).map(|v| format!("{v:.4}"));
        }
    }
    None
}

fn render_progress_modal_if_open(ui: &mut egui::Ui, app: &UiApp, views: &mut EguiShellViewState, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    let Some(snapshot) = views.progress_modal.clone() else {
        return;
    };
    let screen = ui.ctx().viewport_rect();

    egui::Area::new(Id::unique("center_stage.progress_modal.backdrop")).order(Order::Middle).fixed_pos(screen.min).interactable(true).show(
        ui.ctx(),
        |bg_ui| {
            let (_, resp) = bg_ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Region, true, "Progress Backdrop"));
            bg_ui.painter().rect_filled(screen, CornerRadius::ZERO, tokens.scrim);
            meta.record_response(
                bg_ui,
                &resp,
                inspect_input("modal.progress.backdrop", "region", "Progress Backdrop", None, None, true, ZLayer::ModalBackdrop),
            );
        },
    );

    let card_w = (screen.width() - 48.0).clamp(240.0, 400.0);
    let card_h = 136.0;
    let card_rect = Rect::from_center_size(screen.center(), vec2(card_w, card_h));

    egui::Area::new(Id::unique("center_stage.progress_modal.card")).order(Order::Foreground).fixed_pos(card_rect.min).interactable(true).show(
        ui.ctx(),
        |card_ui| {
            card_ui.set_min_size(card_rect.size());
            card_ui.set_max_size(card_rect.size());
            let card_a11y = card_ui.interact(card_rect, Id::unique("center_stage.progress_modal.a11y"), Sense::hover());
            let dlg_title = snapshot.operation_label.clone();
            card_a11y.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, dlg_title.clone()));
            let frame = egui::Frame::window(card_ui.style())
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::same(14));
            let inner = frame.show(card_ui, |body| {
                body.label(RichText::new(&snapshot.operation_label).strong().size(15.0).color(tokens.text_primary));
                body.add_space(4.0);
                body.label(RichText::new(&snapshot.detail).small().color(tokens.text_muted));
                body.add_space(10.0);

                let bar_w = (card_w - 28.0).max(120.0);
                let (bar_rect, _) = body.allocate_exact_size(vec2(bar_w, 10.0), Sense::hover());
                let bar_r = CornerRadius::same(tokens.metrics.radius_md);
                body.painter().rect_filled(bar_rect, bar_r, tokens.surface_bg);

                if let Some(frac) = snapshot.progress_0_to_1 {
                    let clamped = frac.clamp(0.0, 1.0);
                    let fill_rect = Rect::from_min_max(bar_rect.min, pos2(bar_rect.left() + bar_rect.width() * clamped, bar_rect.bottom()));
                    body.painter().rect_filled(fill_rect, bar_r, tokens.accent);
                    body.add_space(4.0);
                    body.label(RichText::new(format!("{:.0}%", clamped * 100.0)).small().monospace().color(tokens.text_muted));
                } else {
                    // chrono-free monotonic pulse driven exclusively by egui input time.
                    let t = body.ctx().input(|i| i.time);
                    let phase = ((t * 0.75) % 1.0) as f32;
                    let span = 0.28_f32;
                    let start = (phase * (1.0 + span) - span).clamp(0.0, 1.0 - 0.08);
                    let end = (start + span).clamp(start + 0.08, 1.0);
                    let pulse_rect = Rect::from_min_max(
                        pos2(bar_rect.left() + bar_rect.width() * start, bar_rect.top()),
                        pos2(bar_rect.left() + bar_rect.width() * end, bar_rect.bottom()),
                    );
                    body.painter().rect_filled(pulse_rect, bar_r, tokens.accent);
                    // Cap indeterminate spinner pulse to ~30 FPS rather than unbounded max-rate spinning.
                    body.ctx().request_repaint_after(std::time::Duration::from_millis(33));
                }

                if snapshot.cancellable {
                    body.add_space(8.0);
                    let cancel_lbl = tr_or_fallback(app.locale.as_str(), "w5.common.cancel", "Cancel");
                    let cancel_resp = body.add_enabled(!snapshot.cancel_requested, egui::Button::new(cancel_lbl));
                    meta.record_response(
                        body,
                        &cancel_resp,
                        inspect_input("modal.progress.cancel", "button", cancel_lbl, None, None, !snapshot.cancel_requested, ZLayer::ModalDialog),
                    );
                    if cancel_resp.clicked()
                        && let Some(m) = views.progress_modal.as_mut()
                    {
                        m.cancel_requested = true;
                    }
                }
            });
            let _ = inner;
            record_region_meta(
                meta,
                card_ui,
                &card_a11y,
                inspect_input("modal.progress.card", "dialog", &snapshot.operation_label, None, None, true, ZLayer::ModalDialog),
            );
        },
    );
}

// Keep doc type imports anchored cleanly when optional features compile.
const _: fn(&GraphSheet) -> usize = |g| g.spec.series.len();
const _: fn(&InfoSheet) -> usize = |i| i.constants.len();
const _: fn(&LayoutSheet) -> usize = |l| l.spec.items.len();
const _: fn(&ResultSheet) -> usize = |r| r.tabs.len();

#[cfg(test)]
mod floating_note_authoring_tests {
    use super::*;
    use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, RectPt};
    use curvebook_engine::Session;

    fn make_app_with_note(frozen: bool) -> UiApp {
        let mut doc = ProjectDocument::new("Notes Test").unwrap();
        let mut ds =
            DataSheet::new("data-1", "Test Sheet", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap()))
                .unwrap();
        ds.frozen = frozen;
        ds.floating_notes.push(FloatingNote::new("n1", RectPt::new(20.0, 24.0, 180.0, 90.0), "Initial note text"));
        doc.data_sheets.push(ds);
        doc.active_sheet_id = Some(SheetId::new("data-1"));
        let mut app = UiApp::from_session(Session::from_document(doc));
        app.sync_views();
        app
    }

    fn run_overlay_pass(
        ctx: &egui::Context,
        app: &UiApp,
        views: &mut EguiShellViewState,
        raw_input: egui::RawInput,
    ) -> (FrameInspectMeta, PendingUiIntents) {
        let tokens = Tokens::default();
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        let stage_bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0));
        let mut input = raw_input;
        input.screen_rect = Some(stage_bounds);
        let mut out = ctx.run_ui(input, |ui| {
            let notes = collect_visible_floating_notes(app.document(), app.document().active_sheet_id.as_ref(), &views.center_stage.note_pinned);
            render_floating_notes_overlay(
                ui,
                app,
                views,
                FloatingNotesSpec { notes, stage_bounds, tokens: &tokens, enabled: true, notes_pane_focused: true },
                &mut meta,
                &mut intents,
            );
        });
        out.textures_delta.clear();
        (meta, intents)
    }

    fn click_raw(pos: egui::Pos2) -> [egui::RawInput; 2] {
        let mut down = egui::RawInput::default();
        down.events.push(egui::Event::PointerMoved(pos));
        down.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        let mut up = egui::RawInput::default();
        up.events.push(egui::Event::PointerMoved(pos));
        up.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        [down, up]
    }

    fn click_widget(ctx: &egui::Context, app: &UiApp, views: &mut EguiShellViewState, widget_id: &str) -> (FrameInspectMeta, PendingUiIntents) {
        let _ = run_overlay_pass(ctx, app, views, egui::RawInput::default());
        let (meta0, _) = run_overlay_pass(ctx, app, views, egui::RawInput::default());
        let rect = meta0.find_by_widget_id(widget_id).unwrap_or_else(|| panic!("missing widget {widget_id}")).clipped_rect;
        let [down, up] = click_raw(rect.center());
        let _ = run_overlay_pass(ctx, app, views, down);
        run_overlay_pass(ctx, app, views, up)
    }

    #[test]
    fn floating_note_color_cycle_covers_all_presets() {
        assert_eq!(next_floating_note_color(FloatingNote::DEFAULT_FILL), "blue");
        assert_eq!(next_floating_note_color(FloatingNote::BLUE_FILL), "green");
        assert_eq!(next_floating_note_color(FloatingNote::GREEN_FILL), "red");
        assert_eq!(next_floating_note_color(FloatingNote::RED_FILL), "gray");
        assert_eq!(next_floating_note_color(FloatingNote::GRAY_FILL), "yellow");
        assert_eq!(next_floating_note_color(curvebook_palette::Rgba8::rgba(0, 0, 0, 0)), "blue");
        assert_eq!(next_floating_note_color(curvebook_palette::Rgba8::rgb(12, 34, 56)), "yellow");
    }

    #[test]
    fn floating_note_header_buttons_do_not_overlap_and_emit_authoring_commands() {
        let app = make_app_with_note(false);
        let mut views = EguiShellViewState::default();
        let ctx = egui::Context::default();

        let (meta, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        let card = meta.find_by_widget_id("floating_note.data-1.n1.card").expect("card");
        assert!(card.clipped_rect.width() >= 220.0, "card width={}", card.clipped_rect.width());

        let header_ids = [
            "floating_note.data-1.n1.drag_handle",
            "floating_note.data-1.n1.edit",
            "floating_note.data-1.n1.color",
            "floating_note.data-1.n1.pin",
            "floating_note.data-1.n1.collapse",
            "floating_note.data-1.n1.delete",
        ];
        let rects: Vec<Rect> = header_ids
            .iter()
            .map(|id| {
                let node = meta.find_by_widget_id(id).unwrap_or_else(|| panic!("missing {id}"));
                assert_eq!(node.z_layer, ZLayer::FloatingOverlay);
                assert!(node.enabled, "{id} should be enabled");
                if !id.ends_with(".drag_handle") {
                    assert!(node.clipped_rect.width() >= 24.0 && node.clipped_rect.height() >= 24.0, "{id} hitbox too small");
                }
                node.clipped_rect
            })
            .collect();
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let inter = rects[i].intersect(rects[j]);
                assert!(!inter.is_positive(), "header controls {} and {} overlap: {:?}", header_ids[i], header_ids[j], inter);
            }
        }

        // Cycle color emits sheet.updateFloatingNote with fill_color: "blue".
        let (_, color_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.color");
        assert!(color_intents.intents().iter().any(|i| matches!(
            i,
            UiIntent::EngineCommand { command_id, params }
                if command_id == "sheet.updateFloatingNote"
                    && params["sheet_id"] == json!("data-1")
                    && params["note_id"] == json!("n1")
                    && params["fill_color"] == json!("blue")
        )));

        // Delete emits sheet.deleteFloatingNote.
        let (_, del_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.delete");
        assert!(del_intents.intents().iter().any(|i| matches!(
            i,
            UiIntent::EngineCommand { command_id, params }
                if command_id == "sheet.deleteFloatingNote"
                    && params["sheet_id"] == json!("data-1")
                    && params["note_id"] == json!("n1")
        )));

        // Edit opens inline editor with Save & Cancel; Save emits sheet.updateFloatingNote.
        let _ = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.edit");
        assert!(views.center_stage.editing_floating_note.is_some());
        if let Some(edit) = views.center_stage.editing_floating_note.as_mut() {
            edit.draft_text = String::from("Updated observation");
        }
        let (edit_meta, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        let ed_rect = edit_meta.find_by_widget_id("floating_note.data-1.n1.editor").expect("editor").clipped_rect;
        let save_rect = edit_meta.find_by_widget_id("floating_note.data-1.n1.save").expect("save").clipped_rect;
        let cancel_rect = edit_meta.find_by_widget_id("floating_note.data-1.n1.cancel").expect("cancel").clipped_rect;
        let resize_rect = edit_meta.find_by_widget_id("floating_note.data-1.n1.resize_handle").expect("resize").clipped_rect;
        assert!(!ed_rect.intersect(save_rect).is_positive());
        assert!(!save_rect.intersect(cancel_rect).is_positive());
        assert!(!save_rect.intersect(resize_rect).is_positive());

        let (_, save_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.save");
        assert!(views.center_stage.editing_floating_note.is_none());
        assert!(save_intents.intents().iter().any(|i| matches!(
            i,
            UiIntent::EngineCommand { command_id, params }
                if command_id == "sheet.updateFloatingNote"
                    && params["sheet_id"] == json!("data-1")
                    && params["note_id"] == json!("n1")
                    && params["body_markdown"] == json!("Updated observation")
        )));

        // Cancel exits edit mode without emitting an engine mutation.
        let _ = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.edit");
        assert!(views.center_stage.editing_floating_note.is_some());
        let (_, cancel_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.cancel");
        assert!(views.center_stage.editing_floating_note.is_none());
        assert!(cancel_intents.is_empty());

        // Collapse on unfrozen sheet persists via sheet.updateFloatingNote.
        let (_, col_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.collapse");
        assert!(col_intents.intents().iter().any(|i| matches!(
            i,
            UiIntent::EngineCommand { command_id, params }
                if command_id == "sheet.updateFloatingNote"
                    && params["sheet_id"] == json!("data-1")
                    && params["note_id"] == json!("n1")
                    && params["minimized"] == json!(true)
        )));
    }

    #[test]
    fn floating_note_frozen_sheet_disables_mutating_actions_and_uses_ephemeral_collapse() {
        let app = make_app_with_note(true);
        let mut views = EguiShellViewState::default();
        let ctx = egui::Context::default();

        let (meta, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        for id in ["floating_note.data-1.n1.edit", "floating_note.data-1.n1.color", "floating_note.data-1.n1.delete"] {
            let node = meta.find_by_widget_id(id).unwrap_or_else(|| panic!("missing {id}"));
            assert!(!node.enabled, "{id} must be disabled on frozen sheet");
            assert!(node.disabled_reason.as_deref().is_some_and(|r| !r.is_empty()), "{id} must explain frozen disabled reason");
        }
        assert!(meta.find_by_widget_id("floating_note.data-1.n1.pin").expect("pin").enabled);
        assert!(meta.find_by_widget_id("floating_note.data-1.n1.collapse").expect("collapse").enabled);

        let (_, col_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-1.n1.collapse");
        assert!(col_intents.intents().iter().any(|i| matches!(
            i,
            UiIntent::ToggleFloatingNoteCollapsed { sheet_id, note_id }
                if sheet_id.as_str() == "data-1" && note_id == "n1"
        )));
        assert!(!col_intents.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { .. })));
    }

    #[test]
    fn floating_note_frozen_nav_collapse_does_not_trap_unfrozen_note_1_collapsed() {
        let mut doc = ProjectDocument::new("Frozen Collapse Isolation").unwrap();
        let mut frozen_ds = DataSheet::new(
            "data-frozen",
            "Frozen Sheet",
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()),
        )
        .unwrap();
        frozen_ds.floating_notes.push(FloatingNote::new("note-1", RectPt::new(20.0, 24.0, 240.0, 110.0), "Frozen note"));
        frozen_ds.frozen = true;

        let mut live_ds =
            DataSheet::new("data-live", "Live Sheet", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()))
                .unwrap();
        live_ds.floating_notes.push(FloatingNote::new("note-1", RectPt::new(20.0, 24.0, 240.0, 110.0), "Live note"));

        doc.data_sheets.extend([frozen_ds, live_ds]);
        doc.active_sheet_id = Some(SheetId::new("data-frozen"));
        let mut app = UiApp::from_session(Session::from_document(doc));
        app.sync_views();

        let mut views = EguiShellViewState::default();
        let ctx = egui::Context::default();

        // 1. Collapse "note-1" on the frozen sheet (writes ("note-1", true) into app.nav.floating_notes_collapsed).
        let (_, mut frozen_col_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-frozen.note-1.collapse");
        assert!(frozen_col_intents.commit_if_settled(&mut app, false).is_ok());
        assert_eq!(app.nav.floating_notes_collapsed.get("note-1").copied(), Some(true));

        // 2. Switch to unfrozen "data-live" which also has auto-generated "note-1": it must render expanded.
        app.execute_command("sheet.select", json!({ "sheet_id": "data-live" })).unwrap();
        app.sync_views();
        let (meta_live, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        let live_card = meta_live.find_by_widget_id("floating_note.data-live.note-1.card").expect("live card");
        assert!(live_card.clipped_rect.height() >= FLOATING_NOTE_MIN_H_PT, "stale frozen nav collapse trapped unfrozen note-1");

        // 3. Collapse then expand "note-1" on "data-live" via persisted engine updates; expand must stay expanded after mutation_seq sync.
        let (_, mut live_col_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-live.note-1.collapse");
        assert!(live_col_intents.commit_if_settled(&mut app, false).is_ok());
        assert!(app.document().data_sheet(&SheetId::new("data-live")).unwrap().floating_notes[0].minimized);
        let (meta_min, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        assert!(meta_min.find_by_widget_id("floating_note.data-live.note-1.card").expect("collapsed live").clipped_rect.height() <= 28.0);

        let (_, mut live_exp_intents) = click_widget(&ctx, &app, &mut views, "floating_note.data-live.note-1.collapse");
        assert!(live_exp_intents.commit_if_settled(&mut app, false).is_ok());
        assert!(!app.document().data_sheet(&SheetId::new("data-live")).unwrap().floating_notes[0].minimized);
        let (meta_exp, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        assert!(
            meta_exp.find_by_widget_id("floating_note.data-live.note-1.card").expect("expanded live").clipped_rect.height() >= FLOATING_NOTE_MIN_H_PT,
            "expanded unfrozen note-1 snapped back to collapsed via app.nav.floating_notes_collapsed"
        );

        // 4. Unfreezing "data-frozen" itself also restores authoritative persisted minimized=false state.
        app.execute_command("sheet.select", json!({ "sheet_id": "data-frozen" })).unwrap();
        app.execute_command("sheet.unfreeze", json!({ "sheet_id": "data-frozen" })).unwrap();
        app.sync_views();
        let (meta_unfrozen, _) = run_overlay_pass(&ctx, &app, &mut views, egui::RawInput::default());
        assert!(
            meta_unfrozen.find_by_widget_id("floating_note.data-frozen.note-1.card").expect("unfrozen card").clipped_rect.height()
                >= FLOATING_NOTE_MIN_H_PT,
            "unfrozen sheet note-1 must follow persisted minimized=false"
        );
    }
}
