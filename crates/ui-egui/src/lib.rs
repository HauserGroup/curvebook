//! Thin desktop and browser UI shell for LumenCraft: project navigator, typed data/results
//! spreadsheet grid, statistical parameter dialogs, graph/layout canvas, menus, localization and
//! local JSON-lines control channel.
//!
//! Layer L6 front-end crate: panels inspect engine state and mutate sessions solely by dispatching
//! commands. Governing spec: `docs/ui-parity.md`, `docs/control-protocol.md`, and `docs/development.md`.
#![forbid(unsafe_code)]

pub mod compositor;
pub mod control_io;
pub mod dialogs_canvas;
pub mod dialogs_r9;
pub mod grid_view;
mod gui_actions;
pub mod human_driver;
pub mod i18n;
pub mod inspect;
pub mod interaction;
pub mod layout;
pub mod menus;
pub mod modal_predicates;
pub mod os_integration;
pub mod power_wizard;
pub mod text_bridge;
pub mod theme;
pub mod views;
pub mod workspace_nav;

pub use lumencraft_chart::{ChartScene, PointPt, PreflightIssue, SourceCellId};
use lumencraft_doc::{ProjectDocument, SheetId, SheetSection, TableKind};
use lumencraft_engine::{CommandOutcome, EngineError, Session};
pub use lumencraft_palette::CanvasPreviewFilter;
pub use lumencraft_render::{RasterImage, RenderError, RenderTargetSpec};
use lumencraft_tools::{Cursor, GridCellCoord, GridVisibleWindow};
pub use serde_json;

pub use compositor::{
    WorkspaceFrameLayout, WorkspacePixelRect, compute_workspace_layout, compute_workspace_layout_map, render_workspace_frame_png,
    render_workspace_rgba,
};
pub use control_io::{
    CONTROL_AUTH_TOKEN_BYTES, CONTROL_HTTP_FORBIDDEN_RESPONSE, ControlAuth, ControlLoopAction, ControlPostRequestContext, ControlRecordedStep,
    ControlRequest, ControlResponse, MAX_CONTROL_LINE_BYTES, MAX_CONTROL_TOKEN_BYTES, PrintJobSpec, PrintSpoolOutput, dispatch_gui_action,
    dispatch_ui_command, handle_control_line, handle_control_line_with_action, is_http_request_line, parse_control_port, serve_control_stream,
    serve_control_stream_with_quit, serve_control_stream_with_quit_and_hook, serve_control_stream_with_quit_and_token,
};
#[cfg(not(target_arch = "wasm32"))]
pub use control_io::{
    bind_control_tcp_listener, run_control_tcp_server, serve_control_listener, serve_control_listener_with_hook, validate_loopback_addr,
};
pub use dialogs_canvas::{
    ActiveDialog, AssumptionDiagnosticBanner, CanvasDialogState, ConvertArchetypeDialogState, ExcludeWithReasonDialogState, PrintDialogState,
    RemediationKind, SplitWorkspaceState,
};
pub use dialogs_r9::{InsertModelEquationDialogState, RandomSampleRowsDialogState, RandomizeGroupsDialogState, SimulateDistributionsDialogState};
pub use grid_view::{GridViewState, ImportReshapeMode, ImportReshapeState};
pub use human_driver::HumanPathDriver;
pub use i18n::{LOCALES, LocalePreset, locale_display_name, normalize_locale, tr};
pub use inspect::{UiTreeSnapshot, inspect_ui_tree};
pub use interaction::{
    CanvasDragTarget, GridHitGeometry, InfoRowHitRect, InlineEditorState, InlineEditorTarget, InteractionControllerState, KeyModifiers,
    LayoutResizeHandle, NavigatorEntryHitRect, PointerButton, ScenePaperMapping, UiEventOutcome, UiInputEvent, ViewportMetrics,
    VirtualWorkspaceFolder, WorkspaceHitTarget, WorkspaceLayoutMap, dispatch_mouse_wheel, dispatch_ui_event, execute_widget_action,
};
pub use layout::{
    AccessibilityMeta, ClipId, FocusScope, LayoutRectPx, LayoutTextRun, LayoutTree, NodeBounds, PaintOp, ParityReport, ParityViolation,
    RetainedLayoutFrame, ScrollAxes, ScrollMeta, WidgetAction, WidgetId, WidgetNode, WidgetRole, WidgetVisualState, ZLayer,
    verify_click_action_parity, verify_pixel_to_hit_box_parity,
};
pub use menus::{
    CommandTooltip, MenuSectionTooltips, ShortcutBinding, ShortcutExecution, ShortcutMatch, ShortcutRegistry, TOOLBAR_BUTTONS, ToolbarButtonSpec,
    ToolbarButtonTooltip, UI_COMMANDS, UiCommandSpec, command_tooltip, menu_bar_tooltips, normalize_shortcut_combo, toolbar_caption_command_id,
    toolbar_tooltips,
};
pub use modal_predicates::ControlEnablement;
pub use os_integration::{
    ClipboardBundle, OsAdapterAvailability, OsAdapterStatus, build_cf_html_payload, discover_os_printers, open_os_print_preview_dialog,
    open_os_print_preview_dialog_titled, read_os_clipboard_pdf_bytes, read_os_clipboard_text,
};
#[cfg(not(target_arch = "wasm32"))]
pub use os_integration::{read_os_pasteboard, submit_os_print_job, submit_os_print_pdf_bytes, write_os_clipboard_bundle, write_os_pasteboard};
pub use power_wizard::{PowerWizardStage, PowerWizardState};
pub use text_bridge::{
    DesktopTextMeasurer, FontFamilyKind, MeasuredTextMetrics, ShapedGlyphRun, ShapedTextLine, SoftwareTextPainter, TextMeasurer, UiTextStyle,
};
pub use theme::{FontBook, UiThemeMode, UiThemeSpec};
pub use workspace_nav::{NavigatorEntry, WorkspaceNavState};

/// Whether the UI crate was compiled with bundled interface/document fonts enabled.
#[must_use]
pub const fn bundled_fonts_enabled() -> bool {
    cfg!(feature = "bundled-fonts")
}

/// Top-level application shell facade owning [`Session`] and panel/tool view state.
/// Compact rollback snapshot of UI shell state excluding `Session` and its undo history.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UiShellSnapshot {
    pub theme: UiThemeSpec,
    pub nav: WorkspaceNavState,
    pub grid: GridViewState,
    pub canvas_dialogs: CanvasDialogState,
    pub shortcuts: ShortcutRegistry,
    pub family_open: bool,
    pub locale: String,
    pub locale_preset: i18n::LocalePreset,
    pub split: SplitWorkspaceState,
    pub brushed_cell: Option<SourceCellId>,
    pub brushed_points: Vec<SourceCellId>,
    pub preview_filter: CanvasPreviewFilter,
    pub presentation_mode: bool,
    pub virtual_workspace: VirtualWorkspaceFolder,
    pub inline_editor: InlineEditorState,
    pub interaction: InteractionControllerState,
    pub hover_command_id: Option<String>,
    pub focus_widget_id: Option<String>,
    pub power_wizards_by_sheet: std::collections::BTreeMap<SheetId, PowerWizardState>,
    pub human_path_only: bool,
}

/// Equality-transparent cache slot storing the latest finalized [`layout::RetainedLayoutFrame`].
#[derive(Debug, Clone, Default)]
pub(crate) struct CachedLayoutSlot(pub Option<layout::RetainedLayoutFrame>);

impl PartialEq for CachedLayoutSlot {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Incremental session rollback log captured for atomic `command.batch` execution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BatchUndoTracker {
    pub net_batch_depth: isize,
    pub baseline_dirty: bool,
    pub baseline_file_path: Option<std::path::PathBuf>,
    pub fallback_session: Option<Session>,
}

impl BatchUndoTracker {
    pub(crate) fn for_session(session: &Session) -> Self {
        Self {
            net_batch_depth: 0,
            baseline_dirty: session.is_dirty(),
            baseline_file_path: session.file_path().map(std::path::Path::to_path_buf),
            fallback_session: None,
        }
    }

    pub(crate) fn capture_fallback_baseline_before(&mut self, session: &Session, force: bool) {
        if self.fallback_session.is_some() || !force {
            return;
        }
        let mut snap = session.clone();
        if self.net_batch_depth < 0 {
            for _ in 0..(-self.net_batch_depth) {
                let _ = snap.redo();
            }
        } else if self.net_batch_depth > 0 {
            for _ in 0..self.net_batch_depth {
                let _ = snap.undo();
            }
        }
        snap.set_file_path(self.baseline_file_path.clone(), !self.baseline_dirty);
        self.fallback_session = Some(snap);
    }

    pub(crate) fn merge_committed_child(&mut self, child: Self) {
        if self.fallback_session.is_none() {
            if let Some(child_fallback) = child.fallback_session {
                let mut snap = child_fallback;
                if self.net_batch_depth < 0 {
                    for _ in 0..(-self.net_batch_depth) {
                        let _ = snap.redo();
                    }
                } else if self.net_batch_depth > 0 {
                    for _ in 0..self.net_batch_depth {
                        let _ = snap.undo();
                    }
                }
                snap.set_file_path(self.baseline_file_path.clone(), !self.baseline_dirty);
                self.fallback_session = Some(snap);
            } else {
                self.net_batch_depth += child.net_batch_depth;
            }
        }
    }
}

/// Top-level application state driving the LumenCraft native desktop UI.
#[derive(Debug, Clone, PartialEq)]
pub struct UiApp {
    pub(crate) session: Session,
    pub theme: UiThemeSpec,
    pub nav: WorkspaceNavState,
    pub grid: GridViewState,
    pub canvas_dialogs: CanvasDialogState,
    pub shortcuts: ShortcutRegistry,
    pub family_open: bool,
    pub locale: String,
    pub locale_preset: i18n::LocalePreset,
    pub split: SplitWorkspaceState,
    pub brushed_cell: Option<SourceCellId>,
    pub brushed_points: Vec<SourceCellId>,
    pub preview_filter: CanvasPreviewFilter,
    pub presentation_mode: bool,
    pub virtual_workspace: VirtualWorkspaceFolder,
    pub inline_editor: InlineEditorState,
    pub interaction: InteractionControllerState,
    pub hover_command_id: Option<String>,
    pub focus_widget_id: Option<String>,
    pub power_wizards_by_sheet: std::collections::BTreeMap<SheetId, PowerWizardState>,
    pub human_path_only: bool,
    pub(crate) retained_layout: CachedLayoutSlot,
    pub(crate) batch_undo_log: Option<BatchUndoTracker>,
}

impl Default for UiApp {
    fn default() -> Self {
        Self::new()
    }
}

impl UiApp {
    /// Construct a new UI application facade with an empty project session.
    #[must_use]
    pub fn new() -> Self {
        Self::from_session(Session::new())
    }

    /// Construct a UI application facade wrapping an existing [`Session`].
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        let mut app = Self {
            session,
            theme: UiThemeSpec::default(),
            nav: WorkspaceNavState::default(),
            grid: GridViewState::default(),
            canvas_dialogs: CanvasDialogState::default(),
            shortcuts: ShortcutRegistry::new(),
            family_open: true,
            locale: String::from("en"),
            locale_preset: i18n::LocalePreset::default(),
            split: SplitWorkspaceState::default(),
            brushed_cell: None,
            brushed_points: Vec::new(),
            preview_filter: CanvasPreviewFilter::default(),
            presentation_mode: false,
            virtual_workspace: VirtualWorkspaceFolder::default(),
            inline_editor: InlineEditorState::default(),
            interaction: InteractionControllerState::default(),
            hover_command_id: None,
            focus_widget_id: None,
            power_wizards_by_sheet: std::collections::BTreeMap::new(),
            human_path_only: false,
            retained_layout: CachedLayoutSlot::default(),
            batch_undo_log: None,
        };
        app.grid.apply_locale_preset(&app.locale_preset);
        app.sync_views();
        app
    }

    /// Capture only the UI shell fields (excluding `Session` and its undo history) for lightweight rollback.
    pub(crate) fn snapshot_ui_shell(&self) -> UiShellSnapshot {
        UiShellSnapshot {
            theme: self.theme.clone(),
            nav: self.nav.clone(),
            grid: self.grid.clone(),
            canvas_dialogs: self.canvas_dialogs.clone(),
            shortcuts: self.shortcuts.clone(),
            family_open: self.family_open,
            locale: self.locale.clone(),
            locale_preset: self.locale_preset.clone(),
            split: self.split.clone(),
            brushed_cell: self.brushed_cell.clone(),
            brushed_points: self.brushed_points.clone(),
            preview_filter: self.preview_filter,
            presentation_mode: self.presentation_mode,
            virtual_workspace: self.virtual_workspace.clone(),
            inline_editor: self.inline_editor.clone(),
            interaction: self.interaction.clone(),
            hover_command_id: self.hover_command_id.clone(),
            focus_widget_id: self.focus_widget_id.clone(),
            power_wizards_by_sheet: self.power_wizards_by_sheet.clone(),
            human_path_only: self.human_path_only,
        }
    }

    /// Restore UI shell fields captured by [`Self::snapshot_ui_shell`] without disturbing `Session`.
    pub(crate) fn restore_ui_shell(&mut self, snap: UiShellSnapshot) {
        self.theme = snap.theme;
        self.nav = snap.nav;
        self.grid = snap.grid;
        self.canvas_dialogs = snap.canvas_dialogs;
        self.shortcuts = snap.shortcuts;
        self.family_open = snap.family_open;
        self.locale = snap.locale;
        self.locale_preset = snap.locale_preset;
        self.split = snap.split;
        self.brushed_cell = snap.brushed_cell;
        self.brushed_points = snap.brushed_points;
        self.preview_filter = snap.preview_filter;
        self.presentation_mode = snap.presentation_mode;
        self.virtual_workspace = snap.virtual_workspace;
        self.inline_editor = snap.inline_editor;
        self.interaction = snap.interaction;
        self.hover_command_id = snap.hover_command_id;
        self.focus_widget_id = snap.focus_widget_id;
        self.power_wizards_by_sheet = snap.power_wizards_by_sheet;
        self.human_path_only = snap.human_path_only;
        self.retained_layout.0 = None;
        self.sync_views();
    }

    fn record_session_cmd_on(
        session: &mut Session,
        mut tracker: Option<&mut BatchUndoTracker>,
        command_id: &str,
        op: impl FnOnce(&mut Session) -> Result<CommandOutcome, EngineError>,
    ) -> Result<CommandOutcome, EngineError> {
        if let Some(t) = tracker.as_deref_mut() {
            let is_undo_or_redo = matches!(command_id, "edit.undo" | "edit.redo");
            let force_fallback = (t.baseline_dirty && command_id == "project.save") || (t.net_batch_depth < 0 && !is_undo_or_redo);
            t.capture_fallback_baseline_before(session, force_fallback);
        }
        let outcome = op(session)?;
        if let Some(t) = tracker
            && t.fallback_session.is_none()
        {
            if command_id == "edit.undo" {
                if outcome.mutated {
                    t.net_batch_depth -= 1;
                }
            } else if outcome.mutated {
                t.net_batch_depth += 1;
            }
        }
        Ok(outcome)
    }

    pub(crate) fn execute_session_ui_cmd(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        Self::record_session_cmd_on(&mut self.session, self.batch_undo_log.as_mut(), command_id, |session| session.execute_as_ui(command_id, params))
    }

    pub(crate) fn execute_power_wizard_create(&mut self) -> Result<power_wizard::PowerWizardCreatedSheets, EngineError> {
        let wizard = &mut self.canvas_dialogs.power_wizard;
        let session = &mut self.session;
        let tracker = &mut self.batch_undo_log;
        wizard.execute_create_with(&mut |cmd_id, params| {
            Self::record_session_cmd_on(session, tracker.as_mut(), cmd_id, |s| s.execute_as_ui(cmd_id, params))
        })
    }

    /// Update active UI locale and synchronize its regional numeric/CSV [`LocalePreset`].
    pub fn set_locale(&mut self, locale: &str) -> &str {
        let normalized = i18n::normalize_locale(locale);
        self.locale = normalized.to_owned();
        self.locale_preset = i18n::LocalePreset::for_locale(normalized);
        self.grid.apply_locale_preset(&self.locale_preset);
        &self.locale
    }

    /// Update active UI theme mode (`Light`, `Dark`, or `HighContrast`).
    pub fn set_theme_mode(&mut self, mode: UiThemeMode) {
        self.theme = UiThemeSpec::for_mode(mode);
    }

    /// Look up the localized UI string for `key` under the currently selected locale.
    #[must_use]
    pub fn tr(&self, key: &str) -> &'static str {
        i18n::tr(&self.locale, key)
    }

    /// Borrow the underlying headless [`Session`].
    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    /// Mutably borrow the underlying headless [`Session`].
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Borrow the active [`ProjectDocument`] stored in [`Session`].
    #[must_use]
    pub fn document(&self) -> &ProjectDocument {
        self.session.document()
    }

    /// Return whether strict `--gui-only` enforcement is active on the underlying [`Session`].
    #[must_use]
    pub fn is_gui_only(&self) -> bool {
        self.session.gui_only() == lumencraft_engine::GuiOnlyPolicy::Strict
    }

    /// Enable or disable strict `--gui-only` enforcement on the underlying [`Session`].
    pub fn set_gui_only(&mut self, gui_only: bool) {
        let policy = if gui_only { lumencraft_engine::GuiOnlyPolicy::Strict } else { lumencraft_engine::GuiOnlyPolicy::Disabled };
        self.session.set_gui_only(policy);
    }

    /// Return whether `R10.2` human-path-only enforcement is active (`HumanPathDriver` / parity tests).
    #[must_use]
    pub const fn is_human_path_only(&self) -> bool {
        self.human_path_only
    }

    /// Enable or disable `R10.2` human-path-only enforcement rejecting synthetic semantic bypasses.
    pub fn set_human_path_only(&mut self, enabled: bool) {
        self.human_path_only = enabled;
    }

    /// Build or return the cached retained [`layout::LayoutTree`] for `metrics`.
    pub fn build_and_cache_layout_tree(&mut self, metrics: ViewportMetrics) -> layout::LayoutTree {
        let hash = layout::compute_ui_state_fingerprint(self, metrics);
        if let Some(cached) = &self.retained_layout.0
            && cached.metrics == metrics
            && cached.state_hash == hash
        {
            return cached.tree.clone();
        }
        let tree = layout::LayoutTree::build(self, metrics);
        self.retained_layout.0 = Some(layout::RetainedLayoutFrame { metrics, state_hash: hash, tree: tree.clone() });
        tree
    }

    /// Borrow the cached [`layout::LayoutTree`] if clean, or build a fresh one for immutable callers.
    #[must_use]
    pub fn layout_tree_cow(&self, metrics: ViewportMetrics) -> std::borrow::Cow<'_, layout::LayoutTree> {
        let hash = layout::compute_ui_state_fingerprint(self, metrics);
        if let Some(cached) = &self.retained_layout.0
            && cached.metrics == metrics
            && cached.state_hash == hash
        {
            return std::borrow::Cow::Borrowed(&cached.tree);
        }
        std::borrow::Cow::Owned(layout::LayoutTree::build(self, metrics))
    }

    /// Execute an internal UI command (`Caller::Ui`) and refresh UI navigator and selection state.
    pub fn execute_command(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let outcome = Self::record_session_cmd_on(&mut self.session, self.batch_undo_log.as_mut(), command_id, |session| {
            session.execute_as_ui(command_id, params)
        })?;
        self.sync_views();
        Ok(outcome)
    }

    /// Execute an external control-protocol command (`Caller::Control`) respecting `--gui-only` locks.
    pub fn execute_control_command(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let outcome = Self::record_session_cmd_on(&mut self.session, self.batch_undo_log.as_mut(), command_id, |session| {
            session.execute_with_caller(command_id, params, lumencraft_engine::Caller::Control, &cancelled)
        })?;
        self.sync_views();
        Ok(outcome)
    }

    /// Process a single JSON-lines control-protocol request line and serialize the JSON reply.
    #[must_use]
    pub fn dispatch_control_line(&mut self, line: &str) -> String {
        handle_control_line(self, line)
    }

    /// Process a single JSON-lines control request and return `(json_reply, loop_action)`.
    #[must_use]
    pub fn dispatch_control_line_with_action(&mut self, line: &str) -> (String, ControlLoopAction) {
        handle_control_line_with_action(self, line)
    }

    /// Render the full application frame deterministically into an RGBA [`RasterImage`].
    ///
    /// # Errors
    /// Returns [`RenderError`] if the target dimensions or scene rasterization fail.
    pub fn render_rgba(&self, target: RenderTargetSpec) -> Result<RasterImage, RenderError> {
        compositor::render_workspace_rgba(self, target)
    }

    /// Render the workspace at the physical HiDPI dimensions described by `metrics`.
    ///
    /// # Errors
    /// Returns [`RenderError`] if rasterization fails.
    pub fn render_rgba_for_metrics(&self, metrics: ViewportMetrics, base_dpi: u16) -> Result<RasterImage, RenderError> {
        compositor::render_workspace_rgba(self, metrics.to_render_target_spec(base_dpi))
    }

    /// Compute interactive pixel/RTL geometry for `metrics`.
    #[must_use]
    pub fn layout_map(&self, metrics: ViewportMetrics) -> WorkspaceLayoutMap {
        compositor::compute_workspace_layout_map(self, metrics)
    }

    /// Dispatch a HiDPI interactive [`UiInputEvent`] through the interaction controller (`G6`, `M7`).
    ///
    /// # Errors
    /// Returns a descriptive error string when a triggered engine or UI command fails.
    pub fn dispatch_ui_event(&mut self, event: UiInputEvent, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
        interaction::dispatch_ui_event(self, event, metrics)
    }

    /// Synchronize [`Self::brushed_cell`] and [`Self::brushed_points`] from a grid cell selection (`QOL-5`).
    pub fn brush_from_grid_cell(&mut self, data_sheet_id: &SheetId, coord: GridCellCoord) {
        let brushed = SourceCellId { sheet_id: data_sheet_id.clone(), row: coord.row, col: coord.col, subcol: coord.subcol };
        self.brushed_cell = Some(brushed.clone());
        self.brushed_points = vec![brushed];
    }

    /// Synchronize [`Self::brushed_cell`] and [`Self::brushed_points`] from all currently selected
    /// rows/cells on `data_sheet_id` (`WF-22`).
    pub fn sync_brush_from_grid_selection(&mut self, data_sheet_id: &SheetId) {
        let Some(sheet) = self.session.document().data_sheet(data_sheet_id) else {
            return;
        };
        let coords = self.grid.all_selected_coords(sheet);
        let points: Vec<SourceCellId> =
            coords.into_iter().map(|c| SourceCellId { sheet_id: data_sheet_id.clone(), row: c.row, col: c.col, subcol: c.subcol }).collect();
        self.brushed_cell = points.first().cloned();
        self.brushed_points = points;
    }

    /// Hit-test a graph/layout scene coordinate (`PointPt`) and mirror the matching data cell into
    /// [`Self::brushed_cell`] and [`Self::brushed_points`] (`QOL-5`).
    pub fn brush_point_from_graph_pt(&mut self, graph_sheet_id: &SheetId, scene_pt: PointPt, tolerance_pt: f64) -> Option<SourceCellId> {
        let scene = self.session.render_sheet_scene(graph_sheet_id).ok()?;
        let hit = scene.hit_test_point(scene_pt, tolerance_pt)?;
        let brushed = hit.source_cell.clone();
        if let Some(ds) = self.session.document().data_sheet(&brushed.sheet_id)
            && self.session.document().active_sheet_id.as_ref() == Some(&brushed.sheet_id)
        {
            let _ = ds;
            self.grid.select_single_sync(GridCellCoord::new(brushed.row, brushed.col, brushed.subcol));
        }
        self.brushed_cell = Some(brushed.clone());
        self.brushed_points = vec![brushed.clone()];
        Some(brushed)
    }

    /// Clear any active data/graph point brush highlight (`QOL-5`).
    pub fn clear_brush(&mut self) {
        self.brushed_cell = None;
        self.brushed_points.clear();
    }

    /// Open the interactive `Print...` dialog for the active (or first printable) sheet (`R5.4`).
    pub fn open_print_dialog(&mut self) {
        let active = self.session.document().active_sheet_id.clone();
        self.open_print_dialog_for(active.as_ref());
    }

    /// Open the interactive `Print...` dialog targeting `sheet_id` (`R5.4`).
    pub fn open_print_dialog_for(&mut self, sheet_id: Option<&SheetId>) {
        let printers = os_integration::discover_os_printers();
        self.canvas_dialogs.open_print_dialog(self.session.document(), sheet_id, printers);
    }

    /// Update desired print copy count (`1..=999`) inside `canvas_dialogs.print`.
    pub fn update_print_dialog_copies(&mut self, copies: u16) -> u16 {
        self.canvas_dialogs.print.update_copies(copies)
    }

    /// Select an OS printer queue name (`None` for host default) inside `canvas_dialogs.print`.
    pub fn select_print_dialog_printer(&mut self, printer: Option<String>) {
        self.canvas_dialogs.print.select_printer(printer);
    }

    /// Update print scale percentage (`10.0..=400.0`) inside `canvas_dialogs.print`.
    pub fn update_print_dialog_scale(&mut self, scale_percent: f64) {
        self.canvas_dialogs.print.set_scale_percent(scale_percent);
    }

    /// Toggle publication crop marks and optional bleed margin on `canvas_dialogs.print`.
    pub fn update_print_dialog_crop_marks(&mut self, crop_marks: bool, bleed_pt: Option<f64>) {
        self.canvas_dialogs.print.job.crop_marks = crop_marks;
        if let Some(bleed) = bleed_pt
            && bleed.is_finite()
            && bleed >= 0.0
        {
            self.canvas_dialogs.print.job.bleed_pt = bleed;
        }
    }

    /// Spool the active `PrintDialogState` into a physical-scale PDF and optionally submit to the OS spooler.
    ///
    /// # Errors
    /// Returns an error if no printable sheet exists, PDF generation fails, or OS spool submission fails.
    pub fn execute_print_dialog_spool(&mut self, submit_to_os: bool) -> Result<PrintSpoolOutput, String> {
        let spool = self.canvas_dialogs.print.job.spool_pdf(&self.session)?;
        self.canvas_dialogs.print.last_spool_bytes = spool.pdf_bytes.len();
        let copies = self.canvas_dialogs.print.copies.clamp(1, 999);
        if submit_to_os {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let printer = self.canvas_dialogs.print.selected_printer.as_deref();
                let status = os_integration::submit_os_print_job(&spool, printer)?;
                self.canvas_dialogs.print.last_status =
                    Some(format!("{} ({copies} copy/copies, {} page(s), {} bytes)", status.detail, spool.page_count, spool.pdf_bytes.len()));
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.canvas_dialogs.print.last_status =
                    Some(format!("Spooled {} bytes ({copies} copy/copies); OS print queue unavailable on wasm32", spool.pdf_bytes.len()));
            }
        } else {
            self.canvas_dialogs.print.last_status = Some(format!(
                "Spooled {} bytes ({} page(s), {:.0}% scale, {copies} copy/copies)",
                spool.pdf_bytes.len(),
                spool.page_count,
                spool.scale_percent
            ));
        }
        Ok(spool)
    }

    /// Spool the active `PrintDialogState` and stage a `<title>.pdf` preview via [`open_os_print_preview_dialog_titled`].
    ///
    /// # Errors
    /// Returns an error if spooling or staging fails.
    pub fn preview_print_dialog_spool(&mut self) -> Result<OsAdapterStatus, String> {
        let spool = self.execute_print_dialog_spool(false)?;
        let title = self.canvas_dialogs.print.job.title.as_deref().unwrap_or("lumencraft-print");
        let status = os_integration::open_os_print_preview_dialog_titled(&spool, title)?;
        self.canvas_dialogs.print.last_status = Some(status.detail.clone());
        Ok(status)
    }

    /// Preview a table archetype conversion cleanly via `table.convertArchetype` (`QOL-1`).
    ///
    /// # Errors
    /// Returns a descriptive error string when preview execution fails.
    pub fn preview_convert_archetype(&mut self, sheet_id: &SheetId, target_kind: TableKind) -> Result<(), String> {
        {
            let ds = self.session.document().data_sheet(sheet_id).ok_or_else(|| format!("data sheet `{}` not found", sheet_id.as_str()))?;
            self.canvas_dialogs.convert_archetype.open_for_sheet(ds);
        }
        self.canvas_dialogs.convert_archetype.set_target_kind(target_kind);
        let intent = self
            .canvas_dialogs
            .convert_archetype
            .preview_intent()
            .ok_or_else(|| String::from("no data sheet bound for archetype conversion preview"))?;
        let outcome = self.execute_session_ui_cmd(&intent.command_id, intent.params).map_err(|err| err.to_string())?;
        self.canvas_dialogs.convert_archetype.apply_preview_payload(&outcome.payload);
        self.canvas_dialogs.active_dialog = ActiveDialog::ConvertArchetype { sheet_id: sheet_id.clone() };
        Ok(())
    }

    /// Commit the active table archetype conversion after any required `allow_lossy` opt-in (`QOL-1`).
    ///
    /// # Errors
    /// Returns an error if lossy conversion is unconfirmed or engine execution fails.
    pub fn commit_convert_archetype(&mut self) -> Result<CommandOutcome, String> {
        let intent = self.canvas_dialogs.convert_archetype.commit_intent()?;
        let outcome = self.execute_command(&intent.command_id, intent.params).map_err(|err| err.to_string())?;
        self.canvas_dialogs.close_dialog();
        Ok(outcome)
    }

    /// Save the active [`ProjectDocument`] into [`Self::virtual_workspace`] (`QOL-10`, WASM-clean).
    ///
    /// # Errors
    /// Returns an error string if serialization fails.
    pub fn save_to_virtual_workspace(&mut self, file_name: &str) -> Result<usize, String> {
        self.virtual_workspace.save_project_archive(file_name, self.session.document())
    }

    /// Load a `.lumen` archive from [`Self::virtual_workspace`] into the active [`Session`] (`QOL-10`).
    ///
    /// # Errors
    /// Returns an error string if the entry is missing or fails validation.
    pub fn load_from_virtual_workspace(&mut self, file_name: &str) -> Result<(), String> {
        let doc = self.virtual_workspace.open_project_archive(file_name)?;
        if let Some(tracker) = self.batch_undo_log.as_mut() {
            tracker.capture_fallback_baseline_before(&self.session, true);
        }
        self.session = Session::from_document(doc);
        self.sync_views();
        Ok(())
    }

    /// Dispatch a keyboard shortcut combination (such as `"F6"`, `"Shift+F6"`, or a customized
    /// binding) through [`Self::shortcuts`].
    ///
    /// Returns `Ok(None)` when no command is bound to the normalized key combination.
    ///
    /// # Errors
    /// Returns a descriptive error string if executing the matched UI or engine command fails.
    pub fn handle_key_combo(&mut self, combo: &str) -> Result<Option<ShortcutExecution>, String> {
        let Some(matched) = self.shortcuts.handle_key_combo(combo) else {
            return Ok(None);
        };
        if let Some(ui_result) = control_io::dispatch_ui_command(self, &matched.command_id, &matched.params)? {
            return Ok(Some(ShortcutExecution { command_id: matched.command_id, ui: true, result: ui_result }));
        }
        let outcome = self.execute_command(&matched.command_id, matched.params).map_err(|err| err.to_string())?;
        let result = serde_json::to_value(outcome).map_err(|err| format!("failed to serialize command outcome: {err}"))?;
        Ok(Some(ShortcutExecution { command_id: matched.command_id, ui: false, result }))
    }

    /// Synchronize navigator entries, family pane links, and status-bar statistics from `Session`.
    pub fn sync_views(&mut self) {
        self.nav.sync_from_document(self.session.document());
        self.grid.refresh_selection_stats(self.session.document());
    }

    /// Visible row/column slice when the active sheet is a data table.
    #[must_use]
    pub fn visible_grid_window(&self) -> Option<GridVisibleWindow> {
        self.grid.visible_window(self.session.document())
    }

    /// Active logical cursor determined by the currently selected sheet section and tool mode.
    #[must_use]
    pub fn active_cursor(&self) -> Cursor {
        let doc = self.session.document();
        match doc.active_sheet_id.as_ref().and_then(|id| doc.sheet_section(id)) {
            Some(SheetSection::Data | SheetSection::Analyses) => self.grid.tool.cursor(),
            Some(SheetSection::Graphs | SheetSection::Layouts) => self.canvas_dialogs.canvas.cursor(),
            Some(SheetSection::Info) | None => Cursor::Select,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use lumencraft_doc::{CellCoord, SheetCellRange};
    use serde_json::{Value, json};

    use super::menus::ui_commands::UI_COMMANDS;
    use super::{ClipboardBundle, ControlAuth, PrintJobSpec, UiApp, parse_control_port, serve_control_stream};

    #[test]
    fn ui_commands_table_covers_parity_surface() {
        assert_eq!(UI_COMMANDS.len(), 45);
        for required in [
            "ui.navigator.toggle",
            "ui.inspector.toggle",
            "ui.family.toggle",
            "ui.palette.open",
            "ui.analyze.open",
            "ui.changeGraphType.open",
            "ui.clonePipeline.open",
            "ui.zoom.fitSheet",
            "ui.gallery.toggle",
            "ui.rulers.toggle",
            "ui.pane.cycle",
            "ui.shortcut.set",
            "ui.shortcut.reset",
            "ui.locale.set",
            "ui.theme.set",
            "ui.screenshot",
            "ui.event.dispatch",
            "ui.split.toggle",
            "ui.brush.point",
            "ui.result.copyMethodsReport",
            "ui.result.applyRemediation",
            "ui.canvas.applyPreset",
            "ui.canvas.preflight",
            "ui.previewFilter.set",
            "ui.presentation.toggle",
            "ui.grid.excludeWithReason",
            "ui.table.convertArchetype.open",
            "ui.import.reshape.open",
            "app.about",
        ] {
            assert!(UI_COMMANDS.iter().any(|spec| spec.id == required), "missing UI command {required}");
        }
        let locale_doc = UI_COMMANDS.iter().find(|spec| spec.id == "ui.locale.set").map(|spec| spec.params_doc).unwrap_or("");
        for tag in ["it", "nl", "pl", "ru", "tr", "ar", "he"] {
            assert!(locale_doc.contains(tag), "ui.locale.set params doc missing `{tag}`: {locale_doc}");
        }
    }

    #[test]
    fn ui_app_facade_and_control_line_smoke() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let reply = app.dispatch_control_line("{\"id\":\"1\",\"method\":\"ui.navigator.toggle\"}");
        let v: Value = serde_json::from_str(&reply)?;
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["id"], json!("1"));
        assert!(!app.nav.navigator_open);

        let create = app.dispatch_control_line(
            "{\"id\":\"2\",\"method\":\"sheet.createData\",\"params\":{\"sheet_id\":\"data-1\",\"title\":\"Assay\",\"kind\":\"xy\",\"auto_graph\":true}}",
        );
        let c: Value = serde_json::from_str(&create)?;
        assert_eq!(c["ok"], json!(true));
        assert!(!app.nav.entries.is_empty());
        assert!(app.visible_grid_window().is_some());
        Ok(())
    }

    #[test]
    fn control_commands_ui_parity_clipboard_and_print_spool() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();

        // Populate data + graph
        let script = [
            json!({"id": 1, "op": "ping"}),
            json!({"id": 2, "op": "command.list"}),
            json!({"id": 3, "op": "command.run", "command": "sheet.createData", "params": {"sheet_id": "xy-1", "graph_sheet_id": "graph-xy-1", "title": "Dose", "kind": "xy", "data_sets": 1, "rows": 3, "auto_graph": true}}),
            json!({"id": 4, "op": "table.setXCells", "params": {"sheet_id": "xy-1", "values": [1.0, 2.0, 3.0]}}),
            json!({"id": 5, "op": "table.setCell", "params": {"sheet_id": "xy-1", "row": 0, "col": 0, "subcol": 0, "value": 10.0}}),
            json!({"id": 6, "op": "table.setCell", "params": {"sheet_id": "xy-1", "row": 1, "col": 0, "subcol": 0, "value": 20.0}}),
            json!({"id": 7, "op": "table.setCell", "params": {"sheet_id": "xy-1", "row": 2, "col": 0, "subcol": 0, "value": 30.0}}),
            json!({"id": 8, "op": "ui.family.toggle"}),
            json!({"id": 9, "op": "ui.palette.open"}),
            json!({"id": 10, "op": "ui.analyze.open", "params": {"sheet_id": "xy-1"}}),
            json!({"id": 11, "op": "ui.changeGraphType.open", "params": {"sheet_id": "graph-xy-1"}}),
            json!({"id": 12, "op": "ui.clonePipeline.open", "params": {"sheet_id": "xy-1"}}),
            json!({"id": 13, "op": "ui.zoom.fitSheet"}),
            json!({"id": 14, "op": "ui.locale.set", "params": {"locale": "de"}}),
            json!({"id": 15, "op": "ui.theme.set", "params": {"mode": "dark"}}),
            json!({"id": 16, "op": "ui.screenshot", "params": {"width_px": 256, "height_px": 160, "dpi": 72}}),
            json!({"id": 17, "op": "render.svg", "sheetId": "graph-xy-1"}),
            json!({"id": 18, "op": "render.pdf", "sheetId": "graph-xy-1"}),
            json!({"id": 19, "op": "doc.inspect"}),
            json!({"id": 20, "op": "ui.state"}),
            json!({"id": 21, "op": "app.about"}),
            json!({"id": 22, "op": "app.quit"}),
        ];

        let auth = ControlAuth::from_token("sample-lib-parity-token")?;
        let mut input = format!("{{\"id\": 0, \"method\": \"auth\", \"params\": {{\"token\": \"{}\"}}}}\n", auth.as_str());
        for req in &script {
            input.push_str(&serde_json::to_string(req)?);
            input.push('\n');
        }
        input.push_str("{\"id\": 99, \"op\": \"ping\"}\n");

        let mut reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut output = Vec::new();
        serve_control_stream(&mut app, &mut reader, &mut output, &auth)?;

        let out_str = String::from_utf8(output)?;
        let lines: Vec<&str> = out_str.lines().collect();
        assert_eq!(lines.len(), script.len() + 1, "stream must stop immediately after app.quit");
        for line in &lines {
            let parsed: Value = serde_json::from_str(line)?;
            assert_eq!(parsed["ok"], json!(true), "unexpected error response: {parsed}");
        }

        let screenshot: Value = serde_json::from_str(lines[16])?;
        assert_eq!(screenshot["result"]["format"], json!("png"));
        assert_eq!(screenshot["result"]["width_px"], json!(256));
        assert!(screenshot["result"]["data_base64"].as_str().is_some_and(|b64| b64.starts_with("iVBORw0K")));

        let svg_resp: Value = serde_json::from_str(lines[17])?;
        assert!(svg_resp["result"]["svg"].as_str().is_some_and(|s| s.contains("<svg")));

        let ui_state: Value = serde_json::from_str(lines[20])?;
        assert_eq!(ui_state["result"]["locale"], json!("de"));
        assert_eq!(ui_state["result"]["theme_mode"], json!("dark"));
        assert_eq!(ui_state["result"]["family_open"], json!(false));

        // Hardware payloads
        let graph_id = app.document().graph_sheets.first().map(|g| g.id.clone()).ok_or("missing auto graph")?;
        let bundle = ClipboardBundle::from_sheet(app.session(), &graph_id, 144)?;
        assert!(bundle.pdf_bytes.starts_with(b"%PDF-"));
        assert!(bundle.svg.contains("<svg"));
        assert!(bundle.html.contains("<div class=\"lumencraft-figure\""));
        assert!(bundle.png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

        let data_sheet = app.document().data_sheets.first().ok_or("missing data sheet")?;
        let tsv = ClipboardBundle::tsv_from_datasheet(data_sheet, ',')?;
        assert!(tsv.contains('\t'));
        let block = ClipboardBundle::tsv_from_datasheet_range(data_sheet, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 1, 0)), '.');
        assert!(block.contains("10"));

        let print_out = PrintJobSpec::publication_100_percent().with_crop_and_bleed(9.0).spool_pdf(app.session())?;
        assert!(print_out.pdf_bytes.starts_with(b"%PDF-"));
        assert_eq!(print_out.page_count, 1);
        assert!((print_out.scale_percent - 100.0).abs() < 1e-9);

        assert!(parse_control_port("0").is_err());
        assert_eq!(parse_control_port("9001").ok(), Some(9001));
        Ok(())
    }

    #[test]
    fn atomic_batch_rolls_back_state_on_failure() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let req = json!({
            "id": "b1",
            "method": "command.batch",
            "params": {
                "atomic": true,
                "commands": [
                    {"command": "sheet.createData", "params": {"sheet_id": "temp-1", "title": "Temp"}},
                    {"command": "nonexistent.command", "params": {}}
                ]
            }
        });
        let reply = app.dispatch_control_line(&serde_json::to_string(&req)?);
        let v: Value = serde_json::from_str(&reply)?;
        assert_eq!(v["ok"], json!(false));
        assert_eq!(app.document().total_sheet_count(), 0);
        Ok(())
    }
}
