//! 5-mode `CenterStage` orchestrator (`Welcome`, `ColumnGrid`, `InfoSheet`, `ResultSheet`,
//! `CanvasViewport`, `PortfolioGallery`, Split Comparison, Clamped `FloatingNote` overlays,
//! `QOL-7` bottom diagnostic/clipping/linked-region banners, and `chrono`-free Progress Modal;
//! `docs/planning/gui-layout-spec.md` §5, `R15.3`).

use std::collections::BTreeMap;

use curvebook_chart::preflight_check_scene;
use curvebook_doc::{
    CellValue, DataSheet, FloatingNote, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, ProjectDocument, ResultSheet, SheetFolderKind, SheetId,
    TableArchetype,
};
use curvebook_palette::Rgba8;
use curvebook_text::expand_template_hooks;
use curvebook_tools::GridCellCoord;
use egui::{Color32, CornerRadius, FontId, Id, Key, Order, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::AssumptionDiagnosticBanner;
use crate::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view_for_sheet};
use crate::egui_shell::data_grid::{ColumnGridRenderContext, render_data_grid};
use crate::egui_shell::info_view::{InfoViewEguiState, render_info_view_for_sheet};
use crate::egui_shell::inspect_bridge::HybridCoexistenceDecision;
use crate::egui_shell::results_view::{ResultsViewEguiState, render_results_view_for_sheet};
use crate::egui_shell::welcome_hub::render_welcome_hub;
use crate::egui_shell::{
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, SplitOrientation, Tokens, UiIntent,
    doc_sheet_title, rgba8_to_color32,
};
use crate::i18n::tr;
use crate::i18n::{normalize_locale, try_tr};
use crate::inspect::ZLayer;
use crate::workspace_nav::{ActiveWorkspacePane, PortfolioCard};

const MIN_SPLIT_RATIO: f32 = 0.15;
const MAX_SPLIT_RATIO: f32 = 0.85;
const DEFAULT_SPLIT_RATIO: f32 = 0.50;
const SPLITTER_VISUAL_PT: f32 = 4.0;
const PANE_HEADER_HEIGHT_PT: f32 = 24.0;
const BOTTOM_BANNER_HEIGHT_PT: f32 = 32.0;
const FLOATING_NOTE_HEADER_H_PT: f32 = 26.0;
const FLOATING_NOTE_MIN_W_PT: f32 = 160.0;
const FLOATING_NOTE_MIN_H_PT: f32 = 76.0;
const FLOATING_NOTE_MARGIN_PT: f32 = 6.0;
const GALLERY_DEFAULT_CARD_W_PT: f32 = 168.0;
const WARNING_ACCENT_LIGHT: Color32 = Color32::from_rgb(200, 120, 16);

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
            secondary_info_view: InfoViewEguiState::default(),
            secondary_results_view: ResultsViewEguiState::default(),
            secondary_canvas_view: CanvasViewEguiState::default(),
            banner_details_open: None,
            last_synced_grid_coord: None,
            gallery_anchor_id: None,
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

fn stage_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    let norm = normalize_locale(locale);
    try_tr(norm, key).or_else(|| try_tr("en", key)).unwrap_or(fallback)
}

fn record_region_meta(meta: &mut FrameInspectMeta, ui: &egui::Ui, resp: &egui::Response, input: FrameInspectEntryInput<'_>) {
    meta.record_response(ui, resp, input);
}

fn inspect_input<'a>(
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
    let split_active = app.split.enabled && !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery);

    handle_f6_split_and_note_focus(ui, app, views, &ctx_in, split_active, active_sheet_id.as_ref(), intents);

    let modal_blocked = !ctx_in.enabled || views.progress_modal.is_some();
    let stage_enabled = !modal_blocked;

    // Reserve bottom 32 pt for QOL-7 banner when visible so scrollbars and footer stay unoccluded.
    // In split view, the primary pane's sheet owns the singleton bottom banner.
    let banner_sheet_id = if split_active { app.split.resolve_sheets(doc).0.or_else(|| active_sheet_id.clone()) } else { active_sheet_id.clone() };
    let banner_kind = if !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery) {
        detect_bottom_banner(app, views, banner_sheet_id.as_ref())
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

    if mode == CenterStageMode::Gallery {
        report.primary_rect_pt = Some(work_rect);
        report.handled_natively = true;
        render_portfolio_gallery(ui, app, views, work_rect, ctx_in.tokens, stage_enabled, meta, intents, &mut report);
        draw_focus_ring_if_active(ui, work_rect, ctx_in.tokens, ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid);
        render_progress_modal_if_open(ui, app, views, ctx_in.tokens, meta);
        return report;
    }

    if split_active {
        render_split_stage(ui, app, views, &ctx_in, work_rect, stage_enabled, meta, intents, &mut report);
    } else {
        report.primary_rect_pt = Some(work_rect);
        let mut pane_ui = ui.new_child(egui::UiBuilder::new().max_rect(work_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        pane_ui.set_clip_rect(work_rect);
        render_single_pane_mode(
            &mut pane_ui,
            app,
            views,
            mode,
            active_sheet_id.as_ref(),
            SplitPaneSlot::Primary,
            &ctx_in,
            stage_enabled,
            meta,
            intents,
            &mut report,
        );
        draw_focus_ring_if_active(
            ui,
            work_rect,
            ctx_in.tokens,
            ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid && mode != CenterStageMode::Welcome,
        );
    }

    if let (Some(banner), Some(b_rect)) = (banner_kind, banner_rect) {
        report.diagnostic_banner_rect_pt = Some(b_rect);
        render_bottom_banner(ui, app, views, &banner, b_rect, ctx_in.tokens, stage_enabled, meta, intents);
    }

    if let Some(sheet_id) = active_sheet_id.as_ref().filter(|_| !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery)) {
        let notes = active_sheet_floating_notes(doc, sheet_id);
        if !notes.is_empty() {
            render_floating_notes_overlay(
                ui,
                app,
                views,
                sheet_id,
                notes,
                work_rect,
                ctx_in.tokens,
                stage_enabled,
                ctx_in.focused_pane == ActiveWorkspacePane::FloatingNotes,
                meta,
                intents,
            );
        }
    }

    render_progress_modal_if_open(ui, app, views, ctx_in.tokens, meta);
    report
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

fn handle_f6_split_and_note_focus(
    ui: &egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: &CenterStageContext<'_>,
    split_active: bool,
    active_sheet_id: Option<&SheetId>,
    intents: &mut PendingUiIntents,
) {
    if !ctx_in.enabled {
        return;
    }

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
        let reverse = cycle_reverse.unwrap_or(false);
        if ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid {
            let consume_internal = match (views.center_stage.focused_split_pane, reverse) {
                (SplitPaneSlot::Primary, false) => {
                    views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
                    true
                }
                (SplitPaneSlot::Secondary, true) => {
                    views.center_stage.focused_split_pane = SplitPaneSlot::Primary;
                    true
                }
                _ => false,
            };
            if consume_internal {
                strip_pane_cycle_intent(intents);
            }
        } else if reverse && ctx_in.focused_pane == ActiveWorkspacePane::Inspector {
            views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
        } else if !reverse && matches!(ctx_in.focused_pane, ActiveWorkspacePane::Navigator | ActiveWorkspacePane::FamilyStrip) {
            views.center_stage.focused_split_pane = SplitPaneSlot::Primary;
        }
        return;
    }

    // Standalone `render_center_stage` fallback when outer `inspect_bridge` shortcut handler wasn't run.
    let (f6_pressed, shift_held) = ui.ctx().input(|i| (i.key_pressed(Key::F6), i.modifiers.shift));
    if !f6_pressed || found_cycle {
        return;
    }

    let doc = app.document();
    let has_notes = active_sheet_id.is_some_and(|id| !active_sheet_floating_notes(doc, id).is_empty());
    if ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid {
        if split_active {
            if !shift_held && views.center_stage.focused_split_pane == SplitPaneSlot::Primary {
                views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
            } else if shift_held && views.center_stage.focused_split_pane == SplitPaneSlot::Secondary {
                views.center_stage.focused_split_pane = SplitPaneSlot::Primary;
            } else if has_notes {
                intents.push_set_focus_pane(ActiveWorkspacePane::FloatingNotes);
            } else {
                views.center_stage.focused_split_pane = if shift_held { SplitPaneSlot::Secondary } else { SplitPaneSlot::Primary };
            }
        } else if has_notes {
            intents.push_set_focus_pane(ActiveWorkspacePane::FloatingNotes);
        } else {
            intents.push_set_focus_pane(ActiveWorkspacePane::Inspector);
        }
    } else if ctx_in.focused_pane == ActiveWorkspacePane::FloatingNotes {
        views.center_stage.focused_split_pane = if shift_held && split_active { SplitPaneSlot::Secondary } else { SplitPaneSlot::Primary };
        intents.push_set_focus_pane(ActiveWorkspacePane::MainCanvasOrGrid);
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

fn resolve_effective_split_ratio(app: &UiApp, state: &mut CenterStageEguiState) -> f32 {
    if state.last_seen_app_split_ratio != Some(app.split.ratio) {
        state.last_seen_app_split_ratio = Some(app.split.ratio);
        state.split_ratio_override = Some((app.split.ratio as f32).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO));
    }
    state.split_ratio_override.unwrap_or(app.split.ratio as f32).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
}

#[allow(clippy::too_many_arguments)]
fn render_split_stage(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: &CenterStageContext<'_>,
    work_rect: Rect,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CenterStageFrameReport,
) {
    let doc = app.document();
    let orientation = if work_rect.width() >= 720.0 { SplitOrientation::Horizontal } else { SplitOrientation::Vertical };
    let mut ratio = resolve_effective_split_ratio(app, &mut views.center_stage);

    let (mut primary_rect, mut divider_visual_rect, divider_hit_rect, mut secondary_rect) =
        compute_split_geometries(work_rect, orientation, ratio, ctx_in.tokens.metrics.splitter_hit_slop_w);

    let divider_resp =
        ui.interact(divider_hit_rect, Id::unique("center_stage.split.divider"), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    divider_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, enabled, "Split Comparison Divider"));
    if enabled {
        let cursor = match orientation {
            SplitOrientation::Horizontal => egui::CursorIcon::ResizeHorizontal,
            SplitOrientation::Vertical => egui::CursorIcon::ResizeVertical,
        };
        if divider_resp.hovered() || divider_resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if divider_resp.double_clicked() {
            ratio = DEFAULT_SPLIT_RATIO;
            views.center_stage.split_ratio_override = Some(ratio);
            views.center_stage.last_seen_app_split_ratio = Some(f64::from(ratio).clamp(0.2, 0.8));
            intents.push_set_split_ratio(f64::from(ratio));
            (primary_rect, divider_visual_rect, _, secondary_rect) =
                compute_split_geometries(work_rect, orientation, ratio, ctx_in.tokens.metrics.splitter_hit_slop_w);
        } else if divider_resp.dragged() {
            views.center_stage.split_dragging = true;
            if let Some(pointer) = divider_resp.interact_pointer_pos() {
                let raw = match orientation {
                    SplitOrientation::Horizontal => (pointer.x - work_rect.left()) / work_rect.width().max(1.0),
                    SplitOrientation::Vertical => (pointer.y - work_rect.top()) / work_rect.height().max(1.0),
                };
                ratio = raw.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
                views.center_stage.split_ratio_override = Some(ratio);
                views.center_stage.last_seen_app_split_ratio = Some(f64::from(ratio).clamp(0.2, 0.8));
                intents.push_set_split_ratio(f64::from(ratio));
                (primary_rect, divider_visual_rect, _, secondary_rect) =
                    compute_split_geometries(work_rect, orientation, ratio, ctx_in.tokens.metrics.splitter_hit_slop_w);
            }
        } else {
            views.center_stage.split_dragging = false;
        }
    }

    let divider_color = if divider_resp.hovered() || divider_resp.dragged() { ctx_in.tokens.accent } else { ctx_in.tokens.divider };
    ui.painter().rect_filled(divider_visual_rect, CornerRadius::ZERO, divider_color);
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

    let (primary_id, secondary_resolved) = app.split.resolve_sheets(doc);
    let secondary_id = secondary_resolved.or_else(|| pick_fallback_secondary_sheet(doc, primary_id.as_ref())).or_else(|| primary_id.clone());

    report.primary_rect_pt = Some(primary_rect);
    report.secondary_rect_pt = Some(secondary_rect);

    let stage_has_focus = ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid;
    let locale = app.locale.as_str();

    render_split_pane_container(
        ui,
        app,
        views,
        ctx_in,
        SplitPaneSlot::Primary,
        primary_rect,
        primary_id.as_ref(),
        secondary_id.as_ref(),
        locale,
        enabled,
        stage_has_focus && views.center_stage.focused_split_pane == SplitPaneSlot::Primary,
        meta,
        intents,
        report,
    );

    render_split_pane_container(
        ui,
        app,
        views,
        ctx_in,
        SplitPaneSlot::Secondary,
        secondary_rect,
        secondary_id.as_ref(),
        primary_id.as_ref(),
        locale,
        enabled,
        stage_has_focus && views.center_stage.focused_split_pane == SplitPaneSlot::Secondary,
        meta,
        intents,
        report,
    );

    sync_two_way_split_brushing(app, views, primary_id.as_ref(), secondary_id.as_ref(), intents);
}

#[allow(clippy::too_many_arguments)]
fn render_split_pane_container(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: &CenterStageContext<'_>,
    slot: SplitPaneSlot,
    pane_rect: Rect,
    sheet_id: Option<&SheetId>,
    other_sheet_id: Option<&SheetId>,
    locale: &str,
    enabled: bool,
    focused: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CenterStageFrameReport,
) {
    let doc = app.document();
    let header_h = PANE_HEADER_HEIGHT_PT.min(pane_rect.height());
    let header_rect = Rect::from_min_max(pane_rect.min, pos2(pane_rect.right(), pane_rect.top() + header_h));
    let body_rect = Rect::from_min_max(pos2(pane_rect.left(), header_rect.bottom()), pane_rect.max);

    render_split_pane_header(ui, app, slot, header_rect, sheet_id, other_sheet_id, locale, ctx_in.tokens, enabled, focused, meta, intents);

    let click_in_pane = ui.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| pane_rect.contains(p)));
    if enabled && click_in_pane {
        views.center_stage.focused_split_pane = slot;
        if ctx_in.focused_pane != ActiveWorkspacePane::MainCanvasOrGrid {
            intents.push_set_focus_pane(ActiveWorkspacePane::MainCanvasOrGrid);
        }
    }

    let pane_mode = sheet_id.map_or(CenterStageMode::Welcome, |id| mode_for_sheet(doc, id));
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    body_ui.set_clip_rect(body_rect);

    let active_matches = sheet_id.is_some_and(|id| doc.active_sheet_id.as_ref() == Some(id));
    let can_use_primary_grid_state = slot == SplitPaneSlot::Primary || other_sheet_id != sheet_id;

    if pane_mode == CenterStageMode::ColumnGrid && !(active_matches && can_use_primary_grid_state) {
        report.handled_natively = true;
        if let Some(ds) = sheet_id.and_then(|id| doc.data_sheet(id)) {
            render_split_column_sheet_preview(&mut body_ui, app, ds, slot, ctx_in.tokens, enabled, meta, intents, report);
        }
    } else if pane_mode == CenterStageMode::DataGrid && !(active_matches && can_use_primary_grid_state) {
        report.handled_natively = true;
        if let Some(ds) = sheet_id.and_then(|id| doc.data_sheet(id)) {
            render_split_non_column_preview(&mut body_ui, app, ds, slot, ctx_in.tokens, enabled, meta, intents);
        }
    } else {
        render_single_pane_mode(&mut body_ui, app, views, pane_mode, sheet_id, slot, ctx_in, enabled, meta, intents, report);
    }

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
    draw_focus_ring_if_active(ui, pane_rect, ctx_in.tokens, focused);
}

#[allow(clippy::too_many_arguments)]
fn render_split_pane_header(
    ui: &mut egui::Ui,
    app: &UiApp,
    slot: SplitPaneSlot,
    header_rect: Rect,
    sheet_id: Option<&SheetId>,
    other_sheet_id: Option<&SheetId>,
    locale: &str,
    tokens: &Tokens,
    enabled: bool,
    focused: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let bg = if focused { tokens.active_bg } else { tokens.panel_bg };
    ui.painter().rect_filled(header_rect, CornerRadius::ZERO, bg);
    ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, tokens.border));

    let mut hdr_ui =
        ui.new_child(egui::UiBuilder::new().max_rect(header_rect.shrink2(vec2(6.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    hdr_ui.set_clip_rect(header_rect);

    let slot_badge = match slot {
        SplitPaneSlot::Primary => stage_tr(locale, "w5.split.primary", "Primary"),
        SplitPaneSlot::Secondary => stage_tr(locale, "w5.split.secondary", "Secondary"),
    };
    hdr_ui.label(RichText::new(slot_badge).small().strong().color(if focused { tokens.accent } else { tokens.text_muted }));

    let current_title = sheet_id.and_then(|id| doc_sheet_title(doc, id)).unwrap_or("(None)");
    let sel_wid = match slot {
        SplitPaneSlot::Primary => "split.primary_selector",
        SplitPaneSlot::Secondary => "split.secondary_selector",
    };
    let all_sheets = collect_all_sheet_ids(doc);
    let combo = egui::ComboBox::from_id_salt(("split_pane_sheet", slot))
        .selected_text(RichText::new(current_title).small())
        .width((header_rect.width() * 0.40).clamp(72.0, 200.0))
        .show_ui(&mut hdr_ui, |combo_ui| {
            for (cand_id, cand_title, kind_badge) in &all_sheets {
                let selected = sheet_id == Some(cand_id);
                if combo_ui.selectable_label(selected, format!("[{kind_badge}] {cand_title}")).clicked() && enabled {
                    let param_key = match slot {
                        SplitPaneSlot::Primary => "primary_sheet_id",
                        SplitPaneSlot::Secondary => "secondary_sheet_id",
                    };
                    intents.push_ui("ui.split.toggle", json!({ "enabled": true, param_key: cand_id.as_str() }));
                }
            }
        });
    meta.record_response(
        &hdr_ui,
        &combo.response,
        inspect_input(sel_wid, "combobox", current_title, Some("ui.split.toggle"), None, enabled, ZLayer::Workspace),
    );

    if let Some(brushed) = app.brushed_cell.as_ref() {
        let brush_label = format!("Brush R{}C{}", brushed.row + 1, brushed.col + 1);
        hdr_ui.label(RichText::new(brush_label).small().color(tokens.accent));
        if slot == SplitPaneSlot::Secondary {
            let clr_resp = hdr_ui.add_enabled(enabled, egui::Button::new(RichText::new("×").small()).min_size(vec2(18.0, 18.0)));
            meta.record_response(
                &hdr_ui,
                &clr_resp,
                inspect_input("split.clear_brush", "button", "Clear Brush", None, None, enabled, ZLayer::Workspace),
            );
            if clr_resp.clicked() && enabled {
                intents.push_clear_brush();
            }
        }
    } else if slot == SplitPaneSlot::Primary && header_rect.width() >= 280.0 {
        hdr_ui.label(RichText::new(stage_tr(locale, "w5.split.brush_sync", "Sync Brush")).small().color(tokens.text_muted));
    }

    if slot == SplitPaneSlot::Secondary {
        hdr_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |right_ui| {
            let close_lbl = stage_tr(locale, "w5.split.close", "Close");
            let close_resp = right_ui.add_enabled(enabled, egui::Button::new(RichText::new(close_lbl).small()).min_size(vec2(44.0, 18.0)));
            meta.record_response(
                right_ui,
                &close_resp,
                inspect_input("split.close", "button", close_lbl, Some("ui.split.toggle"), None, enabled, ZLayer::Workspace),
            );
            if close_resp.clicked() && enabled {
                intents.push_ui("ui.split.toggle", json!({ "enabled": false }));
            }

            let swap_lbl = stage_tr(locale, "w5.split.swap", "Swap");
            let swap_resp = right_ui.add_enabled(
                enabled && sheet_id.is_some() && other_sheet_id.is_some(),
                egui::Button::new(RichText::new(swap_lbl).small()).min_size(vec2(40.0, 18.0)),
            );
            meta.record_response(
                right_ui,
                &swap_resp,
                inspect_input("split.swap", "button", swap_lbl, Some("ui.split.toggle"), None, enabled, ZLayer::Workspace),
            );
            if swap_resp.clicked()
                && enabled
                && let (Some(sec), Some(prim)) = (sheet_id, other_sheet_id)
            {
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
        SplitOrientation::Vertical => {
            let split_y = (work_rect.top() + work_rect.height() * ratio).clamp(work_rect.top() + 48.0, work_rect.bottom() - 48.0);
            let top = Rect::from_min_max(work_rect.min, pos2(work_rect.right(), split_y - half_vis));
            let bottom = Rect::from_min_max(pos2(work_rect.left(), split_y + half_vis), work_rect.max);
            let vis = Rect::from_min_max(pos2(work_rect.left(), split_y - half_vis), pos2(work_rect.right(), split_y + half_vis));
            let hit = Rect::from_min_max(pos2(work_rect.left(), split_y - half_hit), pos2(work_rect.right(), split_y + half_hit));
            (top, vis, hit, bottom)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_single_pane_mode(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    mode: CenterStageMode,
    sheet_id: Option<&SheetId>,
    slot: SplitPaneSlot,
    ctx_in: &CenterStageContext<'_>,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CenterStageFrameReport,
) {
    match mode {
        CenterStageMode::Welcome => {
            report.handled_natively = true;
            let _ = render_welcome_hub(ui, app, &mut views.welcome_hub, ctx_in.tokens, enabled, meta, intents);
        }
        CenterStageMode::ColumnGrid | CenterStageMode::DataGrid => {
            report.handled_natively = true;
            let grid_rep = render_data_grid(
                ui,
                app,
                &mut views.column_grid,
                ColumnGridRenderContext::new(ctx_in.tokens, !enabled || ctx_in.decision.blocks_egui_grid),
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
            let res_rep = render_results_view_for_sheet(ui, app, sheet_id, res_state, ctx_in.tokens, enabled, meta, intents);
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

#[allow(clippy::too_many_arguments)]
fn render_split_column_sheet_preview(
    ui: &mut egui::Ui,
    app: &UiApp,
    ds: &DataSheet,
    slot: SplitPaneSlot,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CenterStageFrameReport,
) {
    let TableArchetype::Column(col_table) = &ds.archetype else {
        return;
    };
    let avail = ui.available_rect_before_wrap();
    let bar_h = 26.0_f32.min(avail.height());
    let bar_rect = Rect::from_min_max(avail.min, pos2(avail.right(), avail.top() + bar_h));
    let body_rect = Rect::from_min_max(pos2(avail.left(), bar_rect.bottom()), avail.max);

    ui.scope_builder(egui::UiBuilder::new().max_rect(bar_rect), |bar_ui| {
        bar_ui.set_clip_rect(bar_rect);
        bar_ui.horizontal(|row| {
            row.label(
                RichText::new(format!("{} ({} cols × {} rows)", ds.title, col_table.data_sets.len(), col_table.row_count()))
                    .small()
                    .color(tokens.text_muted),
            );
            if row.add_enabled(enabled, egui::Button::new(RichText::new("Focus in Grid").small())).clicked() {
                intents.push_engine("sheet.select", json!({ "sheet_id": ds.id.as_str() }));
            }
        });
    });

    if !body_rect.is_positive() {
        return;
    }

    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    body_ui.set_clip_rect(body_rect);

    let scroll =
        egui::ScrollArea::both().id_salt(("split_col_preview", slot, ds.id.as_str())).auto_shrink([false, false]).show(&mut body_ui, |grid_ui| {
            let row_h = 22.0;
            let col_w = 88.0;
            let max_cols = col_table.data_sets.len().min(64);
            let max_rows = col_table.row_count().min(500);
            grid_ui.horizontal(|hdr| {
                hdr.add_sized([44.0, row_h], egui::Label::new(RichText::new("#").small().strong().color(tokens.text_muted)));
                for (c_idx, col) in col_table.data_sets.iter().take(max_cols).enumerate() {
                    let lbl = if col.title.is_empty() { format!("Col {}", c_idx + 1) } else { col.title.clone() };
                    hdr.add_sized([col_w, row_h], egui::Label::new(RichText::new(lbl).small().strong()).truncate());
                }
            });
            for r_idx in 0..max_rows {
                grid_ui.horizontal(|row_ui| {
                    row_ui.add_sized([44.0, row_h], egui::Label::new(RichText::new(format!("{}", r_idx + 1)).small().color(tokens.text_muted)));
                    for c_idx in 0..max_cols {
                        let is_brushed = app.brushed_points.iter().any(|b| b.sheet_id == ds.id && b.row == r_idx && b.col == c_idx)
                            || app.brushed_cell.as_ref().is_some_and(|b| b.sheet_id == ds.id && b.row == r_idx && b.col == c_idx);
                        let cell_txt = col_table
                            .data_sets
                            .get(c_idx)
                            .and_then(|d| d.cells.get(r_idx))
                            .and_then(|sub| sub.first())
                            .map(|c| match &c.value {
                                CellValue::Empty => String::new(),
                                CellValue::Number(v) => format!("{v:.4}"),
                                CellValue::Text(s) => s.clone(),
                            })
                            .unwrap_or_default();
                        let btn = egui::Button::new(RichText::new(cell_txt).small().monospace().color(if is_brushed {
                            tokens.accent
                        } else {
                            tokens.text_primary
                        }))
                        .fill(if is_brushed { tokens.active_bg } else { tokens.surface_bg })
                        .stroke(Stroke::new(if is_brushed { 1.5 } else { 0.5 }, if is_brushed { tokens.accent } else { tokens.border }))
                        .min_size(vec2(col_w, row_h));
                        let resp = row_ui.add_enabled(enabled, btn);
                        if resp.clicked() && enabled {
                            let coord = GridCellCoord::new(r_idx, c_idx, 0);
                            intents.push_brush_point(ds.id.clone(), coord);
                            if app.document().active_sheet_id.as_ref() == Some(&ds.id) {
                                intents.push_select_grid_cell(coord);
                            }
                        }
                        if r_idx < 12 && c_idx < 4 {
                            let wid = format!("split.col_preview.{r_idx}.{c_idx}");
                            let lbl = format!("Split cell R{}C{}", r_idx + 1, c_idx + 1);
                            meta.record_response(row_ui, &resp, inspect_input(&wid, "button", &lbl, None, None, enabled, ZLayer::Workspace));
                        }
                    }
                });
            }
        });

    if report.scrollbar_rect_pt.is_none() && scroll.content_size.y > scroll.inner_rect.height() + 0.5 {
        let bar_w = body_ui.spacing().scroll.bar_width.max(10.0);
        report.scrollbar_rect_pt = Some(Rect::from_min_max(
            pos2((scroll.inner_rect.right() + 2.0).min(body_rect.right()), scroll.inner_rect.top()),
            pos2((scroll.inner_rect.right() + 2.0 + bar_w).min(body_rect.right()), scroll.inner_rect.bottom()),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_split_non_column_preview(
    ui: &mut egui::Ui,
    app: &UiApp,
    ds: &DataSheet,
    slot: SplitPaneSlot,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let avail = ui.available_rect_before_wrap();
    let bar_h = 26.0_f32.min(avail.height());
    let bar_rect = Rect::from_min_max(avail.min, pos2(avail.right(), avail.top() + bar_h));
    let body_rect = Rect::from_min_max(pos2(avail.left(), bar_rect.bottom()), avail.max);

    ui.scope_builder(egui::UiBuilder::new().max_rect(bar_rect), |bar_ui| {
        bar_ui.set_clip_rect(bar_rect);
        bar_ui.horizontal(|row| {
            row.label(
                RichText::new(format!("{} ({:?} · {} rows)", ds.title, ds.archetype.kind(), ds.archetype.row_count()))
                    .small()
                    .color(tokens.text_muted),
            );
            let btn = row.add_enabled(enabled, egui::Button::new(RichText::new("Focus in Grid").small()));
            meta.record_response(
                row,
                &btn,
                inspect_input("split.activate_data_sheet", "button", "Focus in Grid", Some("sheet.select"), None, enabled, ZLayer::Workspace),
            );
            if btn.clicked() && enabled {
                intents.push_engine("sheet.select", json!({ "sheet_id": ds.id.as_str() }));
            }
        });
    });

    if !body_rect.is_positive() {
        return;
    }

    let preview_grid = crate::grid_view::GridViewState { decimal_separator: app.grid.decimal_separator, ..Default::default() };
    let frame = preview_grid.visible_frame_for_sheet(ds);
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    body_ui.set_clip_rect(body_rect);

    egui::ScrollArea::both().id_salt(("split_data_preview", slot, ds.id.as_str())).auto_shrink([false, false]).show(&mut body_ui, |grid_ui| {
        let row_h = 22.0;
        let col_w = 80.0;
        grid_ui.horizontal(|hdr| {
            hdr.add_sized([36.0, row_h], egui::Label::new(RichText::new("#").small().strong().color(tokens.text_muted)));
            if let Some(x_title) = &frame.x_column_title {
                hdr.add_sized([col_w, row_h], egui::Label::new(RichText::new(x_title).small().strong()).truncate());
            }
            for sub in frame.tier2_subcolumns.iter().take(16) {
                hdr.add_sized([col_w, row_h], egui::Label::new(RichText::new(&sub.label).small().strong()).truncate());
            }
        });
        for row in frame.rows.iter().take(64) {
            grid_ui.horizontal(|row_ui| {
                row_ui.add_sized([36.0, row_h], egui::Label::new(RichText::new(format!("{}", row.row + 1)).small().color(tokens.text_muted)));
                if let Some(x_txt) = &row.leading.x_or_time_display {
                    row_ui.add_sized([col_w, row_h], egui::Label::new(RichText::new(x_txt).small().monospace()).truncate());
                }
                for cell in row.cells.iter().take(16) {
                    let is_brushed = app.brushed_points.iter().any(|b| b.sheet_id == ds.id && b.row == row.row && b.col == cell.coord.col)
                        || app.brushed_cell.as_ref().is_some_and(|b| b.sheet_id == ds.id && b.row == row.row && b.col == cell.coord.col);
                    let btn = egui::Button::new(RichText::new(&cell.display_text).small().monospace().color(if is_brushed {
                        tokens.accent
                    } else {
                        tokens.text_primary
                    }))
                    .fill(if is_brushed { tokens.active_bg } else { tokens.surface_bg })
                    .stroke(Stroke::new(if is_brushed { 1.5 } else { 0.5 }, if is_brushed { tokens.accent } else { tokens.border }))
                    .min_size(vec2(col_w, row_h));
                    if row_ui.add_enabled(enabled, btn).clicked() && enabled {
                        intents.push_brush_point(ds.id.clone(), cell.coord);
                    }
                }
            });
        }
    });
}

fn sync_two_way_split_brushing(
    app: &UiApp,
    views: &mut EguiShellViewState,
    primary_id: Option<&SheetId>,
    secondary_id: Option<&SheetId>,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let grid_sheet_id = [primary_id, secondary_id].into_iter().flatten().find(|id| doc.data_sheet(id).is_some()).cloned();
    let Some(grid_id) = grid_sheet_id else {
        return;
    };

    let mut has_brush_intent = false;
    let mut selected_coord: Option<GridCellCoord> = None;
    for intent in intents.intents() {
        match intent {
            UiIntent::BrushPoint { .. } | UiIntent::ClearBrush => has_brush_intent = true,
            UiIntent::SelectGridCell { coord } | UiIntent::ExtendGridSelection { coord } => selected_coord = Some(*coord),
            _ => {}
        }
    }

    if let Some(coord) = selected_coord {
        views.center_stage.last_synced_grid_coord = Some(coord);
        if !has_brush_intent {
            intents.push_brush_point(grid_id, coord);
        }
        return;
    }

    if !has_brush_intent && doc.active_sheet_id.as_ref() == Some(&grid_id) && app.grid.tool.active != views.center_stage.last_synced_grid_coord {
        views.center_stage.last_synced_grid_coord = app.grid.tool.active;
        if let Some(coord) = app.grid.tool.active {
            intents.push_brush_point(grid_id, coord);
        }
    }
}

fn pick_fallback_secondary_sheet(doc: &ProjectDocument, primary_id: Option<&SheetId>) -> Option<SheetId> {
    let all = collect_all_sheet_ids(doc);
    all.into_iter().map(|(id, _, _)| id).find(|id| Some(id) != primary_id)
}

fn collect_all_sheet_ids(doc: &ProjectDocument) -> Vec<(SheetId, String, &'static str)> {
    let mut out = Vec::with_capacity(doc.total_sheet_count());
    for s in &doc.data_sheets {
        out.push((s.id.clone(), s.title.clone(), "Data"));
    }
    for s in &doc.info_sheets {
        out.push((s.id.clone(), s.title.clone(), "Info"));
    }
    for s in &doc.result_sheets {
        out.push((s.id.clone(), s.title().to_owned(), "Result"));
    }
    for s in &doc.graph_sheets {
        out.push((s.id.clone(), s.title.clone(), "Graph"));
    }
    for s in &doc.layout_sheets {
        out.push((s.id.clone(), s.title.clone(), "Layout"));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_portfolio_gallery(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    rect: Rect,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CenterStageFrameReport,
) {
    let doc = app.document();
    let locale = app.locale.as_str();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, tokens.surface_bg);

    let inner_rect = rect.shrink(8.0);
    let mut gal_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    gal_ui.set_clip_rect(rect);

    gal_ui.horizontal_wrapped(|bar| {
        bar.spacing_mut().item_spacing = vec2(6.0, 4.0);
        bar.label(RichText::new(stage_tr(locale, "w5.gallery.title", "Portfolio Gallery")).strong().color(tokens.text_strong));

        let search_hint = stage_tr(locale, "w5.gallery.search", "Search sheets...");
        let search_resp = bar.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut views.center_stage.gallery_search)
                .id(Id::unique("gallery.search_input"))
                .desired_width((rect.width() * 0.24).clamp(110.0, 220.0))
                .hint_text(search_hint),
        );
        meta.record_response(
            bar,
            &search_resp,
            inspect_input("gallery.search_input", "textbox", search_hint, None, None, enabled, ZLayer::Workspace),
        );

        let filters: [(Option<SheetFolderKind>, &str, &str, &str); 6] = [
            (None, "all", "gallery.filter.all", "All"),
            (Some(SheetFolderKind::Graphs), "graphs", "gallery.filter.graphs", "Graphs"),
            (Some(SheetFolderKind::Layouts), "layouts", "gallery.filter.layouts", "Layouts"),
            (Some(SheetFolderKind::Data), "data", "gallery.filter.data", "Data"),
            (Some(SheetFolderKind::Results), "results", "gallery.filter.results", "Results"),
            (Some(SheetFolderKind::Info), "info", "gallery.filter.info", "Info"),
        ];
        for (folder_opt, folder_param, wid, fallback_lbl) in filters {
            let active = app.nav.gallery.filter_folder == folder_opt;
            let btn = egui::Button::new(RichText::new(fallback_lbl).small().color(if active { tokens.paper_white } else { tokens.text_primary }))
                .fill(if active { tokens.accent } else { tokens.panel_bg })
                .stroke(Stroke::new(1.0, if active { tokens.accent } else { tokens.border }));
            let resp = bar.add_enabled(enabled, btn);
            meta.record_response(
                bar,
                &resp,
                inspect_input(
                    wid,
                    "button",
                    fallback_lbl,
                    Some("ui.gallery.toggle"),
                    Some(json!({ "folder": folder_param })),
                    enabled,
                    ZLayer::Workspace,
                ),
            );
            if resp.clicked() && enabled {
                intents.push_ui("ui.gallery.toggle", json!({ "open": true, "folder": folder_param }));
            }
        }

        let slider =
            bar.add_enabled(enabled, egui::Slider::new(&mut views.center_stage.gallery_zoom_pt, 120.0..=260.0).show_value(false).text("Size"));
        meta.record_response(bar, &slider, inspect_input("gallery.zoom_slider", "slider", "Thumbnail Zoom", None, None, enabled, ZLayer::Workspace));

        let selected_graph_ids: Vec<SheetId> = app.nav.selected_gallery_ids().into_iter().filter(|id| doc.graph_sheet(id).is_some()).collect();
        let style_enabled = enabled && !selected_graph_ids.is_empty() && doc.graph_sheets.len() >= 2;
        let style_lbl = tr(locale, "r13.gallery.apply_style_from");
        let style_resp = bar.add_enabled(style_enabled, egui::Button::new(RichText::new(style_lbl).small()));
        meta.record_response(
            bar,
            &style_resp,
            inspect_input("gallery.apply_style_from", "button", style_lbl, Some("ui.applyStyleFrom.open"), None, style_enabled, ZLayer::Workspace),
        );
        if style_resp.clicked() && style_enabled {
            let target_ids: Vec<&str> = selected_graph_ids.iter().map(SheetId::as_str).collect();
            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
            intents.push_ui("ui.applyStyleFrom.open", json!({ "target_sheet_ids": target_ids }));
        }

        let close_lbl = stage_tr(locale, "w5.gallery.close", "Close Gallery");
        let close_resp = bar.add_enabled(enabled, egui::Button::new(RichText::new(close_lbl).small()));
        meta.record_response(
            bar,
            &close_resp,
            inspect_input("gallery.close", "button", close_lbl, Some("ui.gallery.toggle"), None, enabled, ZLayer::Workspace),
        );
        if close_resp.clicked() && enabled {
            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
        }
    });

    gal_ui.add_space(6.0);
    gal_ui.separator();
    gal_ui.add_space(6.0);

    let cards = build_filtered_gallery_cards(doc, app, &views.center_stage.gallery_search);
    if cards.is_empty() {
        gal_ui.label(RichText::new(stage_tr(locale, "w5.gallery.empty", "No matching sheets in this view.")).color(tokens.text_muted));
        return;
    }

    let selected_ids_snapshot = app.nav.gallery.selected_ids.clone();
    let visible_ids: Vec<SheetId> = cards.iter().map(|c| c.sheet_id.clone()).collect();
    let mut gallery_anchor = views.center_stage.gallery_anchor_id.clone();

    let scroll = egui::ScrollArea::vertical().id_salt("center_stage.gallery.scroll").auto_shrink([false, false]).show(&mut gal_ui, |grid_ui| {
        let inner_w = (grid_ui.available_width() - 16.0).max(140.0);
        let spacing = 10.0;
        let desired_card_w = views.center_stage.gallery_zoom_pt.clamp(120.0, inner_w);
        let cols = ((inner_w + spacing) / (desired_card_w + spacing)).floor().max(1.0) as usize;
        let card_w = ((inner_w - spacing * (cols.saturating_sub(1) as f32)) / (cols as f32)).max(110.0);
        let card_h = (card_w * 0.72).clamp(96.0, 190.0);

        for row_cards in cards.chunks(cols) {
            grid_ui.horizontal(|row_ui| {
                row_ui.spacing_mut().item_spacing.x = spacing;
                for card in row_cards {
                    render_gallery_card(
                        row_ui,
                        doc,
                        card,
                        &visible_ids,
                        &selected_ids_snapshot,
                        &mut gallery_anchor,
                        vec2(card_w, card_h),
                        tokens,
                        enabled,
                        meta,
                        intents,
                    );
                }
            });
            grid_ui.add_space(spacing);
        }
    });
    views.center_stage.gallery_anchor_id = gallery_anchor;

    if scroll.content_size.y > scroll.inner_rect.height() + 0.5 {
        let bar_w = gal_ui.spacing().scroll.bar_width.max(10.0);
        report.scrollbar_rect_pt = Some(Rect::from_min_max(
            pos2((scroll.inner_rect.right() + 2.0).min(inner_rect.right()), scroll.inner_rect.top()),
            pos2((scroll.inner_rect.right() + 2.0 + bar_w).min(inner_rect.right()), scroll.inner_rect.bottom()),
        ));
    }
}

fn build_filtered_gallery_cards(doc: &ProjectDocument, app: &UiApp, search: &str) -> Vec<PortfolioCard> {
    let query = search.trim().to_ascii_lowercase();
    let filter = app.nav.gallery.filter_folder;
    let active_id = doc.active_sheet_id.as_ref();

    let mut out = Vec::new();
    let push_if_match =
        |out: &mut Vec<PortfolioCard>, id: &SheetId, title: &str, folder: SheetFolderKind, badge: &str, subtitle: String, color: Option<Rgba8>| {
            if filter.is_some_and(|f| f != folder) {
                return;
            }
            if !query.is_empty()
                && !title.to_ascii_lowercase().contains(&query)
                && !id.as_str().to_ascii_lowercase().contains(&query)
                && !badge.to_ascii_lowercase().contains(&query)
            {
                return;
            }
            out.push(PortfolioCard {
                sheet_id: id.clone(),
                title: title.to_owned(),
                folder,
                badge_label: badge.to_owned(),
                subtitle,
                color_tag: color,
                active: active_id == Some(id),
                selected: app.nav.gallery.selected_ids.contains(id),
            });
        };

    for g in &doc.graph_sheets {
        push_if_match(&mut out, &g.id, &g.title, SheetFolderKind::Graphs, "Graph", format!("{} series", g.spec.series.len()), g.color_tag);
    }
    for l in &doc.layout_sheets {
        push_if_match(&mut out, &l.id, &l.title, SheetFolderKind::Layouts, "Layout", format!("{} panels", l.spec.items.len()), l.color_tag);
    }
    for d in &doc.data_sheets {
        push_if_match(&mut out, &d.id, &d.title, SheetFolderKind::Data, "Data", format!("{:?}", d.archetype.kind()), d.color_tag);
    }
    for r in &doc.result_sheets {
        push_if_match(&mut out, &r.id, r.title(), SheetFolderKind::Results, "Result", format!("{} tabs", r.tabs.len()), r.color_tag);
    }
    for i in &doc.info_sheets {
        push_if_match(&mut out, &i.id, &i.title, SheetFolderKind::Info, "Info", format!("{} constants", i.constants.len()), i.color_tag);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_gallery_card(
    ui: &mut egui::Ui,
    doc: &ProjectDocument,
    card: &PortfolioCard,
    visible_ids: &[SheetId],
    selected_ids_snapshot: &std::collections::BTreeSet<SheetId>,
    gallery_anchor: &mut Option<SheetId>,
    size: Vec2,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let (card_rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let bg = if card.active || card.selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.panel_bg
    };
    let highlighted = card.active || card.selected;
    let border_col = if highlighted { tokens.accent } else { tokens.border };
    ui.painter().rect(card_rect, CornerRadius::same(4), bg, Stroke::new(if highlighted { 1.5 } else { 1.0 }, border_col), StrokeKind::Inside);

    let thumb_rect = Rect::from_min_max(card_rect.min + vec2(8.0, 8.0), pos2(card_rect.right() - 8.0, card_rect.bottom() - 38.0));
    paint_gallery_thumbnail(ui, doc, card, thumb_rect, tokens);

    let title_pos = pos2(card_rect.left() + 8.0, card_rect.bottom() - 32.0);
    let max_title_w = (card_rect.width() - 16.0).max(20.0);
    let galley = ui.painter().layout(card.title.clone(), FontId::proportional(11.5), tokens.text_primary, max_title_w);
    ui.painter().with_clip_rect(card_rect.shrink(4.0)).galley(title_pos, galley, tokens.text_primary);

    let sub_pos = pos2(card_rect.left() + 8.0, card_rect.bottom() - 16.0);
    let sub_text = format!("{} · {}", card.badge_label, card.subtitle);
    let sub_galley = ui.painter().layout(sub_text, FontId::proportional(9.5), tokens.text_muted, max_title_w);
    ui.painter().with_clip_rect(card_rect.shrink(4.0)).galley(sub_pos, sub_galley, tokens.text_muted);

    let wid = format!("gallery.card.{}", card.sheet_id.as_str());
    meta.record_response(
        ui,
        &resp,
        inspect_input(
            &wid,
            "button",
            &card.title,
            Some("sheet.select"),
            Some(json!({ "sheet_id": card.sheet_id.as_str() })),
            enabled,
            ZLayer::Workspace,
        ),
    );

    if resp.clicked() && enabled {
        let modifiers = ui.input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|ev| match ev {
                    egui::Event::PointerButton { pressed: false, modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or(i.modifiers)
        });
        if modifiers.command || modifiers.ctrl {
            let mut next = selected_ids_snapshot.clone();
            if !next.remove(&card.sheet_id) {
                next.insert(card.sheet_id.clone());
            }
            *gallery_anchor = Some(card.sheet_id.clone());
            intents.push_set_gallery_selection(next.into_iter().collect());
        } else if modifiers.shift {
            let anchor_pos = gallery_anchor
                .as_ref()
                .and_then(|aid| visible_ids.iter().position(|id| id == aid))
                .or_else(|| visible_ids.iter().position(|id| selected_ids_snapshot.contains(id)))
                .or_else(|| doc.active_sheet_id.as_ref().and_then(|aid| visible_ids.iter().position(|id| id == aid)))
                .unwrap_or(0);
            let click_pos = visible_ids.iter().position(|id| id == &card.sheet_id).unwrap_or(anchor_pos);
            let (lo, hi) = if anchor_pos <= click_pos { (anchor_pos, click_pos) } else { (click_pos, anchor_pos) };
            let range_ids: Vec<SheetId> = visible_ids[lo..=hi].to_vec();
            if gallery_anchor.is_none() {
                *gallery_anchor = visible_ids.get(anchor_pos).cloned();
            }
            intents.push_set_gallery_selection(range_ids);
        } else {
            *gallery_anchor = Some(card.sheet_id.clone());
            intents.push_set_gallery_selection(vec![card.sheet_id.clone()]);
            intents.push_engine("sheet.select", json!({ "sheet_id": card.sheet_id.as_str() }));
            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
        }
    }
}

fn paint_gallery_thumbnail(ui: &egui::Ui, doc: &ProjectDocument, card: &PortfolioCard, rect: Rect, tokens: &Tokens) {
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(rect, CornerRadius::same(2), tokens.surface_bg, Stroke::new(0.8, tokens.border), StrokeKind::Inside);

    match card.folder {
        SheetFolderKind::Graphs => {
            let origin = pos2(rect.left() + 14.0, rect.bottom() - 10.0);
            painter.line_segment([pos2(rect.left() + 14.0, rect.top() + 8.0), origin], Stroke::new(1.0, tokens.text_primary));
            painter.line_segment([origin, pos2(rect.right() - 8.0, origin.y)], Stroke::new(1.0, tokens.text_primary));
            let pts = [
                pos2(rect.left() + 18.0, rect.bottom() - 14.0),
                pos2(rect.center().x - 8.0, rect.center().y - 2.0),
                pos2(rect.center().x + 12.0, rect.top() + 14.0),
                pos2(rect.right() - 12.0, rect.top() + 12.0),
            ];
            for seg in pts.windows(2) {
                painter.line_segment([seg[0], seg[1]], Stroke::new(1.5, tokens.accent));
            }
        }
        SheetFolderKind::Layouts => {
            if let Some(ls) = doc.layout_sheet(&card.sheet_id) {
                let page = ls.spec.page_size_pt();
                let page_w = page.width.max(1.0) as f32;
                let page_h = page.height.max(1.0) as f32;
                for item in ls.spec.items.iter().take(6) {
                    let b = match item {
                        LayoutItem::LiveGraph { bounds_pt, .. }
                        | LayoutItem::LiveTable { bounds_pt, .. }
                        | LayoutItem::TextCallout { bounds_pt, .. }
                        | LayoutItem::EmbeddedImage { bounds_pt, .. } => *bounds_pt,
                    };
                    let sub = Rect::from_min_max(
                        pos2(rect.left() + (b.x as f32 / page_w) * rect.width(), rect.top() + (b.y as f32 / page_h) * rect.height()),
                        pos2(
                            rect.left() + ((b.x + b.width) as f32 / page_w) * rect.width(),
                            rect.top() + ((b.y + b.height) as f32 / page_h) * rect.height(),
                        ),
                    );
                    painter.rect_stroke(sub.intersect(rect.shrink(3.0)), CornerRadius::same(1), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
                }
            }
        }
        SheetFolderKind::Data | SheetFolderKind::Results | SheetFolderKind::Info => {
            for r in 0..4 {
                let y = rect.top() + 10.0 + (r as f32) * 11.0;
                if y < rect.bottom() - 4.0 {
                    painter.line_segment([pos2(rect.left() + 8.0, y), pos2(rect.right() - 8.0, y)], Stroke::new(1.0, tokens.border));
                }
            }
            if let Some(tag) = card.color_tag {
                painter.circle_filled(pos2(rect.right() - 10.0, rect.top() + 10.0), 4.0, rgba8_to_color32(tag));
            }
        }
    }
}

#[derive(Debug, Clone)]
enum BottomBannerKind {
    ResultDiagnostic { sheet_id: SheetId, banner: AssumptionDiagnosticBanner },
    GraphClipping { sheet_id: SheetId, count: usize, details: Vec<String> },
    DataLinkedRegion { sheet_id: SheetId, region_count: usize, upstream_id: Option<SheetId> },
}

fn detect_bottom_banner(app: &UiApp, views: &EguiShellViewState, active_sheet_id: Option<&SheetId>) -> Option<BottomBannerKind> {
    let sheet_id = active_sheet_id?;
    if views.center_stage.dismissed_banner_sheets.contains(sheet_id) || app.egui_views.center_stage.dismissed_banner_sheets.contains(sheet_id) {
        return None;
    }
    let doc = app.document();

    if let Some(rs) = doc.result_sheet(sheet_id)
        && let Some(banner) = AssumptionDiagnosticBanner::from_result(rs)
    {
        return Some(BottomBannerKind::ResultDiagnostic { sheet_id: sheet_id.clone(), banner });
    }

    if let Some(gs) = doc.graph_sheet(sheet_id) {
        let (count, details) = collect_graph_clipping_warnings(app, gs);
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

fn collect_graph_clipping_warnings(app: &UiApp, gs: &GraphSheet) -> (usize, Vec<String>) {
    let mut details = Vec::new();
    if let Ok(scene) = app.session.render_sheet_scene(&gs.id) {
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

#[allow(clippy::too_many_arguments)]
fn render_bottom_banner(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    kind: &BottomBannerKind,
    rect: Rect,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = app.locale.as_str();
    let warn_bg = if tokens.dark { Color32::from_rgb(66, 48, 22) } else { Color32::from_rgb(255, 245, 224) };
    let (bg, border, text_color) = match kind {
        BottomBannerKind::ResultDiagnostic { banner, .. } => {
            let warn = matches!(banner.severity, curvebook_doc::ChecklistStatus::Violated | curvebook_doc::ChecklistStatus::Warn);
            if warn { (warn_bg, WARNING_ACCENT_LIGHT, tokens.text_primary) } else { (tokens.active_bg, tokens.accent, tokens.text_primary) }
        }
        BottomBannerKind::GraphClipping { .. } => (warn_bg, WARNING_ACCENT_LIGHT, tokens.text_primary),
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

    let text_budget = (inner.width() - 250.0).max(90.0);
    let lbl_resp = bar_ui.add_sized([text_budget, inner.height()], egui::Label::new(RichText::new(&summary).small().color(text_color)).truncate());
    let banner_cmd = (wid_prefix == "result.diagnostic_banner").then_some("ui.result.inspectDiagnostics");
    meta.record_response(&bar_ui, &lbl_resp, inspect_input(wid_prefix, "status", &summary, banner_cmd, None, enabled, ZLayer::FloatingOverlay));

    bar_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btns| {
        let dismiss_lbl = stage_tr(locale, "w5.banner.dismiss", "Dismiss");
        let dismiss_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new(dismiss_lbl).small()).min_size(vec2(54.0, 22.0)));
        let dismiss_wid = format!("{wid_prefix}.dismiss");
        meta.record_response(btns, &dismiss_resp, inspect_input(&dismiss_wid, "button", dismiss_lbl, None, None, enabled, ZLayer::FloatingOverlay));
        if dismiss_resp.clicked() && enabled {
            if views.center_stage.banner_details_open.as_ref() == Some(sheet_id) {
                views.center_stage.banner_details_open = None;
            }
            if !views.center_stage.dismissed_banner_sheets.contains(sheet_id) {
                views.center_stage.dismissed_banner_sheets.push(sheet_id.clone());
            }
            intents.push_dismiss_diagnostic_banner(sheet_id.clone());
        }

        if why_text.is_some() {
            let why_open = views.center_stage.banner_details_open.as_ref() == Some(sheet_id);
            let why_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new("Why?").small()).min_size(vec2(42.0, 22.0)));
            let why_wid = format!("{wid_prefix}.why");
            meta.record_response(btns, &why_resp, inspect_input(&why_wid, "button", "Why?", None, None, enabled, ZLayer::FloatingOverlay));
            if why_resp.clicked() && enabled {
                views.center_stage.banner_details_open = if why_open { None } else { Some(sheet_id.clone()) };
            }
        }

        match kind {
            BottomBannerKind::ResultDiagnostic { sheet_id, banner } => {
                let insp_lbl = stage_tr(locale, "w5.banner.inspect", "Inspect Details");
                let insp_resp = btns.add_enabled(enabled, egui::Button::new(RichText::new(insp_lbl).small()).min_size(vec2(86.0, 22.0)));
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
}

#[allow(clippy::too_many_arguments)]
fn render_floating_notes_overlay(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    sheet_id: &SheetId,
    notes: &[FloatingNote],
    stage_bounds: Rect,
    tokens: &Tokens,
    enabled: bool,
    notes_pane_focused: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if stage_bounds.width() < 180.0 || stage_bounds.height() < 60.0 {
        return;
    }
    let doc = app.document();
    let shard = &app.view_shards.nav_notes;
    let first_id = notes.first().map(|n| n.id.as_str().to_owned());
    if views.center_stage.focused_floating_note_id.is_none() {
        views.center_stage.focused_floating_note_id = first_id;
    }

    for (idx, note) in notes.iter().enumerate() {
        let note_id = note.id.as_str().to_owned();
        let key = (sheet_id.clone(), note_id.clone());
        let shard_key = format!("{}:{}", sheet_id.as_str(), note_id);

        let collapsed = views
            .center_stage
            .note_collapsed
            .get(&key)
            .copied()
            .or_else(|| app.nav.floating_notes_collapsed.get(&note_id).copied())
            .unwrap_or(note.minimized);
        let pinned = views.center_stage.note_pinned.get(&key).copied().unwrap_or(false);

        let [raw_w, raw_h] =
            views.center_stage.note_sizes_pt.get(&key).copied().unwrap_or([note.bounds_pt.width as f32, note.bounds_pt.height as f32]);
        let max_w = (stage_bounds.width() - FLOATING_NOTE_MARGIN_PT * 2.0).max(FLOATING_NOTE_MIN_W_PT);
        let max_h = (stage_bounds.height() - FLOATING_NOTE_MARGIN_PT * 2.0).max(FLOATING_NOTE_MIN_H_PT);
        let card_w = raw_w.clamp(FLOATING_NOTE_MIN_W_PT.min(max_w), max_w);
        let card_h =
            if collapsed { FLOATING_NOTE_HEADER_H_PT } else { (raw_h.max(FLOATING_NOTE_MIN_H_PT)).clamp(FLOATING_NOTE_MIN_H_PT.min(max_h), max_h) };

        let [rel_x, rel_y] = views
            .center_stage
            .note_positions_pt
            .get(&key)
            .copied()
            .or_else(|| shard.note_positions_pt.get(&shard_key).map(|p| [p[0] as f32, p[1] as f32]))
            .unwrap_or([note.bounds_pt.x as f32, note.bounds_pt.y as f32]);

        let min_x = stage_bounds.left() + FLOATING_NOTE_MARGIN_PT;
        let max_x = (stage_bounds.right() - card_w - FLOATING_NOTE_MARGIN_PT).max(min_x);
        let min_y = stage_bounds.top() + FLOATING_NOTE_MARGIN_PT;
        let max_y = (stage_bounds.bottom() - card_h - FLOATING_NOTE_MARGIN_PT).max(min_y);

        let pos = pos2((stage_bounds.left() + rel_x).clamp(min_x, max_x), (stage_bounds.top() + rel_y).clamp(min_y, max_y));
        views.center_stage.note_positions_pt.insert(key.clone(), [pos.x - stage_bounds.left(), pos.y - stage_bounds.top()]);

        let card_rect = Rect::from_min_size(pos, vec2(card_w, card_h));
        let fill = if note.fill_color.a == 0 { rgba8_to_color32(FloatingNote::DEFAULT_FILL) } else { rgba8_to_color32(note.fill_color) };
        let is_focused = notes_pane_focused && views.center_stage.focused_floating_note_id.as_deref() == Some(note_id.as_str());

        let order = if enabled { Order::Middle } else { Order::Background };
        egui::Area::new(Id::unique(("center_stage_floating_note", sheet_id.as_str(), note_id.as_str())))
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
                let drag_rect = Rect::from_min_max(hdr_rect.min, pos2((hdr_rect.right() - 56.0).max(hdr_rect.left() + 40.0), hdr_rect.bottom()));
                let drag_resp = area_ui.interact(
                    drag_rect,
                    Id::unique(("floating_note_drag", sheet_id.as_str(), note_id.as_str())),
                    if enabled { Sense::click_and_drag() } else { Sense::hover() },
                );
                drag_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Move Floating Note"));
                if drag_resp.clicked() && enabled {
                    views.center_stage.focused_floating_note_id = Some(note_id.clone());
                    intents.push_set_focus_pane(ActiveWorkspacePane::FloatingNotes);
                }
                if drag_resp.drag_started() && enabled {
                    views.center_stage.active_floating_note_drag = Some((sheet_id.clone(), idx));
                }
                if drag_resp.dragged() && enabled {
                    let delta = drag_resp.drag_delta();
                    let next_x = (pos.x + delta.x).clamp(min_x, max_x) - stage_bounds.left();
                    let next_y = (pos.y + delta.y).clamp(min_y, max_y) - stage_bounds.top();
                    views.center_stage.note_positions_pt.insert(key.clone(), [next_x, next_y]);
                }
                if drag_resp.drag_stopped() && enabled {
                    views.center_stage.active_floating_note_drag = None;
                    if let Some([nx, ny]) = views.center_stage.note_positions_pt.get(&key).copied() {
                        intents.push_custom_view_action("nav_notes", format!("note.set_pos|{}|{}|{nx:.2}|{ny:.2}", sheet_id.as_str(), note_id));
                    }
                }

                area_ui.painter().text(
                    drag_rect.left_center() + vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("Note · {note_id}"),
                    FontId::proportional(11.0),
                    Color32::from_rgb(35, 32, 24),
                );

                let pin_rect = Rect::from_min_size(pos2(hdr_rect.right() - 52.0, hdr_rect.top() + 3.0), vec2(22.0, 20.0));
                let col_rect = Rect::from_min_size(pos2(hdr_rect.right() - 26.0, hdr_rect.top() + 3.0), vec2(22.0, 20.0));

                let pin_resp = area_ui.interact(
                    pin_rect,
                    Id::unique(("floating_note_pin", sheet_id.as_str(), note_id.as_str())),
                    if enabled { Sense::click() } else { Sense::hover() },
                );
                pin_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Pin Floating Note"));
                area_ui.painter().rect_filled(pin_rect, CornerRadius::same(2), if pinned { tokens.active_bg } else { Color32::TRANSPARENT });
                area_ui.painter().text(
                    pin_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if pinned { "P*" } else { "P" },
                    FontId::proportional(10.5),
                    Color32::from_rgb(35, 32, 24),
                );
                if pin_resp.clicked() && enabled {
                    views.center_stage.note_pinned.insert(key.clone(), !pinned);
                }

                let col_resp = area_ui.interact(
                    col_rect,
                    Id::unique(("floating_note_col", sheet_id.as_str(), note_id.as_str())),
                    if enabled { Sense::click() } else { Sense::hover() },
                );
                col_resp.widget_info(move || {
                    egui::WidgetInfo::labeled(egui::Role::Button, enabled, if collapsed { "Expand Note" } else { "Collapse Note" })
                });
                area_ui.painter().text(
                    col_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if collapsed { "+" } else { "−" },
                    FontId::proportional(12.0),
                    Color32::from_rgb(35, 32, 24),
                );
                if col_resp.clicked() && enabled {
                    views.center_stage.note_collapsed.insert(key.clone(), !collapsed);
                    intents.push_custom_view_action("nav_notes", format!("note.toggle|{}|{}", sheet_id.as_str(), note_id));
                }

                if !collapsed {
                    let body_rect = Rect::from_min_max(
                        pos2(frame_rect.left() + 8.0, hdr_rect.bottom() + 4.0),
                        pos2(frame_rect.right() - 8.0, frame_rect.bottom() - 14.0),
                    );
                    let expanded = expand_template_hooks(&note.body_markdown, |tok| resolve_stage_hook(doc, tok));
                    let body_galley =
                        area_ui.painter().layout(expanded, FontId::proportional(11.0), Color32::from_rgb(35, 32, 24), body_rect.width().max(40.0));
                    area_ui.painter().with_clip_rect(body_rect).galley(body_rect.min, body_galley, Color32::from_rgb(35, 32, 24));

                    let resize_rect = Rect::from_min_max(pos2(frame_rect.right() - 14.0, frame_rect.bottom() - 14.0), frame_rect.max);
                    let resize_resp = area_ui.interact(
                        resize_rect,
                        Id::unique(("floating_note_resize", sheet_id.as_str(), note_id.as_str())),
                        if enabled { Sense::drag() } else { Sense::hover() },
                    );
                    resize_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Resize Note"));
                    area_ui.painter().line_segment(
                        [pos2(resize_rect.left() + 3.0, resize_rect.bottom() - 3.0), pos2(resize_rect.right() - 3.0, resize_rect.top() + 3.0)],
                        Stroke::new(1.2, tokens.text_muted),
                    );
                    if resize_resp.dragged() && enabled {
                        let d = resize_resp.drag_delta();
                        let avail_max_w = (stage_bounds.right() - pos.x - FLOATING_NOTE_MARGIN_PT).max(FLOATING_NOTE_MIN_W_PT);
                        let avail_max_h = (stage_bounds.bottom() - pos.y - FLOATING_NOTE_MARGIN_PT).max(FLOATING_NOTE_MIN_H_PT);
                        let next_w = (card_w + d.x).clamp(FLOATING_NOTE_MIN_W_PT.min(avail_max_w), avail_max_w);
                        let next_h = (card_h + d.y).clamp(FLOATING_NOTE_MIN_H_PT.min(avail_max_h), avail_max_h);
                        views.center_stage.note_sizes_pt.insert(key.clone(), [next_w, next_h]);
                        views.center_stage.active_floating_note_resize = Some((sheet_id.clone(), idx));
                    } else if resize_resp.drag_stopped() {
                        views.center_stage.active_floating_note_resize = None;
                    }
                    let resize_wid = format!("floating_note.{}.{}.resize_handle", sheet_id.as_str(), note_id);
                    meta.record_response(
                        area_ui,
                        &resize_resp,
                        inspect_input(&resize_wid, "button", "Resize Note", None, None, enabled, ZLayer::FloatingOverlay),
                    );
                }

                let card_wid = format!("floating_note.{}.{}.card", sheet_id.as_str(), note_id);
                record_region_meta(
                    meta,
                    area_ui,
                    &frame_resp,
                    inspect_input(&card_wid, "region", &note_id, None, None, enabled, ZLayer::FloatingOverlay),
                );
                let drag_wid = format!("floating_note.{}.{}.drag_handle", sheet_id.as_str(), note_id);
                meta.record_response(
                    area_ui,
                    &drag_resp,
                    inspect_input(&drag_wid, "button", "Move Floating Note", None, None, enabled, ZLayer::FloatingOverlay),
                );
                let col_wid = format!("floating_note.{}.{}.collapse", sheet_id.as_str(), note_id);
                meta.record_response(
                    area_ui,
                    &col_resp,
                    inspect_input(
                        &col_wid,
                        "button",
                        if collapsed { "Expand Note" } else { "Collapse Note" },
                        None,
                        None,
                        enabled,
                        ZLayer::FloatingOverlay,
                    ),
                );
                let pin_wid = format!("floating_note.{}.{}.pin", sheet_id.as_str(), note_id);
                meta.record_response(
                    area_ui,
                    &pin_resp,
                    inspect_input(&pin_wid, "button", "Pin Floating Note", None, None, enabled, ZLayer::FloatingOverlay),
                );
            });
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
                body.painter().rect_filled(bar_rect, CornerRadius::same(5), tokens.surface_bg);

                if let Some(frac) = snapshot.progress_0_to_1 {
                    let clamped = frac.clamp(0.0, 1.0);
                    let fill_rect = Rect::from_min_max(bar_rect.min, pos2(bar_rect.left() + bar_rect.width() * clamped, bar_rect.bottom()));
                    body.painter().rect_filled(fill_rect, CornerRadius::same(5), tokens.accent);
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
                    body.painter().rect_filled(pulse_rect, CornerRadius::same(5), tokens.accent);
                    body.ctx().request_repaint();
                }

                if snapshot.cancellable {
                    body.add_space(8.0);
                    let cancel_lbl = stage_tr(app.locale.as_str(), "w5.common.cancel", "Cancel");
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
