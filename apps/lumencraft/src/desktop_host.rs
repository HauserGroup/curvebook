#![forbid(unsafe_code)]

//! Desktop host event loop, multi-window runtime, HiDPI Retina backing store coordination,
//! OS chrome/menu dispatch, IME text composition, trackpad gesture inertia, file drop routing,
//! clipboard pasteboard integration, isolated crash-recovery journals, and JSONL playback.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use lumencraft_doc::{
    CellCoord, CellValue, ProjectDocument, SheetCellRange, SheetId, SheetSection, SubcolumnMode, TableArchetype, TableKind, VariableSource,
};
use lumencraft_engine::{CommandOutcome, Session, cmd as engine_cmd};
use lumencraft_format::recovery::is_lease_reclaimable;
use lumencraft_format::{
    CrashRecoveryJournal, DirectoryJournalBackend, FormatKind, FormatRegistry, ImportOptions, JournalBackend, JournalRecoveryWarning,
    MAX_ARCHIVE_FILE_BYTES, RecoveryReport, instantiate_template_bytes, is_unpacked_project_dir,
};
use lumencraft_ui_egui::dialogs_canvas::ActiveDialog;
use lumencraft_ui_egui::grid_view::ImportOriginKind;
use lumencraft_ui_egui::interaction::{
    InlineEditorTarget, KeyModifiers, PointerButton, UiEventOutcome, UiInputEvent, ViewportMetrics, WorkspaceHitTarget,
};
use lumencraft_ui_egui::menus::{MenuSectionTooltips, TOOLBAR_BUTTONS, menu_bar_tooltips};
use lumencraft_ui_egui::theme::UiThemeMode;
use lumencraft_ui_egui::workspace_nav::{ActiveWorkspacePane, WelcomeMode};
use lumencraft_ui_egui::{
    CanvasPreviewFilter, ClipboardBundle, ControlAuth, ControlPostRequestContext, LOCALES, UiApp, dispatch_ui_command, is_http_request_line,
    read_os_pasteboard, serve_control_stream_with_quit_and_hook, write_os_clipboard_bundle, write_os_pasteboard,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_RECOVERY_LOCK_TTL_MS: u64 = 5 * 60 * 1000;
const BASE_LOGICAL_DPI: u16 = 72;
const IN_WINDOW_MENU_BAR_HEIGHT_PT: f64 = 24.0;
const VELOCITY_COAST_EPSILON_PT_PER_SEC: f64 = 0.5;

static NEXT_INSTANCE_SEQ: AtomicU64 = AtomicU64::new(1);

fn allocate_instance_seq() -> u64 {
    NEXT_INSTANCE_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Default TTL (5 minutes) used for crash-recovery directory lock leases.
#[must_use]
pub const fn default_recovery_ttl_ms() -> u64 {
    DEFAULT_RECOVERY_LOCK_TTL_MS
}

/// Current UNIX timestamp in milliseconds.
#[must_use]
pub fn current_epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Stable identifier of an open desktop window hosted by [`DesktopHostRuntime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DesktopWindowId(pub u64);

impl fmt::Display for DesktopWindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "win-{}", self.0)
    }
}

/// Top-level window menu chrome placement mode (`--in-window-menus` vs macOS native menu bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MenuChromeMode {
    InWindowMenuBar,
    NativeMenuBar,
}

impl MenuChromeMode {
    /// Whether menus are rendered inside the desktop window frame rather than in a global native bar.
    #[must_use]
    pub const fn is_in_window(self) -> bool {
        matches!(self, Self::InWindowMenuBar)
    }
}

/// One localized desktop menu item exposed in window/native menu chrome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopMenuItem {
    pub command_id: String,
    pub localized_label: String,
    pub shortcut: String,
    pub menu_path: String,
    pub enabled: bool,
    pub summary: String,
    pub beginner: String,
}

/// One localized top-level menu bar section (`File`, `Edit`, `Insert`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopMenuSection {
    pub menu_key: String,
    pub title: String,
    pub items: Vec<DesktopMenuItem>,
}

/// Observed menu bar chrome state for a [`DesktopWindowSession`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopMenuChromeState {
    pub mode: MenuChromeMode,
    pub in_window_menus: bool,
    pub menu_bar_visible: bool,
    pub menu_bar_height_pt: f64,
    pub active_menu_key: Option<String>,
    pub sections: Vec<DesktopMenuSection>,
    pub dispatched_actions: Vec<String>,
}

/// Active IME preedit composition state prior to [`DesktopHostEvent::ImeCommit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImePreeditState {
    pub text: String,
    pub cursor: Option<(usize, usize)>,
}

/// Host runtime configuration constructed from CLI flags (`--in-window-menus`, `--new-instance`, ...)
/// or embedded test harnesses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopHostConfig {
    pub in_window_menus: bool,
    pub new_instance: bool,
    #[serde(default)]
    pub gui_only: bool,
    pub locale_override: Option<String>,
    pub theme_override: Option<String>,
    pub recovery_root: Option<PathBuf>,
    pub initial_viewport: ViewportMetrics,
}

impl Default for DesktopHostConfig {
    fn default() -> Self {
        Self {
            in_window_menus: false,
            new_instance: false,
            gui_only: false,
            locale_override: None,
            theme_override: None,
            recovery_root: None,
            initial_viewport: ViewportMetrics::new(1280.0, 800.0, 1.0),
        }
    }
}

/// Result returned when [`DesktopHostEvent::WindowCloseRequested`] is processed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseDisposition {
    Closed,
    PromptUnsaved { window_id: DesktopWindowId, dirty_commands: Vec<String> },
}

/// Rendered software compositor framebuffer output returned by [`DesktopHostEvent::RedrawRequested`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopFrameOutput {
    pub window_id: DesktopWindowId,
    pub width_px: u32,
    pub height_px: u32,
    pub scale_factor: f32,
    pub rgba_bytes: Vec<u8>,
    pub frame_ms: f64,
    pub window_title: String,
}

const fn default_click_count() -> u8 {
    1
}

/// Native windowing / OS input events processed by [`DesktopHostRuntime::dispatch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DesktopHostEvent {
    WindowOpened {
        #[serde(default)]
        initial_file: Option<PathBuf>,
        #[serde(default)]
        viewport: ViewportMetrics,
    },
    WindowCloseRequested {
        window_id: DesktopWindowId,
        #[serde(default)]
        force: bool,
    },
    WindowFocused {
        window_id: DesktopWindowId,
    },
    Resized {
        window_id: DesktopWindowId,
        width_px: u32,
        height_px: u32,
    },
    ScaleFactorChanged {
        window_id: DesktopWindowId,
        scale_factor: f32,
    },
    PointerMoved {
        window_id: DesktopWindowId,
        x_pt: f64,
        y_pt: f64,
        #[serde(default)]
        modifiers: KeyModifiers,
    },
    PointerButton {
        window_id: DesktopWindowId,
        x_pt: f64,
        y_pt: f64,
        #[serde(default)]
        button: PointerButton,
        pressed: bool,
        #[serde(default = "default_click_count")]
        click_count: u8,
        #[serde(default)]
        modifiers: KeyModifiers,
    },
    MouseWheel {
        window_id: DesktopWindowId,
        x_pt: f64,
        y_pt: f64,
        #[serde(default)]
        delta_x: f64,
        delta_y: f64,
        #[serde(default)]
        modifiers: KeyModifiers,
    },
    TrackpadMagnify {
        window_id: DesktopWindowId,
        anchor_x_pt: f64,
        anchor_y_pt: f64,
        magnification_delta: f64,
    },
    TrackpadScroll {
        window_id: DesktopWindowId,
        delta_x: f64,
        delta_y: f64,
        dt_sec: f64,
        #[serde(default)]
        phase_ended: bool,
    },
    KeyboardInput {
        window_id: DesktopWindowId,
        key: String,
        pressed: bool,
        #[serde(default)]
        modifiers: KeyModifiers,
        #[serde(default)]
        text: Option<String>,
    },
    ImePreedit {
        window_id: DesktopWindowId,
        text: String,
        #[serde(default)]
        cursor: Option<(usize, usize)>,
    },
    ImeCommit {
        window_id: DesktopWindowId,
        text: String,
    },
    FileHovered {
        window_id: DesktopWindowId,
        paths: Vec<PathBuf>,
    },
    FileHoverCancelled {
        window_id: DesktopWindowId,
    },
    FileDropped {
        window_id: DesktopWindowId,
        path: PathBuf,
    },
    MenuAction {
        window_id: DesktopWindowId,
        command_id: String,
    },
    ClipboardCopy {
        window_id: DesktopWindowId,
    },
    ClipboardPaste {
        window_id: DesktopWindowId,
        #[serde(default)]
        text_override: Option<String>,
    },
    AnimationTick {
        dt_sec: f64,
    },
    RedrawRequested {
        window_id: DesktopWindowId,
    },
}

/// Structured outcome emitted for every dispatched [`DesktopHostEvent`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DesktopEventOutcome {
    pub handled: bool,
    pub action: String,
    pub window_id: Option<DesktopWindowId>,
    pub opened_window_id: Option<DesktopWindowId>,
    pub close_disposition: Option<CloseDisposition>,
    pub frame: Option<DesktopFrameOutput>,
    pub command_id: Option<String>,
    pub command_result: Option<Value>,
}

/// Parse a CLI or configuration theme override into [`UiThemeMode`].
///
/// # Errors
/// Returns an error if `raw` does not match `"light"`, `"dark"`, `"high-contrast"`, or `"high_contrast"`.
pub fn parse_theme_mode(raw: &str) -> Result<UiThemeMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "light" | "default" => Ok(UiThemeMode::Light),
        "dark" => Ok(UiThemeMode::Dark),
        "high-contrast" | "high_contrast" | "highcontrast" => Ok(UiThemeMode::HighContrast),
        other => Err(format!("unsupported theme `{other}`; expected light, dark, or high-contrast")),
    }
}

fn has_unpacked_dir_suffix(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.to_ascii_lowercase().ends_with(".lumencraft.d"))
}

fn is_unpacked_project_target(path: &Path) -> bool {
    has_unpacked_dir_suffix(path) || is_unpacked_project_dir(path) || path.is_dir()
}

fn next_csv_sheet_id(doc: &ProjectDocument, hint_idx: usize) -> SheetId {
    let mut seq = hint_idx.saturating_add(1);
    loop {
        let candidate = SheetId::new(format!("data-{seq}"));
        if doc.sheet_section(&candidate).is_none() {
            return candidate;
        }
        seq = seq.saturating_add(1);
    }
}

fn file_stem_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| String::from("Imported Table"), ToOwned::to_owned)
}

struct PointerButtonInput {
    window_id: DesktopWindowId,
    x_pt: f64,
    y_pt: f64,
    button: PointerButton,
    pressed: bool,
    click_count: u8,
    modifiers: KeyModifiers,
}

enum LoadedPathPayload {
    Project { doc: Box<ProjectDocument>, archive_path: Option<PathBuf>, clean_archive: bool },
    DelimitedTable { importer_id: &'static str, title: String, is_tsv: bool },
}

fn classify_and_load_path(path: &Path) -> Result<LoadedPathPayload, String> {
    let registry = FormatRegistry::default_registry();
    let importer = if is_unpacked_project_target(path) { registry.importer_by_format("lumencraft.d") } else { registry.importer_for_path(path) };
    let Some(importer) = importer else {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        return Err(format!(
            "unsupported file extension `.{ext}` for `{}`; expected .lumencraft, .lumencraft.d, .lctemplate, .prism, .pzfx, .csv, or .tsv",
            path.display()
        ));
    };

    let desc = importer.descriptor();
    match desc.kind {
        FormatKind::ProjectDir | FormatKind::Archive => {
            let payload =
                importer.import_path(path, &ImportOptions::default()).map_err(|err| format!("failed to open `{}`: {err}", path.display()))?;
            let doc = payload.into_document().ok_or_else(|| format!("archive `{}` did not decode a project document", path.display()))?;
            Ok(LoadedPathPayload::Project { doc: Box::new(doc), archive_path: Some(path.to_path_buf()), clean_archive: true })
        }
        FormatKind::Template => {
            let bytes = lumencraft_platform::fs::read_regular_file_bytes(path, MAX_ARCHIVE_FILE_BYTES)
                .map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
            let doc =
                instantiate_template_bytes(&bytes, None).map_err(|err| format!("failed to instantiate template `{}`: {err}", path.display()))?;
            Ok(LoadedPathPayload::Project { doc: Box::new(doc), archive_path: None, clean_archive: false })
        }
        FormatKind::ForeignProject => {
            let payload =
                importer.import_path(path, &ImportOptions::default()).map_err(|err| format!("failed to import `{}`: {err}", path.display()))?;
            let doc = payload.into_document().ok_or_else(|| format!("file `{}` did not yield a project document", path.display()))?;
            Ok(LoadedPathPayload::Project { doc: Box::new(doc), archive_path: None, clean_archive: false })
        }
        FormatKind::DelimitedText => {
            let is_tsv = path.extension().and_then(|e| e.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("tsv"));
            Ok(LoadedPathPayload::DelimitedTable { importer_id: desc.id, title: file_stem_title(path), is_tsv })
        }
        FormatKind::ResultTable | FormatKind::FigureVector | FormatKind::FigureRaster => {
            Err(format!("format `{}` cannot be opened directly as a startup workspace document", desc.id))
        }
    }
}

struct PendingDelimitedStartup {
    index: usize,
    path: PathBuf,
    importer_id: &'static str,
    title: String,
}

/// Load one or more startup files (`.lumencraft`, `.lumencraft.d`, `.lctemplate`, `.prism`,
/// `.pzfx`, `.csv`, `.tsv`) into a [`Session`].
///
/// Rejects multiple project archives/templates in one window launch, merges CSV/TSV files before
/// or after a single project file, and marks the resulting session dirty whenever delimited tables
/// are merged into an opened project.
///
/// # Errors
/// Returns an error if multiple project files are passed, or if any path has an unsupported format
/// or fails to parse.
pub fn open_document_from_files(files: &[PathBuf]) -> Result<Session, String> {
    if files.is_empty() {
        return Ok(Session::new());
    }

    let registry = FormatRegistry::default_registry();
    let mut primary_project: Option<(PathBuf, ProjectDocument, Option<PathBuf>, bool)> = None;
    let mut delimited_files: Vec<PendingDelimitedStartup> = Vec::new();

    for (idx, path) in files.iter().enumerate() {
        match classify_and_load_path(path)? {
            LoadedPathPayload::Project { doc, archive_path, clean_archive } => {
                if let Some((first_path, _, _, _)) = &primary_project {
                    return Err(format!(
                        "cannot open multiple project files together (`{}` and `{}`); pass at most one project file and optional .csv/.tsv files",
                        first_path.display(),
                        path.display()
                    ));
                }
                primary_project = Some((path.clone(), *doc, archive_path, clean_archive));
            }
            LoadedPathPayload::DelimitedTable { importer_id, title, .. } => {
                delimited_files.push(PendingDelimitedStartup { index: idx, path: path.clone(), importer_id, title });
            }
        }
    }

    let primary_had_project = primary_project.is_some();
    let (mut doc, primary_archive_path, clean_archive) = match primary_project {
        Some((_, loaded_doc, archive_path, is_clean)) => (loaded_doc, archive_path, is_clean),
        None => (ProjectDocument::default(), None, false),
    };

    let merged_delimited = !delimited_files.is_empty();
    for item in delimited_files {
        let importer = registry.importer_by_format(item.importer_id).ok_or_else(|| format!("missing importer `{}`", item.importer_id))?;
        let sheet_id = next_csv_sheet_id(&doc, item.index);
        let opts = ImportOptions {
            sheet_id: sheet_id.clone(),
            title: item.title,
            table_kind: TableKind::Column,
            subcolumn_mode: SubcolumnMode::Single,
            csv: None,
        };
        let payload = importer.import_path(&item.path, &opts).map_err(|err| format!("failed to import `{}`: {err}", item.path.display()))?;
        let sheet = payload.into_data_sheet().ok_or_else(|| format!("importer `{}` did not yield a data sheet", item.importer_id))?;
        doc.add_data_sheet(sheet).map_err(|err| err.to_string())?;
        if doc.active_sheet_id.is_none() {
            let _ = doc.select_in_navigator(&sheet_id);
        }
    }

    let mut session = Session::from_document(doc);
    if merged_delimited && primary_had_project {
        session
            .execute("project.recalculate", json!({ "include_external_links": false }))
            .map_err(|err| format!("failed to recalculate merged startup project: {err}"))?;
    }
    if let Some(path) = primary_archive_path {
        session.set_file_path(Some(path), clean_archive && !merged_delimited);
    }
    Ok(session)
}

fn build_menu_sections(app: &UiApp) -> Vec<DesktopMenuSection> {
    let raw_sections: Vec<MenuSectionTooltips> = menu_bar_tooltips(app);
    raw_sections
        .into_iter()
        .map(|sec| DesktopMenuSection {
            menu_key: sec.menu_key.to_owned(),
            title: sec.title.to_owned(),
            items: sec
                .items
                .into_iter()
                .map(|item| DesktopMenuItem {
                    command_id: item.command_id.to_owned(),
                    localized_label: item.localized_label.to_owned(),
                    shortcut: item.shortcut,
                    menu_path: item.menu_path.to_owned(),
                    enabled: item.enabled,
                    summary: item.summary.to_owned(),
                    beginner: item.beginner.to_owned(),
                })
                .collect(),
        })
        .collect()
}

fn recovery_owner_for_window(window_id: DesktopWindowId) -> String {
    format!("lumencraft-pid-{}-win-{}", std::process::id(), window_id.0)
}

const RECOVERY_LEASE_MAX_RENEW_INTERVAL_MS: u64 = 30_000;
const WALL_CLOCK_EPOCH_MIN_MS: u64 = 1_000_000_000_000;

fn lease_renewal_timestamp(journal: &CrashRecoveryJournal<DirectoryJournalBackend>) -> u64 {
    if journal.lease().acquired_epoch_ms >= WALL_CLOCK_EPOCH_MIN_MS { current_epoch_ms() } else { journal.lease().acquired_epoch_ms }
}

fn renew_journal_lease_on_command(journal: &mut CrashRecoveryJournal<DirectoryJournalBackend>) -> Result<(), String> {
    let ts = lease_renewal_timestamp(journal);
    journal.renew_lease(ts).map_err(|err| format!("failed to renew recovery lease: {err}"))
}

fn maybe_renew_journal_lease_periodically(journal: &mut CrashRecoveryJournal<DirectoryJournalBackend>) {
    if journal.lease().acquired_epoch_ms < WALL_CLOCK_EPOCH_MIN_MS {
        return;
    }
    let now_ms = current_epoch_ms();
    let interval_ms = (journal.lease().ttl_ms / 4).clamp(1, RECOVERY_LEASE_MAX_RENEW_INTERVAL_MS);
    if now_ms.saturating_sub(journal.lease().acquired_epoch_ms) >= interval_ms
        && let Err(err) = journal.renew_lease(now_ms)
    {
        eprintln!("lumencraft recovery lease renewal warning: {err}");
    }
}

fn abort_journal_lease_best_effort(journal: CrashRecoveryJournal<DirectoryJournalBackend>) {
    let owner_id = journal.lease().owner_id.clone();
    let mut backend = journal.abandon();
    let _ = backend.release_lease(&owner_id);
}

fn dir_has_active_lease(candidate: &Path, now_epoch_ms: u64) -> bool {
    if !candidate.exists() {
        return false;
    }
    let Ok(backend) = DirectoryJournalBackend::open_dir(candidate) else {
        return false;
    };
    matches!(backend.read_lease(), Ok(Some(lease)) if !is_lease_reclaimable(&lease, now_epoch_ms))
}

/// Claim a crash-recovery directory under `root`, isolating `--new-instance` processes and
/// secondary windows away from active writers.
///
/// # Errors
/// Returns an error if creating the directory fails.
pub fn claim_isolated_journal_dir(root: &Path, new_instance: bool, window_id: DesktopWindowId, now_epoch_ms: u64) -> Result<PathBuf, String> {
    let base_candidate = if new_instance {
        root.join(format!("instance-{}-win-{}", std::process::id(), window_id.0))
    } else if window_id.0 == 1 {
        root.to_path_buf()
    } else {
        root.join(format!("win-{}", window_id.0))
    };

    let chosen = if !new_instance || !dir_has_active_lease(&base_candidate, now_epoch_ms) {
        base_candidate
    } else {
        loop {
            let seq = allocate_instance_seq();
            let candidate = root.join(format!("instance-{}-s{}-win-{}", std::process::id(), seq, window_id.0));
            if !dir_has_active_lease(&candidate, now_epoch_ms) {
                break candidate;
            }
        }
    };

    std::fs::create_dir_all(&chosen).map_err(|err| format!("failed to create recovery dir `{}`: {err}", chosen.display()))?;
    Ok(chosen)
}

fn is_readonly_control_method(method: &str) -> bool {
    matches!(
        method,
        "ping"
            | "command.list"
            | "doc.inspect"
            | "doc.export"
            | "render.svg"
            | "render.pdf"
            | "render.preview"
            | "ui.state"
            | "state.snapshot"
            | "app.quit"
    )
}

fn apply_control_journal_steps(
    app: &UiApp,
    recovery_journal: &mut Option<CrashRecoveryJournal<DirectoryJournalBackend>>,
    dirty_commands: &mut Vec<String>,
    ctx: &ControlPostRequestContext<'_>,
) -> Result<(), String> {
    if let Some(journal) = recovery_journal.as_mut() {
        renew_journal_lease_on_command(journal)?;
    }

    let mut recorded_any_step = false;
    for step in ctx.steps {
        if step.mutated {
            recorded_any_step = true;
            dirty_commands.push(step.command_id.clone());
            if let Some(journal) = recovery_journal.as_mut() {
                app.session()
                    .record_outcome_to_journal(journal, &step.command_id, &step.params, true)
                    .map_err(|err| format!("failed to record control WAL entry `{}`: {err}", step.command_id))?;
            }
        } else if step.command_id == "project.save" && !app.session().is_dirty() {
            recorded_any_step = true;
            dirty_commands.clear();
            if let Some(journal) = recovery_journal.as_mut() {
                app.session().checkpoint_journal(journal).map_err(|err| format!("failed to checkpoint control WAL after save: {err}"))?;
            }
        }
    }

    if !recorded_any_step && !ctx.rolled_back && !is_readonly_control_method(ctx.method) {
        let now_dirty = app.session().is_dirty();
        let stack_changed = app.session().undo_depth() != ctx.prev_undo_depth || app.session().redo_depth() != ctx.prev_redo_depth;
        let became_dirty = !ctx.prev_dirty && now_dirty;
        if stack_changed || became_dirty {
            dirty_commands.push(ctx.method.to_owned());
            if let Some(journal) = recovery_journal.as_mut() {
                app.session()
                    .checkpoint_journal(journal)
                    .map_err(|err| format!("failed to checkpoint control UI mutation `{}`: {err}", ctx.method))?;
            }
        }
    }
    Ok(())
}

/// Classification of one incoming control-protocol line before dispatching against [`DesktopWindowSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFrameKind {
    Empty,
    InvalidUtf8,
    Http,
    Auth,
    Command,
}

/// Output produced by [`DesktopWindowSession::dispatch_live_control_frame_with_hook`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveControlFrameOutput {
    pub response_bytes: Vec<u8>,
    pub quit: bool,
    pub close_connection: bool,
    pub auth_succeeded: Option<bool>,
}

/// Classify a raw bounded control line (`Empty`, `InvalidUtf8`, `Http`, `Auth`, or `Command`).
#[must_use]
pub fn classify_control_frame(raw_bytes: &[u8]) -> ControlFrameKind {
    let Ok(text) = std::str::from_utf8(raw_bytes) else {
        return ControlFrameKind::InvalidUtf8;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ControlFrameKind::Empty;
    }
    if is_http_request_line(trimmed) {
        return ControlFrameKind::Http;
    }
    if is_auth_handshake_line(trimmed) {
        return ControlFrameKind::Auth;
    }
    ControlFrameKind::Command
}

fn is_auth_handshake_line(trimmed: &str) -> bool {
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|val| {
            val.as_object().and_then(|obj| obj.get("method").or_else(|| obj.get("op"))).and_then(Value::as_str).map(|m| m.trim() == "auth")
        })
        .unwrap_or(false)
}

/// Build an internal single-line `"auth"` handshake frame for preserving authenticated connection state.
///
/// # Errors
/// Returns an error if serializing the bearer token fails.
pub fn build_control_reauth_line(auth: &ControlAuth) -> Result<String, String> {
    let tok_json = serde_json::to_string(auth.as_str()).map_err(|err| format!("failed to serialize reauth token: {err}"))?;
    Ok(format!("{{\"id\":\"__reauth__\",\"method\":\"auth\",\"params\":{{\"token\":{tok_json}}}}}\n"))
}

fn strip_first_control_response_line(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let Some(nl_pos) = bytes.iter().position(|&b| b == b'\n') else {
        return Err(String::from("missing reauth response delimiter"));
    };
    Ok(bytes.get(nl_pos + 1..).unwrap_or_default().to_vec())
}

fn parse_auth_response_ok(response_bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(response_bytes).ok().and_then(|val| val.get("ok").and_then(Value::as_bool)).unwrap_or(false)
}

fn extract_project_save_path(params: &Value) -> Option<PathBuf> {
    params
        .get("path")
        .or_else(|| params.get("file_path"))
        .or_else(|| params.get("filePath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn dispatch_live_control_frame_with_hook<F>(
    win: &mut DesktopWindowSession,
    raw_bytes: &[u8],
    auth: &ControlAuth,
    conn_authenticated: bool,
    on_post_request: &mut F,
) -> Result<LiveControlFrameOutput, String>
where
    F: FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>,
{
    let kind = classify_control_frame(raw_bytes);
    if kind == ControlFrameKind::Empty {
        return Ok(LiveControlFrameOutput { response_bytes: Vec::new(), quit: false, close_connection: false, auth_succeeded: None });
    }

    let prepended_reauth = conn_authenticated && kind == ControlFrameKind::Command;
    let reauth_line = if prepended_reauth { Some(build_control_reauth_line(auth)?) } else { None };
    let mut out_bytes = Vec::new();
    let mut last_saved_path: Option<PathBuf> = None;
    let mut clean_after_save = false;
    let quit = win
        .with_control_journal_hook(|app, journal_hook| {
            let mut combined_hook = |app_ref: &UiApp, ctx: &ControlPostRequestContext<'_>| -> Result<(), String> {
                if ctx.request_ok && !ctx.rolled_back {
                    for step in ctx.steps {
                        if step.command_id == "project.save"
                            && let Some(path) = extract_project_save_path(&step.params)
                        {
                            last_saved_path = Some(path);
                            clean_after_save = true;
                        } else if step.mutated {
                            clean_after_save = false;
                        }
                    }
                }
                journal_hook(app_ref, ctx)?;
                on_post_request(app_ref, ctx)
            };
            if let Some(ref prefix) = reauth_line {
                let mut reader = BufReader::new(Cursor::new(prefix.as_bytes()).chain(Cursor::new(raw_bytes)));
                serve_control_stream_with_quit_and_hook(app, &mut reader, &mut out_bytes, auth, &mut combined_hook)
            } else {
                let mut reader = BufReader::new(Cursor::new(raw_bytes));
                serve_control_stream_with_quit_and_hook(app, &mut reader, &mut out_bytes, auth, &mut combined_hook)
            }
        })
        .map_err(|err| format!("control dispatch I/O error: {err}"))?;
    if let Some(saved_path) = last_saved_path {
        win.app_mut().session_mut().set_file_path(Some(saved_path), clean_after_save);
        if clean_after_save {
            win.dirty_commands.clear();
            win.checkpoint_recovery_journal()?;
        }
    }
    win.app_mut().sync_views();

    let response_bytes = if prepended_reauth { strip_first_control_response_line(&out_bytes)? } else { out_bytes };
    let auth_succeeded = (kind == ControlFrameKind::Auth).then(|| parse_auth_response_ok(&response_bytes));
    let close_connection = quit || kind == ControlFrameKind::Http;
    Ok(LiveControlFrameOutput { response_bytes, quit, close_connection, auth_succeeded })
}

fn configure_app_overrides(app: &mut UiApp, cfg: &DesktopHostConfig) -> Result<(), String> {
    if let Some(locale) = cfg.locale_override.as_deref() {
        let _ = app.set_locale(locale);
    }
    if let Some(theme_raw) = cfg.theme_override.as_deref() {
        let mode = parse_theme_mode(theme_raw)?;
        app.set_theme_mode(mode);
    }
    app.set_gui_only(cfg.gui_only);
    app.sync_views();
    Ok(())
}

/// Per-window desktop UI + engine + HiDPI viewport + WAL recovery state.
#[derive(Debug)]
pub struct DesktopWindowSession {
    id: DesktopWindowId,
    app: UiApp,
    viewport: ViewportMetrics,
    focused: bool,
    in_window_menus: bool,
    active_menu_key: Option<String>,
    chrome_dispatched_actions: Vec<String>,
    dirty_commands: Vec<String>,
    recovery_dir: Option<PathBuf>,
    recovery_journal: Option<CrashRecoveryJournal<DirectoryJournalBackend>>,
    recovered_from_crash: bool,
    recovery_warnings: Vec<JournalRecoveryWarning>,
    hovered_files: Vec<PathBuf>,
    ime_preedit: Option<ImePreeditState>,
    coasting_active: bool,
    last_pointer_pt: Option<[f64; 2]>,
    last_frame: Option<DesktopFrameOutput>,
}

impl DesktopWindowSession {
    /// Window identifier.
    #[must_use]
    pub const fn id(&self) -> DesktopWindowId {
        self.id
    }

    /// Borrow the underlying UI application state for this window.
    #[must_use]
    pub const fn app(&self) -> &UiApp {
        &self.app
    }

    /// Mutably borrow the underlying UI application state for this window.
    pub fn app_mut(&mut self) -> &mut UiApp {
        &mut self.app
    }

    /// Current HiDPI logical + physical viewport metrics for this window.
    #[must_use]
    pub const fn viewport(&self) -> ViewportMetrics {
        self.viewport
    }

    /// Whether this window currently holds focus in [`DesktopHostRuntime`].
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// Whether this window has unsaved edits that should trigger [`CloseDisposition::PromptUnsaved`].
    #[must_use]
    pub fn has_unsaved_changes(&self) -> bool {
        self.app.session().is_dirty() || !self.dirty_commands.is_empty()
    }

    /// Recorded mutating command IDs since the window was created or last saved/loaded cleanly.
    #[must_use]
    pub fn dirty_commands(&self) -> &[String] {
        &self.dirty_commands
    }

    /// Isolated crash-recovery journal directory bound to this window, if active.
    #[must_use]
    pub fn recovery_dir(&self) -> Option<&Path> {
        self.recovery_dir.as_deref()
    }

    /// Whether opening this window recovered a stale lock, WAL commands, or damaged entries from a previous crash.
    #[must_use]
    pub const fn recovered_from_crash(&self) -> bool {
        self.recovered_from_crash
    }

    /// Whether an active crash-recovery journal is attached to this window.
    #[must_use]
    pub const fn has_recovery_journal(&self) -> bool {
        self.recovery_journal.is_some()
    }

    /// Structured WAL recovery warnings recorded when opening this window, if any.
    #[must_use]
    pub fn recovery_warnings(&self) -> &[JournalRecoveryWarning] {
        &self.recovery_warnings
    }

    /// Paths currently hovering over this window between `FileHovered` and `FileDropped`/`FileHoverCancelled`.
    #[must_use]
    pub fn hovered_files(&self) -> &[PathBuf] {
        &self.hovered_files
    }

    /// Active IME preedit text and cursor span prior to [`DesktopHostEvent::ImeCommit`].
    #[must_use]
    pub fn ime_preedit(&self) -> Option<&ImePreeditState> {
        self.ime_preedit.as_ref()
    }

    /// Whether inertial trackpad momentum coasting is actively ticking for this window.
    #[must_use]
    pub const fn is_coasting(&self) -> bool {
        self.coasting_active
    }

    /// Last rendered software compositor frame, if [`DesktopHostEvent::RedrawRequested`] has run.
    #[must_use]
    pub fn last_frame(&self) -> Option<&DesktopFrameOutput> {
        self.last_frame.as_ref()
    }

    /// Native window title combining document filename/untitled status, dirty marker, and `LumenCraft` brand.
    #[must_use]
    pub fn window_title(&self) -> String {
        let base = self
            .app
            .session()
            .file_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Untitled");
        let dirty_marker = if self.has_unsaved_changes() { " *" } else { "" };
        format!("{base}{dirty_marker} — LumenCraft")
    }

    /// Current menu chrome configuration (`InWindowMenuBar` vs `NativeMenuBar`) and localized sections.
    #[must_use]
    pub fn chrome_state(&self) -> DesktopMenuChromeState {
        let mode = if self.in_window_menus { MenuChromeMode::InWindowMenuBar } else { MenuChromeMode::NativeMenuBar };
        DesktopMenuChromeState {
            mode,
            in_window_menus: self.in_window_menus,
            menu_bar_visible: mode.is_in_window(),
            menu_bar_height_pt: if mode.is_in_window() { IN_WINDOW_MENU_BAR_HEIGHT_PT } else { 0.0 },
            active_menu_key: self.active_menu_key.clone(),
            sections: build_menu_sections(&self.app),
            dispatched_actions: self.chrome_dispatched_actions.clone(),
        }
    }

    /// Execute an engine command against this window, tracking `dirty_commands` and persisting to
    /// the crash-recovery WAL journal when mutated (atomically checkpointing undo/redo and
    /// external-file readers).
    ///
    /// # Errors
    /// Returns an error if command execution or WAL journaling fails.
    pub fn execute_and_journal(&mut self, command_id: &str, params: Value) -> Result<CommandOutcome, String> {
        let outcome = self.app.execute_command(command_id, params.clone()).map_err(|err| err.to_string())?;
        if let Some(journal) = &mut self.recovery_journal {
            renew_journal_lease_on_command(journal)?;
        }
        if outcome.mutated {
            self.dirty_commands.push(command_id.to_owned());
            if let Some(journal) = &mut self.recovery_journal {
                self.app
                    .session()
                    .record_outcome_to_journal(journal, command_id, &params, outcome.mutated)
                    .map_err(|err| format!("failed to record WAL entry `{command_id}`: {err}"))?;
            }
        } else if command_id == "project.save" {
            if let Some(saved_path) = extract_project_save_path(&params) {
                self.app.session_mut().set_file_path(Some(saved_path), true);
            }
            if !self.app.session().is_dirty() {
                self.dirty_commands.clear();
                self.checkpoint_recovery_journal()?;
            }
        }
        self.app.sync_views();
        Ok(outcome)
    }

    /// Run a transport-supplied control server closure against this window's [`UiApp`] while
    /// recording every mutating engine/UI step into this window's crash-recovery journal before
    /// responding.
    pub fn with_control_journal_hook<T>(
        &mut self,
        runner: impl FnOnce(&mut UiApp, &mut dyn FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>) -> T,
    ) -> T {
        let app = &mut self.app;
        let recovery_journal = &mut self.recovery_journal;
        let dirty_commands = &mut self.dirty_commands;
        let mut on_post_request = |app_ref: &UiApp, hook_ctx: &ControlPostRequestContext<'_>| {
            apply_control_journal_steps(app_ref, recovery_journal, dirty_commands, hook_ctx)
        };
        runner(app, &mut on_post_request)
    }

    /// Serve authenticated stream control requests (`BufRead`/`Write`) while recording every
    /// mutating engine/UI step into this window's crash-recovery journal before responding.
    ///
    /// # Errors
    /// Propagates underlying stream I/O errors or oversized control-line rejections.
    pub fn serve_control_stream_with_quit<R: BufRead, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
        auth: &ControlAuth,
    ) -> std::io::Result<bool> {
        self.with_control_journal_hook(|app, on_post_request| serve_control_stream_with_quit_and_hook(app, reader, writer, auth, on_post_request))
    }

    /// Dispatch one raw JSON-lines control frame against this window while invoking both WAL
    /// journaling and `on_post_request`, preserving `conn_authenticated` session state cleanly.
    ///
    /// # Errors
    /// Returns an error if serializing auth framing fails, running the control handler hits an
    /// unrecoverable stream I/O failure, or extracting the response line fails.
    pub fn dispatch_live_control_frame_with_hook<F>(
        &mut self,
        raw_bytes: &[u8],
        auth: &ControlAuth,
        conn_authenticated: bool,
        mut on_post_request: F,
    ) -> Result<LiveControlFrameOutput, String>
    where
        F: FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>,
    {
        dispatch_live_control_frame_with_hook(self, raw_bytes, auth, conn_authenticated, &mut on_post_request)
    }

    /// Persist a full `.lumencraft` recovery snapshot and truncate replayed WAL entries.
    ///
    /// # Errors
    /// Returns an error if writing the snapshot fails.
    pub fn checkpoint_recovery_journal(&mut self) -> Result<(), String> {
        let session = self.app.session();
        if let Some(journal) = &mut self.recovery_journal {
            renew_journal_lease_on_command(journal)?;
            session.checkpoint_journal(journal).map_err(|err| format!("failed to checkpoint recovery journal: {err}"))?;
        }
        Ok(())
    }

    fn record_ui_mutation_checkpoint(
        &mut self,
        command_id: Option<&str>,
        prev_undo: usize,
        prev_redo: usize,
        prev_dirty: bool,
    ) -> Result<(), String> {
        if let Some(journal) = &mut self.recovery_journal {
            maybe_renew_journal_lease_periodically(journal);
        }
        let now_dirty = self.app.session().is_dirty();
        let stack_changed = self.app.session().undo_depth() != prev_undo || self.app.session().redo_depth() != prev_redo;
        let became_dirty = !prev_dirty && now_dirty;
        if stack_changed || became_dirty {
            let label = command_id.unwrap_or("ui.edit").to_owned();
            self.dirty_commands.push(label);
            self.checkpoint_recovery_journal()?;
        }
        if let Some(cmd) = command_id
            && cmd == "project.save"
            && !now_dirty
        {
            self.dirty_commands.clear();
            self.checkpoint_recovery_journal()?;
        }
        Ok(())
    }

    /// Render a single RGBA frame matching `self.viewport` (`physical_width = logical * scale_factor`).
    ///
    /// # Errors
    /// Returns an error if the software compositor fails.
    pub fn render_frame(&mut self) -> Result<DesktopFrameOutput, String> {
        if let Some(journal) = &mut self.recovery_journal {
            maybe_renew_journal_lease_periodically(journal);
        }
        let started = Instant::now();
        let saved_preedit_snapshot = if let Some(preedit) = &self.ime_preedit
            && self.app.inline_editor.active
            && !preedit.text.is_empty()
        {
            let snapshot = (self.app.inline_editor.buffer.clone(), self.app.inline_editor.caret_byte, self.app.grid.tool.editing_buffer.clone());
            self.app.inline_editor.insert_text(&preedit.text);
            Some(snapshot)
        } else {
            None
        };

        let raster_res = self.app.render_rgba_for_metrics(self.viewport, BASE_LOGICAL_DPI);
        if let Some((orig_buf, orig_caret, orig_grid_buf)) = saved_preedit_snapshot {
            self.app.inline_editor.buffer = orig_buf;
            self.app.inline_editor.caret_byte = orig_caret;
            self.app.grid.tool.editing_buffer = orig_grid_buf;
        }

        let raster = raster_res.map_err(|err| format!("compositor render failed: {err}"))?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        #[allow(clippy::cast_possible_truncation)]
        let scale_factor = self.viewport.clamped_scale_factor() as f32;

        let output = DesktopFrameOutput {
            window_id: self.id,
            width_px: raster.width_px,
            height_px: raster.height_px,
            scale_factor,
            rgba_bytes: raster.rgba,
            frame_ms: elapsed_ms,
            window_title: self.window_title(),
        };
        self.last_frame = Some(output.clone());
        Ok(output)
    }

    fn close_journal_cleanly(&mut self) -> Result<(), String> {
        if let Some(journal) = self.recovery_journal.take() {
            let _backend =
                self.app.session_mut().close_journal_cleanly(journal).map_err(|err| format!("failed to close recovery journal cleanly: {err}"))?;
        }
        Ok(())
    }
}

struct PreferencesPersistHook(Box<dyn FnMut(&UiApp)>);

impl std::fmt::Debug for PreferencesPersistHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreferencesPersistHook(..)")
    }
}

/// Multi-window desktop host managing event loop dispatch, HiDPI scaling, IME composition,
/// inertial trackpad coasting, file drops, clipboard pasteboard sync, and crash recovery.
#[derive(Debug)]
pub struct DesktopHostRuntime {
    config: DesktopHostConfig,
    next_window_id: u64,
    windows: BTreeMap<DesktopWindowId, DesktopWindowSession>,
    focused_window: Option<DesktopWindowId>,
    clipboard_text: Option<String>,
    clipboard_bundle: Option<ClipboardBundle>,
    preferences_persist_hook: Option<PreferencesPersistHook>,
}

impl DesktopHostRuntime {
    /// Construct a new empty [`DesktopHostRuntime`] governed by `config`.
    #[must_use]
    pub fn new(config: DesktopHostConfig) -> Self {
        Self {
            config,
            next_window_id: 1,
            windows: BTreeMap::new(),
            focused_window: None,
            clipboard_text: None,
            clipboard_bundle: None,
            preferences_persist_hook: None,
        }
    }

    /// Install a callback invoked before closing windows or when UI preference/shortcut state changes.
    pub fn set_preferences_persist_hook<F>(&mut self, hook: F)
    where
        F: FnMut(&UiApp) + 'static,
    {
        self.preferences_persist_hook = Some(PreferencesPersistHook(Box::new(hook)));
    }

    fn persist_window_preferences(&mut self, window_id: DesktopWindowId) {
        if let Some(hook) = self.preferences_persist_hook.as_mut()
            && let Some(win) = self.windows.get(&window_id)
        {
            (hook.0)(&win.app);
        }
    }

    fn capture_pref_fingerprint(app: &UiApp) -> (String, lumencraft_ui_egui::UiThemeMode, String) {
        (app.locale.clone(), app.theme.mode, app.shortcuts.export_overrides_json().unwrap_or_default())
    }

    /// Borrow the active [`DesktopHostConfig`].
    #[must_use]
    pub const fn config(&self) -> &DesktopHostConfig {
        &self.config
    }

    /// Number of currently open windows.
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Ordered identifiers of currently open windows.
    #[must_use]
    pub fn window_ids(&self) -> Vec<DesktopWindowId> {
        self.windows.keys().copied().collect()
    }

    /// Identifier of the currently focused window, if any.
    #[must_use]
    pub const fn focused_window_id(&self) -> Option<DesktopWindowId> {
        self.focused_window
    }

    /// Immutable reference to an open window session.
    ///
    /// # Errors
    /// Returns an error if `window_id` is not open.
    pub fn window(&self, window_id: DesktopWindowId) -> Result<&DesktopWindowSession, String> {
        self.windows.get(&window_id).ok_or_else(|| format!("unknown desktop window `{window_id}`"))
    }

    /// Mutable reference to an open window session.
    ///
    /// # Errors
    /// Returns an error if `window_id` is not open.
    pub fn window_mut(&mut self, window_id: DesktopWindowId) -> Result<&mut DesktopWindowSession, String> {
        self.windows.get_mut(&window_id).ok_or_else(|| format!("unknown desktop window `{window_id}`"))
    }

    /// Immutable reference to the currently focused window, if any.
    #[must_use]
    pub fn focused_window(&self) -> Option<&DesktopWindowSession> {
        self.focused_window.and_then(|id| self.windows.get(&id))
    }

    /// Mutable reference to the currently focused window, if any.
    pub fn focused_window_mut(&mut self) -> Option<&mut DesktopWindowSession> {
        let id = self.focused_window?;
        self.windows.get_mut(&id)
    }

    /// Last plain/TSV/SVG text copied via [`DesktopHostEvent::ClipboardCopy`].
    #[must_use]
    pub fn clipboard_text(&self) -> Option<&str> {
        self.clipboard_text.as_deref()
    }

    /// Last multi-flavor [`ClipboardBundle`] built from a graph/layout [`DesktopHostEvent::ClipboardCopy`].
    #[must_use]
    pub fn clipboard_bundle(&self) -> Option<&ClipboardBundle> {
        self.clipboard_bundle.as_ref()
    }

    /// Open a new desktop window with an optional initial project/template/CSV file.
    ///
    /// # Errors
    /// Returns an error if loading `initial_file` or initializing crash recovery fails.
    pub fn open_window(&mut self, initial_file: Option<PathBuf>, viewport: ViewportMetrics) -> Result<DesktopWindowId, String> {
        let files: Vec<PathBuf> = initial_file.into_iter().collect();
        let session = open_document_from_files(&files)?;
        let attach_recovery = self.config.recovery_root.is_some() && (self.config.new_instance || files.is_empty() || !self.windows.is_empty());
        self.open_window_with_session(session, viewport, attach_recovery)
    }

    /// Open a new desktop window wrapping a pre-constructed [`Session`].
    ///
    /// # Errors
    /// Returns an error if theme/locale configuration or crash-recovery attachment fails.
    pub fn open_window_with_session(
        &mut self,
        session: Session,
        viewport: ViewportMetrics,
        attach_recovery: bool,
    ) -> Result<DesktopWindowId, String> {
        self.open_window_internal(session, None, viewport, attach_recovery, current_epoch_ms())
    }

    /// Open a new desktop window wrapping a pre-constructed [`Session`] and an already-opened
    /// [`CrashRecoveryJournal`] (for example single-instance startup recovery).
    ///
    /// # Errors
    /// Returns an error if theme/locale configuration fails.
    pub fn open_window_with_recovery(
        &mut self,
        session: Session,
        viewport: ViewportMetrics,
        recovery_dir: PathBuf,
        journal: CrashRecoveryJournal<DirectoryJournalBackend>,
        report: Option<RecoveryReport>,
    ) -> Result<DesktopWindowId, String> {
        let id = DesktopWindowId(self.next_window_id);
        self.install_window_session(id, session, viewport, Some((recovery_dir, journal, report)))
    }

    /// Re-open and crash-recover a window directly from an abandoned journal directory `recovery_dir`.
    ///
    /// # Errors
    /// Returns an error if lease recovery or WAL replay fails.
    pub fn recover_window_from_dir(
        &mut self,
        recovery_dir: PathBuf,
        viewport: ViewportMetrics,
        now_epoch_ms: u64,
    ) -> Result<DesktopWindowId, String> {
        let id = DesktopWindowId(self.next_window_id);
        let owner_id = recovery_owner_for_window(id);
        std::fs::create_dir_all(&recovery_dir).map_err(|err| format!("failed to access recovery dir `{}`: {err}", recovery_dir.display()))?;
        let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, owner_id, DEFAULT_RECOVERY_LOCK_TTL_MS, now_epoch_ms)
            .map_err(|err| format!("failed to recover journal in `{}`: {err}", recovery_dir.display()))?;
        self.install_window_session(id, session, viewport, Some((recovery_dir, journal, Some(report))))
    }

    fn open_window_internal(
        &mut self,
        session: Session,
        explicit_recovery_dir: Option<PathBuf>,
        viewport: ViewportMetrics,
        attach_recovery: bool,
        now_epoch_ms: u64,
    ) -> Result<DesktopWindowId, String> {
        let id = DesktopWindowId(self.next_window_id);
        let recovery = if attach_recovery {
            if let Some(dir) = explicit_recovery_dir.or_else(|| self.config.recovery_root.clone()) {
                let journal_dir = claim_isolated_journal_dir(&dir, self.config.new_instance, id, now_epoch_ms)?;
                let owner_id = recovery_owner_for_window(id);
                let opened = Session::open_dir_with_recovery(&journal_dir, owner_id, DEFAULT_RECOVERY_LOCK_TTL_MS, now_epoch_ms)
                    .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", journal_dir.display()))?;
                // Preserve passed-in file session if initial_file was supplied alongside recovery.
                let (recovered_session, mut journal, report) = opened;
                let final_session = if session.document().data_sheets.is_empty()
                    && session.document().graph_sheets.is_empty()
                    && session.document().info_sheets.is_empty()
                    && session.file_path().is_none()
                {
                    recovered_session
                } else {
                    if let Err(err) = session.checkpoint_journal(&mut journal) {
                        abort_journal_lease_best_effort(journal);
                        return Err(format!("failed to seed initial recovery snapshot: {err}"));
                    }
                    session
                };
                return self.install_window_session(id, final_session, viewport, Some((journal_dir, journal, Some(report))));
            }
            None
        } else {
            None
        };
        self.install_window_session(id, session, viewport, recovery)
    }

    fn install_window_session(
        &mut self,
        id: DesktopWindowId,
        session: Session,
        viewport: ViewportMetrics,
        recovery: Option<(PathBuf, CrashRecoveryJournal<DirectoryJournalBackend>, Option<RecoveryReport>)>,
    ) -> Result<DesktopWindowId, String> {
        let effective_viewport = if viewport.logical_width > 0.0 && viewport.logical_height > 0.0 { viewport } else { self.config.initial_viewport };

        let mut app = UiApp::from_session(session);
        if let Err(err) = configure_app_overrides(&mut app, &self.config) {
            if let Some((_, journal, _)) = recovery {
                abort_journal_lease_best_effort(journal);
            }
            return Err(err);
        }
        self.next_window_id = self.next_window_id.saturating_add(1);

        let (recovery_dir, recovery_journal, recovered_from_crash, recovery_warnings, recovered_cmds) = match recovery {
            Some((dir, journal, report_opt)) => {
                let recovered = report_opt.as_ref().is_some_and(|r| {
                    r.recovered_from_stale_lock.is_some()
                        || !r.recovered_commands.is_empty()
                        || r.total_discarded_corrupt_lines() > 0
                        || !r.warnings.is_empty()
                });
                let warnings = report_opt.as_ref().map(|r| r.warnings.clone()).unwrap_or_default();
                let cmds: Vec<String> = report_opt
                    .as_ref()
                    .map(|r| r.recovered_commands.iter().filter_map(|v| v.get("command_id").and_then(Value::as_str).map(ToOwned::to_owned)).collect())
                    .unwrap_or_default();
                (Some(dir), Some(journal), recovered, warnings, cmds)
            }
            None => (None, None, false, Vec::new(), Vec::new()),
        };

        for win in self.windows.values_mut() {
            win.focused = false;
        }
        self.focused_window = Some(id);

        let win = DesktopWindowSession {
            id,
            app,
            viewport: effective_viewport,
            focused: true,
            in_window_menus: self.config.in_window_menus,
            active_menu_key: None,
            chrome_dispatched_actions: Vec::new(),
            dirty_commands: recovered_cmds,
            recovery_dir,
            recovery_journal,
            recovered_from_crash,
            recovery_warnings,
            hovered_files: Vec::new(),
            ime_preedit: None,
            coasting_active: false,
            last_pointer_pt: None,
            last_frame: None,
        };
        self.windows.insert(id, win);
        Ok(id)
    }

    /// Force-close all open windows and finalize any attached crash-recovery journals cleanly.
    ///
    /// # Errors
    /// Returns an error if closing any window's recovery journal fails.
    pub fn close_all_force(&mut self) -> Result<(), String> {
        let ids: Vec<DesktopWindowId> = self.windows.keys().copied().collect();
        let mut errors = Vec::new();
        for id in ids {
            self.persist_window_preferences(id);
            if let Err(err) = self.handle_window_close_requested(id, true) {
                errors.push(format!("window {id}: {err}"));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }

    /// Dispatch a single [`DesktopHostEvent`] into the multi-window desktop host state machine.
    ///
    /// # Errors
    /// Returns an error if the target window is unknown or a triggered engine/UI operation fails.
    pub fn dispatch(&mut self, event: DesktopHostEvent) -> Result<DesktopEventOutcome, String> {
        match event {
            DesktopHostEvent::WindowOpened { initial_file, viewport } => self.handle_window_opened(initial_file, viewport),
            DesktopHostEvent::WindowCloseRequested { window_id, force } => {
                self.persist_window_preferences(window_id);
                self.handle_window_close_requested(window_id, force)
            }
            DesktopHostEvent::WindowFocused { window_id } => self.handle_window_focused(window_id),
            DesktopHostEvent::Resized { window_id, width_px, height_px } => self.handle_resized(window_id, width_px, height_px),
            DesktopHostEvent::ScaleFactorChanged { window_id, scale_factor } => self.handle_scale_factor_changed(window_id, scale_factor),
            DesktopHostEvent::PointerMoved { window_id, x_pt, y_pt, modifiers } => self.handle_pointer_moved(window_id, x_pt, y_pt, modifiers),
            DesktopHostEvent::PointerButton { window_id, x_pt, y_pt, button, pressed, click_count, modifiers } => {
                self.handle_pointer_button(PointerButtonInput { window_id, x_pt, y_pt, button, pressed, click_count, modifiers })
            }
            DesktopHostEvent::MouseWheel { window_id, x_pt, y_pt, delta_x, delta_y, modifiers } => {
                self.handle_mouse_wheel(window_id, x_pt, y_pt, delta_x, delta_y, modifiers)
            }
            DesktopHostEvent::TrackpadMagnify { window_id, anchor_x_pt, anchor_y_pt, magnification_delta } => {
                self.handle_trackpad_magnify(window_id, anchor_x_pt, anchor_y_pt, magnification_delta)
            }
            DesktopHostEvent::TrackpadScroll { window_id, delta_x, delta_y, dt_sec, phase_ended } => {
                self.handle_trackpad_scroll(window_id, delta_x, delta_y, dt_sec, phase_ended)
            }
            DesktopHostEvent::KeyboardInput { window_id, key, pressed, modifiers, text } => {
                self.handle_keyboard_input(window_id, key, pressed, modifiers, text)
            }
            DesktopHostEvent::ImePreedit { window_id, text, cursor } => self.handle_ime_preedit(window_id, text, cursor),
            DesktopHostEvent::ImeCommit { window_id, text } => self.handle_ime_commit(window_id, text),
            DesktopHostEvent::FileHovered { window_id, paths } => self.handle_file_hovered(window_id, paths),
            DesktopHostEvent::FileHoverCancelled { window_id } => self.handle_file_hover_cancelled(window_id),
            DesktopHostEvent::FileDropped { window_id, path } => self.handle_file_dropped(window_id, path),
            DesktopHostEvent::MenuAction { window_id, command_id } => self.handle_menu_action(window_id, command_id),
            DesktopHostEvent::ClipboardCopy { window_id } => self.handle_clipboard_copy(window_id),
            DesktopHostEvent::ClipboardPaste { window_id, text_override } => self.handle_clipboard_paste(window_id, text_override),
            DesktopHostEvent::AnimationTick { dt_sec } => self.handle_animation_tick(dt_sec),
            DesktopHostEvent::RedrawRequested { window_id } => self.handle_redraw_requested(window_id),
        }
    }

    /// Deterministically replay a newline-delimited JSONL script of [`DesktopHostEvent`] lines.
    ///
    /// Blank lines and `#` / `//` comment lines are skipped cleanly.
    ///
    /// # Errors
    /// Returns an error naming the 1-based line number if parsing or dispatching any event fails.
    pub fn run_event_script_jsonl(&mut self, jsonl: &str) -> Result<Vec<DesktopEventOutcome>, String> {
        let mut outcomes = Vec::new();
        for (idx, raw_line) in jsonl.lines().enumerate() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            let event: DesktopHostEvent =
                serde_json::from_str(trimmed).map_err(|err| format!("event script JSON error on line {}: {err}", idx.saturating_add(1)))?;
            let outcome = self.dispatch(event).map_err(|err| format!("event script error on line {}: {err}", idx.saturating_add(1)))?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    fn handle_window_opened(&mut self, initial_file: Option<PathBuf>, viewport: ViewportMetrics) -> Result<DesktopEventOutcome, String> {
        let id = self.open_window(initial_file, viewport)?;
        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("window.opened"),
            window_id: Some(id),
            opened_window_id: Some(id),
            ..Default::default()
        })
    }

    fn handle_window_close_requested(&mut self, window_id: DesktopWindowId, force: bool) -> Result<DesktopEventOutcome, String> {
        let prompt = {
            let win = self.window(window_id)?;
            if !force && win.has_unsaved_changes() {
                let dirty_commands = if win.dirty_commands.is_empty() { vec![String::from("unsaved.edits")] } else { win.dirty_commands.clone() };
                Some(CloseDisposition::PromptUnsaved { window_id, dirty_commands })
            } else {
                None
            }
        };

        if let Some(disp) = prompt {
            return Ok(DesktopEventOutcome {
                handled: true,
                action: String::from("window.close_prompt_unsaved"),
                window_id: Some(window_id),
                close_disposition: Some(disp),
                ..Default::default()
            });
        }

        let close_res = if let Some(mut removed) = self.windows.remove(&window_id) { removed.close_journal_cleanly() } else { Ok(()) };
        if self.focused_window == Some(window_id) {
            self.focused_window = self.windows.keys().next_back().copied();
            if let Some(next_id) = self.focused_window
                && let Some(next_win) = self.windows.get_mut(&next_id)
            {
                next_win.focused = true;
            }
        }
        close_res?;
        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("window.closed"),
            window_id: Some(window_id),
            close_disposition: Some(CloseDisposition::Closed),
            ..Default::default()
        })
    }

    fn handle_window_focused(&mut self, window_id: DesktopWindowId) -> Result<DesktopEventOutcome, String> {
        if !self.windows.contains_key(&window_id) {
            return Err(format!("unknown desktop window `{window_id}`"));
        }
        for (id, win) in &mut self.windows {
            win.focused = *id == window_id;
        }
        self.focused_window = Some(window_id);
        Ok(DesktopEventOutcome { handled: true, action: String::from("window.focused"), window_id: Some(window_id), ..Default::default() })
    }

    fn handle_resized(&mut self, window_id: DesktopWindowId, width_px: u32, height_px: u32) -> Result<DesktopEventOutcome, String> {
        let win = self.window_mut(window_id)?;
        if width_px == 0 || height_px == 0 {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("window.resize_ignored_zero"),
                window_id: Some(window_id),
                ..Default::default()
            });
        }
        let scale = win.viewport.clamped_scale_factor();
        win.viewport = ViewportMetrics::from_physical(width_px, height_px, scale);
        Ok(DesktopEventOutcome { handled: true, action: String::from("window.resized"), window_id: Some(window_id), ..Default::default() })
    }

    fn handle_scale_factor_changed(&mut self, window_id: DesktopWindowId, scale_factor: f32) -> Result<DesktopEventOutcome, String> {
        let win = self.window_mut(window_id)?;
        let scale = f64::from(scale_factor);
        if !scale.is_finite() || scale <= 0.0 {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("window.scale_factor_ignored"),
                window_id: Some(window_id),
                ..Default::default()
            });
        }
        win.viewport = ViewportMetrics::new(win.viewport.logical_width, win.viewport.logical_height, scale);
        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("window.scale_factor_changed"),
            window_id: Some(window_id),
            ..Default::default()
        })
    }

    fn dispatch_ui_event_on_window(&mut self, window_id: DesktopWindowId, ui_event: UiInputEvent) -> Result<DesktopEventOutcome, String> {
        let check_shortcuts = !matches!(ui_event, UiInputEvent::PointerMove { .. }) && self.preferences_persist_hook.is_some();
        let win = self.window_mut(window_id)?;
        let prev_undo = win.app.session().undo_depth();
        let prev_redo = win.app.session().redo_depth();
        let prev_dirty = win.app.session().is_dirty();
        let prev_basic = (win.app.locale.clone(), win.app.theme.mode);
        let prev_shortcuts = if check_shortcuts { Some(win.app.shortcuts.export_overrides_json().unwrap_or_default()) } else { None };
        let ui_out: UiEventOutcome = win.app.dispatch_ui_event(ui_event, win.viewport)?;
        win.record_ui_mutation_checkpoint(ui_out.command_id.as_deref(), prev_undo, prev_redo, prev_dirty)?;
        let prefs_changed = prev_basic != (win.app.locale.clone(), win.app.theme.mode)
            || prev_shortcuts.as_deref().is_some_and(|before| before != win.app.shortcuts.export_overrides_json().unwrap_or_default());
        if prefs_changed {
            self.persist_window_preferences(window_id);
        }
        Ok(DesktopEventOutcome {
            handled: ui_out.handled,
            action: ui_out.action,
            window_id: Some(window_id),
            command_id: ui_out.command_id,
            command_result: ui_out.command_result,
            ..Default::default()
        })
    }

    fn handle_pointer_moved(
        &mut self,
        window_id: DesktopWindowId,
        x_pt: f64,
        y_pt: f64,
        modifiers: KeyModifiers,
    ) -> Result<DesktopEventOutcome, String> {
        {
            let win = self.window_mut(window_id)?;
            win.last_pointer_pt = Some([x_pt, y_pt]);
        }
        self.dispatch_ui_event_on_window(window_id, UiInputEvent::PointerMove { x: x_pt, y: y_pt, modifiers })
    }

    fn handle_pointer_button(&mut self, input: PointerButtonInput) -> Result<DesktopEventOutcome, String> {
        let PointerButtonInput { window_id, x_pt, y_pt, button, pressed, click_count, modifiers } = input;
        {
            let win = self.window_mut(window_id)?;
            win.last_pointer_pt = Some([x_pt, y_pt]);
            if pressed {
                win.ime_preedit = None;
            }
        }
        if pressed && button == PointerButton::Primary {
            let _ = self.handle_window_focused(window_id)?;
        }
        let ui_event = if pressed && click_count >= 2 {
            UiInputEvent::DoubleClick { x: x_pt, y: y_pt, button, modifiers }
        } else if pressed {
            UiInputEvent::PointerDown { x: x_pt, y: y_pt, button, modifiers }
        } else {
            UiInputEvent::PointerUp { x: x_pt, y: y_pt, button, modifiers }
        };
        self.dispatch_ui_event_on_window(window_id, ui_event)
    }

    fn handle_mouse_wheel(
        &mut self,
        window_id: DesktopWindowId,
        x_pt: f64,
        y_pt: f64,
        delta_x: f64,
        delta_y: f64,
        modifiers: KeyModifiers,
    ) -> Result<DesktopEventOutcome, String> {
        let (grid_sheet_id, over_scene_paper) = {
            let win = self.window(window_id)?;
            let layout = win.app.layout_map(win.viewport);
            let hit = layout.hit_test_logical(x_pt, y_pt);
            let doc = win.app.document();
            let over_scene = matches!(hit, WorkspaceHitTarget::ScenePaper { .. })
                || (matches!(hit, WorkspaceHitTarget::Pane(ActiveWorkspacePane::MainCanvasOrGrid)) && layout.scene_paper.is_some());
            let grid_target = match hit {
                WorkspaceHitTarget::GridCell { sheet_id, .. } | WorkspaceHitTarget::GridXCell { sheet_id, .. } => Some(sheet_id),
                WorkspaceHitTarget::Pane(ActiveWorkspacePane::MainCanvasOrGrid) if layout.scene_paper.is_none() => {
                    doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).cloned()
                }
                _ => None,
            };
            (grid_target, over_scene)
        };

        if let Some(sheet_id) = grid_sheet_id {
            let win = self.window_mut(window_id)?;
            if let Some(sheet) = win.app.document().data_sheet(&sheet_id).cloned() {
                win.app.grid.tool.scroll_for_sheet(&sheet, -delta_x, -delta_y);
                return Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("grid.scroll"),
                    window_id: Some(window_id),
                    command_result: Some(json!({
                        "sheet_id": sheet_id.as_str(),
                        "scroll_x_px": win.app.grid.tool.viewport.scroll_x_px,
                        "scroll_y_px": win.app.grid.tool.viewport.scroll_y_px,
                    })),
                    ..Default::default()
                });
            }
        }

        if !over_scene_paper {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("wheel.non_canvas_ignored"),
                window_id: Some(window_id),
                ..Default::default()
            });
        }

        let ui_event = if !modifiers.ctrl && !modifiers.meta && delta_x.abs() > f64::EPSILON {
            UiInputEvent::InertialPan { delta_x, delta_y, dt_sec: 0.016, step_coast: false }
        } else {
            UiInputEvent::WheelZoom { x: x_pt, y: y_pt, delta_y, modifiers }
        };
        self.dispatch_ui_event_on_window(window_id, ui_event)
    }

    fn handle_trackpad_magnify(
        &mut self,
        window_id: DesktopWindowId,
        anchor_x_pt: f64,
        anchor_y_pt: f64,
        magnification_delta: f64,
    ) -> Result<DesktopEventOutcome, String> {
        if !magnification_delta.is_finite() {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("trackpad.magnify_ignored"),
                window_id: Some(window_id),
                ..Default::default()
            });
        }
        let scale_delta = (1.0 + magnification_delta).clamp(0.05, 10.0);
        self.dispatch_ui_event_on_window(window_id, UiInputEvent::PinchZoom { x: anchor_x_pt, y: anchor_y_pt, scale_delta })
    }

    fn handle_trackpad_scroll(
        &mut self,
        window_id: DesktopWindowId,
        delta_x: f64,
        delta_y: f64,
        dt_sec: f64,
        phase_ended: bool,
    ) -> Result<DesktopEventOutcome, String> {
        let outcome = self.dispatch_ui_event_on_window(window_id, UiInputEvent::InertialPan { delta_x, delta_y, dt_sec, step_coast: false })?;
        let win = self.window_mut(window_id)?;
        let vel = win.app.canvas_dialogs.pan_zoom.vel_pt_per_sec;
        let speed = vel[0].hypot(vel[1]);
        win.coasting_active = phase_ended && speed >= VELOCITY_COAST_EPSILON_PT_PER_SEC;
        Ok(outcome)
    }

    fn handle_keyboard_input(
        &mut self,
        window_id: DesktopWindowId,
        key: String,
        pressed: bool,
        modifiers: KeyModifiers,
        text: Option<String>,
    ) -> Result<DesktopEventOutcome, String> {
        let _ = self.window(window_id)?;
        if !pressed {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("key.released"),
                window_id: Some(window_id),
                ..Default::default()
            });
        }

        let trimmed = key.trim();
        let control_key = matches!(trimmed, "Enter" | "Return" | "Escape" | "Esc" | "Backspace" | "Tab");
        let (inline_active, dialog_active, shortcut_route) = {
            let win = self.window(window_id)?;
            let inline = win.app.inline_editor.active;
            let dialog = win.app.canvas_dialogs.active_dialog != ActiveDialog::None;
            let routed = if !inline && !dialog { match_routed_desktop_shortcut(&win.app, trimmed, modifiers) } else { None };
            (inline, dialog, routed)
        };
        if inline_active
            && !control_key
            && !modifiers.ctrl
            && !modifiers.meta
            && !modifiers.alt
            && let Some(input_text) = text.as_ref().filter(|t| !t.is_empty())
        {
            return self.dispatch_ui_event_on_window(window_id, UiInputEvent::TextInput { text: input_text.clone() });
        }

        if let Some(routed_cmd) = shortcut_route
            && !dialog_active
        {
            return self.dispatch_routed_shortcut(window_id, &routed_cmd);
        }

        let out = self.dispatch_ui_event_on_window(window_id, UiInputEvent::KeyDown { key: key.clone(), modifiers })?;
        if out.handled {
            return Ok(out);
        }

        if (modifiers.meta || modifiers.ctrl) && !modifiers.alt && !modifiers.shift {
            if trimmed.eq_ignore_ascii_case("c") {
                return self.handle_clipboard_copy(window_id);
            }
            if trimmed.eq_ignore_ascii_case("v") {
                return self.handle_clipboard_paste(window_id, None);
            }
        }
        Ok(out)
    }

    fn handle_ime_preedit(
        &mut self,
        window_id: DesktopWindowId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> Result<DesktopEventOutcome, String> {
        let win = self.window_mut(window_id)?;
        win.ime_preedit = if text.is_empty() { None } else { Some(ImePreeditState { text, cursor }) };
        Ok(DesktopEventOutcome { handled: true, action: String::from("ime.preedit"), window_id: Some(window_id), ..Default::default() })
    }

    fn handle_ime_commit(&mut self, window_id: DesktopWindowId, text: String) -> Result<DesktopEventOutcome, String> {
        {
            let win = self.window_mut(window_id)?;
            win.ime_preedit = None;
        }
        self.dispatch_ui_event_on_window(window_id, UiInputEvent::TextInput { text })
    }

    fn handle_file_hovered(&mut self, window_id: DesktopWindowId, paths: Vec<PathBuf>) -> Result<DesktopEventOutcome, String> {
        let win = self.window_mut(window_id)?;
        win.hovered_files = paths;
        Ok(DesktopEventOutcome { handled: true, action: String::from("file.hovered"), window_id: Some(window_id), ..Default::default() })
    }

    fn handle_file_hover_cancelled(&mut self, window_id: DesktopWindowId) -> Result<DesktopEventOutcome, String> {
        let win = self.window_mut(window_id)?;
        win.hovered_files.clear();
        Ok(DesktopEventOutcome { handled: true, action: String::from("file.hover_cancelled"), window_id: Some(window_id), ..Default::default() })
    }

    fn handle_file_dropped(&mut self, window_id: DesktopWindowId, path: PathBuf) -> Result<DesktopEventOutcome, String> {
        {
            let win = self.window_mut(window_id)?;
            win.hovered_files.clear();
        }
        match classify_and_load_path(&path)? {
            LoadedPathPayload::DelimitedTable { title, is_tsv, .. } => {
                let mut params = json!({
                    "path": path.to_string_lossy(),
                    "title": title,
                    "archetype": "column",
                });
                if is_tsv && let Some(obj) = params.as_object_mut() {
                    obj.insert(String::from("delimiter"), json!("\t"));
                }
                let win = self.window_mut(window_id)?;
                let outcome = win.execute_and_journal("table.importCsv", params)?;
                if let Some(sheet_id) = outcome.payload.get("sheet_id").and_then(Value::as_str) {
                    let _ = win.app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }));
                    win.app.sync_views();
                }
                Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("file.dropped_imported_table"),
                    window_id: Some(window_id),
                    command_id: Some(String::from("table.importCsv")),
                    command_result: Some(outcome.payload),
                    ..Default::default()
                })
            }
            LoadedPathPayload::Project { doc, archive_path, clean_archive } => {
                let (open_new, viewport) = {
                    let win = self.window(window_id)?;
                    (self.config.new_instance || win.has_unsaved_changes(), win.viewport)
                };
                if open_new {
                    let mut session = Session::from_document(*doc);
                    if let Some(p) = archive_path {
                        session.set_file_path(Some(p), clean_archive);
                    }
                    let new_id = self.open_window_with_session(session, viewport, self.config.recovery_root.is_some())?;
                    return Ok(DesktopEventOutcome {
                        handled: true,
                        action: String::from("file.dropped_opened_new_window"),
                        window_id: Some(window_id),
                        opened_window_id: Some(new_id),
                        ..Default::default()
                    });
                }

                let mut session = Session::from_document(*doc);
                if let Some(p) = archive_path {
                    session.set_file_path(Some(p), clean_archive);
                }
                let win = self.window_mut(window_id)?;
                let prev_locale = win.app.locale.clone();
                let prev_theme = win.app.theme.mode;
                let prev_shortcuts = win.app.shortcuts.clone();
                let prev_rulers = win.app.canvas_dialogs.rulers_visible;
                let prev_gui_only = win.app.is_gui_only();

                let mut next_app = UiApp::from_session(session);
                let _ = next_app.set_locale(&prev_locale);
                next_app.set_theme_mode(prev_theme);
                next_app.shortcuts = prev_shortcuts;
                next_app.canvas_dialogs.rulers_visible = prev_rulers;
                next_app.set_gui_only(prev_gui_only);
                next_app.sync_views();
                win.app = next_app;
                win.dirty_commands.clear();
                win.ime_preedit = None;
                win.checkpoint_recovery_journal()?;
                Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("file.dropped_loaded_in_window"),
                    window_id: Some(window_id),
                    ..Default::default()
                })
            }
        }
    }

    fn dispatch_routed_shortcut(&mut self, window_id: DesktopWindowId, command_id: &str) -> Result<DesktopEventOutcome, String> {
        if command_id == "sheet.createData" {
            let result_sheet_id = {
                let win = self.window(window_id)?;
                let doc = win.app.document();
                doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()).cloned()
            };
            if let Some(rs_id) = result_sheet_id {
                let win = self.window_mut(window_id)?;
                let payload = open_desktop_dialog(&mut win.app, DesktopDialogOpener::EditAnalysisParameters, Some(rs_id.as_str()), None)?;
                win.app.sync_views();
                return Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("shortcut:EditAnalysisParameters"),
                    window_id: Some(window_id),
                    command_id: Some(String::from("analysis.updateParams")),
                    command_result: Some(payload),
                    ..Default::default()
                });
            }
        }
        let mut out = self.handle_menu_action(window_id, command_id.to_owned())?;
        if out.action.starts_with("menu.") {
            out.action = format!("shortcut:{command_id}");
        }
        Ok(out)
    }

    fn handle_menu_action(&mut self, window_id: DesktopWindowId, command_id: String) -> Result<DesktopEventOutcome, String> {
        let trimmed = command_id.trim();
        if trimmed.is_empty() {
            return Err(String::from("MenuAction command_id must not be empty"));
        }
        {
            let win = self.window_mut(window_id)?;
            win.chrome_dispatched_actions.push(trimmed.to_owned());
        }

        let route = {
            let win = self.window(window_id)?;
            resolve_desktop_command_route(&win.app, win.app.session().file_path(), trimmed)?
        };

        match route {
            DesktopCommandRoute::HostAction { action } => match action {
                "window.new" => {
                    let viewport = self.window(window_id)?.viewport;
                    let new_id = self.open_window(None, viewport)?;
                    Ok(DesktopEventOutcome {
                        handled: true,
                        action: String::from("menu.new_window"),
                        window_id: Some(window_id),
                        opened_window_id: Some(new_id),
                        command_id: Some(trimmed.to_owned()),
                        ..Default::default()
                    })
                }
                "window.close" => {
                    self.persist_window_preferences(window_id);
                    self.handle_window_close_requested(window_id, false)
                }
                "edit.copy" => self.handle_clipboard_copy(window_id),
                "edit.paste" => self.handle_clipboard_paste(window_id, None),
                other => Err(format!("unhandled host action `{other}`")),
            },
            DesktopCommandRoute::UiCommand { ui_command_id, params } => {
                let has_hook = self.preferences_persist_hook.is_some();
                let win = self.window_mut(window_id)?;
                let prev_prefs = has_hook.then(|| Self::capture_pref_fingerprint(&win.app));
                let ui_res = dispatch_ui_command(&mut win.app, &ui_command_id, &params)?
                    .ok_or_else(|| format!("UI command `{ui_command_id}` is not registered in dispatcher"))?;
                win.app.sync_views();
                let changed = prev_prefs.is_some_and(|before| before != Self::capture_pref_fingerprint(&win.app));
                if changed {
                    self.persist_window_preferences(window_id);
                }
                Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("menu.ui_command"),
                    window_id: Some(window_id),
                    command_id: Some(trimmed.to_owned()),
                    command_result: Some(ui_res),
                    ..Default::default()
                })
            }
            DesktopCommandRoute::UiDialogOpener { kind, target_sheet_id, query } => {
                let win = self.window_mut(window_id)?;
                let payload = open_desktop_dialog(&mut win.app, kind, target_sheet_id.as_deref(), query.as_deref())?;
                win.app.sync_views();
                Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("menu.dialog_opened"),
                    window_id: Some(window_id),
                    command_id: Some(trimmed.to_owned()),
                    command_result: Some(payload),
                    ..Default::default()
                })
            }
            DesktopCommandRoute::EngineCommand { engine_command_id, params } => {
                let has_hook = self.preferences_persist_hook.is_some();
                let win = self.window_mut(window_id)?;
                let prev_prefs = has_hook.then(|| Self::capture_pref_fingerprint(&win.app));
                let outcome = win.execute_and_journal(&engine_command_id, params)?;
                if prev_prefs.is_some_and(|before| before != Self::capture_pref_fingerprint(&win.app)) {
                    self.persist_window_preferences(window_id);
                }
                Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("menu.engine_command"),
                    window_id: Some(window_id),
                    command_id: Some(trimmed.to_owned()),
                    command_result: Some(outcome.payload),
                    ..Default::default()
                })
            }
        }
    }

    fn handle_clipboard_copy(&mut self, window_id: DesktopWindowId) -> Result<DesktopEventOutcome, String> {
        let (copied_text, copied_bundle, action) = {
            let win = self.window(window_id)?;
            let doc = win.app.document();
            let active_id = doc.active_sheet_id.as_ref();
            if let Some(sheet_id) = active_id
                && let Some(ds) = doc.data_sheet(sheet_id)
            {
                let dec = win.app.locale_preset.decimal_separator;
                let tsv = if let Some((min, max)) = win.app.grid.tool.selection_bounds() {
                    let range = SheetCellRange::new(CellCoord::new(min.row, min.col, min.subcol), CellCoord::new(max.row, max.col, max.subcol));
                    ClipboardBundle::tsv_from_datasheet_range(ds, range, dec)
                } else {
                    ClipboardBundle::tsv_from_datasheet(ds, dec)?
                };
                let _ = write_os_pasteboard(&tsv);
                (tsv, None, "clipboard.copied_tsv")
            } else if let Some(sheet_id) = active_id
                && matches!(doc.sheet_section(sheet_id), Some(SheetSection::Graphs | SheetSection::Layouts))
            {
                let dpi = win.viewport.dpi_for_base(BASE_LOGICAL_DPI);
                let bundle = ClipboardBundle::from_sheet(win.app.session(), sheet_id, dpi)?;
                let _ = write_os_clipboard_bundle(&bundle);
                let primary = if !bundle.svg.is_empty() { bundle.svg.clone() } else { bundle.html.clone() };
                (primary, Some(bundle), "clipboard.copied_bundle")
            } else {
                return Ok(DesktopEventOutcome {
                    handled: false,
                    action: String::from("clipboard.copy_nothing_selected"),
                    window_id: Some(window_id),
                    ..Default::default()
                });
            }
        };

        self.clipboard_text = Some(copied_text);
        self.clipboard_bundle = copied_bundle;
        Ok(DesktopEventOutcome { handled: true, action: String::from(action), window_id: Some(window_id), ..Default::default() })
    }

    fn handle_clipboard_paste(&mut self, window_id: DesktopWindowId, text_override: Option<String>) -> Result<DesktopEventOutcome, String> {
        let paste_text = match text_override {
            Some(explicit) => explicit,
            None => match self.clipboard_text.clone() {
                Some(cached) => cached,
                None => match read_os_pasteboard() {
                    Ok(status) if status.available => match status.text {
                        Some(os_text) => os_text,
                        None => {
                            return Ok(DesktopEventOutcome {
                                handled: false,
                                action: String::from("clipboard.paste_empty"),
                                window_id: Some(window_id),
                                ..Default::default()
                            });
                        }
                    },
                    _ => {
                        return Ok(DesktopEventOutcome {
                            handled: false,
                            action: String::from("clipboard.paste_empty"),
                            window_id: Some(window_id),
                            ..Default::default()
                        });
                    }
                },
            },
        };

        if self.window(window_id)?.app.inline_editor.active {
            return self.dispatch_ui_event_on_window(window_id, UiInputEvent::TextInput { text: paste_text });
        }

        let win = self.window_mut(window_id)?;
        let target_sheet_id = win
            .app
            .document()
            .active_sheet_id
            .as_ref()
            .filter(|id| win.app.document().data_sheet(id).is_some())
            .cloned()
            .or_else(|| win.app.document().data_sheets.first().map(|ds| ds.id.clone()));

        let Some(sheet_id) = target_sheet_id else {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("clipboard.paste_no_datasheet"),
                window_id: Some(window_id),
                ..Default::default()
            });
        };

        let anchor = win
            .app
            .grid
            .tool
            .active
            .map_or_else(|| json!({ "row": 0, "col": 0, "subcol": 0 }), |c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol }));
        let dec = win.app.locale_preset.decimal_separator.to_string();
        let params = json!({
            "sheet_id": sheet_id.as_str(),
            "text": paste_text,
            "decimal_separator": dec,
            "anchor": anchor,
        });
        let outcome = win.execute_and_journal("table.paste", params)?;
        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("clipboard.pasted_table"),
            window_id: Some(window_id),
            command_id: Some(String::from("table.paste")),
            command_result: Some(outcome.payload),
            ..Default::default()
        })
    }

    fn handle_animation_tick(&mut self, dt_sec: f64) -> Result<DesktopEventOutcome, String> {
        for win in self.windows.values_mut() {
            if let Some(journal) = &mut win.recovery_journal {
                maybe_renew_journal_lease_periodically(journal);
            }
        }
        if !dt_sec.is_finite() || dt_sec <= 0.0 {
            return Ok(DesktopEventOutcome { handled: false, action: String::from("animation.tick_ignored"), ..Default::default() });
        }

        let coasting_ids: Vec<DesktopWindowId> = self.windows.iter().filter_map(|(id, win)| win.coasting_active.then_some(*id)).collect();
        let mut any_moving = false;
        for id in &coasting_ids {
            let out = self.dispatch_ui_event_on_window(*id, UiInputEvent::InertialPan { delta_x: 0.0, delta_y: 0.0, dt_sec, step_coast: true })?;
            let still_moving = out.command_result.as_ref().and_then(|v| v.get("moving")).and_then(Value::as_bool).unwrap_or(false);
            if let Some(win) = self.windows.get_mut(id) {
                win.coasting_active = still_moving;
            }
            any_moving |= still_moving;
        }
        Ok(DesktopEventOutcome {
            handled: !coasting_ids.is_empty(),
            action: if any_moving { String::from("animation.coasting") } else { String::from("animation.idle") },
            window_id: coasting_ids.first().copied(),
            command_result: Some(json!({ "ticked_windows": coasting_ids.len(), "moving": any_moving })),
            ..Default::default()
        })
    }

    fn handle_redraw_requested(&mut self, window_id: DesktopWindowId) -> Result<DesktopEventOutcome, String> {
        let win = self.window_mut(window_id)?;
        let frame = win.render_frame()?;
        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("window.redrawn"),
            window_id: Some(window_id),
            frame: Some(frame),
            ..Default::default()
        })
    }
}

/// Interactive dialog, wizard, or inline editor opened directly on [`UiApp`] when a menu/shortcut
/// command needs user-supplied parameters rather than executing a headless engine command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopDialogOpener {
    WelcomeEmptyTable,
    WelcomeOpenProject,
    WelcomeFromTemplate,
    ImportWizardPaste,
    ImportWizardPasteLink,
    ImportWizardExternalFile,
    VariablesDialog,
    EditAnalysisParameters,
    ChangeDataAnalyzed,
    ApplyStyleFrom,
    InfoNotesEditor,
    InfoConstantHookPicker,
    CommandPalette,
}

/// Resolved desktop command dispatch plan produced by [`resolve_desktop_command_route`].
#[derive(Debug, Clone, PartialEq)]
pub enum DesktopCommandRoute {
    /// Window lifecycle or host clipboard command handled directly by [`DesktopHostRuntime`].
    HostAction { action: &'static str },
    /// UI command executed through [`lumencraft_ui_egui::dispatch_ui_command`].
    UiCommand { ui_command_id: String, params: Value },
    /// Interactive UI dialog, wizard, or inline editor opened directly on [`UiApp`].
    UiDialogOpener { kind: DesktopDialogOpener, target_sheet_id: Option<String>, query: Option<String> },
    /// Engine command executed with non-null context parameters synthesized from active [`UiApp`] state.
    EngineCommand { engine_command_id: String, params: Value },
}

/// Resolve how a menu bar, toolbar, context menu, or shortcut command ID should be dispatched in
/// the desktop shell without passing `Value::Null` to schema-validated commands.
///
/// Supports optional `:variant` suffixes for submenu choices such as `ui.theme.set:dark`,
/// `ui.locale.set:de`, `ui.previewFilter.set:deuteranopia`, and `ui.canvas.applyPreset:Slide16x9`.
///
/// # Errors
/// Returns a descriptive error string when `command_id` is empty or lacks a required active target
/// sheet/cell in the current document.
pub fn resolve_desktop_command_route(app: &UiApp, session_file_path: Option<&Path>, command_id: &str) -> Result<DesktopCommandRoute, String> {
    let trimmed = command_id.trim();
    if trimmed.is_empty() {
        return Err(String::from("command_id must not be empty"));
    }
    let (base_cmd, variant) = split_variant_command(trimmed);
    if let Some(btn) = TOOLBAR_BUTTONS.iter().find(|b| b.id == base_cmd) {
        return resolve_desktop_command_route(app, session_file_path, btn.command_id);
    }
    let doc = app.document();

    if let Some(route) = resolve_host_or_ui_route(app, doc, base_cmd, variant)? {
        return Ok(route);
    }
    if let Some(route) = resolve_project_or_sheet_route(doc, session_file_path, base_cmd)? {
        return Ok(route);
    }
    if let Some(route) = resolve_table_route(app, doc, base_cmd)? {
        return Ok(route);
    }
    if let Some(route) = resolve_analysis_export_graph_layout_info_route(doc, base_cmd, variant)? {
        return Ok(route);
    }

    Ok(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({}) })
}

fn split_variant_command(command_id: &str) -> (&str, Option<&str>) {
    let Some((prefix, suffix)) = command_id.split_once(':') else {
        return (command_id, None);
    };
    if matches!(
        prefix,
        "ui.theme.set" | "ui.locale.set" | "ui.previewFilter.set" | "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset"
    ) && !suffix.trim().is_empty()
    {
        (prefix, Some(suffix.trim()))
    } else {
        (command_id, None)
    }
}

fn next_theme_mode_name(current: UiThemeMode) -> &'static str {
    match current {
        UiThemeMode::Light => "dark",
        UiThemeMode::Dark => "high_contrast",
        UiThemeMode::HighContrast => "light",
    }
}

fn next_preview_filter_name(current: CanvasPreviewFilter) -> &'static str {
    match current {
        CanvasPreviewFilter::Normal => "deuteranopia",
        CanvasPreviewFilter::Deuteranopia => "protanopia",
        CanvasPreviewFilter::Protanopia => "tritanopia",
        CanvasPreviewFilter::Tritanopia => "grayscale",
        CanvasPreviewFilter::GrayscalePrint => "normal",
    }
}

fn next_locale_code(current: &str) -> &'static str {
    if LOCALES.is_empty() {
        return "en";
    }
    let idx = LOCALES.iter().position(|&loc| loc.eq_ignore_ascii_case(current)).unwrap_or(0);
    LOCALES.get((idx + 1) % LOCALES.len()).copied().unwrap_or("en")
}

fn resolve_any_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id
        .clone()
        .or_else(|| doc.data_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.result_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.info_sheets.first().map(|s| s.id.clone()))
}

fn resolve_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        return Some(active.clone());
    }
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if let Some(graph) = doc.graph_sheet(active) {
            let src = &graph.spec.primary_source_sheet_id;
            if doc.data_sheet(src).is_some() {
                return Some(src.clone());
            }
        }
        if let Some(res) = doc.result_sheet(active) {
            let src = &res.analysis.source_sheet_id;
            if doc.data_sheet(src).is_some() {
                return Some(src.clone());
            }
        }
    }
    doc.data_sheets.first().map(|s| s.id.clone())
}

fn resolve_xy_or_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && let Some(ds) = doc.data_sheet(active)
        && matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_))
    {
        return Some(ds.id.clone());
    }
    doc.data_sheets
        .iter()
        .find(|ds| matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)))
        .map(|ds| ds.id.clone())
        .or_else(|| resolve_data_sheet_id(doc))
}

fn resolve_mv_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && let Some(ds) = doc.data_sheet(active)
        && matches!(ds.archetype, TableArchetype::MultipleVariables(_))
    {
        return Some(ds.id.clone());
    }
    doc.data_sheets.iter().find(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_))).map(|ds| ds.id.clone())
}

fn resolve_result_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()).cloned().or_else(|| doc.result_sheets.first().map(|r| r.id.clone()))
}

fn resolve_graph_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some()).cloned().or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
}

fn resolve_uninit_or_graph_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.graph_sheet(id).is_some())
        .cloned()
        .or_else(|| doc.graph_sheets.iter().find(|g| !g.initialized).map(|g| g.id.clone()))
        .or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
}

fn resolve_layout_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.layout_sheet(id).is_some()).cloned().or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

fn resolve_renderable_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && matches!(doc.sheet_section(active), Some(SheetSection::Graphs | SheetSection::Layouts))
    {
        return Some(active.clone());
    }
    doc.graph_sheets.first().map(|g| g.id.clone()).or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

fn resolve_info_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if doc.info_sheet(active).is_some() {
            return Some(active.clone());
        }
        if let Some(ds) = doc.data_sheet(active)
            && let Some(linked) = &ds.info_sheet_id
            && doc.info_sheet(linked).is_some()
        {
            return Some(linked.clone());
        }
    }
    doc.info_sheets.first().map(|i| i.id.clone())
}

fn sheet_id_obj_or_empty(sheet_id: Option<SheetId>) -> Value {
    match sheet_id {
        Some(id) => json!({ "sheet_id": id.as_str() }),
        None => json!({}),
    }
}

fn resolve_host_or_ui_route(
    app: &UiApp,
    doc: &ProjectDocument,
    base_cmd: &str,
    variant: Option<&str>,
) -> Result<Option<DesktopCommandRoute>, String> {
    match base_cmd {
        "window.new" | "file.newWindow" => Ok(Some(DesktopCommandRoute::HostAction { action: "window.new" })),
        "window.close" | "file.close" => Ok(Some(DesktopCommandRoute::HostAction { action: "window.close" })),
        "edit.copy" => Ok(Some(DesktopCommandRoute::HostAction { action: "edit.copy" })),
        "edit.paste" => Ok(Some(DesktopCommandRoute::HostAction { action: "edit.paste" })),
        "toolbar.axes" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.graph.axes.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "file.print" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.print.open"),
            params: sheet_id_obj_or_empty(resolve_any_sheet_id(doc)),
        })),
        "ui.theme.set" => {
            let mode = variant.unwrap_or_else(|| next_theme_mode_name(app.theme.mode));
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.theme.set"), params: json!({ "mode": mode }) }))
        }
        "ui.previewFilter.set" => {
            let filter = variant.unwrap_or_else(|| next_preview_filter_name(app.preview_filter));
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.previewFilter.set"), params: json!({ "filter": filter }) }))
        }
        "ui.locale.set" => {
            let locale = variant.unwrap_or_else(|| next_locale_code(&app.locale));
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.locale.set"), params: json!({ "locale": locale }) }))
        }
        "ui.canvas.applyPreset" => {
            let preset = variant.unwrap_or("SingleColumn88mm");
            let mut params = sheet_id_obj_or_empty(resolve_renderable_sheet_id(doc));
            if let Some(obj) = params.as_object_mut() {
                obj.insert(String::from("preset"), Value::String(preset.to_owned()));
                obj.insert(String::from("resize_mode"), Value::String(String::from("KeepTypographyPt")));
            }
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.canvas.applyPreset"), params }))
        }
        "ui.shortcut.set" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: None,
            query: Some(String::from("shortcut")),
        })),
        "ui.brush.point" => {
            let ds_id = resolve_data_sheet_id(doc);
            let params = match (ds_id, app.grid.tool.active, app.brushed_cell.as_ref()) {
                (Some(id), Some(coord), _) => {
                    json!({ "sheet_id": id.as_str(), "row": coord.row, "col": coord.col, "subcol": coord.subcol })
                }
                (_, None, Some(brushed)) => {
                    json!({ "sheet_id": brushed.sheet_id.as_str(), "row": brushed.row, "col": brushed.col, "subcol": brushed.subcol })
                }
                (Some(id), None, None) => json!({ "sheet_id": id.as_str(), "row": 0, "col": 0, "subcol": 0 }),
                (None, _, _) => json!({}),
            };
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.brush.point"), params }))
        }
        "ui.result.applyRemediation" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.result.inspectDiagnostics"),
            params: sheet_id_obj_or_empty(resolve_result_sheet_id(doc)),
        })),
        "ui.table.openFormulaBar" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.table.openFormulaBar"),
            params: sheet_id_obj_or_empty(resolve_mv_sheet_id(doc)),
        })),
        "ui.event.dispatch" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.event.dispatch"),
            params: json!({
                "event": { "PointerMove": { "x": 0.0, "y": 0.0, "modifiers": { "shift": false, "ctrl": false, "alt": false, "meta": false } } },
                "metrics": { "logical_width": 1024.0, "logical_height": 640.0, "scale_factor": 1.0 }
            }),
        })),
        cmd if cmd.starts_with("ui.") || cmd == "app.about" => {
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: cmd.to_owned(), params: json!({}) }))
        }
        _ => Ok(None),
    }
}

fn resolve_project_or_sheet_route(
    doc: &ProjectDocument,
    session_file_path: Option<&Path>,
    base_cmd: &str,
) -> Result<Option<DesktopCommandRoute>, String> {
    match base_cmd {
        "edit.undo"
        | "edit.redo"
        | "file.new"
        | "project.new"
        | "project.inspect"
        | "project.recalculate"
        | "project.listCheckpoints"
        | "layout.create" => Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({}) })),
        "project.open" | "file.open" => {
            Ok(Some(DesktopCommandRoute::UiDialogOpener { kind: DesktopDialogOpener::WelcomeOpenProject, target_sheet_id: None, query: None }))
        }
        "project.instantiateTemplate" => {
            Ok(Some(DesktopCommandRoute::UiDialogOpener { kind: DesktopDialogOpener::WelcomeFromTemplate, target_sheet_id: None, query: None }))
        }
        "project.save" | "file.save" | "project.saveAs" | "file.save_as" => {
            let params = match session_file_path {
                Some(path) => json!({ "path": path.to_string_lossy(), "unpacked": false }),
                None => json!({ "return_bytes": true }),
            };
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: String::from("project.save"), params }))
        }
        "project.saveTemplate" => Ok(Some(DesktopCommandRoute::EngineCommand {
            engine_command_id: String::from("project.saveTemplate"),
            params: json!({ "return_bytes": true }),
        })),
        "project.createCheckpoint" => Ok(Some(DesktopCommandRoute::EngineCommand {
            engine_command_id: String::from("project.createCheckpoint"),
            params: json!({ "name": format!("Checkpoint {}", doc.checkpoints.len() + 1) }),
        })),
        "project.restoreCheckpoint" | "project.deleteCheckpoint" | "sheet.rename" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: resolve_any_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: Some(base_cmd.to_owned()),
        })),
        "sheet.createData" => {
            Ok(Some(DesktopCommandRoute::UiDialogOpener { kind: DesktopDialogOpener::WelcomeEmptyTable, target_sheet_id: None, query: None }))
        }
        "sheet.clonePipeline" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.clonePipeline.open"),
            params: sheet_id_obj_or_empty(resolve_any_sheet_id(doc)),
        })),
        "sheet.extractResultTab" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.analyze.open"),
            params: sheet_id_obj_or_empty(resolve_result_sheet_id(doc).or_else(|| resolve_data_sheet_id(doc))),
        })),
        "sheet.reorder" => {
            let params = build_sheet_reorder_params(doc)?;
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: String::from("sheet.reorder"), params }))
        }
        "sheet.delete" | "sheet.duplicate" | "sheet.duplicateFamily" | "sheet.freeze" | "sheet.unfreeze" | "sheet.select" => {
            let id = resolve_any_sheet_id(doc).ok_or_else(|| format!("no active sheet available for `{base_cmd}`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({ "sheet_id": id.as_str() }) }))
        }
        _ => Ok(None),
    }
}

fn build_sheet_reorder_params(doc: &ProjectDocument) -> Result<Value, String> {
    let sheet_id = resolve_any_sheet_id(doc).ok_or_else(|| String::from("no active sheet available for `sheet.reorder`"))?;
    let (current_idx, len) = match doc.sheet_section(&sheet_id) {
        Some(SheetSection::Data) => (doc.data_sheets.iter().position(|s| s.id == sheet_id).unwrap_or(0), doc.data_sheets.len()),
        Some(SheetSection::Info) => (doc.info_sheets.iter().position(|s| s.id == sheet_id).unwrap_or(0), doc.info_sheets.len()),
        Some(SheetSection::Analyses) => (doc.result_sheets.iter().position(|s| s.id == sheet_id).unwrap_or(0), doc.result_sheets.len()),
        Some(SheetSection::Graphs) => (doc.graph_sheets.iter().position(|s| s.id == sheet_id).unwrap_or(0), doc.graph_sheets.len()),
        Some(SheetSection::Layouts) => (doc.layout_sheets.iter().position(|s| s.id == sheet_id).unwrap_or(0), doc.layout_sheets.len()),
        None => (0, 1),
    };
    let next_idx = if len > 1 { (current_idx + 1) % len } else { current_idx };
    Ok(json!({ "sheet_id": sheet_id.as_str(), "index": next_idx }))
}

fn resolve_table_route(app: &UiApp, doc: &ProjectDocument, base_cmd: &str) -> Result<Option<DesktopCommandRoute>, String> {
    match base_cmd {
        "table.exclusionAuditReport" | "table.simulateXy" => {
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({}) }))
        }
        "table.setCell" | "table.setCells" | "table.setXCells" | "table.clearCells" | "table.excludeCells" => {
            let params = build_table_context_params(app, doc, base_cmd)?;
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params }))
        }
        "table.excludeCellsWithReason" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.grid.excludeWithReason"),
            params: sheet_id_obj_or_empty(resolve_data_sheet_id(doc)),
        })),
        "table.randomizeGroups" => {
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.table.randomizeGroups"), params: json!({}) }))
        }
        "table.randomSampleRows" => {
            let params = match resolve_data_sheet_id(doc) {
                Some(id) => json!({ "source_sheet_id": id.as_str() }),
                None => json!({}),
            };
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.table.randomSampleRows"), params }))
        }
        "table.simulateDistributions" => {
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.table.simulateDistributions"), params: json!({}) }))
        }
        "table.convertArchetype" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.table.convertArchetype.open"),
            params: sheet_id_obj_or_empty(resolve_data_sheet_id(doc)),
        })),
        "table.importPlateMap" => {
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.import.reshape.open"), params: json!({ "mode": "Plate96" }) }))
        }
        "table.importTidyLong" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.import.reshape.open"),
            params: json!({ "mode": "TidyLongPivot" }),
        })),
        "table.setCalculatedVariable" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.table.openFormulaBar"),
            params: sheet_id_obj_or_empty(resolve_mv_sheet_id(doc).or_else(|| resolve_data_sheet_id(doc))),
        })),
        "table.setVariableRole" | "table.recodeLevels" | "table.setCategoricalOrder" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::VariablesDialog,
            target_sheet_id: resolve_mv_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.paste" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ImportWizardPaste,
            target_sheet_id: resolve_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.pasteLink" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ImportWizardPasteLink,
            target_sheet_id: resolve_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.importCsv" | "table.importPrism" | "table.importPzfx" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ImportWizardExternalFile,
            target_sheet_id: resolve_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.setRowTitles" | "table.setGroupTitles" | "table.setTitles" | "table.setSubcolumnTitles" => {
            Ok(Some(DesktopCommandRoute::UiDialogOpener {
                kind: DesktopDialogOpener::CommandPalette,
                target_sheet_id: resolve_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
                query: Some(base_cmd.to_owned()),
            }))
        }
        "table.exportCsv" => {
            let target = doc
                .active_sheet_id
                .as_ref()
                .filter(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some())
                .cloned()
                .or_else(|| resolve_data_sheet_id(doc))
                .or_else(|| resolve_result_sheet_id(doc))
                .ok_or_else(|| String::from("no data or result sheet available for `table.exportCsv`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("table.exportCsv"),
                params: json!({ "sheet_id": target.as_str() }),
            }))
        }
        _ => Ok(None),
    }
}

fn cell_value_to_json(val: &CellValue) -> Value {
    match val {
        CellValue::Empty => Value::Null,
        CellValue::Number(n) => json!(n),
        CellValue::Text(s) => json!(s),
    }
}

fn first_editable_col_idx(ds: &lumencraft_doc::DataSheet) -> usize {
    if let TableArchetype::MultipleVariables(mv) = &ds.archetype
        && let Some(idx) = mv.variables.iter().position(|v| !matches!(v.source, VariableSource::Calculated { .. }))
    {
        return idx;
    }
    0
}

fn build_table_context_params(app: &UiApp, doc: &ProjectDocument, cmd: &str) -> Result<Value, String> {
    if cmd == "table.setXCells" {
        let id = resolve_xy_or_data_sheet_id(doc).ok_or_else(|| format!("no data sheet available for `{cmd}`"))?;
        let ds = doc.data_sheet(&id).ok_or_else(|| format!("data sheet `{id}` not found"))?;
        let row_limit = ds.archetype.row_count().max(1);
        let (raw_row, subcol) = app.grid.active_x_cell.unwrap_or((0, 0));
        let row = raw_row.min(row_limit - 1);
        let existing = ds.x_or_time_cell(row, subcol).map_or(Value::Null, |c| cell_value_to_json(&c.value));
        return Ok(json!({ "sheet_id": id.as_str(), "row": row, "subcol": subcol, "value": existing }));
    }

    let id = resolve_data_sheet_id(doc).ok_or_else(|| format!("no data sheet available for `{cmd}`"))?;
    let ds = doc.data_sheet(&id).ok_or_else(|| format!("data sheet `{id}` not found"))?;
    let total_rows = ds.archetype.row_count();
    let total_cols = ds.archetype.column_count();
    if total_rows == 0 || total_cols == 0 {
        return Err(format!("data sheet `{id}` has no data cells for `{cmd}`"));
    }

    if cmd == "table.excludeCells"
        && let Some((x_row, x_subcol)) = app.grid.active_x_cell
        && matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_))
    {
        let row = x_row.min(total_rows - 1);
        let cur_excl = ds.x_or_time_cell(row, x_subcol).is_some_and(|c| c.excluded);
        return Ok(json!({ "sheet_id": id.as_str(), "target": "x", "row": row, "subcol": x_subcol, "excluded": !cur_excl }));
    }

    let bounds = app.grid.tool.selection_bounds();
    let fallback_col = first_editable_col_idx(ds).min(total_cols - 1);
    let active = app.grid.tool.active.unwrap_or_default();
    let row = active.row.min(total_rows - 1);
    let col = if active.col < total_cols { active.col } else { fallback_col };
    let subcol = active.subcol;

    match cmd {
        "table.setCell" => {
            let existing = ds.cell(CellCoord::new(row, col, subcol)).map_or(Value::Null, |c| cell_value_to_json(&c.value));
            Ok(json!({ "sheet_id": id.as_str(), "row": row, "col": col, "subcol": subcol, "value": existing }))
        }
        "table.setCells" => {
            let existing = ds.cell(CellCoord::new(row, col, subcol)).map_or(Value::Null, |c| cell_value_to_json(&c.value));
            Ok(json!({ "sheet_id": id.as_str(), "edits": [{ "row": row, "col": col, "subcol": subcol, "value": existing }] }))
        }
        "table.clearCells" => {
            if let Some((a, b)) = bounds {
                let start_row = a.row.min(total_rows - 1);
                let end_row = b.row.min(total_rows - 1);
                let start_col = a.col.min(total_cols - 1);
                let end_col = b.col.min(total_cols - 1);
                Ok(json!({
                    "sheet_id": id.as_str(),
                    "range": {
                        "start": { "row": start_row, "col": start_col, "subcol": a.subcol },
                        "end": { "row": end_row, "col": end_col, "subcol": b.subcol }
                    }
                }))
            } else {
                Ok(json!({ "sheet_id": id.as_str(), "row": row, "col": col, "subcol": subcol }))
            }
        }
        "table.excludeCells" => {
            let probe = bounds.map_or_else(
                || CellCoord::new(row, col, subcol),
                |(a, _)| CellCoord::new(a.row.min(total_rows - 1), a.col.min(total_cols - 1), a.subcol),
            );
            let next_excl = !ds.cell(probe).is_some_and(|c| c.excluded);
            if let Some((a, b)) = bounds {
                let start_row = a.row.min(total_rows - 1);
                let end_row = b.row.min(total_rows - 1);
                let start_col = a.col.min(total_cols - 1);
                let end_col = b.col.min(total_cols - 1);
                Ok(json!({
                    "sheet_id": id.as_str(),
                    "excluded": next_excl,
                    "range": {
                        "start": { "row": start_row, "col": start_col, "subcol": a.subcol },
                        "end": { "row": end_row, "col": end_col, "subcol": b.subcol }
                    }
                }))
            } else {
                Ok(json!({ "sheet_id": id.as_str(), "row": row, "col": col, "subcol": subcol, "excluded": next_excl }))
            }
        }
        _ => Ok(json!({ "sheet_id": id.as_str() })),
    }
}

fn resolve_analysis_export_graph_layout_info_route(
    doc: &ProjectDocument,
    base_cmd: &str,
    variant: Option<&str>,
) -> Result<Option<DesktopCommandRoute>, String> {
    match base_cmd {
        "analyze.run" => Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: String::from("analyze.run"), params: json!({}) })),
        "analysis.add" | "analysis.run" | "stats.run" | "transform.run" | "fit.run" | "stats.pvalueStack" => {
            Ok(Some(DesktopCommandRoute::UiCommand {
                ui_command_id: String::from("ui.analyze.open"),
                params: sheet_id_obj_or_empty(resolve_data_sheet_id(doc)),
            }))
        }
        "analysis.updateParams" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::EditAnalysisParameters,
            target_sheet_id: resolve_result_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "analysis.retargetData" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ChangeDataAnalyzed,
            target_sheet_id: resolve_result_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "analysis.exportJson" | "analysis.exportCsv" => {
            let id = resolve_result_sheet_id(doc).ok_or_else(|| format!("no result sheet available for `{base_cmd}`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({ "sheet_id": id.as_str() }) }))
        }
        "result.generateReport" => {
            let id = resolve_result_sheet_id(doc).ok_or_else(|| String::from("no result sheet available for `result.generateReport`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("result.generateReport"),
                params: json!({ "result_sheet_id": id.as_str(), "format": "markdown" }),
            }))
        }
        "export.svg" | "file.export_svg" | "export.pdf" | "file.export_pdf" | "export.png" | "file.export_png" | "export.tiff" => {
            let engine_id = match base_cmd {
                "file.export_svg" => "export.svg",
                "file.export_pdf" => "export.pdf",
                "file.export_png" => "export.png",
                other => other,
            };
            let id = resolve_renderable_sheet_id(doc).ok_or_else(|| format!("no graph or layout sheet available for `{engine_id}`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: engine_id.to_owned(), params: json!({ "sheet_id": id.as_str() }) }))
        }
        "graph.exportSvg" | "graph.exportPdf" | "graph.exportPng" | "graph.exportEmf" => {
            let id = resolve_graph_sheet_id(doc).ok_or_else(|| format!("no graph sheet available for `{base_cmd}`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({ "sheet_id": id.as_str() }) }))
        }
        "layout.exportEmf" => {
            let id = resolve_layout_sheet_id(doc).ok_or_else(|| String::from("no layout sheet available for `layout.exportEmf`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("layout.exportEmf"),
                params: json!({ "sheet_id": id.as_str() }),
            }))
        }
        _ => resolve_graph_layout_info_only_route(doc, base_cmd, variant),
    }
}

fn resolve_graph_layout_info_only_route(doc: &ProjectDocument, base_cmd: &str, variant: Option<&str>) -> Result<Option<DesktopCommandRoute>, String> {
    match base_cmd {
        "graph.create" => {
            let ds_id = resolve_data_sheet_id(doc).ok_or_else(|| String::from("no data sheet available for `graph.create`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("graph.create"),
                params: json!({ "data_sheet_id": ds_id.as_str() }),
            }))
        }
        "graph.initialize" => {
            let gid = resolve_uninit_or_graph_sheet_id(doc).ok_or_else(|| String::from("no graph sheet available for `graph.initialize`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("graph.initialize"),
                params: json!({ "sheet_id": gid.as_str() }),
            }))
        }
        "graph.setKind" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.changeGraphType.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "graph.setSpec" | "graph.setSeries" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.graph.format.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "graph.setAxes" | "graph.setAdditionalTicks" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.graph.axes.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "graph.addComparisonBrackets" | "graph.setCompactLetterDisplay" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.graph.pairwise.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "graph.setRegressionOverlay" | "graph.addAnnotation" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.graph.insertModelEquation"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "graph.formatSelectedPoints" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.formatPoints.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "graph.applyStyleFrom" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ApplyStyleFrom,
            target_sheet_id: resolve_graph_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "graph.applyCanvasPreset" => {
            let preset = variant.unwrap_or("SingleColumn88mm");
            let mut params = sheet_id_obj_or_empty(resolve_graph_sheet_id(doc));
            if let Some(obj) = params.as_object_mut() {
                obj.insert(String::from("preset"), Value::String(preset.to_owned()));
                obj.insert(String::from("resize_mode"), Value::String(String::from("KeepTypographyPt")));
            }
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.canvas.applyPreset"), params }))
        }
        "graph.preflightCheck" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.canvas.preflight"),
            params: sheet_id_obj_or_empty(resolve_renderable_sheet_id(doc)),
        })),
        "layout.setSpec" | "layout.addPanel" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.layout.composer.open"),
            params: sheet_id_obj_or_empty(resolve_layout_sheet_id(doc)),
        })),
        "layout.alignOrigins" => {
            let id = resolve_layout_sheet_id(doc).ok_or_else(|| String::from("no layout sheet available for `layout.alignOrigins`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("layout.alignOrigins"),
                params: json!({ "sheet_id": id.as_str(), "align": true, "normalize_bounds": true }),
            }))
        }
        "layout.applyCanvasPreset" => {
            let preset = variant.unwrap_or("SingleColumn88mm");
            let mut params = sheet_id_obj_or_empty(resolve_layout_sheet_id(doc));
            if let Some(obj) = params.as_object_mut() {
                obj.insert(String::from("preset"), Value::String(preset.to_owned()));
                obj.insert(String::from("resize_mode"), Value::String(String::from("KeepTypographyPt")));
            }
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.canvas.applyPreset"), params }))
        }
        "info.setNote" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::InfoNotesEditor,
            target_sheet_id: resolve_info_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "info.setConstant" | "info.removeConstant" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::InfoConstantHookPicker,
            target_sheet_id: resolve_info_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        _ => Ok(None),
    }
}

fn open_desktop_dialog(app: &mut UiApp, opener: DesktopDialogOpener, target_sheet_id: Option<&str>, query: Option<&str>) -> Result<Value, String> {
    let doc = app.document().clone();
    let parsed_target = target_sheet_id.and_then(|raw| SheetId::try_new(raw).ok());
    match opener {
        DesktopDialogOpener::WelcomeEmptyTable => {
            app.nav.welcome.open = true;
            app.nav.welcome.mode = WelcomeMode::EmptyTypedTable;
            Ok(json!({ "dialog": "WelcomeWizard", "mode": "EmptyTypedTable" }))
        }
        DesktopDialogOpener::WelcomeOpenProject => {
            app.nav.welcome.open = true;
            app.nav.welcome.mode = WelcomeMode::TutorialSample;
            Ok(json!({ "dialog": "WelcomeWizard", "mode": "TutorialSample" }))
        }
        DesktopDialogOpener::WelcomeFromTemplate => {
            app.nav.welcome.open = true;
            app.nav.welcome.mode = WelcomeMode::FromTemplate;
            Ok(json!({ "dialog": "WelcomeWizard", "mode": "FromTemplate" }))
        }
        DesktopDialogOpener::ImportWizardPaste => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.is_live_link = false;
            app.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
            app.grid.import_wizard.source_sheet_id = parsed_target.or_else(|| resolve_data_sheet_id(&doc));
            Ok(json!({ "dialog": "ImportPasteWizard", "is_live_link": false }))
        }
        DesktopDialogOpener::ImportWizardPasteLink => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.is_live_link = true;
            app.grid.import_wizard.source_sheet_id = parsed_target.or_else(|| resolve_data_sheet_id(&doc));
            Ok(json!({ "dialog": "ImportPasteWizard", "is_live_link": true }))
        }
        DesktopDialogOpener::ImportWizardExternalFile => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.is_live_link = false;
            app.grid.import_wizard.origin_kind = ImportOriginKind::ExternalDelimitedFile;
            Ok(json!({ "dialog": "ImportPasteWizard", "origin_kind": "ExternalDelimitedFile" }))
        }
        DesktopDialogOpener::VariablesDialog => {
            let sid = parsed_target
                .or_else(|| resolve_mv_sheet_id(&doc))
                .ok_or_else(|| String::from("Variables dialog requires a MultipleVariables table"))?;
            let ds = doc.data_sheet(&sid).ok_or_else(|| format!("data sheet `{sid}` not found"))?;
            let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
                return Err(format!("sheet `{sid}` is not a MultipleVariables table"));
            };
            let col = app.grid.tool.active.map_or(0, |c| c.col).min(mv.variables.len().saturating_sub(1));
            app.grid.variables_dialog.open_for_column(ds, col);
            Ok(json!({ "dialog": "VariablesDialog", "sheet_id": sid.as_str(), "column": col }))
        }
        DesktopDialogOpener::EditAnalysisParameters => {
            let sid = parsed_target
                .or_else(|| resolve_result_sheet_id(&doc))
                .ok_or_else(|| String::from("no result sheet available to edit analysis parameters"))?;
            if !app.canvas_dialogs.open_edit_parameters(&doc, &sid) {
                return Err(format!("result sheet `{sid}` not found"));
            }
            Ok(json!({ "dialog": "EditAnalysisParameters", "result_sheet_id": sid.as_str() }))
        }
        DesktopDialogOpener::ChangeDataAnalyzed => {
            let sid = parsed_target
                .or_else(|| resolve_result_sheet_id(&doc))
                .ok_or_else(|| String::from("no result sheet available to retarget analyzed data"))?;
            if !app.canvas_dialogs.open_change_data_analyzed(&doc, &sid) {
                return Err(format!("result sheet `{sid}` not found"));
            }
            Ok(json!({ "dialog": "ChangeDataAnalyzed", "result_sheet_id": sid.as_str() }))
        }
        DesktopDialogOpener::ApplyStyleFrom => {
            let gid =
                parsed_target.or_else(|| resolve_graph_sheet_id(&doc)).ok_or_else(|| String::from("no graph sheet available to copy style onto"))?;
            if !app.canvas_dialogs.open_apply_style_from(&doc, &gid) {
                return Err(format!("graph sheet `{gid}` not found"));
            }
            Ok(json!({ "dialog": "ApplyStyleFrom", "target_graph_id": gid.as_str() }))
        }
        DesktopDialogOpener::InfoNotesEditor => {
            let iid = parsed_target.or_else(|| resolve_info_sheet_id(&doc)).ok_or_else(|| String::from("no info sheet available to edit notes"))?;
            let notes = doc.info_sheet(&iid).map_or_else(String::new, |i| i.notes_markdown.clone());
            app.inline_editor.begin(InlineEditorTarget::InfoNotes { sheet_id: iid.clone() }, notes);
            Ok(json!({ "editor": "InfoNotes", "sheet_id": iid.as_str() }))
        }
        DesktopDialogOpener::InfoConstantHookPicker => {
            let iid = parsed_target
                .or_else(|| resolve_info_sheet_id(&doc))
                .or_else(|| resolve_any_sheet_id(&doc))
                .ok_or_else(|| String::from("no info or target sheet available for constant picker"))?;
            app.canvas_dialogs.open_hook_picker(&doc, &iid, None);
            Ok(json!({ "dialog": "ConstantHookPicker", "target_sheet_id": iid.as_str() }))
        }
        DesktopDialogOpener::CommandPalette => {
            app.canvas_dialogs.open_command_palette();
            if let Some(q) = query {
                app.canvas_dialogs.command_palette.query = q.to_owned();
            }
            Ok(json!({ "dialog": "CommandPalette", "query": app.canvas_dialogs.command_palette.query }))
        }
    }
}

fn build_shortcut_candidates_for(key: &str, modifiers: KeyModifiers) -> Vec<String> {
    if key.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(2);
    let build = |use_cmd_for_ctrl: bool| -> String {
        let mut parts = Vec::with_capacity(5);
        if modifiers.meta || (use_cmd_for_ctrl && modifiers.ctrl) {
            parts.push("Cmd");
        } else if modifiers.ctrl {
            parts.push("Ctrl");
        }
        if modifiers.alt {
            parts.push("Alt");
        }
        if modifiers.shift {
            parts.push("Shift");
        }
        parts.push(key);
        parts.join("+")
    };
    out.push(build(false));
    if modifiers.ctrl && !modifiers.meta {
        out.push(build(true));
    }
    out
}

fn shortcut_uses_desktop_routing(command_id: &str) -> bool {
    if matches!(
        command_id,
        "window.new"
            | "file.newWindow"
            | "window.close"
            | "file.close"
            | "edit.copy"
            | "edit.paste"
            | "project.open"
            | "file.open"
            | "project.save"
            | "file.save"
            | "project.saveAs"
            | "file.save_as"
            | "sheet.createData"
            | "ui.theme.set"
            | "ui.locale.set"
            | "ui.previewFilter.set"
            | "ui.canvas.applyPreset"
            | "ui.shortcut.set"
            | "ui.result.applyRemediation"
            | "ui.event.dispatch"
    ) {
        return true;
    }
    engine_cmd::registry()
        .get(command_id)
        .is_some_and(|spec| (spec.input_schema)().get("required").and_then(Value::as_array).is_some_and(|arr| !arr.is_empty()))
}

fn match_routed_desktop_shortcut(app: &UiApp, key: &str, modifiers: KeyModifiers) -> Option<String> {
    if (modifiers.meta || modifiers.ctrl) && !modifiers.alt && !modifiers.shift && key.eq_ignore_ascii_case("v") {
        // Let clipboard paste shortcut use host pasteboard instead of opening Paste Special wizard.
        return None;
    }
    for candidate in build_shortcut_candidates_for(key, modifiers) {
        if let Some(matched) = app.shortcuts.handle_key_combo(&candidate)
            && shortcut_uses_desktop_routing(&matched.command_id)
        {
            return Some(matched.command_id);
        }
    }
    None
}
