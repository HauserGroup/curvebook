//! Thin desktop and browser UI shell for CurveBook: project navigator, typed data/results
//! spreadsheet grid, statistical parameter dialogs, graph/layout canvas, menus, localization and
//! local JSON-lines control channel.
//!
//! Layer L6 front-end crate: panels inspect engine state and mutate sessions solely by dispatching
//! commands. Governing spec: `docs/evidence/user-interface.md`, `docs/control-protocol.md`, and `docs/development.md`.
#![forbid(unsafe_code)]

pub mod control_io;
pub mod dialogs_canvas;
pub mod dialogs_r9;
#[cfg(not(target_arch = "wasm32"))]
pub mod egui_shell;
pub mod feedback;
pub mod grid_view;
mod gui_actions;
pub mod i18n;
pub mod inspect;
pub mod interaction;
pub mod menus;
pub mod modal_predicates;
pub mod os_integration;
pub mod power_wizard;
pub mod text_bridge;
pub mod theme;
pub mod views;
pub mod workspace_nav;

pub use curvebook_chart::{ChartScene, PointPt, PreflightIssue, SourceCellId};
use curvebook_doc::{ProjectDocument, SheetId, SheetSection, TableKind};
use curvebook_engine::{CommandOutcome, EngineError, Session};
pub use curvebook_palette::CanvasPreviewFilter;
pub use curvebook_render::{RasterImage, RenderError, RenderTargetSpec};
use curvebook_tools::{Cursor, GridCellCoord, GridVisibleWindow};
pub use serde_json;

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
pub use feedback::{
    BuiltFeedbackUrl, FeedbackCategory, FeedbackDiagnostics, FeedbackDraft, FeedbackSurface, GITHUB_ISSUES_NEW_BASE, MAX_GITHUB_ISSUE_URL_BYTES,
    build_feedback_markdown, build_github_issue_url, build_github_issue_url_capped, percent_encode_query_component, validate_github_issue_url,
};
pub use grid_view::{GridViewState, ImportReshapeMode, ImportReshapeState};
pub use i18n::{
    BETA_ACTIVE_LOCALES, LOCALES, LocalePreset, is_beta_active_locale, is_draft_translation_locale, locale_display_name, locale_picker_label,
    normalize_beta_gui_locale, normalize_locale, tr,
};
pub use inspect::{ParityReport, ParityViolation, ScreenshotRegionTarget, UiTreeSnapshot, ZLayer, inspect_ui_tree, inspect_ui_tree_for_metrics};
pub use interaction::{
    CanvasDragTarget, InlineEditorState, InlineEditorTarget, InteractionControllerState, KeyModifiers, LayoutResizeHandle, PointerButton,
    UiEventOutcome, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder, dispatch_mouse_wheel, dispatch_ui_event,
};
pub use menus::{
    CommandTooltip, MenuSectionTooltips, ShortcutBinding, ShortcutExecution, ShortcutMatch, ShortcutRegistry, ShortcutScope, TOOLBAR_BUTTONS,
    ToolbarButtonSpec, ToolbarButtonTooltip, UI_COMMANDS, UiCommandSpec, command_tooltip, menu_bar_tooltips, normalize_shortcut_combo,
    toolbar_caption_command_id, toolbar_tooltips,
};
pub use modal_predicates::ControlEnablement;
pub use os_integration::{
    ClipboardBundle, OsAdapterAvailability, OsAdapterStatus, PlatformClipboardAdapter, PlatformUrlOpener, StubPlatformClipboardAdapter,
    StubPlatformUrlOpener, SystemPlatformClipboardAdapter, SystemPlatformUrlOpener, build_cf_html_payload, discover_os_printers,
    open_github_issue_url, open_os_print_preview_dialog, open_os_print_preview_dialog_titled, read_os_clipboard_pdf_bytes, read_os_clipboard_text,
};
#[cfg(not(target_arch = "wasm32"))]
pub use os_integration::{read_os_pasteboard, submit_os_print_job, submit_os_print_pdf_bytes, write_os_clipboard_bundle, write_os_pasteboard};
pub use power_wizard::{PowerWizardStage, PowerWizardState};
pub use text_bridge::{DesktopTextMeasurer, FontFamilyKind, MeasuredTextMetrics, ShapedGlyphRun, ShapedTextLine, TextMeasurer, UiTextStyle};
pub use theme::{FontBook, UiThemeMode, UiThemeSpec};
pub use workspace_nav::{NavigatorEntry, WorkspaceNavState};

/// Whether the UI crate was compiled with bundled interface/document fonts enabled.
#[must_use]
pub const fn bundled_fonts_enabled() -> bool {
    cfg!(feature = "bundled-fonts")
}

/// Default interface zoom scale factor (`1.0x`).
pub const DEFAULT_UI_SCALE: f32 = 1.0;

/// Minimum allowed interface zoom scale factor (`75%`).
pub const MIN_UI_SCALE: f32 = 0.75;

/// Maximum allowed interface zoom scale factor (`300%`).
pub const MAX_UI_SCALE: f32 = 3.0;

/// Discrete step applied by `Cmd/Ctrl +` and `Cmd/Ctrl -` (`10%`).
pub const UI_SCALE_STEP: f32 = 0.1;

/// Current persisted `preferences.json` schema version supported for `ui_scale` migration.
pub const CURRENT_UI_PREFERENCES_VERSION: u32 = 1;

/// Clamp and quantize a requested `ui_scale` into `[MIN_UI_SCALE, MAX_UI_SCALE]` (2 decimal places).
#[must_use]
pub fn clamp_ui_scale(scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0 {
        return DEFAULT_UI_SCALE;
    }
    let clamped = scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
    (clamped * 100.0).round() / 100.0
}

/// Extract and migrate `ui_scale` (`"ui_scale"` or legacy v0 `"scale"`/`"uiScale"`) from a
/// `PreferencesFile.extra` map.
///
/// # Errors
/// Returns an error if a present `ui_scale` entry is not a finite positive JSON number.
pub fn extract_ui_scale_from_preferences_extra(extra: &serde_json::Map<String, serde_json::Value>) -> Result<Option<f32>, String> {
    let raw_val = extra.get("ui_scale").or_else(|| extra.get("scale")).or_else(|| extra.get("uiScale"));
    let Some(val) = raw_val else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(num) = val.as_f64() else {
        return Err(String::from("`ui_scale` preference must be a finite positive number"));
    };
    if !num.is_finite() || num <= 0.0 {
        return Err(format!("`ui_scale` preference `{num}` must be finite and positive"));
    }
    #[allow(clippy::cast_possible_truncation)]
    let scale_f32 = num as f32;
    Ok(Some(clamp_ui_scale(scale_f32)))
}

/// Store migrated `ui_scale` cleanly inside a `PreferencesFile.extra` map, clearing legacy v0 keys.
pub fn write_ui_scale_to_preferences_extra(extra: &mut serde_json::Map<String, serde_json::Value>, scale: f32) {
    extra.remove("scale");
    extra.remove("uiScale");
    let clamped = clamp_ui_scale(scale);
    if (clamped - DEFAULT_UI_SCALE).abs() > 0.001 {
        let rounded_f64 = (f64::from(clamped) * 100.0).round() / 100.0;
        extra.insert(String::from("ui_scale"), serde_json::json!(rounded_f64));
    } else {
        extra.remove("ui_scale");
    }
}

/// Migrate a raw `preferences.json` JSON object (`v0` -> `v1`), extracting normalized `ui_scale`
/// while refusing any schema version newer than [`CURRENT_UI_PREFERENCES_VERSION`].
///
/// # Errors
/// Returns an error if the root value is not a JSON object, its `version` field is invalid,
/// `version > CURRENT_UI_PREFERENCES_VERSION`, or `ui_scale` is malformed.
pub fn migrate_preferences_value_with_ui_scale(raw: &serde_json::Value) -> Result<(serde_json::Value, f32), String> {
    let mut obj = raw.as_object().cloned().ok_or_else(|| String::from("preferences root must be a JSON object"))?;
    let version = match obj.get("version") {
        None => 0_u64,
        Some(v) => v.as_u64().ok_or_else(|| String::from("preferences `version` must be an unsigned integer"))?,
    };
    if version > u64::from(CURRENT_UI_PREFERENCES_VERSION) {
        return Err(format!(
            "preferences schema version {version} is newer than supported version {CURRENT_UI_PREFERENCES_VERSION}; refusing to migrate or overwrite"
        ));
    }
    let scale = extract_ui_scale_from_preferences_extra(&obj)?.unwrap_or(DEFAULT_UI_SCALE);
    obj.insert(String::from("version"), serde_json::json!(CURRENT_UI_PREFERENCES_VERSION));
    write_ui_scale_to_preferences_extra(&mut obj, scale);
    Ok((serde_json::Value::Object(obj), scale))
}

#[cfg(not(target_arch = "wasm32"))]
/// Custom hook intercepting `ui.*` actions when an external `egui` host/driver is attached to [`UiApp`].
pub type EguiDispatchHook =
    std::sync::Arc<dyn Fn(&mut UiApp, &str, &serde_json::Value) -> Option<Result<serde_json::Value, EngineError>> + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Default)]
pub(crate) struct EguiDispatchHookSlot(pub Option<EguiDispatchHook>);

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Debug for EguiDispatchHookSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EguiDispatchHookSlot").field(&self.0.is_some()).finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PartialEq for EguiDispatchHookSlot {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
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
    pub host_draws_menu_bar: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub ui_backend: egui_shell::UiBackend,
    #[cfg(not(target_arch = "wasm32"))]
    pub egui_views: egui_shell::EguiShellViewState,
    pub view_shards: views::ViewShardStates,
    pub clipboard_adapter: ClipboardAdapterSlot,
    #[cfg(not(target_arch = "wasm32"))]
    pub egui_dispatch_hook: EguiDispatchHookSlot,
    pub mutation_seq: MutationCounter,
}

/// Equality-transparent monotonic mutation counter tracking UI/session mutations even across undo eviction.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MutationCounter(pub u64);

impl PartialEq for MutationCounter {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Equality-transparent holder for [`UiApp`]'s active [`PlatformClipboardAdapter`].
#[derive(Debug, Clone)]
pub(crate) struct ClipboardAdapterSlot {
    pub adapter: std::sync::Arc<dyn PlatformClipboardAdapter>,
    pub is_custom: bool,
}

impl Default for ClipboardAdapterSlot {
    fn default() -> Self {
        Self { adapter: std::sync::Arc::new(os_integration::SystemPlatformClipboardAdapter), is_custom: false }
    }
}

impl PartialEq for ClipboardAdapterSlot {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Equality-transparent holder for [`UiApp`]'s active [`PlatformUrlOpener`].
#[derive(Debug, Clone)]
pub(crate) struct UrlOpenerSlot {
    pub opener: std::sync::Arc<dyn PlatformUrlOpener>,
    pub is_custom: bool,
}

impl Default for UrlOpenerSlot {
    fn default() -> Self {
        Self { opener: std::sync::Arc::new(os_integration::SystemPlatformUrlOpener::guarded()), is_custom: false }
    }
}

impl PartialEq for UrlOpenerSlot {
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

/// Top-level application state driving the CurveBook native desktop UI.
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
    pub host_draws_menu_bar: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub ui_backend: egui_shell::UiBackend,
    #[cfg(not(target_arch = "wasm32"))]
    pub egui_views: egui_shell::EguiShellViewState,
    pub view_shards: views::ViewShardStates,
    pub(crate) clipboard_adapter: ClipboardAdapterSlot,
    pub(crate) url_opener: UrlOpenerSlot,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) egui_dispatch_hook: EguiDispatchHookSlot,
    pub(crate) batch_undo_log: Option<BatchUndoTracker>,
    pub(crate) mutation_seq: MutationCounter,
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
            host_draws_menu_bar: false,
            #[cfg(not(target_arch = "wasm32"))]
            ui_backend: egui_shell::UiBackend::Egui,
            #[cfg(not(target_arch = "wasm32"))]
            egui_views: egui_shell::EguiShellViewState::default(),
            view_shards: views::ViewShardStates::default(),
            clipboard_adapter: ClipboardAdapterSlot::default(),
            url_opener: UrlOpenerSlot::default(),
            #[cfg(not(target_arch = "wasm32"))]
            egui_dispatch_hook: EguiDispatchHookSlot::default(),
            batch_undo_log: None,
            mutation_seq: MutationCounter(0),
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
            host_draws_menu_bar: self.host_draws_menu_bar,
            #[cfg(not(target_arch = "wasm32"))]
            ui_backend: self.ui_backend,
            #[cfg(not(target_arch = "wasm32"))]
            egui_views: self.egui_views.clone(),
            view_shards: self.view_shards.clone(),
            clipboard_adapter: self.clipboard_adapter.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            egui_dispatch_hook: self.egui_dispatch_hook.clone(),
            mutation_seq: self.mutation_seq,
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
        self.host_draws_menu_bar = snap.host_draws_menu_bar;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ui_backend = snap.ui_backend;
            self.egui_views = snap.egui_views;
            self.egui_dispatch_hook = snap.egui_dispatch_hook;
        }
        self.view_shards = snap.view_shards;
        self.clipboard_adapter = snap.clipboard_adapter;
        self.mutation_seq = snap.mutation_seq;
        self.sync_views();
    }

    fn record_session_cmd_on(
        session: &mut Session,
        mut tracker: Option<&mut BatchUndoTracker>,
        mutation_seq: &mut MutationCounter,
        command_id: &str,
        op: impl FnOnce(&mut Session) -> Result<CommandOutcome, EngineError>,
    ) -> Result<CommandOutcome, EngineError> {
        if let Some(t) = tracker.as_deref_mut() {
            let is_undo_or_redo = matches!(command_id, "edit.undo" | "edit.redo");
            let force_fallback = (t.baseline_dirty && command_id == "project.save") || (t.net_batch_depth < 0 && !is_undo_or_redo);
            t.capture_fallback_baseline_before(session, force_fallback);
        }
        let outcome = op(session)?;
        if outcome.mutated {
            mutation_seq.0 = mutation_seq.0.wrapping_add(1);
        }
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
        Self::record_session_cmd_on(&mut self.session, self.batch_undo_log.as_mut(), &mut self.mutation_seq, command_id, |session| {
            session.execute_as_ui(command_id, params)
        })
    }

    pub(crate) fn execute_power_wizard_create(&mut self) -> Result<power_wizard::PowerWizardCreatedSheets, EngineError> {
        let wizard = &mut self.canvas_dialogs.power_wizard;
        let session = &mut self.session;
        let tracker = &mut self.batch_undo_log;
        let mutation_seq = &mut self.mutation_seq;
        wizard.execute_create_with(&mut |cmd_id, params| {
            Self::record_session_cmd_on(session, tracker.as_mut(), mutation_seq, cmd_id, |s| s.execute_as_ui(cmd_id, params))
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

    /// Update active UI theme mode (`Light`, `Dark`, or `HighContrast`) while preserving `ui_scale`.
    pub fn set_theme_mode(&mut self, mode: UiThemeMode) {
        let preserved_scale = clamp_ui_scale(self.theme.ui_scale);
        self.theme = UiThemeSpec::for_mode(mode);
        self.theme.ui_scale = preserved_scale;
    }

    /// Return the current clamped user interface scale factor (`0.75..=3.0`, default `1.0`).
    #[must_use]
    pub fn ui_scale(&self) -> f32 {
        clamp_ui_scale(self.theme.ui_scale)
    }

    /// Set the user interface scale factor (`0.75..=3.0`) and return the clamped value.
    pub fn set_ui_scale(&mut self, scale: f32) -> f32 {
        let clamped = clamp_ui_scale(scale);
        self.theme.ui_scale = clamped;
        clamped
    }

    /// Increase user interface zoom scale by [`UI_SCALE_STEP`] (`Cmd/Ctrl +`).
    pub fn zoom_ui_scale_in(&mut self) -> f32 {
        self.set_ui_scale(self.ui_scale() + UI_SCALE_STEP)
    }

    /// Decrease user interface zoom scale by [`UI_SCALE_STEP`] (`Cmd/Ctrl -`).
    pub fn zoom_ui_scale_out(&mut self) -> f32 {
        self.set_ui_scale(self.ui_scale() - UI_SCALE_STEP)
    }

    /// Reset user interface zoom scale to [`DEFAULT_UI_SCALE`] (`1.0x`, `Cmd/Ctrl 0`).
    pub fn reset_ui_scale(&mut self) -> f32 {
        self.set_ui_scale(DEFAULT_UI_SCALE)
    }

    /// Active desktop UI rendering/dispatch backend (`Legacy` default, or `Egui`).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn ui_backend(&self) -> egui_shell::UiBackend {
        self.ui_backend
    }

    /// Switch the active UI backend between `Legacy` and `Egui`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_ui_backend(&mut self, backend: egui_shell::UiBackend) {
        self.ui_backend = backend;
    }

    /// Builder-style helper setting [`Self::ui_backend`].
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn with_ui_backend(mut self, backend: egui_shell::UiBackend) -> Self {
        self.ui_backend = backend;
        self
    }

    /// Install a custom `egui` dispatch hook overriding control `ui.*` routing on this [`UiApp`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_egui_dispatch_hook(&mut self, hook: EguiDispatchHook) {
        self.egui_dispatch_hook = EguiDispatchHookSlot(Some(hook));
    }

    /// Remove any previously installed [`EguiDispatchHook`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn clear_egui_dispatch_hook(&mut self) {
        self.egui_dispatch_hook = EguiDispatchHookSlot(None);
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

    /// Monotonic counter incremented on every mutating engine/workspace transition.
    #[must_use]
    pub const fn mutation_seq(&self) -> u64 {
        self.mutation_seq.0
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
        self.session.gui_only() == curvebook_engine::GuiOnlyPolicy::Strict
    }

    /// Enable or disable strict `--gui-only` enforcement on the underlying [`Session`].
    pub fn set_gui_only(&mut self, gui_only: bool) {
        let policy = if gui_only { curvebook_engine::GuiOnlyPolicy::Strict } else { curvebook_engine::GuiOnlyPolicy::Disabled };
        self.session.set_gui_only(policy);
    }

    /// Return whether `R10.2` human-path-only enforcement is active (`EguiHumanPathDriver` / parity tests).
    #[must_use]
    pub const fn is_human_path_only(&self) -> bool {
        self.human_path_only
    }

    /// Enable or disable `R10.2` human-path-only enforcement rejecting synthetic semantic bypasses.
    pub fn set_human_path_only(&mut self, enabled: bool) {
        self.human_path_only = enabled;
    }

    /// Install a custom [`PlatformClipboardAdapter`] (such as [`StubPlatformClipboardAdapter`])
    /// for deterministic clipboard capture without touching the host OS pasteboard.
    pub fn set_clipboard_adapter(&mut self, adapter: std::sync::Arc<dyn PlatformClipboardAdapter>) {
        self.clipboard_adapter = ClipboardAdapterSlot { adapter, is_custom: true };
    }

    /// Borrow the currently configured [`PlatformClipboardAdapter`].
    #[must_use]
    pub fn clipboard_adapter(&self) -> &dyn PlatformClipboardAdapter {
        self.clipboard_adapter.adapter.as_ref()
    }

    /// Return whether a caller explicitly installed a custom [`PlatformClipboardAdapter`].
    #[must_use]
    pub const fn has_custom_clipboard_adapter(&self) -> bool {
        self.clipboard_adapter.is_custom
    }

    /// Install a custom [`PlatformUrlOpener`] (such as [`SystemPlatformUrlOpener::live`] for
    /// interactive desktop windows or [`StubPlatformUrlOpener`] for deterministic tests).
    pub fn set_url_opener(&mut self, opener: std::sync::Arc<dyn PlatformUrlOpener>) {
        self.url_opener = UrlOpenerSlot { opener, is_custom: true };
    }

    /// Borrow the currently configured [`PlatformUrlOpener`].
    #[must_use]
    pub fn url_opener(&self) -> &dyn PlatformUrlOpener {
        self.url_opener.opener.as_ref()
    }

    /// Return whether a caller explicitly installed a custom [`PlatformUrlOpener`].
    #[must_use]
    pub const fn has_custom_url_opener(&self) -> bool {
        self.url_opener.is_custom
    }

    /// Capture strictly non-identifying environment diagnostics for the active feedback form.
    #[must_use]
    pub fn feedback_diagnostics(&self) -> FeedbackDiagnostics {
        FeedbackDiagnostics::from_app(self)
    }

    /// Build the prefilled GitHub new-issue URL for the current `canvas_dialogs.feedback` draft.
    #[must_use]
    pub fn build_feedback_issue_url(&self) -> BuiltFeedbackUrl {
        let diag = self.canvas_dialogs.feedback.include_diagnostics.then(|| self.feedback_diagnostics());
        build_github_issue_url(&self.canvas_dialogs.feedback, diag.as_ref())
    }

    /// Build the complete, untruncated Markdown report for the current `canvas_dialogs.feedback` draft.
    #[must_use]
    pub fn build_feedback_markdown(&self) -> String {
        let diag = self.canvas_dialogs.feedback.include_diagnostics.then(|| self.feedback_diagnostics());
        build_feedback_markdown(&self.canvas_dialogs.feedback, diag.as_ref())
    }

    /// Validate and open the current feedback draft as a prefilled GitHub issue in the configured
    /// [`PlatformUrlOpener`], updating `canvas_dialogs.feedback` status fields.
    ///
    /// # Errors
    /// Returns an error if URL validation fails or the configured opener returns an error.
    pub fn open_feedback_on_github(&mut self) -> Result<(BuiltFeedbackUrl, OsAdapterStatus), String> {
        let built = self.build_feedback_issue_url();
        let status = if !self.url_opener.is_custom && (self.is_gui_only() || self.is_human_path_only()) {
            validate_github_issue_url(&built.url)?;
            OsAdapterStatus::unavailable("os-browser", "browser launch disabled under gui_only without custom URL opener")
        } else {
            self.url_opener.opener.open_external_url(&built.url)?
        };
        self.canvas_dialogs.feedback.last_built_url = Some(built.url.clone());
        self.canvas_dialogs.feedback.last_url_truncated = built.truncated;
        self.canvas_dialogs.feedback.status_message = Some(if status.available {
            if built.truncated {
                String::from("Opened GitHub issue form (trimmed for URL length — use Copy Markdown for full text).")
            } else {
                String::from("Opened GitHub issue form in your browser.")
            }
        } else {
            format!("Browser unavailable ({}). Use Copy Markdown to paste into GitHub.", status.detail)
        });
        Ok((built, status))
    }

    /// Copy the complete Markdown feedback report to the configured [`PlatformClipboardAdapter`].
    ///
    /// # Errors
    /// Returns an error if the clipboard adapter reports an error.
    pub fn copy_feedback_markdown(&mut self) -> Result<(String, OsAdapterStatus), String> {
        let markdown = self.build_feedback_markdown();
        let status = self.write_clipboard_text(&markdown)?;
        self.canvas_dialogs.feedback.copied_markdown = true;
        self.canvas_dialogs.feedback.status_message = Some(if status.available {
            String::from("Copied Markdown feedback report to clipboard.")
        } else {
            String::from("Prepared Markdown feedback report (host clipboard unavailable in this session).")
        });
        Ok((markdown, status))
    }

    pub(crate) fn write_clipboard_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
        if !self.clipboard_adapter.is_custom && (self.is_gui_only() || self.is_human_path_only()) {
            return Ok(OsAdapterStatus::unavailable("os-pasteboard", "live OS pasteboard disabled under gui_only without stub adapter"));
        }
        self.clipboard_adapter.adapter.write_text(text)
    }

    pub(crate) fn write_clipboard_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        if !self.clipboard_adapter.is_custom && (self.is_gui_only() || self.is_human_path_only()) {
            return Ok(OsAdapterStatus::unavailable("os-pasteboard", "live OS pasteboard disabled under gui_only without stub adapter"));
        }
        self.clipboard_adapter.adapter.write_bundle(bundle)
    }

    pub(crate) fn read_clipboard_text(&self) -> Result<OsAdapterStatus, String> {
        if !self.clipboard_adapter.is_custom && (self.is_gui_only() || self.is_human_path_only()) {
            return Ok(OsAdapterStatus::unavailable("os-pasteboard", "live OS pasteboard disabled under gui_only without stub adapter"));
        }
        self.clipboard_adapter.adapter.read_text()
    }

    pub(crate) fn read_clipboard_image(&self) -> Result<Option<Vec<u8>>, String> {
        if !self.clipboard_adapter.is_custom && (self.is_gui_only() || self.is_human_path_only()) {
            return Ok(None);
        }
        self.clipboard_adapter.adapter.read_image_bytes()
    }

    pub(crate) fn toggle_navigator_pane(&mut self) -> bool {
        self.nav.navigator_open = !self.nav.navigator_open;
        if !self.nav.navigator_open {
            if self.focus_widget_id.as_deref().is_some_and(|id| id.starts_with("nav.") || id.starts_with("navigator.") || id.starts_with("family.")) {
                self.focus_widget_id = None;
            }
            if matches!(self.nav.active_pane, workspace_nav::ActiveWorkspacePane::Navigator | workspace_nav::ActiveWorkspacePane::FamilyStrip) {
                self.nav.active_pane = workspace_nav::ActiveWorkspacePane::MainCanvasOrGrid;
            }
        }
        self.nav.navigator_open
    }

    pub(crate) fn toggle_inspector_pane(&mut self) -> bool {
        self.nav.inspector_open = !self.nav.inspector_open;
        if !self.nav.inspector_open {
            if self.focus_widget_id.as_deref().is_some_and(|id| id.starts_with("inspector.")) {
                self.focus_widget_id = None;
            }
            if self.nav.active_pane == workspace_nav::ActiveWorkspacePane::Inspector {
                self.nav.active_pane = workspace_nav::ActiveWorkspacePane::MainCanvasOrGrid;
            }
        }
        self.nav.inspector_open
    }

    pub(crate) fn toggle_family_pane(&mut self) -> bool {
        self.family_open = !self.family_open;
        if !self.family_open {
            if self.focus_widget_id.as_deref().is_some_and(|id| id.starts_with("family.")) {
                self.focus_widget_id = None;
            }
            if self.nav.active_pane == workspace_nav::ActiveWorkspacePane::FamilyStrip {
                self.nav.active_pane = if self.nav.navigator_open {
                    workspace_nav::ActiveWorkspacePane::Navigator
                } else {
                    workspace_nav::ActiveWorkspacePane::MainCanvasOrGrid
                };
            }
        }
        self.family_open
    }

    /// Return whether `pane` is currently visible in the workspace layout.
    #[must_use]
    pub fn is_workspace_pane_visible(&self, pane: workspace_nav::ActiveWorkspacePane) -> bool {
        if self.presentation_mode {
            return matches!(pane, workspace_nav::ActiveWorkspacePane::MainCanvasOrGrid);
        }
        match pane {
            workspace_nav::ActiveWorkspacePane::Navigator => self.nav.navigator_open,
            workspace_nav::ActiveWorkspacePane::FamilyStrip => self.nav.navigator_open && self.family_open,
            workspace_nav::ActiveWorkspacePane::MainCanvasOrGrid => true,
            workspace_nav::ActiveWorkspacePane::Inspector => self.nav.inspector_open,
            workspace_nav::ActiveWorkspacePane::FloatingNotes => {
                !self.nav.gallery.open
                    && !self.nav.welcome.open
                    && self.document().total_sheet_count() > 0
                    && self
                        .document()
                        .active_sheet_id
                        .as_ref()
                        .is_some_and(|id| !views::nav_notes::sheet_floating_notes(self.document(), id).is_empty())
            }
        }
    }

    /// Ordered list of currently visible workspace focus panes.
    #[must_use]
    pub fn visible_workspace_panes(&self) -> Vec<workspace_nav::ActiveWorkspacePane> {
        workspace_nav::ActiveWorkspacePane::CYCLE_ORDER.into_iter().filter(|&p| self.is_workspace_pane_visible(p)).collect()
    }

    /// Cycle keyboard focus across currently visible workspace panes (`F6` / `Shift+F6`).
    pub fn cycle_focus_pane(&mut self, reverse: bool) -> workspace_nav::ActiveWorkspacePane {
        let visible = self.visible_workspace_panes();
        if visible.is_empty() {
            self.nav.active_pane = workspace_nav::ActiveWorkspacePane::MainCanvasOrGrid;
            return self.nav.active_pane;
        }
        let next = if let Some(pos) = visible.iter().position(|&p| p == self.nav.active_pane) {
            let idx = if reverse { (pos + visible.len() - 1) % visible.len() } else { (pos + 1) % visible.len() };
            visible[idx]
        } else if reverse {
            visible[visible.len() - 1]
        } else {
            visible[0]
        };
        self.nav.active_pane = next;
        self.focus_widget_id = None;
        next
    }

    /// Execute an internal UI command (`Caller::Ui`) and refresh UI navigator and selection state.
    pub fn execute_command(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let outcome = Self::record_session_cmd_on(&mut self.session, self.batch_undo_log.as_mut(), &mut self.mutation_seq, command_id, |session| {
            session.execute_as_ui(command_id, params)
        })?;
        self.sync_views();
        Ok(outcome)
    }

    /// Execute an external control-protocol command (`Caller::Control`) respecting `--gui-only` locks.
    pub fn execute_control_command(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let outcome = Self::record_session_cmd_on(&mut self.session, self.batch_undo_log.as_mut(), &mut self.mutation_seq, command_id, |session| {
            session.execute_with_caller(command_id, params, curvebook_engine::Caller::Control, &cancelled)
        })?;
        self.sync_views();
        Ok(outcome)
    }

    /// Process a single JSON-lines control-protocol request line and serialize the JSON reply.
    #[must_use]
    pub fn dispatch_control_line(&mut self, line: &str) -> String {
        self.dispatch_control_line_with_action(line).0
    }

    /// Process a single JSON-lines control request and return `(json_reply, loop_action)`.
    #[must_use]
    pub fn dispatch_control_line_with_action(&mut self, line: &str) -> (String, ControlLoopAction) {
        #[cfg(not(target_arch = "wasm32"))]
        if line.len() <= MAX_CONTROL_LINE_BYTES
            && let Ok(req) = serde_json::from_str::<ControlRequest>(line.trim())
            && let Some(res) = self.try_dispatch_egui_backend(&req.method, &req.params)
        {
            let reply = encode_egui_control_response(req.id, res);
            if let Ok(json_line) = serde_json::to_string(&reply) {
                return (json_line, ControlLoopAction::Continue);
            }
        }
        handle_control_line_with_action(self, line)
    }

    /// Drive an offscreen `egui` frame until `!ctx.will_discard()` settles, executing queued
    /// [`egui_shell::PendingUiIntents`] strictly once after settlement.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the layout fails to settle within 8 passes or committing
    /// settled UI/engine intents fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_settled_egui_shell_frame(
        &mut self,
        logical_size_pt: [f32; 2],
        pixels_per_point: f32,
        rasterizer: Option<&mut egui_shell::rasterizer::EguiSoftwareRasterizer>,
        build_ui: impl FnMut(&mut egui::Ui, &mut Self, &mut egui_shell::FrameInspectMeta, &mut egui_shell::PendingUiIntents),
    ) -> Result<(egui::Context, egui::FullOutput, egui_shell::FrameInspectMeta, egui_shell::GeometryStamp), EngineError> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
        let (output, meta, stamp) = run_settled_egui_on_ctx(&ctx, self, logical_size_pt, pixels_per_point, rasterizer, build_ui)?;
        Ok((ctx, output, meta, stamp))
    }

    /// Build a settled `ui.inspect` [`UiTreeSnapshot`] through [`egui_shell::inspect_bridge`] on the
    /// minimal shell / `About` view.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn inspect_egui_shell_snapshot(&mut self, metrics: ViewportMetrics, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
        self.sync_views();
        #[allow(clippy::cast_possible_truncation)]
        let w_pt = metrics.logical_width.max(1.0) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let h_pt = metrics.logical_height.max(1.0) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let ppp = metrics.clamped_scale_factor() as f32;
        if let Ok((_, mut output, meta, _stamp)) = self.run_settled_egui_shell_frame([w_pt, h_pt], ppp, None, |ui, app_ref, meta, intents| {
            let mut views = std::mem::take(&mut app_ref.egui_views);
            let _ = egui_shell::inspect_bridge::render_egui_shell_frame(ui, app_ref, &mut views, meta, intents);
            app_ref.egui_views = views;
        }) {
            output.textures_delta.clear();
            egui_shell::inspect_bridge::build_ui_tree_snapshot_from_egui(
                self,
                &meta,
                output.platform_output.accesskit_update.as_ref(),
                scope,
                include_bounds,
                metrics,
            )
        } else {
            inspect::inspect_semantic_snapshot(self, metrics, scope, include_bounds)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn capture_settled_egui_shell_output(
        &mut self,
        target: RenderTargetSpec,
        rasterizer: &mut egui_shell::rasterizer::EguiSoftwareRasterizer,
    ) -> Result<(egui::Context, egui::FullOutput, egui_shell::rasterizer::EguiScreenshotRegions, egui::Color32), RenderError> {
        let ppp = egui_shell::rasterizer::pixels_per_point_for_dpi(target.dpi);
        #[allow(clippy::cast_precision_loss)]
        let w_pt = (target.width_px.max(1) as f32) / ppp;
        #[allow(clippy::cast_precision_loss)]
        let h_pt = (target.height_px.max(1) as f32) / ppp;
        let mut regions = egui_shell::rasterizer::EguiScreenshotRegions::default();
        let prev_sync_primary = self.egui_views.canvas_view.synchronous_raster;
        let prev_sync_secondary = self.egui_views.center_stage.secondary_canvas_view.synchronous_raster;
        self.egui_views.canvas_view.synchronous_raster = true;
        self.egui_views.canvas_view.slot.cached_texture = None;
        self.egui_views.center_stage.secondary_canvas_view.synchronous_raster = true;
        self.egui_views.center_stage.secondary_canvas_view.slot.cached_texture = None;
        let run_res = self.run_settled_egui_shell_frame([w_pt, h_pt], ppp, Some(rasterizer), |ui, app_ref, meta, intents| {
            let mut views = std::mem::take(&mut app_ref.egui_views);
            let rep = egui_shell::inspect_bridge::render_egui_shell_frame(ui, app_ref, &mut views, meta, intents);
            app_ref.egui_views = views;
            regions = rep.regions;
        });
        self.egui_views.canvas_view.synchronous_raster = prev_sync_primary;
        self.egui_views.canvas_view.slot.cached_texture = None;
        self.egui_views.center_stage.secondary_canvas_view.synchronous_raster = prev_sync_secondary;
        self.egui_views.center_stage.secondary_canvas_view.slot.cached_texture = None;
        let (ctx, output, _meta, _stamp) = run_res.map_err(|err| RenderError::InvalidTarget { message: err.to_string() })?;
        let tokens = egui_shell::Tokens::from_theme_spec(&self.theme, &self.locale);
        Ok((ctx, output, regions, tokens.surface_bg))
    }

    /// Render a settled `egui` shell frame into a straight-RGBA [`RasterImage`] cropped to `region`.
    ///
    /// # Errors
    /// Returns [`RenderError`] when screenshot dimensions fail validation or the requested crop
    /// region (such as `Dialog` when no modal is open) is unavailable.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_egui_shell_rgba(&mut self, target: RenderTargetSpec, region: inspect::ScreenshotRegionTarget) -> Result<RasterImage, RenderError> {
        let mut rasterizer = egui_shell::rasterizer::EguiSoftwareRasterizer::new();
        let (ctx, mut output, regions, clear_color) = self.capture_settled_egui_shell_output(target, &mut rasterizer)?;
        rasterizer.render_egui_screenshot_rgba(&ctx, &mut output, target, region, regions, clear_color)
    }

    /// Render a settled `ui.screenshot` PNG through [`egui_shell::rasterizer::EguiSoftwareRasterizer`].
    ///
    /// # Errors
    /// Returns [`RenderError`] when screenshot dimensions fail validation or the requested crop
    /// region (such as `Dialog` when no modal is open) is unavailable.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_egui_shell_screenshot_png(
        &mut self,
        target: RenderTargetSpec,
        region: inspect::ScreenshotRegionTarget,
    ) -> Result<(Vec<u8>, u32, u32), RenderError> {
        let mut rasterizer = egui_shell::rasterizer::EguiSoftwareRasterizer::new();
        let (ctx, mut output, regions, clear_color) = self.capture_settled_egui_shell_output(target, &mut rasterizer)?;
        rasterizer.render_egui_screenshot_png(&ctx, &mut output, target, region, regions, clear_color)
    }

    /// Intercept control actions via custom [`EguiDispatchHook`] or the built-in `egui` shell
    /// backend (`ui.inspect` / `ui.screenshot`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn try_dispatch_egui_backend(&mut self, method: &str, params: &serde_json::Value) -> Option<Result<serde_json::Value, EngineError>> {
        if let Some(hook) = self.egui_dispatch_hook.0.clone()
            && let Some(res) = hook(self, method, params)
        {
            self.sync_views();
            return Some(res);
        }
        match method {
            "ui.inspect" => Some(dispatch_egui_inspect_action(self, params)),
            "ui.screenshot" => Some(dispatch_egui_screenshot_action(self, params)),
            _ => None,
        }
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
        let title = self.canvas_dialogs.print.job.title.as_deref().unwrap_or("curvebook-print");
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

    /// Load a `.curvebook` archive from [`Self::virtual_workspace`] into the active [`Session`] (`QOL-10`).
    ///
    /// # Errors
    /// Returns an error string if the entry is missing or fails validation.
    pub fn load_from_virtual_workspace(&mut self, file_name: &str) -> Result<(), String> {
        let doc = self.virtual_workspace.open_project_archive(file_name)?;
        if let Some(tracker) = self.batch_undo_log.as_mut() {
            tracker.capture_fallback_baseline_before(&self.session, true);
        }
        let gui_only = self.session.gui_only();
        let path_policy = self.session.path_policy().clone();
        let mut next = Session::from_document(doc);
        next.set_gui_only(gui_only);
        next.set_path_policy(path_policy);
        self.session = next;
        self.mutation_seq.0 = self.mutation_seq.0.wrapping_add(1);
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
    /// Return `true` when the active workspace sheet or focused split pane targets a ResultSheet (`RV-75`).
    #[must_use]
    pub fn is_result_sheet_shortcut_context(&self) -> bool {
        interaction::resolve_result_sheet_for_shortcut(self, true).is_some()
    }

    pub fn handle_key_combo(&mut self, combo: &str) -> Result<Option<ShortcutExecution>, String> {
        let on_result_sheet = self.is_result_sheet_shortcut_context();
        self.handle_key_combo_for_context(combo, on_result_sheet)
    }

    /// Resolve and execute `combo` within an explicit ResultSheet vs non-ResultSheet context (`RV-75`).
    ///
    /// # Errors
    /// Returns a descriptive error string if executing the matched UI or engine command fails.
    pub fn handle_key_combo_for_context(&mut self, combo: &str, on_result_sheet: bool) -> Result<Option<ShortcutExecution>, String> {
        let Some(matched) = self.shortcuts.handle_key_combo_for_context(combo, on_result_sheet) else {
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

/// Encode a `ControlResponse` preserving structured [`EngineError`] details cleanly.
#[must_use]
pub fn encode_egui_control_response(id: serde_json::Value, res: Result<serde_json::Value, EngineError>) -> ControlResponse {
    match res {
        Ok(value) => ControlResponse { id, ok: true, result: value, error: None, error_detail: None },
        Err(err) => {
            let detail = serde_json::to_value(&err).unwrap_or_else(|_| {
                serde_json::json!({
                    "code": err.code.as_str(),
                    "category": err.category,
                    "message": err.message(),
                    "retryable": err.retryable,
                    "remediation": err.remediation,
                    "details": err.details,
                })
            });
            ControlResponse { id, ok: false, result: serde_json::Value::Null, error: Some(err.to_string()), error_detail: Some(detail) }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn egui_bad_input(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(curvebook_doc::ErrorCode::BadInput, msg)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_egui_inspect_metrics(params: &serde_json::Value) -> Result<ViewportMetrics, EngineError> {
    let Some(m) = params.get("metrics").filter(|v| !v.is_null()) else {
        return Ok(ViewportMetrics::default());
    };
    if !m.is_object() {
        return Err(egui_bad_input("`metrics` must be a JSON object"));
    }
    let mut metrics = ViewportMetrics::default();
    if let Some(w) = control_io::opt_finite_f64_param(m, &["logical_width", "width_pt"]).map_err(egui_bad_input)? {
        if w < 160.0 || w > 16_384.0 {
            return Err(egui_bad_input("`metrics.logical_width` out of range [160, 16384]"));
        }
        metrics.logical_width = w;
    }
    if let Some(h) = control_io::opt_finite_f64_param(m, &["logical_height", "height_pt"]).map_err(egui_bad_input)? {
        if h < 120.0 || h > 16_384.0 {
            return Err(egui_bad_input("`metrics.logical_height` out of range [120, 16384]"));
        }
        metrics.logical_height = h;
    }
    if let Some(s) = control_io::opt_finite_f64_param(m, &["scale_factor", "dpi_scale"]).map_err(egui_bad_input)? {
        if !(0.5..=8.0).contains(&s) {
            return Err(egui_bad_input("`metrics.scale_factor` out of range [0.5, 8.0]"));
        }
        metrics.scale_factor = s;
    }
    Ok(metrics)
}

#[cfg(not(target_arch = "wasm32"))]
fn dispatch_egui_inspect_action(app: &mut UiApp, params: &serde_json::Value) -> Result<serde_json::Value, EngineError> {
    control_io::require_params_object_or_null(params, "ui.inspect").map_err(egui_bad_input)?;
    let metrics = parse_egui_inspect_metrics(params)?;
    let scope = control_io::opt_str_param(params, &["scope"]).map_err(egui_bad_input)?;
    let include_bounds = control_io::opt_bool_param(params, &["include_bounds"]).map_err(egui_bad_input)?.unwrap_or(true);
    let snap = app.inspect_egui_shell_snapshot(metrics, scope, include_bounds);
    serde_json::to_value(snap)
        .map_err(|err| EngineError::with_code(curvebook_doc::ErrorCode::InternalError, format!("failed to serialize egui inspect tree: {err}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn dispatch_egui_screenshot_action(app: &mut UiApp, params: &serde_json::Value) -> Result<serde_json::Value, EngineError> {
    control_io::require_params_object_or_null(params, "ui.screenshot").map_err(egui_bad_input)?;
    let target_raw = control_io::opt_str_param(params, &["target"]).map_err(egui_bad_input)?.unwrap_or("full_window").trim().to_ascii_lowercase();
    let region = match target_raw.as_str() {
        "" | "full_window" | "full" | "window" | "workspace" | "viewport" => inspect::ScreenshotRegionTarget::FullWindow,
        "canvas" | "main" => inspect::ScreenshotRegionTarget::Canvas,
        "dialog" | "modal" => inspect::ScreenshotRegionTarget::Dialog,
        other => {
            return Err(egui_bad_input(format!(
                "unsupported screenshot target `{other}`; expected full_window, workspace, viewport, canvas, or dialog"
            )));
        }
    };

    let width_px = control_io::parse_bounded_u32(
        params.get("width_px"),
        control_io::DEFAULT_SCREENSHOT_WIDTH_PX,
        control_io::MIN_SCREENSHOT_DIM_PX,
        control_io::MAX_SCREENSHOT_DIM_PX,
        "width_px",
    )
    .map_err(egui_bad_input)?;
    let height_px = control_io::parse_bounded_u32(
        params.get("height_px"),
        control_io::DEFAULT_SCREENSHOT_HEIGHT_PX,
        control_io::MIN_SCREENSHOT_DIM_PX,
        control_io::MAX_SCREENSHOT_DIM_PX,
        "height_px",
    )
    .map_err(egui_bad_input)?;
    let dpi_u32 =
        control_io::parse_bounded_u32(params.get("dpi"), u32::from(control_io::DEFAULT_SCREENSHOT_DPI), u32::from(control_io::MIN_DPI), 600, "dpi")
            .map_err(egui_bad_input)?;
    let dpi = u16::try_from(dpi_u32).map_err(|_| egui_bad_input("dpi out of range"))?;

    if app.is_gui_only() && params.get("path").is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty())) {
        return Err(EngineError::with_code(
            curvebook_doc::ErrorCode::GuiOnlyViolation,
            "direct filesystem writes via `ui.screenshot` `path` are blocked under --gui-only",
        )
        .with_remediation("Omit `path` and inspect the returned in-memory `data_base64` PNG instead."));
    }
    let opt_path = control_io::opt_str_param(params, &["path"]).map_err(egui_bad_input)?.map(str::trim).filter(|s| !s.is_empty());

    let (png_bytes, out_w, out_h) = app
        .render_egui_shell_screenshot_png(RenderTargetSpec { width_px, height_px, dpi }, region)
        .map_err(|err| EngineError::with_code(curvebook_doc::ErrorCode::Conflict, format!("screenshot capture failed: {err}")))?;

    let saved_path = match opt_path {
        Some(path) => {
            control_io::write_host_file(path, &png_bytes).map_err(|err| EngineError::with_code(curvebook_doc::ErrorCode::IoError, err))?;
            Some(path.to_owned())
        }
        None => None,
    };
    let target_label = match region {
        inspect::ScreenshotRegionTarget::FullWindow => "full_window",
        inspect::ScreenshotRegionTarget::Canvas => "canvas",
        inspect::ScreenshotRegionTarget::Dialog => "dialog",
    };

    Ok(serde_json::json!({
        "format": "png",
        "target": target_label,
        "width_px": out_w,
        "height_px": out_h,
        "dpi": dpi,
        "bytes_len": png_bytes.len(),
        "data_base64": control_io::encode_base64(&png_bytes),
        "path": saved_path,
    }))
}

/// Accessible label/value snapshot recorded for a single rendered `egui_shell` widget.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default)]
pub struct EguiSurfaceA11yEntry {
    pub widget_id: String,
    pub label: String,
    pub value_text: Option<String>,
    pub command_id: Option<String>,
    pub disabled_reason: Option<String>,
    pub full_label: Option<String>,
}

/// Captured painted text runs and accessibility metadata from a settled `egui_shell` frame.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default)]
pub struct EguiSurfaceCapture {
    pub text_runs: Vec<(String, String)>,
    pub a11y_entries: Vec<EguiSurfaceA11yEntry>,
    pub accesskit_labels: Vec<String>,
}

/// Reusable offscreen `egui_shell` surface scanner sharing cached fonts across multiple frames.
#[cfg(not(target_arch = "wasm32"))]
pub struct EguiSurfaceScanner {
    ctx: egui::Context,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for EguiSurfaceScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl EguiSurfaceScanner {
    /// Initialize a reusable offscreen `egui::Context` with AccessKit and cached UI fonts enabled.
    #[must_use]
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
        Self { ctx }
    }

    /// Render `app` under `metrics` until settled and return all painted text runs,
    /// `FrameInspectNodeMeta` entries, and AccessKit node labels/values.
    #[must_use]
    pub fn scan(&self, app: &mut UiApp, metrics: ViewportMetrics) -> EguiSurfaceCapture {
        app.sync_views();
        #[allow(clippy::cast_possible_truncation)]
        let w_pt = metrics.logical_width.max(1.0) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let h_pt = metrics.logical_height.max(1.0) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let ppp = metrics.clamped_scale_factor() as f32;

        let Ok((output, meta, _stamp)) = run_settled_egui_on_ctx(&self.ctx, app, [w_pt, h_pt], ppp, None, |ui, app_ref, meta, intents| {
            let mut views = std::mem::take(&mut app_ref.egui_views);
            let _ = egui_shell::inspect_bridge::render_egui_shell_frame(ui, app_ref, &mut views, meta, intents);
            app_ref.egui_views = views;
        }) else {
            return EguiSurfaceCapture::default();
        };

        let text_runs = egui_shell::driver::extract_rendered_text_runs(&output.shapes)
            .into_iter()
            .map(|run| {
                let owner = meta.entries.iter().find(|e| e.clipped_rect.intersects(run.text_rect)).map(|e| e.widget_id.clone()).unwrap_or_default();
                (owner, run.full_text)
            })
            .collect();

        let a11y_entries = meta
            .entries
            .into_iter()
            .map(|e| EguiSurfaceA11yEntry {
                widget_id: e.widget_id,
                label: e.label,
                value_text: e.value.map(|v| match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                }),
                command_id: e.command_id,
                disabled_reason: e.disabled_reason,
                full_label: e.full_label,
            })
            .collect();

        let mut accesskit_labels = Vec::new();
        if let Some(update) = &output.platform_output.accesskit_update {
            for (_, node) in &update.nodes {
                if let Some(lbl) = node.label()
                    && !lbl.trim().is_empty()
                {
                    accesskit_labels.push(lbl.to_owned());
                }
                if let Some(val) = node.value()
                    && !val.trim().is_empty()
                {
                    accesskit_labels.push(val.to_owned());
                }
            }
        }

        EguiSurfaceCapture { text_runs, a11y_entries, accesskit_labels }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_settled_egui_on_ctx(
    ctx: &egui::Context,
    app: &mut UiApp,
    logical_size_pt: [f32; 2],
    pixels_per_point: f32,
    mut rasterizer: Option<&mut egui_shell::rasterizer::EguiSoftwareRasterizer>,
    mut build_ui: impl FnMut(&mut egui::Ui, &mut UiApp, &mut egui_shell::FrameInspectMeta, &mut egui_shell::PendingUiIntents),
) -> Result<(egui::FullOutput, egui_shell::FrameInspectMeta, egui_shell::GeometryStamp), EngineError> {
    let w_pt = if logical_size_pt[0].is_finite() && logical_size_pt[0] > 0.0 { logical_size_pt[0] } else { 1024.0 };
    let h_pt = if logical_size_pt[1].is_finite() && logical_size_pt[1] > 0.0 { logical_size_pt[1] } else { 640.0 };
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let zoom = app.ui_scale();
    // Pre-seed `zoom_factor` BEFORE calling `apply_theme_to_ctx` (never `set_zoom_factor` prior
    // to frame 0, which queues `new_zoom_factor` and makes `egui` overwrite pass-0 `screen_rect`
    // with its `10_000x10_000` default `content_rect`). Apply theme tokens before `ctx.run_ui`
    // so pass-0 `ui.style().visuals` reflects the target light/dark theme immediately.
    let native_ppp = (ppp / zoom).max(0.05);
    ctx.options_mut(|o| {
        o.zoom_with_keyboard = false;
        o.zoom_factor = zoom;
    });
    egui_shell::Tokens::from_theme_spec(&app.theme, &app.locale).apply_theme_to_ctx(ctx);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width_px = (w_pt * ppp).round().clamp(1.0, 16_384.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height_px = (h_pt * ppp).round().clamp(1.0, 16_384.0) as u32;

    let mut meta = egui_shell::FrameInspectMeta::default();
    for pass in 0..8_u32 {
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(native_ppp), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut intents = egui_shell::PendingUiIntents::default();
        let mut settled_in_pass = false;
        let mut output = ctx.run_ui(raw_input, |ui| {
            build_ui(ui, app, &mut meta, &mut intents);
            settled_in_pass = !ui.ctx().will_discard();
        });
        let stamp = egui_shell::GeometryStamp::from_context(u64::from(pass) + 1, ctx, [width_px, height_px], settled_in_pass);
        if let Some(rast) = rasterizer.as_deref_mut() {
            rast.consume_textures_set(&mut output.textures_delta);
            if !settled_in_pass {
                rast.consume_textures_free(&mut output.textures_delta);
            }
        } else {
            output.textures_delta.clear();
        }
        if settled_in_pass {
            intents.commit_if_settled(app, false)?;
            app.sync_views();
            return Ok((output, meta, stamp));
        }
    }
    Err(EngineError::with_code(curvebook_doc::ErrorCode::InternalError, "egui shell layout pass exceeded maximum discard retries without settling"))
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use curvebook_doc::{CellCoord, SheetCellRange};
    use serde_json::{Value, json};

    use super::menus::ui_commands::UI_COMMANDS;
    use super::{ClipboardBundle, ControlAuth, PrintJobSpec, UiApp, parse_control_port, serve_control_stream};

    #[test]
    fn ui_commands_table_covers_parity_surface() {
        assert_eq!(UI_COMMANDS.len(), 54);
        for required in [
            "ui.navigator.toggle",
            "ui.inspector.toggle",
            "ui.family.toggle",
            "ui.palette.open",
            "ui.analyze.open",
            "ui.editParams.open",
            "ui.changeGraphType.open",
            "ui.clonePipeline.open",
            "ui.applyStyleFrom.open",
            "ui.graph.create.open",
            "ui.layout.equalizeScaling.open",
            "ui.view.rowTitles.toggle",
            "ui.gridOverlay.toggle",
            "ui.embeddedTable.format.open",
            "ui.image.format.open",
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
            "app.feedback",
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
            json!({"id": 3, "op": "command.run", "command": "sheet.createData", "params": {"sheet_id": "xy-1", "graph_sheet_id": "graph-xy-1", "title": "Dose", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": true}}),
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
        assert!(bundle.html.contains("<div class=\"curvebook-figure\""));
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
