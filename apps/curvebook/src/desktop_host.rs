#![forbid(unsafe_code)]

//! Desktop host event loop, multi-window runtime, HiDPI Retina backing store coordination,
//! OS chrome/menu dispatch, IME text composition, trackpad gesture inertia, file drop routing,
//! clipboard pasteboard integration, isolated crash-recovery journals, and JSONL playback.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use curvebook_doc::{CellCoord, CellValue, ProjectDocument, SheetId, SheetSection, SubcolumnMode, TableArchetype, TableKind, VariableSource};
use curvebook_engine::{CommandOutcome, Session, cmd as engine_cmd};
use curvebook_format::recovery::is_lease_reclaimable;
use curvebook_format::{
    CrashRecoveryJournal, DirectoryJournalBackend, FormatKind, FormatRegistry, ImportOptions, JournalBackend, JournalRecoveryWarning,
    MAX_ARCHIVE_FILE_BYTES, RecoveryReport, instantiate_template_bytes, is_unpacked_project_dir,
};
use curvebook_ui_egui::control_io::host_routing::{
    active_data_sheet_id, capture_pref_fingerprint, is_read_only_control_method, next_locale_code, next_preview_filter_name, next_theme_mode_name,
    resolve_any_sheet_id, resolve_data_sheet_id, resolve_graph_sheet_id, resolve_info_sheet_id, resolve_layout_sheet_id, resolve_mv_sheet_id,
    resolve_renderable_sheet_id, resolve_result_sheet_id, resolve_uninit_or_graph_sheet_id, resolve_xy_or_data_sheet_id, sheet_id_obj_or_empty,
    split_variant_command,
};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::{InlineEditorTarget, KeyModifiers, ViewportMetrics, build_shortcut_candidates};
use curvebook_ui_egui::menus::{CuratedMenuNode, TOOLBAR_BUTTONS, curated_menu_bar, is_desktop_host_routed_command};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::WelcomeMode;
use curvebook_ui_egui::{
    ClipboardBundle, ControlAuth, ControlPostRequestContext, OsAdapterStatus, UiApp, dispatch_ui_command, is_http_request_line,
    normalize_beta_gui_locale, read_os_pasteboard, serve_control_stream_with_quit_and_hook, tr, write_os_clipboard_bundle,
    write_os_clipboard_bundle_for_slides, write_os_pasteboard,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_RECOVERY_LOCK_TTL_MS: u64 = 5 * 60 * 1000;
const BASE_LOGICAL_DPI: u16 = 72;
const IN_WINDOW_MENU_BAR_HEIGHT_PT: f64 = 24.0;

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

/// Hierarchical menu node in a top-level [`DesktopMenuSection`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopMenuNode {
    Action(DesktopMenuItem),
    Separator,
    Submenu { id: String, title: String, children: Vec<Self> },
}

/// One localized top-level menu bar section (`File`, `Edit`, `View`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopMenuSection {
    pub menu_key: String,
    pub title: String,
    #[serde(default)]
    pub nodes: Vec<DesktopMenuNode>,
    pub items: Vec<DesktopMenuItem>,
}

fn default_chrome_locale() -> String {
    String::from("en")
}

/// Observed menu bar chrome state for a [`DesktopWindowSession`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopMenuChromeState {
    pub mode: MenuChromeMode,
    pub in_window_menus: bool,
    pub menu_bar_visible: bool,
    pub menu_bar_height_pt: f64,
    pub active_menu_key: Option<String>,
    #[serde(default = "default_chrome_locale")]
    pub locale: String,
    pub sections: Vec<DesktopMenuSection>,
    pub dispatched_actions: Vec<String>,
}

/// Host runtime configuration constructed from CLI flags (`--in-window-menus`, `--new-instance`, ...)
/// or embedded test harnesses.
#[derive(Clone, Serialize, Deserialize)]
pub struct DesktopHostConfig {
    pub in_window_menus: bool,
    pub new_instance: bool,
    #[serde(default)]
    pub gui_only: bool,
    pub locale_override: Option<String>,
    pub theme_override: Option<String>,
    pub recovery_root: Option<PathBuf>,
    pub initial_viewport: ViewportMetrics,
    #[serde(skip)]
    pub path_policy: curvebook_engine::ExternalPathPolicy,
    #[serde(default)]
    pub human_path_only: bool,
    #[serde(skip)]
    pub clipboard_adapter: Option<Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>>,
}

impl fmt::Debug for DesktopHostConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DesktopHostConfig")
            .field("in_window_menus", &self.in_window_menus)
            .field("new_instance", &self.new_instance)
            .field("gui_only", &self.gui_only)
            .field("locale_override", &self.locale_override)
            .field("theme_override", &self.theme_override)
            .field("recovery_root", &self.recovery_root)
            .field("initial_viewport", &self.initial_viewport)
            .field("path_policy", &self.path_policy)
            .field("human_path_only", &self.human_path_only)
            .field("clipboard_adapter", &self.clipboard_adapter.is_some())
            .finish()
    }
}

impl PartialEq for DesktopHostConfig {
    fn eq(&self, other: &Self) -> bool {
        let clip_eq = match (&self.clipboard_adapter, &other.clipboard_adapter) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.in_window_menus == other.in_window_menus
            && self.new_instance == other.new_instance
            && self.gui_only == other.gui_only
            && self.locale_override == other.locale_override
            && self.theme_override == other.theme_override
            && self.recovery_root == other.recovery_root
            && self.initial_viewport == other.initial_viewport
            && self.path_policy == other.path_policy
            && self.human_path_only == other.human_path_only
            && clip_eq
    }
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
            path_policy: curvebook_engine::ExternalPathPolicy::default(),
            human_path_only: false,
            clipboard_adapter: None,
        }
    }
}

/// Result returned when [`DesktopHostEvent::WindowCloseRequested`] is processed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseDisposition {
    Closed,
    PromptUnsaved { window_id: DesktopWindowId, dirty_commands: Vec<String> },
}

mod arc_bytes_serde {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Arc<[u8]>, serializer: S) -> Result<S::Ok, S::Error> {
        bytes.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<[u8]>, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(Arc::from)
    }
}

/// Rendered software `egui` framebuffer output returned by [`DesktopHostEvent::RedrawRequested`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopFrameOutput {
    pub window_id: DesktopWindowId,
    pub width_px: u32,
    pub height_px: u32,
    pub scale_factor: f32,
    #[serde(with = "arc_bytes_serde")]
    pub rgba_bytes: Arc<[u8]>,
    pub frame_ms: f64,
    pub window_title: String,
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
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.to_ascii_lowercase().ends_with(".curvebook.d"))
}

fn is_unpacked_project_target(path: &Path) -> bool {
    (has_unpacked_dir_suffix(path) && !path.is_file()) || is_unpacked_project_dir(path)
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

enum LoadedPathPayload {
    Project {
        doc: Box<ProjectDocument>,
        archive_path: Option<PathBuf>,
        clean_archive: bool,
        import_warnings: Vec<curvebook_format::ImportWarning>,
        import_command_id: Option<&'static str>,
    },
    DelimitedTable {
        importer_id: &'static str,
        title: String,
        is_tsv: bool,
    },
}

fn classify_and_load_path(path: &Path) -> Result<LoadedPathPayload, String> {
    let registry = FormatRegistry::default_registry();
    let importer = if is_unpacked_project_target(path) { registry.importer_by_format("curvebook.d") } else { registry.importer_for_path(path) };
    let Some(importer) = importer else {
        if path.is_dir() {
            return Err(format!(
                "unsupported directory `{}`: expected a `.curvebook.d` unpacked project bundle containing `manifest.json`",
                path.display()
            ));
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        return Err(format!(
            "unsupported file extension `.{ext}` for `{}`; expected .curvebook, .curvebook.d, .cbtemplate, .prism, .pzfx, .xlsx, .csv, or .tsv",
            path.display()
        ));
    };

    let desc = importer.descriptor();
    match desc.kind {
        FormatKind::ProjectDir | FormatKind::Archive => {
            let payload =
                importer.import_path(path, &ImportOptions::default()).map_err(|err| format!("failed to open `{}`: {err}", path.display()))?;
            let doc = payload.into_document().ok_or_else(|| format!("archive `{}` did not decode a project document", path.display()))?;
            Ok(LoadedPathPayload::Project {
                doc: Box::new(doc),
                archive_path: Some(path.to_path_buf()),
                clean_archive: true,
                import_warnings: Vec::new(),
                import_command_id: None,
            })
        }
        FormatKind::Template => {
            let bytes = curvebook_platform::fs::read_regular_file_bytes(path, MAX_ARCHIVE_FILE_BYTES)
                .map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
            let doc =
                instantiate_template_bytes(&bytes, None).map_err(|err| format!("failed to instantiate template `{}`: {err}", path.display()))?;
            Ok(LoadedPathPayload::Project {
                doc: Box::new(doc),
                archive_path: None,
                clean_archive: false,
                import_warnings: Vec::new(),
                import_command_id: None,
            })
        }
        FormatKind::ForeignProject => {
            let payload =
                importer.import_path(path, &ImportOptions::default()).map_err(|err| format!("failed to import `{}`: {err}", path.display()))?;
            let (doc, import_warnings) = match payload {
                curvebook_format::ImportPayload::Report(report) => (report.document, report.warnings),
                curvebook_format::ImportPayload::Document(doc) => (doc, Vec::new()),
                curvebook_format::ImportPayload::DataSheet(_) => {
                    return Err(format!("file `{}` did not yield a project document", path.display()));
                }
            };
            let import_command_id = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                Some("prism") => Some("table.importPrism"),
                Some("pzfx") => Some("table.importPzfx"),
                Some("xlsx") => Some("table.importXlsx"),
                _ => None,
            };
            Ok(LoadedPathPayload::Project { doc: Box::new(doc), archive_path: None, clean_archive: false, import_warnings, import_command_id })
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

/// Load one or more startup files (`.curvebook`, `.curvebook.d`, `.cbtemplate`, `.prism`,
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
            LoadedPathPayload::Project { doc, archive_path, clean_archive, .. } => {
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
    let (mut doc, pre_merge_doc, primary_archive_path, clean_archive) = match primary_project {
        Some((_, loaded_doc, archive_path, is_clean)) => {
            let baseline = loaded_doc.clone();
            (loaded_doc, Some(baseline), archive_path, is_clean)
        }
        None => (ProjectDocument::default(), None, None, false),
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
            xlsx: None,
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
    let should_mark_dirty = primary_archive_path.is_none() && (merged_delimited || primary_had_project) || !clean_archive || merged_delimited;
    if let Some(path) = primary_archive_path.clone() {
        session.set_file_path(Some(path), clean_archive && !merged_delimited);
    }
    if should_mark_dirty && !session.is_dirty() {
        let mut baseline = match (primary_archive_path.as_ref(), pre_merge_doc) {
            (Some(path), Some(doc_before)) => {
                let mut b = Session::from_document(doc_before);
                b.set_file_path(Some(path.clone()), true);
                b
            }
            _ => Session::new(),
        };
        if let Some(path) = primary_archive_path {
            baseline.set_file_path(Some(path), true);
        }
        let _ = session.collapse_atomic_group(baseline);
    }
    Ok(session)
}

fn build_desktop_menu_nodes(nodes: Vec<CuratedMenuNode>, flat_items: &mut Vec<DesktopMenuItem>) -> Vec<DesktopMenuNode> {
    nodes
        .into_iter()
        .map(|node| match node {
            CuratedMenuNode::Action(item) => {
                let desktop_item = DesktopMenuItem {
                    command_id: item.command_id.to_owned(),
                    localized_label: item.localized_label.to_owned(),
                    shortcut: item.shortcut,
                    menu_path: item.menu_path.to_owned(),
                    enabled: item.enabled,
                    summary: item.summary.to_owned(),
                    beginner: item.beginner.to_owned(),
                };
                flat_items.push(desktop_item.clone());
                DesktopMenuNode::Action(desktop_item)
            }
            CuratedMenuNode::Separator => DesktopMenuNode::Separator,
            CuratedMenuNode::Submenu { id, title, children, .. } => {
                let child_nodes = build_desktop_menu_nodes(children, flat_items);
                DesktopMenuNode::Submenu { id: id.to_owned(), title, children: child_nodes }
            }
        })
        .collect()
}

fn build_menu_sections(app: &UiApp) -> Vec<DesktopMenuSection> {
    curated_menu_bar(app)
        .into_iter()
        .map(|sec| {
            let mut items = Vec::new();
            let nodes = build_desktop_menu_nodes(sec.nodes, &mut items);
            DesktopMenuSection { menu_key: sec.menu_key.to_owned(), title: sec.title.to_owned(), nodes, items }
        })
        .collect()
}

fn recovery_owner_for_window(window_id: DesktopWindowId) -> String {
    format!("curvebook-pid-{}-win-{}", std::process::id(), window_id.0)
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
        eprintln!("curvebook recovery lease renewal warning: {err}");
    }
}

fn abort_journal_lease_best_effort(journal: CrashRecoveryJournal<DirectoryJournalBackend>) {
    let owner_id = journal.lease().owner_id.clone();
    let mut backend = journal.abandon();
    let _ = backend.release_lease(&owner_id);
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

fn paths_contain_equivalent(paths: &[PathBuf], target: &Path) -> bool {
    paths.iter().any(|candidate| paths_equal(candidate, target))
}

fn session_has_document(session: &Session) -> bool {
    let doc = session.document();
    !doc.data_sheets.is_empty()
        || !doc.graph_sheets.is_empty()
        || !doc.layout_sheets.is_empty()
        || !doc.result_sheets.is_empty()
        || !doc.info_sheets.is_empty()
        || session.file_path().is_some()
}

/// Check whether `candidate` currently holds a live, non-reclaimable recovery lock lease.
#[must_use]
pub fn dir_has_active_lease(candidate: &Path, now_epoch_ms: u64) -> bool {
    if !candidate.exists() {
        return false;
    }
    let Ok(backend) = DirectoryJournalBackend::open_dir(candidate) else {
        return false;
    };
    matches!(backend.read_lease(), Ok(Some(lease)) if !is_lease_reclaimable(&lease, now_epoch_ms))
}

/// Check whether `candidate` contains an unrecovered crash journal (a stale reclaimable lock
/// lease or non-empty pending WAL entries in `ops.journal`).
#[must_use]
pub fn dir_has_unrecovered_crash(candidate: &Path, now_epoch_ms: u64) -> bool {
    if !candidate.is_dir() {
        return false;
    }
    let Ok(backend) = DirectoryJournalBackend::open_dir(candidate) else {
        return false;
    };
    match backend.read_lease() {
        Ok(Some(lease)) => is_lease_reclaimable(&lease, now_epoch_ms),
        Ok(None) => backend.read_journal_bytes().is_ok_and(|bytes| bytes.iter().any(|byte| !byte.is_ascii_whitespace())),
        Err(_) => false,
    }
}

/// Scan `recovery_root` for stale crashed child journal directories (`instance-*` or `win-*`)
/// whose recovery lock leases are reclaimable or which contain uncheckpointed WAL entries.
#[must_use]
pub fn find_stale_recovery_dirs(recovery_root: &Path, now_epoch_ms: u64) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(recovery_root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path.file_name().and_then(|n| n.to_str()).is_some_and(|name| name.starts_with("instance-") || name.starts_with("win-"))
                && dir_has_unrecovered_crash(path, now_epoch_ms)
        })
        .collect();
    dirs.sort();
    dirs
}

/// Allocate a fresh, collision-free isolated recovery directory under `root` without
/// touching any active lease or unrecovered crash journal.
///
/// # Errors
/// Returns an error if creating the chosen directory fails.
pub fn allocate_fresh_isolated_journal_dir(root: &Path, window_id: DesktopWindowId, now_epoch_ms: u64) -> Result<PathBuf, String> {
    let chosen = loop {
        let seq = allocate_instance_seq();
        let candidate = root.join(format!("instance-{}-s{}-win-{}", std::process::id(), seq, window_id.0));
        if !dir_has_active_lease(&candidate, now_epoch_ms) && !dir_has_unrecovered_crash(&candidate, now_epoch_ms) {
            break candidate;
        }
    };
    std::fs::create_dir_all(&chosen).map_err(|err| format!("failed to create recovery dir `{}`: {err}", chosen.display()))?;
    Ok(chosen)
}

fn dir_occupies_journal(candidate: &Path, now_epoch_ms: u64) -> bool {
    dir_has_active_lease(candidate, now_epoch_ms) || dir_has_unrecovered_crash(candidate, now_epoch_ms)
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

    if !new_instance || !dir_occupies_journal(&base_candidate, now_epoch_ms) {
        std::fs::create_dir_all(&base_candidate).map_err(|err| format!("failed to create recovery dir `{}`: {err}", base_candidate.display()))?;
        return Ok(base_candidate);
    }
    allocate_fresh_isolated_journal_dir(root, window_id, now_epoch_ms)
}

fn is_readonly_control_method(method: &str) -> bool {
    is_read_only_control_method(method) || method == "app.quit"
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
    let mut requires_final_checkpoint = false;
    if !ctx.rolled_back {
        for step in ctx.steps {
            if step.mutated {
                recorded_any_step = true;
                dirty_commands.push(step.command_id.clone());
                if Session::command_requires_wal_checkpoint(&step.command_id) || step.command_id.starts_with("ui.") {
                    requires_final_checkpoint = true;
                }
            } else if step.command_id == "project.save" {
                recorded_any_step = true;
                if !app.session().is_dirty() {
                    dirty_commands.clear();
                }
                requires_final_checkpoint = true;
            }
        }

        if !is_readonly_control_method(ctx.method) {
            let recorded_mutating_steps = u64::try_from(ctx.steps.iter().filter(|s| s.mutated).count()).unwrap_or(u64::MAX);
            let seq_delta = app.mutation_seq().wrapping_sub(ctx.prev_mutation_seq);
            let now_dirty = app.session().is_dirty();
            let stack_changed = app.session().undo_depth() != ctx.prev_undo_depth || app.session().redo_depth() != ctx.prev_redo_depth;
            let became_dirty = !ctx.prev_dirty && now_dirty;
            let unrecorded_mutation = seq_delta > recorded_mutating_steps || (!recorded_any_step && (seq_delta > 0 || stack_changed || became_dirty));
            if unrecorded_mutation {
                recorded_any_step = true;
                requires_final_checkpoint = true;
                dirty_commands.push(ctx.method.to_owned());
            }
        }
    }

    if recorded_any_step && let Some(journal) = recovery_journal.as_mut() {
        if requires_final_checkpoint {
            // A batch containing any checkpoint-requiring step (`edit.undo`, `edit.redo`,
            // `READS_FILESYSTEM`, `project.save`, a UI command, or an unrecorded mutation) must snapshot
            // the final post-batch state ONCE without also appending subsequent steps that would
            // double-replay on crash recovery.
            app.session().checkpoint_journal(journal).map_err(|err| format!("failed to checkpoint control WAL batch `{}`: {err}", ctx.method))?;
        } else {
            for step in ctx.steps {
                if step.mutated {
                    app.session()
                        .record_outcome_to_journal(journal, &step.command_id, &step.params, true)
                        .map_err(|err| format!("failed to record control WAL entry `{}`: {err}", step.command_id))?;
                }
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
                if !ctx.rolled_back {
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
                    let recorded_mutating_steps = u64::try_from(ctx.steps.iter().filter(|s| s.mutated).count()).unwrap_or(u64::MAX);
                    if app_ref.mutation_seq().wrapping_sub(ctx.prev_mutation_seq) > recorded_mutating_steps {
                        clean_after_save = false;
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
        let _ = app.set_locale(normalize_beta_gui_locale(locale));
    }
    if let Some(theme_raw) = cfg.theme_override.as_deref() {
        let mode = parse_theme_mode(theme_raw)?;
        app.set_theme_mode(mode);
    }
    app.set_gui_only(cfg.gui_only);
    app.session_mut().set_path_policy(cfg.path_policy.clone());
    app.set_human_path_only(cfg.human_path_only);
    if let Some(adapter) = cfg.clipboard_adapter.as_ref() {
        app.set_clipboard_adapter(Arc::clone(adapter));
    }
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

    /// Last rendered software `egui` frame, if [`DesktopHostEvent::RedrawRequested`] has run.
    #[must_use]
    pub fn last_frame(&self) -> Option<&DesktopFrameOutput> {
        self.last_frame.as_ref()
    }

    /// Native window title combining document filename/untitled status, dirty marker, and `CurveBook` brand.
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
        format!("{base}{dirty_marker} — CurveBook")
    }

    /// Whether menu bars are drawn inside the window chrome rather than in a global native bar.
    #[must_use]
    pub const fn in_window_menus(&self) -> bool {
        self.in_window_menus
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
            locale: self.app.locale.clone(),
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

    /// Persist a full `.curvebook` recovery snapshot and truncate replayed WAL entries.
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

    pub(crate) fn record_ui_mutation_checkpoint(
        &mut self,
        command_id: Option<&str>,
        prev_undo: usize,
        prev_redo: usize,
        prev_dirty: bool,
        prev_mutation_seq: u64,
    ) -> Result<(), String> {
        if let Some(journal) = &mut self.recovery_journal {
            maybe_renew_journal_lease_periodically(journal);
        }
        let now_dirty = self.app.session().is_dirty();
        let mutated = self.app.mutation_seq() != prev_mutation_seq;
        let stack_changed = self.app.session().undo_depth() != prev_undo || self.app.session().redo_depth() != prev_redo;
        let became_dirty = !prev_dirty && now_dirty;
        if mutated || stack_changed || became_dirty {
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
    /// Returns an error if the software `egui` rasterizer fails.
    pub fn render_frame(&mut self) -> Result<DesktopFrameOutput, String> {
        if let Some(journal) = &mut self.recovery_journal {
            maybe_renew_journal_lease_periodically(journal);
        }
        let started = Instant::now();
        let target = self.viewport.to_render_target_spec(BASE_LOGICAL_DPI);
        let raster = self
            .app
            .render_egui_shell_rgba(target, curvebook_ui_egui::ScreenshotRegionTarget::FullWindow)
            .map_err(|err| format!("egui rasterize failed: {err}"))?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        #[allow(clippy::cast_possible_truncation)]
        let scale_factor = self.viewport.clamped_scale_factor() as f32;

        let output = DesktopFrameOutput {
            window_id: self.id,
            width_px: raster.width_px,
            height_px: raster.height_px,
            scale_factor,
            rgba_bytes: Arc::from(raster.rgba),
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

struct OpenFileDialogHook(Box<dyn FnMut(DesktopWindowId) -> Result<Option<PathBuf>, String>>);

impl std::fmt::Debug for OpenFileDialogHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenFileDialogHook(..)")
    }
}

/// Canonical file extensions accepted by native project/data `Open...` dialogs (`classify_and_load_path`).
pub const NATIVE_OPEN_FILE_EXTENSIONS: &[&str] = &["curvebook", "cbtemplate", "prism", "pzfx", "xlsx", "csv", "tsv"];
/// Canonical file extensions accepted by native template save and instantiate dialogs.
pub const NATIVE_TEMPLATE_FILE_EXTENSIONS: &[&str] = &["cbtemplate"];
/// Default filename suggested when saving a reusable `.cbtemplate` archive.
pub const DEFAULT_TEMPLATE_FILE_NAME: &str = "untitled.cbtemplate";

pub use curvebook_ui_egui::os_integration::{
    NativeFileDialogKind, NativeFileDialogRequest, PlatformFileDialogAdapter as NativeFileDialogAdapter,
    StubPlatformFileDialogAdapter as StubNativeFileDialogAdapter,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeExportTargetKind {
    Renderable,
    Graph,
    Layout,
    TableOrResult,
    Result,
}

#[derive(Debug, Clone, Copy)]
struct NativeExportSpec {
    command: &'static str,
    filter_label: &'static str,
    extensions: &'static [&'static str],
    default_name: &'static str,
    target_kind: NativeExportTargetKind,
}

fn native_export_spec_for_command(command_id: &str) -> Option<NativeExportSpec> {
    let spec = match command_id {
        "export.svg" | "file.export_svg" => NativeExportSpec {
            command: "export.svg",
            filter_label: "SVG Vector",
            extensions: &["svg"],
            default_name: "figure.svg",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "export.pdf" | "file.export_pdf" => NativeExportSpec {
            command: "export.pdf",
            filter_label: "PDF Document",
            extensions: &["pdf"],
            default_name: "figure.pdf",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "export.png" | "file.export_png" => NativeExportSpec {
            command: "export.png",
            filter_label: "PNG Image",
            extensions: &["png"],
            default_name: "figure.png",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "export.tiff" => NativeExportSpec {
            command: "export.tiff",
            filter_label: "TIFF Image",
            extensions: &["tiff", "tif"],
            default_name: "figure.tiff",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "graph.exportSvg" => NativeExportSpec {
            command: "graph.exportSvg",
            filter_label: "SVG Vector",
            extensions: &["svg"],
            default_name: "figure.svg",
            target_kind: NativeExportTargetKind::Graph,
        },
        "graph.exportPdf" => NativeExportSpec {
            command: "graph.exportPdf",
            filter_label: "PDF Document",
            extensions: &["pdf"],
            default_name: "figure.pdf",
            target_kind: NativeExportTargetKind::Graph,
        },
        "graph.exportPng" => NativeExportSpec {
            command: "graph.exportPng",
            filter_label: "PNG Image",
            extensions: &["png"],
            default_name: "figure.png",
            target_kind: NativeExportTargetKind::Graph,
        },
        "graph.exportEmf" => NativeExportSpec {
            command: "graph.exportEmf",
            filter_label: "EMF Vector",
            extensions: &["emf"],
            default_name: "figure.emf",
            target_kind: NativeExportTargetKind::Graph,
        },
        "layout.exportEmf" => NativeExportSpec {
            command: "layout.exportEmf",
            filter_label: "EMF Vector",
            extensions: &["emf"],
            default_name: "layout.emf",
            target_kind: NativeExportTargetKind::Layout,
        },
        "table.exportCsv" => NativeExportSpec {
            command: "table.exportCsv",
            filter_label: "CSV Table",
            extensions: &["csv", "tsv"],
            default_name: "table.csv",
            target_kind: NativeExportTargetKind::TableOrResult,
        },
        "analysis.exportJson" => NativeExportSpec {
            command: "analysis.exportJson",
            filter_label: "JSON Results",
            extensions: &["json"],
            default_name: "analysis.json",
            target_kind: NativeExportTargetKind::Result,
        },
        "analysis.exportCsv" => NativeExportSpec {
            command: "analysis.exportCsv",
            filter_label: "CSV Results",
            extensions: &["csv", "tsv"],
            default_name: "analysis.csv",
            target_kind: NativeExportTargetKind::Result,
        },
        "result.generateReport" => NativeExportSpec {
            command: "result.generateReport",
            filter_label: "Methods Report",
            extensions: &["md", "txt", "tex"],
            default_name: "methods-report.md",
            target_kind: NativeExportTargetKind::Result,
        },
        _ => return None,
    };
    Some(spec)
}

/// File extensions accepted when opening an embedded image via the native OS picker (`WF-57`).
pub const NATIVE_IMAGE_FILE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff"];

/// Returns `true` when clicking `command_id` in the native desktop window opens an OS file dialog.
#[must_use]
pub fn menu_action_triggers_native_file_dialog(command_id: &str) -> bool {
    matches!(
        command_id,
        "project.open"
            | "file.open"
            | "file.open_unpacked_dir"
            | "project.save"
            | "file.save"
            | "project.saveAs"
            | "file.save_as"
            | "project.saveTemplate"
            | "project.instantiateTemplate"
            | "table.importCsv"
            | "table.importPrism"
            | "table.importPzfx"
            | "table.importXlsx"
            | "graph.insertImage"
            | "layout.insertImage"
            | "ui.export.allTablesCsv"
    ) || native_export_spec_for_command(command_id).is_some()
}

/// Returns `true` when an import command replaces the full active document and must detach any
/// previously associated `.curvebook` archive path so subsequent `Cmd+S` prompts cleanly.
#[must_use]
pub fn native_import_replaces_project_file_path(command_id: &str) -> bool {
    matches!(command_id, "project.instantiateTemplate" | "table.importPrism" | "table.importPzfx" | "table.importXlsx")
}

/// Execute a file-backed import or template instantiation command at `path`, optionally merging
/// into the active project (`table.importPrism`, `table.importPzfx`, `table.importXlsx`), clearing
/// the active session's archive association when replacing the full document, and recording any
/// structured import warnings into `UiApp`.
///
/// # Errors
/// Returns an error string if executing the underlying engine command fails.
pub fn execute_native_import_at_path(win: &mut DesktopWindowSession, command: &str, path: &Path, merge: bool) -> Result<(), String> {
    let path_str = path.to_string_lossy().into_owned();
    let params = if matches!(command, "table.importPrism" | "table.importPzfx" | "table.importXlsx") {
        json!({ "path": path_str, "merge": merge })
    } else {
        json!({ "path": path_str })
    };
    let outcome = win.execute_and_journal(command, params)?;
    if !merge && native_import_replaces_project_file_path(command) {
        win.app_mut().session_mut().set_file_path(None, false);
    }
    let source_label = path.file_name().and_then(|n| n.to_str()).unwrap_or(&path_str);
    win.app_mut().record_import_warnings(command, source_label, &outcome);
    Ok(())
}

fn sanitize_export_table_stem(title: &str, fallback_id: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_was_sep = false;
    for ch in title.trim().chars() {
        let is_bad = matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || ch.is_control();
        if is_bad {
            if !last_was_sep && !out.is_empty() {
                out.push('_');
                last_was_sep = true;
            }
        } else {
            out.push(ch);
            last_was_sep = false;
        }
    }
    let trimmed = out.trim_matches(|c: char| c.is_whitespace() || c == '.' || c == '_');
    if trimmed.is_empty() { fallback_id.to_owned() } else { trimmed.to_owned() }
}

fn resolve_native_export_sheet_id_with_hint(doc: &ProjectDocument, kind: NativeExportTargetKind, hint: Option<&str>) -> Option<SheetId> {
    if let Some(raw) = hint
        && let Ok(id) = SheetId::try_new(raw)
    {
        let valid = match kind {
            NativeExportTargetKind::Renderable => matches!(doc.sheet_section(&id), Some(SheetSection::Graphs | SheetSection::Layouts)),
            NativeExportTargetKind::Graph => doc.graph_sheet(&id).is_some(),
            NativeExportTargetKind::Layout => doc.layout_sheet(&id).is_some(),
            NativeExportTargetKind::TableOrResult => doc.data_sheet(&id).is_some() || doc.result_sheet(&id).is_some(),
            NativeExportTargetKind::Result => doc.result_sheet(&id).is_some(),
        };
        if valid {
            return Some(id);
        }
    }
    resolve_native_export_sheet_id(doc, kind)
}

fn resolve_native_export_sheet_id(doc: &ProjectDocument, kind: NativeExportTargetKind) -> Option<SheetId> {
    let active = doc.active_sheet_id.as_ref();
    match kind {
        NativeExportTargetKind::Renderable => active
            .filter(|id| matches!(doc.sheet_section(id), Some(SheetSection::Graphs | SheetSection::Layouts)))
            .cloned()
            .or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
            .or_else(|| doc.layout_sheets.first().map(|l| l.id.clone())),
        NativeExportTargetKind::Graph => {
            active.filter(|id| doc.graph_sheet(id).is_some()).cloned().or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
        }
        NativeExportTargetKind::Layout => {
            active.filter(|id| doc.layout_sheet(id).is_some()).cloned().or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
        }
        NativeExportTargetKind::TableOrResult => active
            .filter(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some())
            .cloned()
            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()))
            .or_else(|| doc.result_sheets.first().map(|r| r.id.clone())),
        NativeExportTargetKind::Result => {
            active.filter(|id| doc.result_sheet(id).is_some()).cloned().or_else(|| doc.result_sheets.first().map(|r| r.id.clone()))
        }
    }
}

/// Injectable OS clipboard adapter used by [`DesktopHostRuntime`].
pub trait OsClipboardAdapter: fmt::Debug + Send + Sync {
    /// Write plain UTF-8 / TSV text to the OS pasteboard.
    ///
    /// # Errors
    /// Returns an error if the OS clipboard backend fails unexpectedly.
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String>;

    /// Write a multi-flavor [`ClipboardBundle`] (SVG, PDF, PNG, TIFF, EMF, HTML) to the OS pasteboard.
    ///
    /// # Errors
    /// Returns an error if the OS clipboard backend fails unexpectedly.
    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String>;

    /// Write a slide/document-focused [`ClipboardBundle`] (omitting raw `<svg>` XML plain-text fallback).
    ///
    /// # Errors
    /// Returns an error if the OS clipboard backend fails unexpectedly.
    fn write_slide_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        self.write_bundle(bundle)
    }

    /// Read plain UTF-8 text from the OS pasteboard.
    ///
    /// # Errors
    /// Returns an error if the OS clipboard backend fails unexpectedly.
    fn read_text(&self) -> Result<OsAdapterStatus, String>;
}

#[derive(Debug, Default)]
struct SystemOsClipboardAdapter;

impl OsClipboardAdapter for SystemOsClipboardAdapter {
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
        write_os_pasteboard(text)
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        write_os_clipboard_bundle(bundle)
    }

    fn write_slide_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        write_os_clipboard_bundle_for_slides(bundle)
    }

    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        read_os_pasteboard()
    }
}

#[derive(Debug)]
struct PlatformOsClipboardBridge(Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);

impl OsClipboardAdapter for PlatformOsClipboardBridge {
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
        self.0.write_text(text)
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        self.0.write_bundle(bundle)
    }

    fn write_slide_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        self.0.write_slide_bundle(bundle)
    }

    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        self.0.read_text()
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
    clipboard_adapter: Box<dyn OsClipboardAdapter>,
    preferences_persist_hook: Option<PreferencesPersistHook>,
    open_file_dialog_hook: Option<OpenFileDialogHook>,
    file_dialog_adapter: Option<Arc<dyn NativeFileDialogAdapter>>,
    url_opener: Option<Arc<dyn curvebook_ui_egui::PlatformUrlOpener>>,
}

impl DesktopHostRuntime {
    /// Construct a new empty [`DesktopHostRuntime`] governed by `config`.
    #[must_use]
    pub fn new(config: DesktopHostConfig) -> Self {
        let clipboard_adapter: Box<dyn OsClipboardAdapter> = match config.clipboard_adapter.as_ref() {
            Some(adapter) => Box::new(PlatformOsClipboardBridge(Arc::clone(adapter))),
            None => Box::new(SystemOsClipboardAdapter),
        };
        Self {
            config,
            next_window_id: 1,
            windows: BTreeMap::new(),
            focused_window: None,
            clipboard_text: None,
            clipboard_bundle: None,
            clipboard_adapter,
            preferences_persist_hook: None,
            open_file_dialog_hook: None,
            file_dialog_adapter: None,
            url_opener: None,
        }
    }

    /// Replace the OS clipboard backend (used in deterministic tests to simulate
    /// external OS pasteboard changes or OS pasteboard failures).
    pub fn set_clipboard_adapter(&mut self, adapter: Box<dyn OsClipboardAdapter>) {
        self.clipboard_adapter = adapter;
    }

    /// Install a [`curvebook_ui_egui::PlatformUrlOpener`] across all current and future windows
    /// owned by this runtime.
    pub fn set_url_opener(&mut self, opener: Arc<dyn curvebook_ui_egui::PlatformUrlOpener>) {
        for win in self.windows.values_mut() {
            win.app.set_url_opener(Arc::clone(&opener));
        }
        self.url_opener = Some(opener);
    }

    /// Install a callback invoked before closing windows or when UI preference/shortcut state changes.
    pub fn set_preferences_persist_hook<F>(&mut self, hook: F)
    where
        F: FnMut(&UiApp) + 'static,
    {
        self.preferences_persist_hook = Some(PreferencesPersistHook(Box::new(hook)));
    }

    /// Install a host file-picker callback invoked when the UI emits `"welcome.open_file_requested"`.
    pub fn set_open_file_dialog_hook<F>(&mut self, hook: F)
    where
        F: FnMut(DesktopWindowId) -> Result<Option<PathBuf>, String> + 'static,
    {
        self.open_file_dialog_hook = Some(OpenFileDialogHook(Box::new(hook)));
    }

    /// Return `true` when a [`NativeFileDialogAdapter`] is already installed.
    #[must_use]
    pub fn has_file_dialog_adapter(&self) -> bool {
        self.file_dialog_adapter.is_some()
    }

    /// Install a [`NativeFileDialogAdapter`] used by drawn File menu items, toolbar Open/Save/Export,
    /// Welcome Open, and native menu file actions.
    pub fn set_file_dialog_adapter(&mut self, adapter: Arc<dyn NativeFileDialogAdapter>) {
        self.file_dialog_adapter = Some(adapter);
    }

    /// Route a file-backed desktop command (`project.open`, `project.save`, `file.save_as`, `export.svg`,
    /// `export.pdf`, `export.png`, `graph.exportEmf`, `layout.exportEmf`, `table.importPzfx`, ...)
    /// through the installed [`NativeFileDialogAdapter`].
    ///
    /// Returns `Ok(None)` when no [`NativeFileDialogAdapter`] is installed or `command_id` does not
    /// trigger a native file dialog.
    ///
    /// # Errors
    /// Returns an error if the adapter fails, no export target sheet exists, or loading/saving/exporting fails.
    pub fn dispatch_native_file_command(&mut self, window_id: DesktopWindowId, command_id: &str) -> Result<Option<DesktopEventOutcome>, String> {
        self.dispatch_native_file_command_with_params(window_id, command_id, &json!({}))
    }

    /// Route a file-backed desktop command with explicit JSON `params` (such as `{"path": "..."}`
    /// from the Welcome Hub open pane or `{"sheet_id": "..."}` from the Export dialog) through the
    /// desktop host file/save flow (`R15-GUI-13`, `R15-GUI-23`).
    ///
    /// # Errors
    /// Returns an error if the adapter fails, no export target sheet exists, or loading/saving/exporting fails.
    pub fn dispatch_native_file_command_with_params(
        &mut self,
        window_id: DesktopWindowId,
        command_id: &str,
        params: &Value,
    ) -> Result<Option<DesktopEventOutcome>, String> {
        if matches!(command_id, "project.open" | "file.open")
            && let Some(explicit_path) = params.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
        {
            return self.handle_file_dropped(window_id, PathBuf::from(explicit_path)).map(Some);
        }
        if matches!(command_id, "table.importPrism" | "table.importPzfx" | "table.importXlsx")
            && let Some(explicit_path) = params.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
            && let Some(merge) = params.get("merge").and_then(Value::as_bool)
        {
            let path = PathBuf::from(explicit_path);
            let win = self.window_mut(window_id)?;
            execute_native_import_at_path(win, command_id, &path, merge)?;
            return Ok(Some(Self::completed_file_dialog_outcome(window_id, command_id, &path)));
        }
        if command_id == "project.open"
            && let Some(mut hook) = self.open_file_dialog_hook.take()
        {
            let picked = (hook.0)(window_id);
            self.open_file_dialog_hook = Some(hook);
            if let Some(path) = picked? {
                return self.handle_file_dropped(window_id, path).map(Some);
            }
            return Ok(Some(Self::cancelled_file_dialog_outcome(window_id, command_id)));
        }
        let Some(adapter) = self.file_dialog_adapter.as_ref().map(Arc::clone) else {
            return Ok(None);
        };
        if !menu_action_triggers_native_file_dialog(command_id) {
            return Ok(None);
        }
        let win_locale = self.window(window_id).map(|w| w.app().locale.clone()).unwrap_or_else(|_| String::from("en"));
        let outcome = match command_id {
            "project.open" | "file.open" => {
                let req = NativeFileDialogRequest {
                    command_id: command_id.to_owned(),
                    kind: NativeFileDialogKind::OpenFile,
                    filter_label: String::from("CurveBook / Data"),
                    extensions: NATIVE_OPEN_FILE_EXTENSIONS.iter().map(|s| (*s).to_owned()).collect(),
                    default_file_name: None,
                };
                match adapter.pick(&req)? {
                    Some(path) => self.handle_file_dropped(window_id, path)?,
                    None => Self::cancelled_file_dialog_outcome(window_id, command_id),
                }
            }
            "file.open_unpacked_dir" => {
                let req = NativeFileDialogRequest {
                    command_id: command_id.to_owned(),
                    kind: NativeFileDialogKind::PickFolder,
                    filter_label: String::from("CurveBook Unpacked Project"),
                    extensions: Vec::new(),
                    default_file_name: None,
                };
                match adapter.pick(&req)? {
                    Some(path) => self.handle_file_dropped(window_id, path)?,
                    None => Self::cancelled_file_dialog_outcome(window_id, command_id),
                }
            }
            "project.save" | "file.save" => self.execute_native_save_with_adapter(window_id, command_id, false, adapter.as_ref())?,
            "project.saveAs" | "file.save_as" => self.execute_native_save_with_adapter(window_id, command_id, true, adapter.as_ref())?,
            "project.saveTemplate" => {
                let req = NativeFileDialogRequest {
                    command_id: command_id.to_owned(),
                    kind: NativeFileDialogKind::SaveFile,
                    filter_label: tr(&win_locale, "w6.view.shell.filter_template").to_owned(),
                    extensions: NATIVE_TEMPLATE_FILE_EXTENSIONS.iter().map(|s| (*s).to_owned()).collect(),
                    default_file_name: Some(String::from(DEFAULT_TEMPLATE_FILE_NAME)),
                };
                match adapter.pick(&req)? {
                    Some(path) => {
                        let path_str = path.to_string_lossy().into_owned();
                        let win = self.window_mut(window_id)?;
                        win.execute_and_journal("project.saveTemplate", json!({ "path": path_str }))?;
                        Self::completed_file_dialog_outcome(window_id, "project.saveTemplate", &path)
                    }
                    None => Self::cancelled_file_dialog_outcome(window_id, "project.saveTemplate"),
                }
            }
            "project.instantiateTemplate" | "table.importCsv" | "table.importPrism" | "table.importPzfx" | "table.importXlsx" => {
                let (filter_label, extensions): (String, Vec<String>) = match command_id {
                    "project.instantiateTemplate" => (
                        tr(&win_locale, "w6.view.shell.filter_template").to_owned(),
                        NATIVE_TEMPLATE_FILE_EXTENSIONS.iter().map(|s| (*s).to_owned()).collect(),
                    ),
                    "table.importCsv" => (
                        tr(&win_locale, "w6.view.shell.filter_delimited").to_owned(),
                        vec![String::from("csv"), String::from("tsv"), String::from("txt")],
                    ),
                    "table.importPrism" => (tr(&win_locale, "w6.view.shell.filter_archive").to_owned(), vec![String::from("prism")]),
                    "table.importXlsx" => (tr(&win_locale, "cmd.table.importXlsx").to_owned(), vec![String::from("xlsx")]),
                    _ => (tr(&win_locale, "w6.view.shell.filter_xml_project").to_owned(), vec![String::from("pzfx")]),
                };
                let req = NativeFileDialogRequest {
                    command_id: command_id.to_owned(),
                    kind: NativeFileDialogKind::OpenFile,
                    filter_label,
                    extensions,
                    default_file_name: None,
                };
                match adapter.pick(&req)? {
                    Some(path) => {
                        let win = self.window_mut(window_id)?;
                        if matches!(command_id, "table.importPrism" | "table.importPzfx" | "table.importXlsx") {
                            if let Some(merge) = params.get("merge").and_then(Value::as_bool) {
                                execute_native_import_at_path(win, command_id, &path, merge)?;
                                Self::completed_file_dialog_outcome(window_id, command_id, &path)
                            } else if win.app().document().total_sheet_count() > 0 || win.has_unsaved_changes() {
                                let has_unsaved = win.has_unsaved_changes();
                                win.app_mut().open_import_project_mode_dialog(command_id, path.to_string_lossy().into_owned(), has_unsaved);
                                DesktopEventOutcome {
                                    handled: true,
                                    action: format!("native_file_dialog.confirm_import_mode:{command_id}"),
                                    window_id: Some(window_id),
                                    command_id: Some(command_id.to_owned()),
                                    command_result: Some(json!({ "path": path.to_string_lossy(), "prompted": true })),
                                    ..Default::default()
                                }
                            } else {
                                execute_native_import_at_path(win, command_id, &path, false)?;
                                Self::completed_file_dialog_outcome(window_id, command_id, &path)
                            }
                        } else {
                            execute_native_import_at_path(win, command_id, &path, false)?;
                            Self::completed_file_dialog_outcome(window_id, command_id, &path)
                        }
                    }
                    None => Self::cancelled_file_dialog_outcome(window_id, command_id),
                }
            }
            "ui.export.allTablesCsv" => {
                let sheets: Vec<(SheetId, String)> = {
                    let win = self.window(window_id)?;
                    win.app().document().data_sheets.iter().map(|ds| (ds.id.clone(), ds.title.clone())).collect()
                };
                if sheets.is_empty() {
                    return Err(String::from("no data tables available to export"));
                }
                let req = NativeFileDialogRequest {
                    command_id: String::from("ui.export.allTablesCsv"),
                    kind: NativeFileDialogKind::PickFolder,
                    filter_label: String::from("Export All Data Tables"),
                    extensions: Vec::new(),
                    default_file_name: None,
                };
                match adapter.pick(&req)? {
                    Some(dir) => {
                        std::fs::create_dir_all(&dir).map_err(|err| format!("failed to prepare export folder `{}`: {err}", dir.display()))?;
                        let delimiter = params.get("delimiter").and_then(Value::as_str).unwrap_or(",").to_owned();
                        let default_decimal =
                            self.window(window_id).map(|w| w.app().locale_preset.decimal_separator.to_string()).unwrap_or_else(|_| String::from("."));
                        let decimal_separator = params.get("decimal_separator").and_then(Value::as_str).map_or(default_decimal, ToOwned::to_owned);
                        let ext = if delimiter == "\t" { "tsv" } else { "csv" };
                        let mut used_stems: std::collections::HashSet<String> = std::collections::HashSet::new();
                        let mut planned: Vec<(SheetId, PathBuf)> = Vec::with_capacity(sheets.len());
                        for (sheet_id, title) in sheets {
                            let base = sanitize_export_table_stem(&title, sheet_id.as_str());
                            let stem = if used_stems.insert(base.to_ascii_lowercase()) {
                                base
                            } else {
                                let mut suffix = 2usize;
                                loop {
                                    let candidate = format!("{base}-{suffix}");
                                    if used_stems.insert(candidate.to_ascii_lowercase()) {
                                        break candidate;
                                    }
                                    suffix = suffix.saturating_add(1);
                                }
                            };
                            planned.push((sheet_id, dir.join(format!("{stem}.{ext}"))));
                        }
                        let mut exported_files: Vec<String> = Vec::with_capacity(planned.len());
                        let win = self.window_mut(window_id)?;
                        for (sheet_id, file_path) in planned {
                            let file_path_str = file_path.to_string_lossy().into_owned();
                            win.execute_and_journal(
                                "table.exportCsv",
                                json!({
                                    "sheet_id": sheet_id.as_str(),
                                    "path": file_path_str,
                                    "delimiter": delimiter,
                                    "decimal_separator": decimal_separator,
                                }),
                            )?;
                            exported_files.push(file_path_str);
                        }
                        DesktopEventOutcome {
                            handled: true,
                            action: String::from("native_file_dialog.completed:ui.export.allTablesCsv"),
                            window_id: Some(window_id),
                            command_id: Some(String::from("ui.export.allTablesCsv")),
                            command_result: Some(json!({
                                "path": dir.to_string_lossy(),
                                "exported_files": exported_files,
                                "count": exported_files.len(),
                            })),
                            ..Default::default()
                        }
                    }
                    None => Self::cancelled_file_dialog_outcome(window_id, "ui.export.allTablesCsv"),
                }
            }
            "graph.insertImage" | "layout.insertImage" => {
                let target_kind = if command_id == "graph.insertImage" { NativeExportTargetKind::Graph } else { NativeExportTargetKind::Layout };
                let sheet_hint = params.get("sheet_id").and_then(Value::as_str);
                let target_sheet_id = {
                    let win = self.window(window_id)?;
                    resolve_native_export_sheet_id_with_hint(win.app().document(), target_kind, sheet_hint)
                        .ok_or_else(|| format!("no matching target sheet available for `{command_id}`"))?
                };
                let req = NativeFileDialogRequest {
                    command_id: command_id.to_owned(),
                    kind: NativeFileDialogKind::OpenFile,
                    filter_label: String::from("Images (PNG, JPEG, TIFF)"),
                    extensions: NATIVE_IMAGE_FILE_EXTENSIONS.iter().map(|s| (*s).to_owned()).collect(),
                    default_file_name: None,
                };
                match adapter.pick(&req)? {
                    Some(path) => {
                        let path_str = path.to_string_lossy().into_owned();
                        let win = self.window_mut(window_id)?;
                        win.execute_and_journal(command_id, json!({ "sheet_id": target_sheet_id.as_str(), "path": path_str }))?;
                        Self::completed_file_dialog_outcome(window_id, command_id, &path)
                    }
                    None => Self::cancelled_file_dialog_outcome(window_id, command_id),
                }
            }
            _ => {
                let spec = native_export_spec_for_command(command_id).ok_or_else(|| format!("unhandled file command `{command_id}`"))?;
                let sheet_hint = params.get("sheet_id").or_else(|| params.get("result_sheet_id")).and_then(Value::as_str);
                let target_sheet_id = {
                    let win = self.window(window_id)?;
                    resolve_native_export_sheet_id_with_hint(win.app().document(), spec.target_kind, sheet_hint)
                        .ok_or_else(|| format!("no matching sheet available for `{}`", spec.command))?
                };
                let req = NativeFileDialogRequest {
                    command_id: spec.command.to_owned(),
                    kind: NativeFileDialogKind::SaveFile,
                    filter_label: spec.filter_label.to_owned(),
                    extensions: spec.extensions.iter().map(|s| (*s).to_owned()).collect(),
                    default_file_name: Some(spec.default_name.to_owned()),
                };
                match adapter.pick(&req)? {
                    Some(path) => {
                        let path_str = path.to_string_lossy().into_owned();
                        let export_params = Self::build_native_export_params(spec.command, &target_sheet_id, &path, &path_str, params);
                        let win = self.window_mut(window_id)?;
                        win.execute_and_journal(spec.command, export_params)?;
                        Self::completed_file_dialog_outcome(window_id, spec.command, &path)
                    }
                    None => Self::cancelled_file_dialog_outcome(window_id, spec.command),
                }
            }
        };
        Ok(Some(outcome))
    }

    fn copy_present_params(dst: &mut serde_json::Map<String, Value>, src: &Value, allowed_keys: &[&str]) {
        let Some(obj) = src.as_object() else {
            return;
        };
        for &key in allowed_keys {
            if let Some(val) = obj.get(key)
                && !val.is_null()
            {
                dst.insert(key.to_owned(), val.clone());
            }
        }
    }

    fn allowed_native_export_keys(command: &str) -> &'static [&'static str] {
        match command {
            "export.svg" | "graph.exportSvg" => {
                &["pretty", "include_xml_declaration", "include_metadata", "embed_fonts", "precision", "outline_text"]
            }
            "export.pdf" | "graph.exportPdf" => &["color_space", "compress_streams", "title", "outline_text", "sheet_ids"],
            "export.png" | "graph.exportPng" => &["width_px", "height_px", "dpi"],
            "export.tiff" => &["width_px", "height_px", "dpi", "compression"],
            "table.exportCsv" => &["delimiter", "decimal_separator"],
            "analysis.exportJson" => &["pretty"],
            "analysis.exportCsv" => &["tab_id", "tab_index", "delimiter", "decimal_separator"],
            _ => &[],
        }
    }

    fn build_native_export_params(
        command: &str,
        target_sheet_id: &curvebook_doc::SheetId,
        path: &std::path::Path,
        path_str: &str,
        caller_params: &Value,
    ) -> Value {
        let mut map = serde_json::Map::new();
        if command == "result.generateReport" {
            let report_fmt = caller_params
                .get("format")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| matches!(*s, "plain" | "markdown" | "latex"))
                .unwrap_or_else(|| match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                    Some("tex") => "latex",
                    Some("txt") => "plain",
                    _ => "markdown",
                });
            map.insert(String::from("result_sheet_id"), Value::String(target_sheet_id.as_str().to_owned()));
            map.insert(String::from("format"), Value::String(report_fmt.to_owned()));
            map.insert(String::from("path"), Value::String(path_str.to_owned()));
            return Value::Object(map);
        }

        Self::copy_present_params(&mut map, caller_params, Self::allowed_native_export_keys(command));
        if matches!(command, "table.exportCsv" | "analysis.exportCsv")
            && path.extension().and_then(|e| e.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("tsv"))
        {
            map.insert(String::from("delimiter"), Value::String(String::from("\t")));
        }
        let has_multi_pdf =
            matches!(command, "export.pdf" | "graph.exportPdf") && map.get("sheet_ids").and_then(Value::as_array).is_some_and(|arr| !arr.is_empty());
        if !has_multi_pdf {
            map.insert(String::from("sheet_id"), Value::String(target_sheet_id.as_str().to_owned()));
        }
        map.insert(String::from("path"), Value::String(path_str.to_owned()));
        Value::Object(map)
    }

    fn execute_native_save_with_adapter(
        &mut self,
        window_id: DesktopWindowId,
        command_id: &str,
        force_dialog: bool,
        adapter: &dyn NativeFileDialogAdapter,
    ) -> Result<DesktopEventOutcome, String> {
        let existing_path = self.window(window_id).ok().and_then(|win| win.app().session().file_path().map(Path::to_path_buf));
        let target_path = if !force_dialog && let Some(path) = existing_path {
            path
        } else {
            let req = NativeFileDialogRequest {
                command_id: command_id.to_owned(),
                kind: NativeFileDialogKind::SaveFile,
                filter_label: String::from("CurveBook Project"),
                extensions: vec![String::from("curvebook")],
                default_file_name: Some(String::from("untitled.curvebook")),
            };
            match adapter.pick(&req)? {
                Some(path) => path,
                None => return Ok(Self::cancelled_file_dialog_outcome(window_id, command_id)),
            }
        };
        let is_unpacked = is_unpacked_project_dir(&target_path)
            || target_path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().ends_with(".curvebook.d"));
        let path_str = target_path.to_string_lossy().into_owned();
        let win = self.window_mut(window_id)?;
        win.execute_and_journal("project.save", json!({ "path": path_str, "unpacked": is_unpacked }))?;
        win.app_mut().session_mut().set_file_path(Some(target_path.clone()), true);
        win.dirty_commands.clear();
        win.checkpoint_recovery_journal()?;
        Ok(Self::completed_file_dialog_outcome(window_id, command_id, &target_path))
    }

    fn completed_file_dialog_outcome(window_id: DesktopWindowId, command_id: &str, path: &Path) -> DesktopEventOutcome {
        DesktopEventOutcome {
            handled: true,
            action: format!("native_file_dialog.completed:{command_id}"),
            window_id: Some(window_id),
            command_id: Some(command_id.to_owned()),
            command_result: Some(json!({ "path": path.to_string_lossy() })),
            ..Default::default()
        }
    }

    fn cancelled_file_dialog_outcome(window_id: DesktopWindowId, command_id: &str) -> DesktopEventOutcome {
        DesktopEventOutcome {
            handled: true,
            action: format!("native_file_dialog.cancelled:{command_id}"),
            window_id: Some(window_id),
            command_id: Some(command_id.to_owned()),
            ..Default::default()
        }
    }

    fn persist_window_preferences(&mut self, window_id: DesktopWindowId) {
        if let Some(hook) = self.preferences_persist_hook.as_mut()
            && let Some(win) = self.windows.get(&window_id)
        {
            (hook.0)(&win.app);
        }
    }

    /// Persist `preferences.json` for `window_id` when a persistence hook is attached.
    pub fn notify_preferences_changed(&mut self, window_id: DesktopWindowId) {
        self.persist_window_preferences(window_id);
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

    /// Focus an existing open window by `window_id` and return `true` when focus changed.
    ///
    /// # Errors
    /// Returns an error if `window_id` is not an open window.
    pub fn focus_window(&mut self, window_id: DesktopWindowId) -> Result<bool, String> {
        let changed = self.focused_window != Some(window_id);
        let _ = self.handle_window_focused(window_id)?;
        Ok(changed)
    }

    /// Close all clean windows or prompt on the first dirty window (`preferred_window` first if dirty).
    ///
    /// Returns [`CloseDisposition::Closed`] once every open window has closed cleanly.
    ///
    /// # Errors
    /// Returns an error if closing a window fails.
    pub fn request_app_quit(&mut self, preferred_window: Option<DesktopWindowId>, force: bool) -> Result<DesktopEventOutcome, String> {
        if force {
            for id in self.window_ids() {
                self.persist_window_preferences(id);
            }
            self.close_all_force()?;
            return Ok(DesktopEventOutcome {
                handled: true,
                action: String::from("app.quit"),
                window_id: None,
                close_disposition: Some(CloseDisposition::Closed),
                command_id: Some(String::from("app.quit")),
                ..Default::default()
            });
        }

        let mut ordered = Vec::with_capacity(self.windows.len());
        if let Some(pref) = preferred_window.or(self.focused_window)
            && self.windows.contains_key(&pref)
        {
            ordered.push(pref);
        }
        for id in self.window_ids() {
            if !ordered.contains(&id) {
                ordered.push(id);
            }
        }

        for id in &ordered {
            let is_dirty = self.windows.get(id).is_some_and(DesktopWindowSession::has_unsaved_changes);
            if is_dirty {
                let _ = self.focus_window(*id);
                let dirty_commands = self.windows.get(id).map_or_else(
                    || vec![String::from("unsaved.edits")],
                    |w| {
                        if w.dirty_commands.is_empty() { vec![String::from("unsaved.edits")] } else { w.dirty_commands.clone() }
                    },
                );
                return Ok(DesktopEventOutcome {
                    handled: true,
                    action: String::from("app.quit_prompt_unsaved"),
                    window_id: Some(*id),
                    close_disposition: Some(CloseDisposition::PromptUnsaved { window_id: *id, dirty_commands }),
                    command_id: Some(String::from("app.quit")),
                    ..Default::default()
                });
            }
        }

        for id in ordered {
            self.persist_window_preferences(id);
            let outcome = self.handle_window_close_requested(id, false)?;
            if matches!(outcome.close_disposition, Some(CloseDisposition::PromptUnsaved { .. })) {
                return Ok(outcome);
            }
        }

        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("app.quit"),
            window_id: None,
            close_disposition: Some(CloseDisposition::Closed),
            command_id: Some(String::from("app.quit")),
            ..Default::default()
        })
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
        let attach_recovery = self.config.recovery_root.is_some();
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

    /// Scan `config.recovery_root` for stale crashed journals (`win-*`, `instance-*`, or an
    /// unopened root) and reopen each one into its own recovered desktop window.
    ///
    /// # Errors
    /// Returns an error if recovering any discovered stale journal fails.
    pub fn recover_stale_journals(&mut self, viewport: ViewportMetrics, now_epoch_ms: u64) -> Result<Vec<DesktopWindowId>, String> {
        let Some(root) = self.config.recovery_root.clone() else {
            return Ok(Vec::new());
        };
        let mut open_dirs: Vec<PathBuf> = self.windows.values().filter_map(|win| win.recovery_dir.clone()).collect();
        let mut candidates = Vec::new();
        if dir_has_unrecovered_crash(&root, now_epoch_ms) {
            candidates.push(root.clone());
        }
        candidates.extend(find_stale_recovery_dirs(&root, now_epoch_ms));

        let mut recovered_ids = Vec::new();
        for dir in candidates {
            if paths_contain_equivalent(&open_dirs, &dir) {
                continue;
            }
            let win_id = self.recover_window_from_dir(dir.clone(), viewport, now_epoch_ms)?;
            open_dirs.push(dir);
            recovered_ids.push(win_id);
        }
        Ok(recovered_ids)
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
                let has_doc = session_has_document(&session);
                let journal_dir = self.choose_window_journal_dir(&dir, id, has_doc, now_epoch_ms)?;
                let owner_id = recovery_owner_for_window(id);
                let (recovered_session, journal, report) =
                    Session::open_dir_with_recovery(&journal_dir, &owner_id, DEFAULT_RECOVERY_LOCK_TTL_MS, now_epoch_ms)
                        .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", journal_dir.display()))?;
                if !has_doc {
                    return self.install_window_session(id, recovered_session, viewport, Some((journal_dir, journal, Some(report))));
                }
                let had_prior_crash = report.recovered_from_stale_lock.is_some() || !report.recovered_commands.is_empty();
                let (final_dir, mut final_journal) = if had_prior_crash {
                    drop(journal.abandon());
                    let fresh_dir = allocate_fresh_isolated_journal_dir(&dir, id, now_epoch_ms)?;
                    let (_, fresh_journal, _) = Session::open_dir_with_recovery(&fresh_dir, &owner_id, DEFAULT_RECOVERY_LOCK_TTL_MS, now_epoch_ms)
                        .map_err(|err| format!("failed to open fresh recovery journal in `{}`: {err}", fresh_dir.display()))?;
                    (fresh_dir, fresh_journal)
                } else {
                    (journal_dir, journal)
                };
                if let Err(err) = session.checkpoint_journal(&mut final_journal) {
                    abort_journal_lease_best_effort(final_journal);
                    return Err(format!("failed to seed initial recovery snapshot: {err}"));
                }
                return self.install_window_session(id, session, viewport, Some((final_dir, final_journal, None)));
            }
            None
        } else {
            None
        };
        self.install_window_session(id, session, viewport, recovery)
    }

    fn choose_window_journal_dir(&self, root: &Path, id: DesktopWindowId, has_doc: bool, now_epoch_ms: u64) -> Result<PathBuf, String> {
        let candidate = claim_isolated_journal_dir(root, self.config.new_instance, id, now_epoch_ms)?;
        let already_open = self.windows.values().any(|win| win.recovery_dir.as_deref().is_some_and(|p| paths_equal(p, &candidate)));
        let preserve_existing_crash = has_doc && dir_has_unrecovered_crash(&candidate, now_epoch_ms);
        let non_primary_active = (self.config.new_instance || id.0 > 1) && dir_has_active_lease(&candidate, now_epoch_ms);
        if already_open || preserve_existing_crash || non_primary_active {
            return allocate_fresh_isolated_journal_dir(root, id, now_epoch_ms);
        }
        Ok(candidate)
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
        if let Some(opener) = self.url_opener.as_ref() {
            app.set_url_opener(Arc::clone(opener));
        }
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
            LoadedPathPayload::Project { doc, archive_path, clean_archive, import_warnings, import_command_id } => {
                let record_dropped_warnings = |app: &mut UiApp, path: &Path| {
                    if let Some(cmd_id) = import_command_id {
                        let source_label =
                            path.file_name().and_then(|n| n.to_str()).map_or_else(|| path.to_string_lossy().into_owned(), str::to_owned);
                        let mut synthetic = CommandOutcome::mutated(cmd_id, "imported file", json!({ "warnings": &import_warnings }));
                        synthetic.warnings = import_warnings
                            .iter()
                            .map(|w| {
                                if w.source.trim().is_empty() {
                                    format!("[{:?}] {}", w.kind, w.message)
                                } else {
                                    format!("[{:?}] {}: {}", w.kind, w.source, w.message)
                                }
                            })
                            .collect();
                        app.record_import_warnings(cmd_id, &source_label, &synthetic);
                    }
                };
                let (open_new, viewport) = {
                    let win = self.window(window_id)?;
                    (self.config.new_instance || win.has_unsaved_changes(), win.viewport)
                };
                if open_new {
                    let mut session = Session::from_document(*doc);
                    if let Some(p) = archive_path {
                        session.set_file_path(Some(p), clean_archive);
                    } else if !session.is_dirty() {
                        let _ = session.collapse_atomic_group(Session::new());
                    }
                    let new_id = self.open_window_with_session(session, viewport, self.config.recovery_root.is_some())?;
                    if let Ok(new_win) = self.window_mut(new_id) {
                        record_dropped_warnings(new_win.app_mut(), &path);
                    }
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
                } else if !session.is_dirty() {
                    let _ = session.collapse_atomic_group(Session::new());
                }
                let inherited_url_opener = self.url_opener.clone();
                let cfg_path_policy = self.config.path_policy.clone();
                let cfg_human_path_only = self.config.human_path_only;
                let cfg_clipboard_adapter = self.config.clipboard_adapter.clone();
                let win = self.window_mut(window_id)?;
                win.app.replace_session_preserving_policies(session);
                if let Some(opener) = inherited_url_opener {
                    win.app.set_url_opener(opener);
                }
                if cfg_path_policy != curvebook_engine::ExternalPathPolicy::default() {
                    win.app.session_mut().set_path_policy(cfg_path_policy);
                }
                if cfg_human_path_only {
                    win.app.set_human_path_only(true);
                }
                if let Some(adapter) = cfg_clipboard_adapter
                    && !win.app.has_custom_clipboard_adapter()
                {
                    win.app.set_clipboard_adapter(adapter);
                }
                record_dropped_warnings(win.app_mut(), &path);
                win.dirty_commands.clear();
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
                "app.quit" => self.request_app_quit(Some(window_id), false),
                "edit.cut" => self.cut_selection(window_id),
                "edit.copy" => self.handle_clipboard_copy(window_id),
                "ui.clipboard.copyForSlides" => self.handle_clipboard_copy_for_slides(window_id, 300),
                "edit.paste" => self.handle_clipboard_paste(window_id, None),
                other => Err(format!("unhandled host action `{other}`")),
            },
            DesktopCommandRoute::UiCommand { ui_command_id, params } => {
                let has_hook = self.preferences_persist_hook.is_some();
                let win = self.window_mut(window_id)?;
                let prev_prefs = has_hook.then(|| capture_pref_fingerprint(&win.app));
                let prev_undo = win.app.session().undo_depth();
                let prev_redo = win.app.session().redo_depth();
                let prev_dirty = win.app.session().is_dirty();
                let prev_mutation_seq = win.app.mutation_seq();
                let ui_res = dispatch_ui_command(&mut win.app, &ui_command_id, &params)?
                    .ok_or_else(|| format!("UI command `{ui_command_id}` is not registered in dispatcher"))?;
                win.app.sync_views();
                win.record_ui_mutation_checkpoint(Some(trimmed), prev_undo, prev_redo, prev_dirty, prev_mutation_seq)?;
                let changed = prev_prefs.is_some_and(|before| before != capture_pref_fingerprint(&win.app));
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
                let prev_prefs = has_hook.then(|| capture_pref_fingerprint(&win.app));
                let outcome = win.execute_and_journal(&engine_command_id, params)?;
                if prev_prefs.is_some_and(|before| before != capture_pref_fingerprint(&win.app)) {
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

    /// Copy from the active inline editor, selected table cells, or active graph/layout into both
    /// the OS pasteboard and the host's fallback clipboard state.
    ///
    /// # Errors
    /// Returns an error if `window_id` is unknown or clipboard serialization fails.
    pub fn cut_selection(&mut self, window_id: DesktopWindowId) -> Result<DesktopEventOutcome, String> {
        let copy_out = self.handle_clipboard_copy(window_id)?;
        if !copy_out.handled {
            return Ok(copy_out);
        }
        let win = self.window_mut(window_id)?;
        if win.app.inline_editor.active {
            win.app.inline_editor.buffer.clear();
            win.app.inline_editor.caret_byte = 0;
        } else if let Some(sheet_id) = win.app.document().active_sheet_id.clone()
            && let Some(ds) = win.app.document().data_sheet(&sheet_id).cloned()
            && !ds.frozen
        {
            for params in win.app.grid.clipboard_target(&ds).clear_cells_requests(&ds) {
                let _ = win.execute_and_journal("table.clearCells", params)?;
            }
        }
        Ok(DesktopEventOutcome { action: copy_out.action.replacen("copied", "cut", 1), ..copy_out })
    }

    fn format_os_clipboard_write(base_action: &str, write_res: Result<OsAdapterStatus, String>) -> Result<(String, Value), String> {
        match write_res {
            Ok(status) => {
                let action = if status.available { base_action.to_owned() } else { format!("{base_action}_os_unavailable") };
                let payload = serde_json::to_value(&status).map_err(|err| format!("failed to serialize clipboard status: {err}"))?;
                Ok((action, json!({ "os_clipboard": payload })))
            }
            Err(err) => Ok((format!("{base_action}_os_failed"), json!({ "os_clipboard": { "available": false, "synced": false, "error": err } }))),
        }
    }

    fn read_paste_text_from_os_or_cache(
        &mut self,
        window_id: DesktopWindowId,
        inline_active: bool,
    ) -> Result<Result<(String, Option<Value>), DesktopEventOutcome>, String> {
        match self.clipboard_adapter.read_text() {
            Ok(status) if status.available => {
                let Some(os_text) = status.text.clone() else {
                    return Ok(Err(DesktopEventOutcome {
                        handled: false,
                        action: String::from("clipboard.paste_empty"),
                        window_id: Some(window_id),
                        command_result: Some(json!({ "os_clipboard": status })),
                        ..Default::default()
                    }));
                };
                let is_own_bundle_markup = !inline_active
                    && self
                        .clipboard_bundle
                        .as_ref()
                        .is_some_and(|b| (!b.svg.is_empty() && os_text == b.svg) || (!b.html.is_empty() && os_text == b.html));
                if is_own_bundle_markup {
                    return Ok(Err(DesktopEventOutcome {
                        handled: false,
                        action: String::from("clipboard.paste_bundle_not_tabular"),
                        window_id: Some(window_id),
                        ..Default::default()
                    }));
                }
                Ok(Ok((os_text, None)))
            }
            Ok(status) => {
                let status_val = serde_json::to_value(&status).ok();
                if let Some(cached) = self.clipboard_text.clone() {
                    return Ok(Ok((cached, status_val)));
                }
                let action = if self.clipboard_bundle.is_some() {
                    String::from("clipboard.paste_bundle_not_tabular")
                } else {
                    String::from("clipboard.paste_os_unavailable")
                };
                Ok(Err(DesktopEventOutcome {
                    handled: false,
                    action,
                    window_id: Some(window_id),
                    command_result: Some(json!({ "os_clipboard": status })),
                    ..Default::default()
                }))
            }
            Err(err) => {
                let err_val = json!({ "available": false, "synced": false, "error": err });
                if let Some(cached) = self.clipboard_text.clone() {
                    return Ok(Ok((cached, Some(err_val))));
                }
                let action = if self.clipboard_bundle.is_some() {
                    String::from("clipboard.paste_bundle_not_tabular")
                } else {
                    String::from("clipboard.paste_os_failed")
                };
                Ok(Err(DesktopEventOutcome {
                    handled: false,
                    action,
                    window_id: Some(window_id),
                    command_result: Some(json!({ "os_clipboard": err_val })),
                    ..Default::default()
                }))
            }
        }
    }

    fn handle_clipboard_copy(&mut self, window_id: DesktopWindowId) -> Result<DesktopEventOutcome, String> {
        enum CopyTarget {
            InlineText(String),
            TableTsv(String),
            Bundle(Box<ClipboardBundle>),
            NothingSelected,
        }

        let target = {
            let win = self.window_mut(window_id)?;
            if win.app.inline_editor.active {
                CopyTarget::InlineText(win.app.inline_editor.buffer.clone())
            } else {
                let doc = win.app.document().clone();
                let active_id = doc.active_sheet_id.as_ref();
                if let Some(sheet_id) = active_id
                    && let Some(ds) = doc.data_sheet(sheet_id)
                {
                    let dec = win.app.locale_preset.decimal_separator;
                    let plan = win.app.grid.prepare_clipboard_selection(ds, dec)?;
                    CopyTarget::TableTsv(plan.tsv)
                } else if active_id.is_some_and(|id| doc.result_sheet(id).is_some()) {
                    if let Some(tsv) = win.app.copy_active_result_selection_tsv() { CopyTarget::TableTsv(tsv) } else { CopyTarget::NothingSelected }
                } else if let Some(sheet_id) = active_id
                    && matches!(doc.sheet_section(sheet_id), Some(SheetSection::Graphs | SheetSection::Layouts))
                {
                    let dpi = win.viewport.dpi_for_base(BASE_LOGICAL_DPI);
                    CopyTarget::Bundle(Box::new(ClipboardBundle::from_sheet(win.app.session(), sheet_id, dpi)?))
                } else {
                    CopyTarget::NothingSelected
                }
            }
        };

        let (copied_text, copied_bundle, base_action, write_res) = match target {
            CopyTarget::InlineText(text) => {
                let res = self.clipboard_adapter.write_text(&text);
                (Some(text), None, "clipboard.copied_inline_text", res)
            }
            CopyTarget::TableTsv(tsv) => {
                let res = self.clipboard_adapter.write_text(&tsv);
                (Some(tsv), None, "clipboard.copied_tsv", res)
            }
            CopyTarget::Bundle(bundle) => {
                let res = self.clipboard_adapter.write_bundle(&bundle);
                (None, Some(*bundle), "clipboard.copied_bundle", res)
            }
            CopyTarget::NothingSelected => {
                return Ok(DesktopEventOutcome {
                    handled: false,
                    action: String::from("clipboard.copy_nothing_selected"),
                    window_id: Some(window_id),
                    ..Default::default()
                });
            }
        };

        self.clipboard_text = copied_text;
        self.clipboard_bundle = copied_bundle;
        let (action, result_json) = Self::format_os_clipboard_write(base_action, write_res)?;
        Ok(DesktopEventOutcome { handled: true, action, window_id: Some(window_id), command_result: Some(result_json), ..Default::default() })
    }

    /// Copy the active (or first renderable) Graph/Layout sheet at `dpi` (default 300 DPI)
    /// using the slide/document pasteboard profile (no raw XML plain-text fallback).
    ///
    /// # Errors
    /// Returns an error if the window does not exist or vector/raster clipboard generation fails.
    pub fn handle_clipboard_copy_for_slides(&mut self, window_id: DesktopWindowId, dpi: u16) -> Result<DesktopEventOutcome, String> {
        let effective_dpi = dpi.max(72);
        let sheet_id = {
            let win = self.window(window_id)?;
            let doc = win.app.document();
            doc.active_sheet_id
                .as_ref()
                .filter(|id| matches!(doc.sheet_section(id), Some(SheetSection::Graphs | SheetSection::Layouts)))
                .cloned()
                .or_else(|| resolve_renderable_sheet_id(doc))
        };
        let Some(sheet_id) = sheet_id else {
            return Ok(DesktopEventOutcome {
                handled: false,
                action: String::from("clipboard.copy_nothing_selected"),
                window_id: Some(window_id),
                ..Default::default()
            });
        };

        let bundle = {
            let win = self.window(window_id)?;
            ClipboardBundle::from_sheet(win.app.session(), &sheet_id, effective_dpi)?
        };
        let flavors = bundle.available_flavors();
        let pdf_bytes = bundle.pdf_bytes.len();
        let svg_bytes = bundle.svg.len();
        let png_bytes = bundle.png_bytes.len();
        let tiff_bytes = bundle.tiff_bytes.as_ref().map_or(0, Vec::len);
        let emf_bytes = bundle.emf_bytes.as_ref().map_or(0, Vec::len);
        let width_px = bundle.width_px;
        let height_px = bundle.height_px;
        let write_res = self.clipboard_adapter.write_slide_bundle(&bundle);

        self.clipboard_text = None;
        self.clipboard_bundle = Some(bundle);
        let (action, mut result_json) = Self::format_os_clipboard_write("clipboard.copied_slide_bundle", write_res)?;
        if let Some(obj) = result_json.as_object_mut() {
            obj.insert(String::from("sheet_id"), json!(sheet_id.as_str()));
            obj.insert(String::from("dpi"), json!(effective_dpi));
            obj.insert(String::from("width_px"), json!(width_px));
            obj.insert(String::from("height_px"), json!(height_px));
            obj.insert(String::from("flavors"), json!(flavors));
            obj.insert(String::from("pdf_bytes"), json!(pdf_bytes));
            obj.insert(String::from("svg_bytes"), json!(svg_bytes));
            obj.insert(String::from("png_bytes"), json!(png_bytes));
            obj.insert(String::from("tiff_bytes"), json!(tiff_bytes));
            obj.insert(String::from("emf_bytes"), json!(emf_bytes));
            obj.insert(String::from("profile"), json!("slides_and_documents"));
            obj.insert(String::from("omits_plain_text_svg"), json!(true));
        }
        Ok(DesktopEventOutcome {
            handled: true,
            action,
            window_id: Some(window_id),
            command_id: Some(String::from("ui.clipboard.copyForSlides")),
            command_result: Some(result_json),
            ..Default::default()
        })
    }

    fn attach_os_clipboard_note(command_result: &mut Option<Value>, note: Value) {
        match command_result {
            Some(Value::Object(map)) => {
                map.insert(String::from("os_clipboard"), note);
            }
            _ => {
                *command_result = Some(json!({ "os_clipboard": note }));
            }
        }
    }

    fn handle_clipboard_paste(&mut self, window_id: DesktopWindowId, text_override: Option<String>) -> Result<DesktopEventOutcome, String> {
        let inline_active = self.window(window_id)?.app.inline_editor.active;
        let (paste_text, os_note) = match text_override {
            Some(explicit) => (explicit, None),
            None => match self.read_paste_text_from_os_or_cache(window_id, inline_active)? {
                Ok(pair) => pair,
                Err(outcome) => return Ok(outcome),
            },
        };

        if inline_active {
            let win = self.window_mut(window_id)?;
            win.app.inline_editor.insert_text(&paste_text);
            if matches!(win.app.inline_editor.target, Some(InlineEditorTarget::GridCell { .. } | InlineEditorTarget::GridXCell { .. })) {
                win.app.grid.tool.editing_buffer = Some(win.app.inline_editor.buffer.clone());
            }
            let mut command_result = Some(json!({ "text": win.app.inline_editor.buffer }));
            if let Some(note) = os_note {
                Self::attach_os_clipboard_note(&mut command_result, note);
            }
            return Ok(DesktopEventOutcome {
                handled: true,
                action: String::from("inline_edit.text_input"),
                window_id: Some(window_id),
                command_result,
                ..Default::default()
            });
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
        let dec_char = win.app.locale_preset.decimal_separator;
        let clean_paste = paste_text.strip_prefix('\u{FEFF}').unwrap_or(&paste_text).replace('\0', "");
        let mut params = json!({
            "sheet_id": sheet_id.as_str(),
            "text": clean_paste,
            "decimal_separator": if dec_char == ',' { "," } else { "auto" },
            "anchor": anchor,
        });
        if dec_char == ','
            && !clean_paste.contains(';')
            && (clean_paste.contains('\t') || clean_paste.lines().all(|line| line.matches(',').count() <= 1))
            && let Some(obj) = params.as_object_mut()
        {
            obj.insert(String::from("delimiter"), json!("\t"));
        }
        let outcome = win.execute_and_journal("table.paste", params)?;
        let mut command_result = Some(outcome.payload);
        if let Some(note) = os_note {
            Self::attach_os_clipboard_note(&mut command_result, note);
        }
        Ok(DesktopEventOutcome {
            handled: true,
            action: String::from("clipboard.pasted_table"),
            window_id: Some(window_id),
            command_id: Some(String::from("table.paste")),
            command_result,
            ..Default::default()
        })
    }

    fn handle_animation_tick(&mut self, dt_sec: f64) -> Result<DesktopEventOutcome, String> {
        for win in self.windows.values_mut() {
            if let Some(journal) = &mut win.recovery_journal {
                maybe_renew_journal_lease_periodically(journal);
            }
        }
        let _ = dt_sec;
        Ok(DesktopEventOutcome { handled: false, action: String::from("animation.idle"), ..Default::default() })
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
    ImportWizardXlsx,
    VariablesDialog,
    EditAnalysisParameters,
    ChangeDataAnalyzed,
    ApplyStyleFrom,
    InfoNotesEditor,
    InfoConstantHookPicker,
    Checkpoints,
    CommandPalette,
    Preferences,
}

/// Resolved desktop command dispatch plan produced by [`resolve_desktop_command_route`].
#[derive(Debug, Clone, PartialEq)]
pub enum DesktopCommandRoute {
    /// Window lifecycle or host clipboard command handled directly by [`DesktopHostRuntime`].
    HostAction { action: &'static str },
    /// UI command executed through [`curvebook_ui_egui::dispatch_ui_command`].
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

fn resolve_host_or_ui_route(
    app: &UiApp,
    doc: &ProjectDocument,
    base_cmd: &str,
    variant: Option<&str>,
) -> Result<Option<DesktopCommandRoute>, String> {
    match base_cmd {
        "window.new" | "file.newWindow" => Ok(Some(DesktopCommandRoute::HostAction { action: "window.new" })),
        "window.close" | "file.close" => Ok(Some(DesktopCommandRoute::HostAction { action: "window.close" })),
        "file.quit" | "app.quit" => Ok(Some(DesktopCommandRoute::HostAction { action: "app.quit" })),
        "edit.cut" => Ok(Some(DesktopCommandRoute::HostAction { action: "edit.cut" })),
        "edit.copy" => Ok(Some(DesktopCommandRoute::HostAction { action: "edit.copy" })),
        "ui.clipboard.copyForSlides" => Ok(Some(DesktopCommandRoute::HostAction { action: "ui.clipboard.copyForSlides" })),
        "edit.paste" => Ok(Some(DesktopCommandRoute::HostAction { action: "edit.paste" })),
        "toolbar.axes" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.graph.axes.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "file.print" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.print.open"),
            params: sheet_id_obj_or_empty(resolve_any_sheet_id(doc)),
        })),
        "ui.export.allTablesCsv" => {
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.export.allTablesCsv.open"), params: json!({}) }))
        }
        "ui.theme.set" => {
            let mode = variant.unwrap_or_else(|| next_theme_mode_name(app.theme.mode));
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.theme.set"), params: json!({ "mode": mode }) }))
        }
        "ui.previewFilter.set" => {
            let filter = variant.unwrap_or_else(|| next_preview_filter_name(app.preview_filter));
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.previewFilter.set"), params: json!({ "filter": filter }) }))
        }
        "ui.workspacePreset.set" => {
            let preset = variant.unwrap_or("default");
            Ok(Some(DesktopCommandRoute::UiCommand { ui_command_id: String::from("ui.workspacePreset.set"), params: json!({ "preset": preset }) }))
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
        "app.preferences" => {
            Ok(Some(DesktopCommandRoute::UiDialogOpener { kind: DesktopDialogOpener::Preferences, target_sheet_id: None, query: None }))
        }
        cmd if cmd.starts_with("ui.") || cmd == "app.about" || cmd == "app.feedback" => {
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
        "edit.undo" | "edit.redo" | "file.new" | "project.new" | "project.inspect" | "project.recalculate" | "layout.create" => {
            Ok(Some(DesktopCommandRoute::EngineCommand { engine_command_id: base_cmd.to_owned(), params: json!({}) }))
        }
        "project.open" | "file.open" | "file.open_unpacked_dir" => {
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
        "project.createCheckpoint" | "project.listCheckpoints" | "project.restoreCheckpoint" | "project.deleteCheckpoint" => {
            Ok(Some(DesktopCommandRoute::UiDialogOpener { kind: DesktopDialogOpener::Checkpoints, target_sheet_id: None, query: None }))
        }
        "sheet.rename" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
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
            target_sheet_id: active_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.pasteLink" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ImportWizardPasteLink,
            target_sheet_id: active_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.importXlsx" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ImportWizardXlsx,
            target_sheet_id: active_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: None,
        })),
        "table.importCsv" | "table.importPrism" | "table.importPzfx" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::ImportWizardExternalFile,
            target_sheet_id: active_data_sheet_id(doc).map(|id| id.as_str().to_owned()),
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

fn first_editable_col_idx(ds: &curvebook_doc::DataSheet) -> usize {
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
            if let Some(first_req) = app.grid.clipboard_target(ds).clear_cells_requests(ds).into_iter().next() {
                Ok(first_req)
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
        "edit.pasteResultTable" => {
            let src = resolve_result_sheet_id(doc).ok_or_else(|| String::from("no result sheet available for `edit.pasteResultTable`"))?;
            let target =
                resolve_renderable_sheet_id(doc).ok_or_else(|| String::from("no graph or layout sheet available for `edit.pasteResultTable`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("edit.pasteResultTable"),
                params: json!({ "source_sheet_id": src.as_str(), "target_sheet_id": target.as_str() }),
            }))
        }
        "edit.pasteImage" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: resolve_renderable_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: Some(String::from("edit.pasteImage")),
        })),
        "graph.insertImage" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: resolve_graph_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: Some(String::from("graph.insertImage")),
        })),
        "graph.updateImage" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.image.format.open"),
            params: sheet_id_obj_or_empty(resolve_graph_sheet_id(doc)),
        })),
        "layout.insertImage" => Ok(Some(DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: resolve_layout_sheet_id(doc).map(|id| id.as_str().to_owned()),
            query: Some(String::from("layout.insertImage")),
        })),
        "layout.updateImage" => Ok(Some(DesktopCommandRoute::UiCommand {
            ui_command_id: String::from("ui.image.format.open"),
            params: sheet_id_obj_or_empty(resolve_layout_sheet_id(doc)),
        })),
        "layout.equalizeScaling" => {
            let id = resolve_layout_sheet_id(doc).ok_or_else(|| String::from("no layout sheet available for `layout.equalizeScaling`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("layout.equalizeScaling"),
                params: json!({ "sheet_id": id.as_str(), "mode": "match_smallest" }),
            }))
        }
        "layout.reorderPanel" => {
            let id = resolve_layout_sheet_id(doc).ok_or_else(|| String::from("no layout sheet available for `layout.reorderPanel`"))?;
            Ok(Some(DesktopCommandRoute::EngineCommand {
                engine_command_id: String::from("layout.reorderPanel"),
                params: json!({ "sheet_id": id.as_str(), "panel_index": 0, "action": "bring_to_front" }),
            }))
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
            app.grid.import_wizard.target_sheet_id = parsed_target.or_else(|| active_data_sheet_id(&doc));
            Ok(json!({ "dialog": "ImportPasteWizard", "is_live_link": false }))
        }
        DesktopDialogOpener::ImportWizardPasteLink => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.is_live_link = true;
            app.grid.import_wizard.target_sheet_id = parsed_target.or_else(|| active_data_sheet_id(&doc));
            Ok(json!({ "dialog": "ImportPasteWizard", "is_live_link": true }))
        }
        DesktopDialogOpener::ImportWizardExternalFile => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.is_live_link = false;
            app.grid.import_wizard.origin_kind = ImportOriginKind::ExternalDelimitedFile;
            app.grid.import_wizard.target_sheet_id = parsed_target.or_else(|| active_data_sheet_id(&doc));
            Ok(json!({ "dialog": "ImportPasteWizard", "origin_kind": "ExternalDelimitedFile" }))
        }
        DesktopDialogOpener::ImportWizardXlsx => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.is_live_link = false;
            app.grid.import_wizard.origin_kind = ImportOriginKind::XlsxWorkbook;
            app.grid.import_wizard.target_sheet_id = parsed_target.or_else(|| active_data_sheet_id(&doc));
            Ok(json!({ "dialog": "ImportPasteWizard", "origin_kind": "XlsxWorkbook" }))
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
        DesktopDialogOpener::Checkpoints => {
            app.canvas_dialogs.open_checkpoints(&doc);
            Ok(json!({ "dialog": "Checkpoints", "count": doc.checkpoints.len() }))
        }
        DesktopDialogOpener::CommandPalette => {
            app.canvas_dialogs.open_command_palette();
            if let Some(q) = query {
                app.canvas_dialogs.command_palette.query = q.to_owned();
            }
            Ok(json!({ "dialog": "CommandPalette", "query": app.canvas_dialogs.command_palette.query }))
        }
        DesktopDialogOpener::Preferences => {
            let shortcuts = app.shortcuts.clone();
            app.egui_views.app_modals.open_preferences(&shortcuts, None);
            Ok(json!({ "dialog": "Preferences" }))
        }
    }
}

fn shortcut_uses_desktop_routing(command_id: &str) -> bool {
    if is_desktop_host_routed_command(command_id)
        || matches!(
            command_id,
            "sheet.createData"
                | "ui.analyze.open"
                | "ui.editParams.open"
                | "ui.print.open"
                | "ui.changeGraphType.open"
                | "ui.zoom.fitSheet"
                | "app.preferences"
                | "app.quit"
                | "file.quit"
                | "ui.theme.set"
                | "ui.locale.set"
                | "ui.previewFilter.set"
                | "ui.workspacePreset.set"
                | "ui.canvas.applyPreset"
                | "ui.shortcut.set"
                | "ui.result.applyRemediation"
                | "ui.event.dispatch"
        )
    {
        return true;
    }
    engine_cmd::registry()
        .get(command_id)
        .is_some_and(|spec| (spec.input_schema)().get("required").and_then(Value::as_array).is_some_and(|arr| !arr.is_empty()))
}

/// Match a customized key combination in `app.shortcuts` to a desktop-routed command ID.
#[must_use]
pub fn match_routed_desktop_shortcut(app: &UiApp, key: &str, modifiers: KeyModifiers) -> Option<String> {
    if (modifiers.meta || modifiers.ctrl) && !modifiers.alt && !modifiers.shift && key.eq_ignore_ascii_case("v") {
        // Let clipboard paste shortcut use host pasteboard instead of opening Paste Special wizard.
        return None;
    }
    let on_result_sheet = app.is_result_sheet_shortcut_context();
    for candidate in build_shortcut_candidates(key, modifiers) {
        if let Some(matched) = app.shortcuts.handle_key_combo_for_context(&candidate, on_result_sheet)
            && shortcut_uses_desktop_routing(&matched.command_id)
        {
            return Some(matched.command_id);
        }
    }
    None
}

#[cfg(test)]
mod wave_b_host_tests {
    use super::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime};
    use curvebook_doc::{Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, ProjectDocument, SheetId, TableArchetype};
    use curvebook_engine::Session;
    use curvebook_ui_egui::interaction::ViewportMetrics;
    use serde_json::json;

    fn cell_number(ds: &DataSheet, row: usize, col: usize) -> Option<f64> {
        match ds.cell(CellCoord::new(row, col, 0))?.value {
            CellValue::Number(v) => Some(v),
            _ => None,
        }
    }

    #[test]
    fn desktop_host_large_2x5000_copy_and_cut_do_not_truncate_at_4096_cells() -> Result<(), String> {
        let rows = 5_000usize;
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, rows).map_err(|e| e.to_string())?;
        for r in 0..rows {
            col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
            col.data_sets[1].cells[r][0] = Cell::number((10_000 + r + 1) as f64);
        }
        let mut doc = ProjectDocument::new("Host Large Clipboard").map_err(|e| e.to_string())?;
        let ds = DataSheet::new("ds-large", "Large", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.active_sheet_id = Some(SheetId::new("ds-large"));

        let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
        let wid = rt.open_window_with_session(Session::from_document(doc), ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
        {
            let win = rt.window_mut(wid)?;
            let ds = win.app.document().data_sheet(&SheetId::new("ds-large")).ok_or("missing ds-large")?.clone();
            win.app.grid.select_single_sync(CellCoord::new(0, 0, 0).into());
            win.app.grid.extend_shift_range(&ds, CellCoord::new(rows - 1, 1, 0).into());
        }

        let out = rt.cut_selection(wid)?;
        assert!(out.handled);
        let tsv = rt.clipboard_text.as_deref().ok_or("missing host clipboard_text")?;
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), rows);
        assert_eq!(lines.first().copied(), Some("1\t10001"));
        assert_eq!(lines.last().copied(), Some("5000\t15000"));

        let ds = rt.window(wid)?.app.document().data_sheet(&SheetId::new("ds-large")).ok_or("missing ds-large")?;
        assert!(ds.cell(CellCoord::new(rows - 1, 1, 0)).is_some_and(Cell::is_empty));
        Ok(())
    }

    #[test]
    fn desktop_host_discontiguous_cut_leaves_unselected_interior_cells_intact() -> Result<(), String> {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).map_err(|e| e.to_string())?;
        for r in 0..6 {
            for c in 0..3 {
                col.data_sets[c].cells[r][0] = Cell::number(((r + 1) * 10 + (c + 1)) as f64);
            }
        }
        let mut doc = ProjectDocument::new("Host Discontiguous Cut").map_err(|e| e.to_string())?;
        let ds = DataSheet::new("ds-1", "Table", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.active_sheet_id = Some(SheetId::new("ds-1"));

        let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
        let wid = rt.open_window_with_session(Session::from_document(doc), ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
        {
            let win = rt.window_mut(wid)?;
            let ds = win.app.document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?.clone();
            win.app.grid.select_single_sync(CellCoord::new(0, 0, 0).into());
            win.app.grid.toggle_cmd_selection(&ds, CellCoord::new(4, 2, 0).into());
        }

        let out = rt.cut_selection(wid)?;
        assert!(out.handled);
        let tsv = rt.clipboard_text.as_deref().ok_or("missing host clipboard_text")?;
        assert!(tsv.contains("11") && tsv.contains("53") && !tsv.contains("22"), "unexpected TSV: {tsv:?}");

        let ds = rt.window(wid)?.app.document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
        assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty));
        assert!(ds.cell(CellCoord::new(4, 2, 0)).is_some_and(Cell::is_empty));
        assert_eq!(cell_number(ds, 1, 1), Some(22.0));
        Ok(())
    }

    #[test]
    fn desktop_host_paste_link_opener_sets_target_sheet_id_without_clobbering_source_sheet_id() -> Result<(), String> {
        let mut doc = ProjectDocument::new("Host Import Opener").map_err(|e| e.to_string())?;
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).map_err(|e| e.to_string())?;
        let ds_a = DataSheet::new("ds-a", "A", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        let graph = GraphSheet::new_uninitialized_for_data("g-1", &ds_a).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds_a).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(graph).map_err(|e| e.to_string())?;
        doc.active_sheet_id = Some(SheetId::new("ds-a"));

        let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
        let wid = rt.open_window_with_session(Session::from_document(doc), ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
        let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("table.pasteLink") })?;
        let wiz = &rt.window(wid)?.app.grid.import_wizard;
        assert!(wiz.open && wiz.is_live_link);
        assert_eq!(wiz.target_sheet_id, Some(SheetId::new("ds-a")));
        assert_eq!(wiz.source_sheet_id, None, "desktop opener must not overwrite source_sheet_id with target");

        // When a Graph is active, opening Paste Link must leave target_sheet_id = None (never defaulting to source A).
        rt.window_mut(wid)?.app.execute_command("sheet.select", json!({ "sheet_id": "g-1" })).map_err(|e| e.to_string())?;
        let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("table.pasteLink") })?;
        let wiz2 = &rt.window(wid)?.app.grid.import_wizard;
        assert_eq!(wiz2.target_sheet_id, None, "graph-active opener must not silently target primary source sheet");
        Ok(())
    }

    #[test]
    fn desktop_host_menu_cut_and_result_sheet_copy_work_cleanly() -> Result<(), String> {
        let mut doc = ProjectDocument::new("Host Cut & Result Copy").map_err(|e| e.to_string())?;
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
        col.data_sets[0].cells[0][0] = Cell::number(10.0);
        col.data_sets[1].cells[0][0] = Cell::number(20.0);
        col.data_sets[0].cells[1][0] = Cell::number(30.0);
        col.data_sets[1].cells[1][0] = Cell::number(40.0);
        let ds = DataSheet::new("ds-1", "Table", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.active_sheet_id = Some(SheetId::new("ds-1"));

        let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
        let wid = rt.open_window_with_session(Session::from_document(doc), ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
        rt.window_mut(wid)?.app.grid.select_single_sync(CellCoord::new(0, 0, 0).into());

        // `edit.cut` via host menu action copies and clears cell.
        let cut_out = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("edit.cut") })?;
        assert!(cut_out.handled, "edit.cut host menu action should handle DataSheet selection");
        assert!(rt.clipboard_text.as_deref().is_some_and(|t| t.contains("10")));
        let ds_after = rt.window(wid)?.app.document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
        assert!(ds_after.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty));

        // Paste back and run analysis, then test `edit.copy` on ResultSheet.
        let _ = rt.window_mut(wid)?.app.execute_command("table.setCell", json!({ "sheet_id": "ds-1", "row": 0, "col": 0, "value": 10.0 }));
        let res = rt
            .window_mut(wid)?
            .app
            .execute_command(
                "analysis.run",
                json!({
                    "source_sheet_id": "ds-1",
                    "kind": {
                        "DescriptiveStats": {
                            "aggregation": "PerSubcolumn",
                            "custom_upper_percentile": null,
                            "run_normality_battery": false,
                            "hypothetical_value": null
                        }
                    }
                }),
            )
            .map_err(|e| e.to_string())?;
        let rs_id = res.payload["sheet_id"].as_str().ok_or("missing result sheet_id")?.to_owned();
        let _ = rt.window_mut(wid)?.app.execute_command("sheet.select", json!({ "sheet_id": rs_id }));

        rt.clipboard_text = None;
        let copy_out = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("edit.copy") })?;
        assert!(copy_out.handled, "edit.copy on ResultSheet should copy active tab TSV");
        assert!(rt.clipboard_text.as_deref().is_some_and(|t| !t.trim().is_empty()), "expected non-empty ResultSheet TSV");
        Ok(())
    }
}
