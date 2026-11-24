//! Minimal strict-CSP web chrome placeholder (`R10.7` / `R15.5`): command/shortcut routing,
//! modal commits, context menu projection, and semantic placeholder nodes until post-Beta `R11.11`.
#![forbid(unsafe_code)]

use curvebook_engine::cmd;
pub use curvebook_ui_egui::control_io::host_routing::contextual_command_params;
use curvebook_ui_egui::control_io::host_routing::{
    self, dialog_ui_command_for_engine_cmd, first_data_sheet_id as resolve_first_data_sheet_id, has_non_null_path_param, is_read_only_control_method,
    is_read_only_ui_command, resolve_renderable_sheet_id, resolve_result_sheet_id,
};
use curvebook_ui_egui::menus::normalize_shortcut_combo;
use curvebook_ui_egui::{
    ActiveDialog, KeyModifiers, PointerButton, UiApp, UiEventOutcome, UiInputEvent, ViewportMetrics, dispatch_ui_command, menu_bar_tooltips,
    menus::toolbar_tooltips,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::text_bridge::{LayoutRectPx, ZLayer};

/// Result of exporting a file from the web workspace for FS Access or Blob download fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebExportFormat {
    CurveBookArchive,
    CurveBookDirectory,
    Csv,
    Svg,
    Pdf,
    Png,
    Tiff,
}

impl WebExportFormat {
    /// Canonical wire token for JSON bridge payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CurveBookArchive => "curvebook",
            Self::CurveBookDirectory => "curvebook.d",
            Self::Csv => "csv",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Tiff => "tiff",
        }
    }

    /// Parse a wire format token (case-insensitive).
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "curvebook" | "archive" => Some(Self::CurveBookArchive),
            "curvebook.d" | "directory" | "dir" => Some(Self::CurveBookDirectory),
            "csv" => Some(Self::Csv),
            "svg" => Some(Self::Svg),
            "pdf" => Some(Self::Pdf),
            "png" => Some(Self::Png),
            "tiff" | "tif" => Some(Self::Tiff),
            _ => None,
        }
    }

    /// Default MIME content type for Blob downloads.
    #[must_use]
    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::CurveBookArchive | Self::CurveBookDirectory => "application/octet-stream",
            Self::Csv => "text/csv;charset=utf-8",
            Self::Svg => "image/svg+xml;charset=utf-8",
            Self::Pdf => "application/pdf",
            Self::Png => "image/png",
            Self::Tiff => "image/tiff",
        }
    }

    /// Default file extension (without leading dot).
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::CurveBookArchive => "curvebook",
            Self::CurveBookDirectory => "curvebook.d",
            Self::Csv => "csv",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Tiff => "tiff",
        }
    }
}

/// Severity level for status/error notices surfaced in the browser shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebNoticeLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl WebNoticeLevel {
    /// Wire identifier string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Parse a wire notice level string.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "success" => Some(Self::Success),
            "warning" | "warn" => Some(Self::Warning),
            "error" | "err" => Some(Self::Error),
            _ => None,
        }
    }
}

/// Documented Web shortcut mapping avoiding browser-reserved tab/window/omnibox clashes (`R10.7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebShortcutMapping {
    pub command_id: &'static str,
    pub desktop_combo: &'static str,
    pub web_combo: &'static str,
    pub browser_reserved: bool,
    pub prevent_default_on_canvas: bool,
    pub reservation_reason: Option<&'static str>,
    pub menu_fallback_path: &'static str,
}

/// Complete Web shortcut policy table mapping desktop combinations to conflict-free browser keys.
pub const WEB_SHORTCUT_MAPPINGS: &[WebShortcutMapping] = &[
    WebShortcutMapping {
        command_id: "file.close",
        desktop_combo: "Cmd+W",
        web_combo: "Alt+W",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browser tabs reserve Cmd/Ctrl+W to close the tab"),
        menu_fallback_path: "File/Close Project",
    },
    WebShortcutMapping {
        command_id: "sheet.createData",
        desktop_combo: "Cmd+T",
        web_combo: "Alt+T",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browser tabs reserve Cmd/Ctrl+T to open a new browser tab"),
        menu_fallback_path: "Insert/New Data Table",
    },
    WebShortcutMapping {
        command_id: "file.new",
        desktop_combo: "Cmd+N",
        web_combo: "Alt+Shift+N",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+N to open a new browser window"),
        menu_fallback_path: "File/New Project",
    },
    WebShortcutMapping {
        command_id: "project.new",
        desktop_combo: "Cmd+N",
        web_combo: "Alt+Shift+N",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+N to open a new browser window"),
        menu_fallback_path: "File/New Project",
    },
    WebShortcutMapping {
        command_id: "ui.navigator.toggle",
        desktop_combo: "Cmd+Shift+N",
        web_combo: "Alt+Shift+B",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+Shift+N for private windows"),
        menu_fallback_path: "View/Toggle Project Navigator",
    },
    WebShortcutMapping {
        command_id: "ui.inspector.toggle",
        desktop_combo: "Cmd+Shift+I",
        web_combo: "Alt+Shift+I",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+Shift+I for Developer Tools"),
        menu_fallback_path: "View/Toggle Format Inspector",
    },
    WebShortcutMapping {
        command_id: "sheet.freeze",
        desktop_combo: "Ctrl+L",
        web_combo: "Alt+L",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+L to focus the location bar"),
        menu_fallback_path: "Change/Freeze Sheet",
    },
    WebShortcutMapping {
        command_id: "ui.palette.open",
        desktop_combo: "Cmd+K",
        web_combo: "Alt+K",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Some browsers bind Cmd/Ctrl+K to the search bar; Alt+K always opens Command Palette"),
        menu_fallback_path: "View/Open Command Palette",
    },
    WebShortcutMapping {
        command_id: "ui.sheet.find",
        desktop_combo: "Cmd+F",
        web_combo: "Alt+F",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers bind Cmd/Ctrl+F to browser page find; Alt+F always opens Find in Sheet"),
        menu_fallback_path: "Edit/Find in Sheet...",
    },
    WebShortcutMapping {
        command_id: "ui.zoom.fitSheet",
        desktop_combo: "Cmd+0",
        web_combo: "Alt+0",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+0 to reset browser page zoom"),
        menu_fallback_path: "View/Zoom to Fit Sheet",
    },
    WebShortcutMapping {
        command_id: "project.open",
        desktop_combo: "Cmd+O",
        web_combo: "Cmd+O",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "File/Open...",
    },
    WebShortcutMapping {
        command_id: "project.save",
        desktop_combo: "Cmd+S",
        web_combo: "Cmd+S",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "File/Save",
    },
    WebShortcutMapping {
        command_id: "ui.print.open",
        desktop_combo: "Cmd+P",
        web_combo: "Cmd+P",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "File/Print...",
    },
    WebShortcutMapping {
        command_id: "edit.undo",
        desktop_combo: "Cmd+Z",
        web_combo: "Cmd+Z",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Edit/Undo",
    },
    WebShortcutMapping {
        command_id: "edit.redo",
        desktop_combo: "Cmd+Shift+Z",
        web_combo: "Cmd+Shift+Z",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Edit/Redo",
    },
    WebShortcutMapping {
        command_id: "table.excludeCells",
        desktop_combo: "Cmd+E",
        web_combo: "Cmd+E",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Change/Exclude Values",
    },
    WebShortcutMapping {
        command_id: "ui.grid.excludeWithReason",
        desktop_combo: "Cmd+Shift+E",
        web_combo: "Cmd+Shift+E",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Edit/Exclude Selection with Auditable Reason...",
    },
    WebShortcutMapping {
        command_id: "ui.analyze.open",
        desktop_combo: "Cmd+Shift+A",
        web_combo: "Cmd+Shift+A",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Analyze/Analyze...",
    },
    WebShortcutMapping {
        command_id: "ui.changeGraphType.open",
        desktop_combo: "Cmd+Shift+G",
        web_combo: "Cmd+Shift+G",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Graph/Change Graph Type...",
    },
    WebShortcutMapping {
        command_id: "ui.family.toggle",
        desktop_combo: "Cmd+Shift+F",
        web_combo: "Cmd+Shift+F",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "View/Toggle Family Strip",
    },
    WebShortcutMapping {
        command_id: "ui.split.toggle",
        desktop_combo: "Cmd+\\",
        web_combo: "Cmd+\\",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "View/Toggle Split Data + Graph Workspace",
    },
    WebShortcutMapping {
        command_id: "ui.pane.cycle",
        desktop_combo: "F6",
        web_combo: "F6",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "View/Cycle Active Workspace Pane",
    },
    WebShortcutMapping {
        command_id: "analyze.run",
        desktop_combo: "Cmd+Enter",
        web_combo: "Cmd+Enter",
        browser_reserved: false,
        prevent_default_on_canvas: true,
        reservation_reason: None,
        menu_fallback_path: "Analyze/Run Analysis",
    },
];

/// Host-level side effect triggered by a web keyboard shortcut or chrome click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebShortcutAction {
    None,
    OpenFilePicker,
    SaveArchive,
    ExportFormat(WebExportFormat),
    PrintCurrentSheet,
    CopyClipboard,
    CloseWorkspacePrompt,
    OpenEditAnalysisParameters(String),
    ExecuteCommand { command_id: String, params: Value },
}

/// Resolution result produced by [`WebShortcutRouter::route_key_down`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebKeyRouteResult {
    pub handled: bool,
    pub prevent_default: bool,
    pub used_web_alias: bool,
    pub browser_reserved_note: Option<&'static str>,
    pub action: WebShortcutAction,
}

/// Pure-Rust browser shortcut router resolving desktop + conflict-free Web aliases (`R10.7`).
#[derive(Debug, Clone, Copy, Default)]
pub struct WebShortcutRouter;

impl WebShortcutRouter {
    /// Return the Web-safe shortcut string to display for `command_id` (falling back to `desktop`).
    #[must_use]
    pub fn display_shortcut<'a>(command_id: &str, desktop: &'a str) -> &'a str {
        WEB_SHORTCUT_MAPPINGS.iter().find(|m| m.command_id == command_id).map_or(desktop, |m| m.web_combo)
    }

    /// Look up the explicit Web shortcut policy entry for `command_id`, if any.
    #[must_use]
    pub fn policy_for_command(command_id: &str) -> Option<&'static WebShortcutMapping> {
        WEB_SHORTCUT_MAPPINGS.iter().find(|m| m.command_id == command_id)
    }

    /// Resolve a browser `keydown` combo against Web-safe aliases and contextual sheet state.
    #[must_use]
    pub fn route_key_down(app: &UiApp, key: &str, modifiers: KeyModifiers) -> WebKeyRouteResult {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return WebKeyRouteResult {
                handled: false,
                prevent_default: false,
                used_web_alias: false,
                browser_reserved_note: None,
                action: WebShortcutAction::None,
            };
        }

        // Normalize `Ctrl` -> `Cmd` equivalence for browser platforms (macOS uses Meta, Windows/Linux Ctrl).
        let primary_mod = modifiers.meta || modifiers.ctrl;

        // 1. Safe browser `Alt+...` aliases for reserved shortcuts (`Alt+W`, `Alt+T`, `Alt+K`, `Alt+Shift+N`, etc.)
        if modifiers.alt
            && !primary_mod
            && let Some(res) = Self::match_web_alt_alias(app, trimmed, modifiers.shift)
        {
            return res;
        }

        // 2. Interceptable primary-modifier combos on focused `<canvas>` (including `Cmd+T` / `Cmd+K` when delivered).
        if primary_mod
            && !modifiers.alt
            && let Some(res) = Self::match_primary_mod_combo(app, trimmed, modifiers.shift)
        {
            return res;
        }

        // 3. Function key pane cycling (`F6`, `Shift+F6`).
        if trimmed.eq_ignore_ascii_case("F6") && !primary_mod && !modifiers.alt {
            return WebKeyRouteResult {
                handled: true,
                prevent_default: true,
                used_web_alias: false,
                browser_reserved_note: None,
                action: WebShortcutAction::ExecuteCommand {
                    command_id: String::from("ui.pane.cycle"),
                    params: json!({ "reverse": modifiers.shift }),
                },
            };
        }

        // 4. Lookup any user-customized shortcut in `app.shortcuts`.
        for candidate in build_combo_candidates(trimmed, modifiers) {
            if let Some(matched) = app.shortcuts.handle_key_combo(&candidate) {
                let params = if matched.params.is_null() { contextual_command_params(app, &matched.command_id) } else { matched.params };
                return WebKeyRouteResult {
                    handled: true,
                    prevent_default: true,
                    used_web_alias: false,
                    browser_reserved_note: None,
                    action: WebShortcutAction::ExecuteCommand { command_id: matched.command_id, params },
                };
            }
        }

        WebKeyRouteResult {
            handled: false,
            prevent_default: false,
            used_web_alias: false,
            browser_reserved_note: None,
            action: WebShortcutAction::None,
        }
    }

    fn match_web_alt_alias(app: &UiApp, key: &str, shift: bool) -> Option<WebKeyRouteResult> {
        let lower = key.to_ascii_lowercase();
        let (cmd_id, note) = match (lower.as_str(), shift) {
            ("w", false) => {
                return Some(WebKeyRouteResult {
                    handled: true,
                    prevent_default: true,
                    used_web_alias: true,
                    browser_reserved_note: Some("Alt+W mapped from browser-reserved Cmd/Ctrl+W"),
                    action: WebShortcutAction::CloseWorkspacePrompt,
                });
            }
            ("t", false) => {
                if let Some(rs_id) = active_result_sheet_id(app) {
                    return Some(WebKeyRouteResult {
                        handled: true,
                        prevent_default: true,
                        used_web_alias: true,
                        browser_reserved_note: Some("Alt+T opens Edit Analysis Parameters on ResultSheet"),
                        action: WebShortcutAction::OpenEditAnalysisParameters(rs_id),
                    });
                }
                ("sheet.createData", "Alt+T mapped from browser-reserved Cmd/Ctrl+T")
            }
            ("n", true) => ("file.new", "Alt+Shift+N mapped from browser-reserved Cmd/Ctrl+N"),
            ("b", true) => ("ui.navigator.toggle", "Alt+Shift+B mapped from browser-reserved Cmd/Ctrl+Shift+N"),
            ("i", true) => ("ui.inspector.toggle", "Alt+Shift+I mapped from browser-reserved Cmd/Ctrl+Shift+I"),
            ("l", false) => ("sheet.freeze", "Alt+L mapped from browser-reserved Ctrl+L"),
            ("k", false) => ("ui.palette.open", "Alt+K opens Command Palette without browser address-bar clash"),
            ("f", false) => ("ui.sheet.find", "Alt+F opens Find in Sheet without browser page-find clash"),
            ("0", false) => ("ui.zoom.fitSheet", "Alt+0 mapped from browser-reserved Cmd/Ctrl+0"),
            ("e", false) => ("table.excludeCells", "Alt+E aliases Exclude Values"),
            ("e", true) => ("ui.grid.excludeWithReason", "Alt+Shift+E aliases Exclude With Reason"),
            _ => return None,
        };

        Some(WebKeyRouteResult {
            handled: true,
            prevent_default: true,
            used_web_alias: true,
            browser_reserved_note: Some(note),
            action: WebShortcutAction::ExecuteCommand { command_id: cmd_id.to_owned(), params: contextual_command_params(app, cmd_id) },
        })
    }

    fn match_primary_mod_combo(app: &UiApp, key: &str, shift: bool) -> Option<WebKeyRouteResult> {
        let lower = key.to_ascii_lowercase();
        let make_cmd =
            |cmd_id: &str| WebShortcutAction::ExecuteCommand { command_id: cmd_id.to_owned(), params: contextual_command_params(app, cmd_id) };
        let action = match (lower.as_str(), shift) {
            ("o", false) => WebShortcutAction::OpenFilePicker,
            ("s", false) => WebShortcutAction::SaveArchive,
            ("p", false) => WebShortcutAction::PrintCurrentSheet,
            ("c", false) => WebShortcutAction::CopyClipboard,
            ("k", false) => make_cmd("ui.palette.open"),
            ("f", false) => make_cmd("ui.sheet.find"),
            ("t", false) => {
                if let Some(rs_id) = active_result_sheet_id(app) {
                    WebShortcutAction::OpenEditAnalysisParameters(rs_id)
                } else {
                    make_cmd("sheet.createData")
                }
            }
            ("z", false) => make_cmd("edit.undo"),
            ("z", true) | ("y", false) => make_cmd("edit.redo"),
            ("e", false) => make_cmd("table.excludeCells"),
            ("e", true) => make_cmd("ui.grid.excludeWithReason"),
            ("a", true) => make_cmd("ui.analyze.open"),
            ("g", true) => make_cmd("ui.changeGraphType.open"),
            ("f", true) => make_cmd("ui.family.toggle"),
            ("0", false) => make_cmd("ui.zoom.fitSheet"),
            ("\\", false) => make_cmd("ui.split.toggle"),
            ("enter", false) => make_cmd("analyze.run"),
            _ => return None,
        };
        let note = match lower.as_str() {
            "t" => Some("Cmd/Ctrl+T handled on canvas; use Alt+T when browser reserves Ctrl/Cmd+T"),
            "k" => Some("Cmd/Ctrl+K prevented on canvas; Alt+K always available"),
            "f" => Some("Cmd/Ctrl+F prevented on canvas; Alt+F always available"),
            _ => None,
        };
        Some(WebKeyRouteResult { handled: true, prevent_default: true, used_web_alias: false, browser_reserved_note: note, action })
    }
}

fn active_result_sheet_id(app: &UiApp) -> Option<String> {
    let doc = app.document();
    doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()).map(|id| id.as_str().to_owned())
}

fn build_combo_candidates(key: &str, modifiers: KeyModifiers) -> Vec<String> {
    let mut out = Vec::with_capacity(2);
    let build = |use_cmd_for_ctrl: bool| -> String {
        let mut parts = Vec::with_capacity(5);
        if modifiers.meta || (use_cmd_for_ctrl && modifiers.ctrl) {
            parts.push("Cmd");
        }
        if modifiers.ctrl && !(use_cmd_for_ctrl && !modifiers.meta) {
            parts.push("Ctrl");
        }
        if modifiers.alt {
            parts.push("Alt");
        }
        if modifiers.shift {
            parts.push("Shift");
        }
        parts.push(key);
        normalize_shortcut_combo(&parts.join("+")).unwrap_or_else(|_| parts.join("+"))
    };
    out.push(build(false));
    if modifiers.ctrl && !modifiers.meta {
        out.push(build(true));
    }
    out
}

/// Interactive role of a web chrome widget node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebChromeRole {
    MenuBar,
    MenuSection,
    MenuItem,
    ToolbarButton,
    DropdownMenu,
    ModalBackdrop,
    ModalDialog,
    ModalCloseButton,
    ModalInput,
    ModalItem,
    ModalButton,
    ContextMenu,
    StatusBar,
    Tooltip,
}

/// Action executed when a [`WebChromeNode`] is clicked or activated by keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebChromeAction {
    None,
    ToggleMenuSection(String),
    InvokeCommand(String),
    ExportFormat(WebExportFormat),
    OpenFilePicker,
    OpenDirectoryPicker,
    SelectDialogItem(usize),
    DialogConfirm,
    DialogCancel,
    DialogStepBack,
    DialogStepNext,
    FocusModalInput,
    CloseModalBackdrop,
    DismissContextMenu,
}

/// Hit-testable chrome widget node sharing geometry with [`WebTextDrawOp`](crate::text_bridge::WebTextDrawOp)s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebChromeNode {
    pub id: String,
    pub role: WebChromeRole,
    pub z_layer: ZLayer,
    pub paint_order: u32,
    pub rect_px: LayoutRectPx,
    pub clip_px: LayoutRectPx,
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub tooltip: Option<String>,
    pub action: WebChromeAction,
}

fn default_active_dialog_none() -> String {
    String::from("none")
}

/// Accessible summary of current Web chrome state mirrored to `RenderRgbaFrame.chrome`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebChromeSnapshot {
    pub menu_bar_visible: bool,
    pub open_menu_key: Option<String>,
    pub context_menu_open: bool,
    #[serde(default = "default_active_dialog_none")]
    pub active_dialog: String,
    pub hovered_node_id: Option<String>,
    pub focused_node_id: Option<String>,
    pub ime_preedit: Option<String>,
    pub drag_active: bool,
    pub node_count: usize,
    pub menu_titles: Vec<String>,
    pub toolbar_ids: Vec<String>,
    pub status_summary: String,
    pub dirty: bool,
    pub a11y_tree: Option<Box<Value>>,
}

impl WebChromeSnapshot {
    /// Encode chrome state snapshot into JSON for `RenderRgbaFrame.chrome`.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Decode a [`WebChromeSnapshot`] from JSON, tolerating missing optional fields.
    #[must_use]
    pub fn from_json_value(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

/// Active right-click context menu target & origin in physical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebContextMenuState {
    pub target: String,
    pub anchor_px: [i32; 2],
}

/// Minimal strict-CSP placeholder chrome layout projected from semantic `UiTreeSnapshot`.
#[derive(Debug, Clone)]
pub struct WebChromeLayout {
    pub viewport: ViewportMetrics,
    pub nodes: Vec<WebChromeNode>,
    pub snapshot: WebChromeSnapshot,
}

impl WebChromeLayout {
    /// Hit-test physical pixels against projected [`WebChromeNode`] rectangles in top-most Z order.
    #[must_use]
    pub fn hit_test_physical(&self, px: i32, py: i32) -> Option<&WebChromeNode> {
        self.nodes.iter().filter(|n| n.rect_px.contains(px, py)).max_by_key(|n| (n.z_layer, n.paint_order))
    }

    /// Hit-test logical coordinates against projected [`WebChromeNode`] rectangles.
    #[must_use]
    pub fn hit_test_logical(&self, lx: f64, ly: f64) -> Option<&WebChromeNode> {
        let [px_f, py_f] = self.viewport.logical_to_physical_px([lx, ly]);
        #[allow(clippy::cast_possible_truncation)]
        self.hit_test_physical(px_f.round() as i32, py_f.round() as i32)
    }

    /// Look up a projected node by stable `id`.
    #[must_use]
    pub fn find_node(&self, id: &str) -> Option<&WebChromeNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

/// Outcome returned when [`WebChromeBridge`] processes a pointer/wheel/key/IME event.
#[derive(Debug, Clone, PartialEq)]
pub enum WebChromeDispatch {
    PassToWorkspace(UiInputEvent),
    WheelScrollGrid { dx_px: f64, dy_px: f64 },
    HandledRedraw { action: String, command_result: Option<Value> },
    ShortcutTriggered(WebShortcutAction),
    InvokeChromeAction(WebChromeAction),
    BlockedByModalBackdrop,
    Ignored,
}

/// Stateful browser chrome placeholder synchronized with [`UiApp`].
#[derive(Debug, Clone, Default)]
pub struct WebChromeBridge {
    pub open_menu_key: Option<String>,
    pub popup_scroll_y: i32,
    pub hovered_node_id: Option<String>,
    pub focused_node_id: Option<String>,
    pub context_menu: Option<WebContextMenuState>,
    pub ime_preedit: Option<String>,
    pub drag_active: bool,
    pub last_notice: Option<(WebNoticeLevel, String)>,
    pub last_layout_metrics: Option<(u32, u32, u16)>,
}

impl WebChromeBridge {
    /// Construct an idle chrome bridge.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset transient popup/menu state when document switches or closes.
    pub fn close_popups(&mut self) {
        self.open_menu_key = None;
        self.popup_scroll_y = 0;
        self.context_menu = None;
    }

    /// Synchronize mirrored chrome state fields from [`UiApp`].
    pub fn sync_from_app(&mut self, app: &UiApp) {
        if self.open_menu_key != app.interaction.open_menu {
            self.popup_scroll_y = 0;
        }
        self.open_menu_key = app.interaction.open_menu.clone();
        self.hovered_node_id = app.interaction.hovered_widget_id.clone();
        self.focused_node_id = app.focus_widget_id.clone();
        if app.canvas_dialogs.active_dialog != ActiveDialog::None || app.interaction.context_menu_target.is_none() {
            self.context_menu = None;
        }
    }

    /// Build the minimal placeholder [`WebChromeLayout`] from semantic `UiApp` metadata.
    #[must_use]
    pub fn build_layout(&self, app: &UiApp, viewport: ViewportMetrics) -> WebChromeLayout {
        let vw = viewport.physical_width_px().max(320);
        let vh = viewport.physical_height_px().max(240);
        let mut nodes = Vec::new();
        let mut order = 0u32;
        let mut next_order = || {
            order = order.saturating_add(1);
            order
        };

        let mut menu_titles = Vec::new();
        let mut toolbar_ids = Vec::new();
        let open_menu = self.open_menu_key.as_deref().or(app.interaction.open_menu.as_deref());

        let bar_h = 28u32;
        nodes.push(WebChromeNode {
            id: String::from("menu.bar"),
            role: WebChromeRole::MenuBar,
            z_layer: ZLayer::Workspace,
            paint_order: next_order(),
            rect_px: LayoutRectPx::new(0, 0, vw, bar_h),
            clip_px: LayoutRectPx::new(0, 0, vw, bar_h),
            label: String::from("Menu Bar"),
            shortcut: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            action: WebChromeAction::None,
        });

        let mut mx = 8i32;
        let max_dropdown_h = vh.saturating_sub(32).max(28);
        for sec in menu_bar_tooltips(app) {
            menu_titles.push(sec.title.to_owned());
            let mid = sec.menu_key.to_owned();
            let mw = 64u32;
            let rect = LayoutRectPx::new(mx, 2, mw, 24);
            nodes.push(WebChromeNode {
                id: mid.clone(),
                role: WebChromeRole::MenuSection,
                z_layer: ZLayer::Workspace,
                paint_order: next_order(),
                rect_px: rect,
                clip_px: rect,
                label: sec.title.to_owned(),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::ToggleMenuSection(mid.clone()),
            });
            if open_menu == Some(mid.as_str()) || open_menu == Some(mid.trim_start_matches("menu.")) {
                let items: Vec<_> = sec.items.into_iter().filter(|tip| tip.command_id != "ui.event.dispatch").collect();
                let row_h = 14i32;
                let raw_pop_h = u32::try_from(items.len()).unwrap_or(1).saturating_mul(14).saturating_add(8).max(28);
                let pop_h = raw_pop_h.min(max_dropdown_h);
                let pop_w = 220u32.min(vw);
                let max_pop_x = i32::try_from(vw.saturating_sub(pop_w)).unwrap_or(0);
                let pop_x = mx.clamp(0, max_pop_x);
                let pop_rect = LayoutRectPx::new(pop_x, 28, pop_w, pop_h);
                let max_scroll = i32::try_from(raw_pop_h.saturating_sub(pop_h)).unwrap_or(0).max(0);
                let scroll_y = self.popup_scroll_y.clamp(0, max_scroll);
                let pop_bottom = pop_rect.y.saturating_add(i32::try_from(pop_rect.height).unwrap_or(28));
                nodes.push(WebChromeNode {
                    id: format!("popup.{mid}"),
                    role: WebChromeRole::DropdownMenu,
                    z_layer: ZLayer::DropdownMenu,
                    paint_order: next_order(),
                    rect_px: pop_rect,
                    clip_px: pop_rect,
                    label: sec.title.to_owned(),
                    shortcut: None,
                    enabled: true,
                    disabled_reason: None,
                    tooltip: None,
                    action: WebChromeAction::None,
                });
                for (idx, tip) in items.iter().enumerate() {
                    let row_offset = i32::try_from(idx).unwrap_or(i32::MAX / 16).saturating_mul(row_h);
                    let iy = 32_i32.saturating_add(row_offset).saturating_sub(scroll_y);
                    let visible_top = iy.max(pop_rect.y);
                    let visible_bottom = iy.saturating_add(12).min(pop_bottom);
                    if visible_bottom <= visible_top {
                        continue;
                    }
                    let clipped_h = u32::try_from(visible_bottom - visible_top).unwrap_or(0);
                    let item_rect = LayoutRectPx::new(pop_x + 4, visible_top, pop_w.saturating_sub(8).max(1), clipped_h);
                    let wid = format!("menu.{}", tip.command_id);
                    let is_rem = tip.command_id == "ui.result.applyRemediation";
                    let enabled = tip.enabled && !is_rem;
                    let disabled_reason = if is_rem {
                        Some(String::from("Diagnostic-only (QOL-7): inspect Result Diagnostic Tabs instead"))
                    } else {
                        (!enabled).then(|| format!("{} is currently unavailable in this context.", tip.label))
                    };
                    nodes.push(WebChromeNode {
                        id: wid,
                        role: WebChromeRole::MenuItem,
                        z_layer: ZLayer::DropdownMenu,
                        paint_order: next_order(),
                        rect_px: item_rect,
                        clip_px: pop_rect,
                        label: tip.localized_label.to_owned(),
                        shortcut: (!tip.shortcut.is_empty()).then(|| tip.shortcut.clone()),
                        enabled,
                        disabled_reason,
                        tooltip: Some(tip.format_inline_tooltip()),
                        action: if enabled { WebChromeAction::InvokeCommand(tip.command_id.to_owned()) } else { WebChromeAction::None },
                    });
                }
            }
            mx += 68;
        }

        let mut tx = 8i32;
        for btn in toolbar_tooltips(app) {
            toolbar_ids.push(btn.button.id.to_owned());
            let rect = LayoutRectPx::new(tx, 30, 56, 28);
            let cmd = if btn.button.id == "toolbar.axes" { "ui.graph.axes.open" } else { btn.button.command_id };
            let enabled = btn.tooltip.enabled;
            nodes.push(WebChromeNode {
                id: btn.button.id.to_owned(),
                role: WebChromeRole::ToolbarButton,
                z_layer: ZLayer::Workspace,
                paint_order: next_order(),
                rect_px: rect,
                clip_px: rect,
                label: btn.caption.to_owned(),
                shortcut: (!btn.tooltip.shortcut.is_empty()).then(|| btn.tooltip.shortcut.clone()),
                enabled,
                disabled_reason: (!enabled).then(|| format!("{} is currently unavailable in this context.", btn.caption)),
                tooltip: Some(btn.tooltip.format_inline_tooltip()),
                action: if enabled { WebChromeAction::InvokeCommand(cmd.to_owned()) } else { WebChromeAction::None },
            });
            tx += 60;
        }

        let doc = app.document();
        if let Some(active_rs) = doc.active_sheet_id.as_ref().and_then(|id| doc.result_sheet(id)) {
            let _ = active_rs;
            let btn_rect = LayoutRectPx::new(tx.min(i32::try_from(vw.saturating_sub(140)).unwrap_or(8)), 30, 132, 28);
            nodes.push(WebChromeNode {
                id: String::from("result.edit_params_btn"),
                role: WebChromeRole::ToolbarButton,
                z_layer: ZLayer::Workspace,
                paint_order: next_order(),
                rect_px: btn_rect,
                clip_px: btn_rect,
                label: String::from("Edit Parameters..."),
                shortcut: Some(String::from("Alt+T")),
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::InvokeCommand(String::from("ui.editParams.open")),
            });
        }

        let focused_id = app.focus_widget_id.clone().or_else(|| self.focused_node_id.clone());
        if app.canvas_dialogs.active_dialog != ActiveDialog::None {
            let full_rect = LayoutRectPx::new(0, 0, vw, vh);
            nodes.push(WebChromeNode {
                id: String::from("modal.backdrop"),
                role: WebChromeRole::ModalBackdrop,
                z_layer: ZLayer::ModalBackdrop,
                paint_order: next_order(),
                rect_px: full_rect,
                clip_px: full_rect,
                label: String::from("Modal Backdrop"),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::CloseModalBackdrop,
            });
            let dw = vw.min(520);
            let dh = vh.min(360);
            let dx = i32::try_from((vw - dw) / 2).unwrap_or(0);
            let dy = i32::try_from((vh - dh) / 2).unwrap_or(0);
            let dlg_rect = LayoutRectPx::new(dx, dy, dw, dh);
            nodes.push(WebChromeNode {
                id: String::from("modal.dialog"),
                role: WebChromeRole::ModalDialog,
                z_layer: ZLayer::ModalDialog,
                paint_order: next_order(),
                rect_px: dlg_rect,
                clip_px: dlg_rect,
                label: format!("{:?}", app.canvas_dialogs.active_dialog),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::None,
            });
            let ok_rect = LayoutRectPx::new(
                dx + i32::try_from(dw.saturating_sub(180)).unwrap_or(16),
                dy + i32::try_from(dh.saturating_sub(40)).unwrap_or(40),
                76,
                28,
            );
            nodes.push(WebChromeNode {
                id: String::from("modal.ok"),
                role: WebChromeRole::ModalButton,
                z_layer: ZLayer::ModalDialog,
                paint_order: next_order(),
                rect_px: ok_rect,
                clip_px: ok_rect,
                label: String::from("OK"),
                shortcut: Some(String::from("Enter")),
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::DialogConfirm,
            });
            let cancel_rect = LayoutRectPx::new(
                dx + i32::try_from(dw.saturating_sub(96)).unwrap_or(96),
                dy + i32::try_from(dh.saturating_sub(40)).unwrap_or(40),
                76,
                28,
            );
            nodes.push(WebChromeNode {
                id: String::from("modal.cancel"),
                role: WebChromeRole::ModalButton,
                z_layer: ZLayer::ModalDialog,
                paint_order: next_order(),
                rect_px: cancel_rect,
                clip_px: cancel_rect,
                label: String::from("Cancel"),
                shortcut: Some(String::from("Escape")),
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::DialogCancel,
            });
            append_modal_content_nodes(app, dlg_rect, focused_id.as_deref(), &mut next_order, &mut nodes);
        }

        if let Some(ctx) = &self.context_menu {
            append_context_menu_nodes(app, ctx, vw, vh, self.popup_scroll_y, &mut next_order, &mut nodes);
        }

        let status_summary = build_status_summary(app, self);
        nodes.push(WebChromeNode {
            id: String::from("status.bar"),
            role: WebChromeRole::StatusBar,
            z_layer: ZLayer::Workspace,
            paint_order: next_order(),
            rect_px: LayoutRectPx::new(0, i32::try_from(vh.saturating_sub(24)).unwrap_or(0), vw, 24),
            clip_px: LayoutRectPx::new(0, 0, vw, vh),
            label: status_summary.clone(),
            shortcut: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            action: WebChromeAction::None,
        });

        let a11y_tree = build_chrome_a11y_tree(app, self, &nodes, focused_id.as_deref(), &status_summary);
        let snapshot = WebChromeSnapshot {
            menu_bar_visible: !menu_titles.is_empty(),
            open_menu_key: self.open_menu_key.clone().or_else(|| app.interaction.open_menu.clone()),
            context_menu_open: self.context_menu.is_some(),
            active_dialog: format!("{:?}", app.canvas_dialogs.active_dialog),
            hovered_node_id: app.interaction.hovered_widget_id.clone().or_else(|| self.hovered_node_id.clone()),
            focused_node_id: focused_id,
            ime_preedit: self.ime_preedit.clone(),
            drag_active: self.drag_active,
            node_count: nodes.len(),
            menu_titles,
            toolbar_ids,
            status_summary,
            dirty: app.session().is_dirty(),
            a11y_tree: Some(Box::new(a11y_tree)),
        };
        WebChromeLayout { viewport, nodes, snapshot }
    }

    /// Update hover state against projected nodes and return `(hover_changed, occluded_above_workspace)`.
    pub fn handle_pointer_move(&mut self, app: &mut UiApp, layout: &WebChromeLayout, logical_pos: [f64; 2]) -> (bool, bool) {
        let hit = layout.hit_test_logical(logical_pos[0], logical_pos[1]);
        let next_hover = hit.map(|n| n.id.clone());
        let next_cmd = hit.and_then(|n| match &n.action {
            WebChromeAction::InvokeCommand(cmd) => Some(cmd.clone()),
            _ => None,
        });
        let changed = next_hover != app.interaction.hovered_widget_id || next_cmd != app.hover_command_id || next_hover != self.hovered_node_id;
        app.interaction.hovered_widget_id = next_hover.clone();
        app.hover_command_id = next_cmd;
        self.hovered_node_id = next_hover;
        let occluded_above_workspace = hit.is_some_and(|n| n.z_layer >= ZLayer::DropdownMenu);
        (changed, occluded_above_workspace)
    }

    /// Route pointer press through placeholder chrome, popup menus, and modal backdrop guards.
    pub fn handle_pointer_down(
        &mut self,
        app: &mut UiApp,
        layout: &WebChromeLayout,
        logical_pos: [f64; 2],
        phys_pos: [i32; 2],
        button: PointerButton,
    ) -> WebChromeDispatch {
        let [px, py] = phys_pos;

        if button == PointerButton::Secondary {
            if app.canvas_dialogs.active_dialog != ActiveDialog::None {
                return WebChromeDispatch::BlockedByModalBackdrop;
            }
            app.interaction.open_menu = None;
            app.interaction.open_dropdown = None;
            self.open_menu_key = None;
            self.popup_scroll_y = 0;
            let target = String::from(if is_active_data_sheet(app) { "grid" } else { "canvas" });
            app.interaction.context_menu_target = Some(target.clone());
            self.context_menu = Some(WebContextMenuState { target: target.clone(), anchor_px: [px, py] });
            return WebChromeDispatch::HandledRedraw { action: format!("context_menu.open:{target}"), command_result: None };
        }

        if button != PointerButton::Primary {
            return WebChromeDispatch::Ignored;
        }

        let hit = layout.hit_test_logical(logical_pos[0], logical_pos[1]);
        if self.context_menu.is_some() {
            if let Some(node) = hit.filter(|n| n.z_layer == ZLayer::ContextMenu) {
                return self.arm_pressed_chrome_node(app, node);
            }
            self.context_menu = None;
            app.interaction.context_menu_target = None;
            app.interaction.pressed_widget_id = None;
            return WebChromeDispatch::HandledRedraw { action: String::from("context_menu.dismissed"), command_result: None };
        }

        if hit.is_some_and(|n| n.id == "modal.backdrop") {
            self.open_menu_key = None;
            app.interaction.open_menu = None;
            app.interaction.open_dropdown = None;
            app.interaction.pressed_widget_id = None;
            return WebChromeDispatch::BlockedByModalBackdrop;
        }

        let popup_open = self.open_menu_key.is_some() || app.interaction.open_menu.is_some() || app.interaction.open_dropdown.is_some();
        if popup_open && hit.is_none_or(|n| n.z_layer < ZLayer::DropdownMenu && n.role != WebChromeRole::MenuSection) {
            self.close_popups();
            app.interaction.open_menu = None;
            app.interaction.open_dropdown = None;
            app.interaction.pressed_widget_id = None;
            return WebChromeDispatch::HandledRedraw { action: String::from("chrome.popup_dismissed"), command_result: None };
        }

        if let Some(node) = hit.filter(|n| n.action != WebChromeAction::None || n.z_layer >= ZLayer::DropdownMenu) {
            return self.arm_pressed_chrome_node(app, node);
        }

        WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown {
            x: logical_pos[0],
            y: logical_pos[1],
            button,
            modifiers: KeyModifiers::default(),
        })
    }

    fn arm_pressed_chrome_node(&mut self, app: &mut UiApp, node: &WebChromeNode) -> WebChromeDispatch {
        if !node.enabled {
            let reason = node.disabled_reason.clone();
            if let Some(ref msg) = reason {
                self.last_notice = Some((WebNoticeLevel::Warning, msg.clone()));
            }
            return WebChromeDispatch::HandledRedraw {
                action: format!("chrome.disabled:{}", node.id),
                command_result: Some(json!({ "disabled_reason": reason })),
            };
        }
        if node.action != WebChromeAction::None {
            app.interaction.pressed_widget_id = Some(node.id.clone());
            return WebChromeDispatch::HandledRedraw { action: format!("chrome.pressed:{}", node.id), command_result: None };
        }
        WebChromeDispatch::HandledRedraw { action: format!("chrome.surface:{}", node.id), command_result: None }
    }

    /// Complete pointer release against pressed placeholder chrome nodes or pass through to workspace.
    pub fn handle_pointer_up(
        &mut self,
        app: &mut UiApp,
        layout: &WebChromeLayout,
        logical_pos: [f64; 2],
        button: PointerButton,
    ) -> WebChromeDispatch {
        if button != PointerButton::Primary {
            if app.canvas_dialogs.active_dialog != ActiveDialog::None {
                return WebChromeDispatch::BlockedByModalBackdrop;
            }
            if button == PointerButton::Secondary && self.context_menu.is_some() {
                return WebChromeDispatch::HandledRedraw { action: String::from("context_menu.pointer_up_consumed"), command_result: None };
            }
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerUp {
                x: logical_pos[0],
                y: logical_pos[1],
                button,
                modifiers: KeyModifiers::default(),
            });
        }
        let hit = layout.hit_test_logical(logical_pos[0], logical_pos[1]);
        if let Some(pressed_id) = app.interaction.pressed_widget_id.take() {
            let Some(node) = hit.filter(|n| n.id == pressed_id) else {
                return WebChromeDispatch::HandledRedraw { action: String::from("chrome.release_cancelled"), command_result: None };
            };
            if node.z_layer == ZLayer::ContextMenu {
                self.context_menu = None;
                self.popup_scroll_y = 0;
                app.interaction.context_menu_target = None;
            }
            if !node.enabled {
                return WebChromeDispatch::HandledRedraw {
                    action: format!("chrome.disabled:{}", node.id),
                    command_result: Some(json!({ "disabled_reason": node.disabled_reason })),
                };
            }
            return WebChromeDispatch::InvokeChromeAction(node.action.clone());
        }
        WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerUp {
            x: logical_pos[0],
            y: logical_pos[1],
            button,
            modifiers: KeyModifiers::default(),
        })
    }

    /// Route mouse-wheel / trackpad scroll over placeholder layout, open popups, and active sheet.
    pub fn route_wheel(
        &mut self,
        app: &mut UiApp,
        _layout: &WebChromeLayout,
        pos_logical: [f64; 2],
        delta_px: [f64; 2],
        modifiers: KeyModifiers,
    ) -> WebChromeDispatch {
        if app.canvas_dialogs.active_dialog != ActiveDialog::None {
            return WebChromeDispatch::BlockedByModalBackdrop;
        }
        if self.context_menu.is_some() || self.open_menu_key.is_some() || app.interaction.open_menu.is_some() {
            #[allow(clippy::cast_possible_truncation)]
            let delta_y = delta_px[1].round() as i32;
            let max_scroll = popup_max_scroll_px(app, self, _layout.viewport);
            self.popup_scroll_y = self.popup_scroll_y.saturating_add(delta_y).clamp(0, max_scroll);
            return WebChromeDispatch::HandledRedraw {
                action: String::from("popup.wheel_scroll"),
                command_result: Some(json!({ "popup_scroll_y": self.popup_scroll_y })),
            };
        }
        if modifiers.ctrl || modifiers.meta {
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::WheelZoom {
                x: pos_logical[0],
                y: pos_logical[1],
                delta_y: delta_px[1],
                modifiers,
            });
        }
        let doc = app.document();
        let active_is_grid = doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some());
        if active_is_grid {
            return WebChromeDispatch::WheelScrollGrid { dx_px: delta_px[0], dy_px: delta_px[1] };
        }
        WebChromeDispatch::PassToWorkspace(UiInputEvent::InertialPan {
            delta_x: -delta_px[0],
            delta_y: -delta_px[1],
            dt_sec: 1.0 / 60.0,
            step_coast: false,
        })
    }

    /// Route keyboard `KeyDown` events through popups, modals, and Web shortcuts.
    pub fn handle_key_down(&mut self, app: &mut UiApp, _layout: &WebChromeLayout, key: &str, modifiers: KeyModifiers) -> WebChromeDispatch {
        let trimmed = key.trim();

        if matches!(trimmed, "Escape" | "Esc") {
            if self.context_menu.is_some() || self.open_menu_key.is_some() || app.interaction.open_menu.is_some() {
                self.close_popups();
                app.interaction.open_menu = None;
                app.interaction.open_dropdown = None;
                app.interaction.context_menu_target = None;
                return WebChromeDispatch::HandledRedraw { action: String::from("chrome.escape_popup"), command_result: None };
            }
            if app.canvas_dialogs.active_dialog != ActiveDialog::None && !app.inline_editor.active {
                app.canvas_dialogs.close_dialog();
                app.focus_widget_id = None;
                self.focused_node_id = None;
                return WebChromeDispatch::HandledRedraw { action: String::from("dialog.escape_close"), command_result: None };
            }
        }

        if app.inline_editor.active {
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers });
        }

        if app.canvas_dialogs.active_dialog != ActiveDialog::None {
            return self.handle_modal_key_down(app, trimmed, modifiers);
        }

        let routed = WebShortcutRouter::route_key_down(app, trimmed, modifiers);
        if routed.handled {
            return WebChromeDispatch::ShortcutTriggered(routed.action);
        }

        if !modifiers.ctrl
            && !modifiers.meta
            && !modifiers.alt
            && is_active_data_sheet(app)
            && let Some(disp) = handle_grid_navigation_key(app, trimmed, modifiers.shift)
        {
            return disp;
        }

        WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers })
    }

    fn handle_modal_key_down(&mut self, app: &mut UiApp, key: &str, modifiers: KeyModifiers) -> WebChromeDispatch {
        if matches!(key, "Enter" | "Return") {
            let ctrl_or_meta_submit = (modifiers.ctrl || modifiers.meta) && !modifiers.alt;
            let plain_enter = !modifiers.ctrl && !modifiers.meta && !modifiers.alt;
            let multiline_focused = app.focus_widget_id.as_deref().is_some_and(is_web_multiline_textbox);
            if ctrl_or_meta_submit || (plain_enter && !multiline_focused) {
                return WebChromeDispatch::InvokeChromeAction(WebChromeAction::DialogConfirm);
            }
        }
        match &app.canvas_dialogs.active_dialog {
            ActiveDialog::CommandPalette => {
                let hits_len = app.canvas_dialogs.command_palette.matches_for_session(app.session()).len();
                match key {
                    "Backspace" => {
                        app.canvas_dialogs.command_palette.query.pop();
                        app.canvas_dialogs.command_palette.selected_index = 0;
                        sync_modal_text_buffer(app, "palette.query", &app.canvas_dialogs.command_palette.query.clone());
                        WebChromeDispatch::HandledRedraw { action: String::from("palette.backspace"), command_result: None }
                    }
                    "ArrowDown" | "Down" if hits_len > 0 => {
                        let next = (app.canvas_dialogs.command_palette.selected_index + 1).min(hits_len - 1);
                        app.canvas_dialogs.command_palette.selected_index = next;
                        WebChromeDispatch::HandledRedraw { action: String::from("palette.next"), command_result: None }
                    }
                    "ArrowUp" | "Up" => {
                        app.canvas_dialogs.command_palette.selected_index = app.canvas_dialogs.command_palette.selected_index.saturating_sub(1);
                        WebChromeDispatch::HandledRedraw { action: String::from("palette.prev"), command_result: None }
                    }
                    _ if printable_text_input_for_key(key, modifiers).is_some() => {
                        app.canvas_dialogs.command_palette.query.push_str(key);
                        app.canvas_dialogs.command_palette.selected_index = 0;
                        sync_modal_text_buffer(app, "palette.query", &app.canvas_dialogs.command_palette.query.clone());
                        WebChromeDispatch::HandledRedraw { action: String::from("palette.type"), command_result: None }
                    }
                    _ => WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers }),
                }
            }
            ActiveDialog::AnalyzeSheet { .. } if app.focus_widget_id.as_deref().is_none_or(|id| id == "analyze.search") => {
                let hits_len = app.canvas_dialogs.analyze.compatible_entries().len();
                match key {
                    "Backspace" => {
                        app.canvas_dialogs.analyze.search_query.pop();
                        sync_modal_text_buffer(app, "analyze.search", &app.canvas_dialogs.analyze.search_query.clone());
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.backspace"), command_result: None }
                    }
                    "ArrowDown" | "Down" if hits_len > 0 => {
                        let entries = app.canvas_dialogs.analyze.compatible_entries();
                        let cur = entries.iter().position(|e| Some(e.id) == app.canvas_dialogs.analyze.selected_entry_id.as_deref()).unwrap_or(0);
                        let next = (cur + 1).min(entries.len() - 1);
                        if let Some(entry) = entries.get(next) {
                            app.canvas_dialogs.analyze.selected_entry_id = Some(entry.id.to_owned());
                        }
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.next"), command_result: None }
                    }
                    "ArrowUp" | "Up" if hits_len > 0 => {
                        let entries = app.canvas_dialogs.analyze.compatible_entries();
                        let cur = entries.iter().position(|e| Some(e.id) == app.canvas_dialogs.analyze.selected_entry_id.as_deref()).unwrap_or(0);
                        let prev = cur.saturating_sub(1);
                        if let Some(entry) = entries.get(prev) {
                            app.canvas_dialogs.analyze.selected_entry_id = Some(entry.id.to_owned());
                        }
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.prev"), command_result: None }
                    }
                    _ if printable_text_input_for_key(key, modifiers).is_some() => {
                        app.canvas_dialogs.analyze.search_query.push_str(key);
                        if let Some(first) = app.canvas_dialogs.analyze.compatible_entries().first() {
                            app.canvas_dialogs.analyze.selected_entry_id = Some(first.id.to_owned());
                        }
                        sync_modal_text_buffer(app, "analyze.search", &app.canvas_dialogs.analyze.search_query.clone());
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.type"), command_result: None }
                    }
                    _ => WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers }),
                }
            }
            ActiveDialog::ExcludeWithReason { .. }
                if app.focus_widget_id.as_deref().is_none_or(|id| matches!(id, "exclude.note" | "exclude.note_input")) =>
            {
                match key {
                    "Backspace" => {
                        app.canvas_dialogs.exclude_with_reason.note.pop();
                        sync_modal_text_buffer(app, "exclude.note", &app.canvas_dialogs.exclude_with_reason.note.clone());
                        WebChromeDispatch::HandledRedraw { action: String::from("exclude.backspace"), command_result: None }
                    }
                    _ if printable_text_input_for_key(key, modifiers).is_some() => {
                        app.canvas_dialogs.exclude_with_reason.note.push_str(key);
                        sync_modal_text_buffer(app, "exclude.note", &app.canvas_dialogs.exclude_with_reason.note.clone());
                        WebChromeDispatch::HandledRedraw { action: String::from("exclude.type"), command_result: None }
                    }
                    _ => WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers }),
                }
            }
            _ => WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers }),
        }
    }

    /// Route committed text or IME composition into active modal inputs or workspace inline/text editors.
    pub fn handle_ime_commit(&mut self, app: &mut UiApp, text: &str) -> WebChromeDispatch {
        self.ime_preedit = None;
        if text.is_empty() {
            return WebChromeDispatch::Ignored;
        }
        match &app.canvas_dialogs.active_dialog {
            ActiveDialog::CommandPalette => {
                app.canvas_dialogs.command_palette.query.push_str(text);
                app.canvas_dialogs.command_palette.selected_index = 0;
                sync_modal_text_buffer(app, "palette.query", &app.canvas_dialogs.command_palette.query.clone());
                WebChromeDispatch::HandledRedraw { action: String::from("palette.ime_commit"), command_result: None }
            }
            ActiveDialog::AnalyzeSheet { .. } if app.focus_widget_id.as_deref().is_none_or(|id| id == "analyze.search") => {
                app.canvas_dialogs.analyze.search_query.push_str(text);
                if let Some(first) = app.canvas_dialogs.analyze.compatible_entries().first() {
                    app.canvas_dialogs.analyze.selected_entry_id = Some(first.id.to_owned());
                }
                sync_modal_text_buffer(app, "analyze.search", &app.canvas_dialogs.analyze.search_query.clone());
                WebChromeDispatch::HandledRedraw { action: String::from("analyze.ime_commit"), command_result: None }
            }
            ActiveDialog::ExcludeWithReason { .. }
                if app.focus_widget_id.as_deref().is_none_or(|id| matches!(id, "exclude.note" | "exclude.note_input")) =>
            {
                app.canvas_dialogs.exclude_with_reason.note.push_str(text);
                sync_modal_text_buffer(app, "exclude.note", &app.canvas_dialogs.exclude_with_reason.note.clone());
                WebChromeDispatch::HandledRedraw { action: String::from("exclude.ime_commit"), command_result: None }
            }
            _ => WebChromeDispatch::PassToWorkspace(UiInputEvent::TextInput { text: text.to_owned() }),
        }
    }
}

/// Extract a printable single-grapheme text payload for unhandled non-modifier `KeyDown` fallback routing.
#[must_use]
pub fn printable_text_input_for_key(key: &str, modifiers: KeyModifiers) -> Option<String> {
    if modifiers.ctrl || modifiers.meta || modifiers.alt || key.is_empty() || key.chars().any(char::is_control) {
        return None;
    }
    let mut base_count = 0usize;
    let mut prev_zwj = false;
    for ch in key.chars() {
        if ch == '\u{200D}' {
            if base_count == 0 {
                return None;
            }
            prev_zwj = true;
            continue;
        }
        if is_grapheme_extender(ch) {
            if base_count == 0 {
                return None;
            }
            prev_zwj = false;
            continue;
        }
        if !prev_zwj {
            base_count = base_count.saturating_add(1);
            if base_count > 1 {
                return None;
            }
        }
        prev_zwj = false;
    }
    (base_count == 1 && !prev_zwj).then(|| key.to_owned())
}

fn is_grapheme_extender(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

fn is_web_multiline_textbox(widget_id: &str) -> bool {
    matches!(
        widget_id,
        "checkpoints.note"
            | "import_paste.input.csv"
            | "import_paste.input.notes"
            | "import_paste.input.plate_values"
            | "import.tidy_csv"
            | "edit.fit.custom_dsl"
            | "analyze.fit.custom_dsl"
    )
}

fn sync_modal_text_buffer(app: &mut UiApp, widget_id: &str, value: &str) {
    if let Some(buf) = app.interaction.text_field_buffers.get_mut(widget_id) {
        buf.text = value.to_owned();
        buf.cursor_char = buf.text.chars().count();
        buf.selection_anchor = None;
        buf.preedit = None;
    }
}

fn popup_max_scroll_px(app: &UiApp, bridge: &WebChromeBridge, viewport: ViewportMetrics) -> i32 {
    let vh = viewport.physical_height_px().max(240);
    if let Some(ctx) = &bridge.context_menu {
        let count = u32::try_from(context_menu_items_for_target(&ctx.target, app).len()).unwrap_or(1);
        let raw_h = count.saturating_mul(22).saturating_add(8).max(32);
        let max_h = vh.saturating_sub(28).max(32);
        return i32::try_from(raw_h.saturating_sub(max_h)).unwrap_or(0).max(0);
    }
    let open_menu = bridge.open_menu_key.as_deref().or(app.interaction.open_menu.as_deref());
    let Some(open_key) = open_menu else {
        return 0;
    };
    let max_dropdown_h = vh.saturating_sub(32).max(28);
    for sec in menu_bar_tooltips(app) {
        let mid = sec.menu_key;
        if open_key == mid || open_key == mid.trim_start_matches("menu.") {
            let count = u32::try_from(sec.items.into_iter().filter(|tip| tip.command_id != "ui.event.dispatch").count()).unwrap_or(1);
            let raw_h = count.saturating_mul(14).saturating_add(8).max(28);
            return i32::try_from(raw_h.saturating_sub(max_dropdown_h)).unwrap_or(0).max(0);
        }
    }
    0
}

fn chrome_dom_id(widget_id: &str) -> String {
    let sanitized: String = widget_id.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' }).collect();
    format!("cb-chrome-{sanitized}")
}

fn is_rtl_locale(locale: &str) -> bool {
    let primary = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    matches!(primary.as_str(), "ar" | "fa" | "he" | "iw" | "ur" | "ps" | "sd" | "ug" | "yi")
}

fn push_modal_input<F>(nodes: &mut Vec<WebChromeNode>, next_order: &mut F, rect_px: LayoutRectPx, id: &str, label: String)
where
    F: FnMut() -> u32,
{
    nodes.push(WebChromeNode {
        id: id.to_owned(),
        role: WebChromeRole::ModalInput,
        z_layer: ZLayer::ModalDialog,
        paint_order: next_order(),
        rect_px,
        clip_px: rect_px,
        label,
        shortcut: None,
        enabled: true,
        disabled_reason: None,
        tooltip: None,
        action: WebChromeAction::FocusModalInput,
    });
}

fn append_modal_content_nodes<F>(app: &UiApp, dlg_rect: LayoutRectPx, focused_id: Option<&str>, next_order: &mut F, nodes: &mut Vec<WebChromeNode>)
where
    F: FnMut() -> u32,
{
    let dx = dlg_rect.x;
    let dy = dlg_rect.y;
    let dw = dlg_rect.width;
    let input_rect = LayoutRectPx::new(dx + 16, dy + 44, dw.saturating_sub(32).max(40), 28);

    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::CommandPalette => {
            push_modal_input(nodes, next_order, input_rect, "palette.query", app.canvas_dialogs.command_palette.query.clone());
            let matches = app.canvas_dialogs.command_palette.matches_for_session(app.session());
            for (idx, entry) in matches.into_iter().take(8).enumerate() {
                let row_y = dy + 80 + i32::try_from(idx).unwrap_or(0) * 22;
                let item_rect = LayoutRectPx::new(dx + 16, row_y, dw.saturating_sub(32).max(40), 20);
                nodes.push(WebChromeNode {
                    id: format!("palette.item.{idx}"),
                    role: WebChromeRole::ModalItem,
                    z_layer: ZLayer::ModalDialog,
                    paint_order: next_order(),
                    rect_px: item_rect,
                    clip_px: dlg_rect,
                    label: entry.label.to_owned(),
                    shortcut: (!entry.shortcut.is_empty()).then(|| entry.shortcut.to_owned()),
                    enabled: true,
                    disabled_reason: None,
                    tooltip: None,
                    action: WebChromeAction::SelectDialogItem(idx),
                });
            }
        }
        ActiveDialog::AnalyzeSheet { .. } => {
            push_modal_input(nodes, next_order, input_rect, "analyze.search", app.canvas_dialogs.analyze.search_query.clone());
            for (idx, entry) in app.canvas_dialogs.analyze.compatible_entries().into_iter().take(8).enumerate() {
                let row_y = dy + 80 + i32::try_from(idx).unwrap_or(0) * 22;
                let item_rect = LayoutRectPx::new(dx + 16, row_y, dw.saturating_sub(32).max(40), 20);
                nodes.push(WebChromeNode {
                    id: format!("analyze.item.{idx}"),
                    role: WebChromeRole::ModalItem,
                    z_layer: ZLayer::ModalDialog,
                    paint_order: next_order(),
                    rect_px: item_rect,
                    clip_px: dlg_rect,
                    label: entry.label.to_owned(),
                    shortcut: None,
                    enabled: true,
                    disabled_reason: None,
                    tooltip: None,
                    action: WebChromeAction::SelectDialogItem(idx),
                });
            }
        }
        ActiveDialog::ConstantHookPicker { .. } => {
            push_modal_input(nodes, next_order, input_rect, "hook.search_query", app.canvas_dialogs.hook_picker.search_query.clone());
            for (idx, opt) in app.canvas_dialogs.hook_picker.options.iter().take(8).enumerate() {
                let row_y = dy + 80 + i32::try_from(idx).unwrap_or(0) * 22;
                let item_rect = LayoutRectPx::new(dx + 16, row_y, dw.saturating_sub(32).max(40), 20);
                nodes.push(WebChromeNode {
                    id: format!("hook.item.{idx}"),
                    role: WebChromeRole::ModalItem,
                    z_layer: ZLayer::ModalDialog,
                    paint_order: next_order(),
                    rect_px: item_rect,
                    clip_px: dlg_rect,
                    label: opt.label.clone(),
                    shortcut: None,
                    enabled: true,
                    disabled_reason: None,
                    tooltip: None,
                    action: WebChromeAction::SelectDialogItem(idx),
                });
            }
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            push_modal_input(nodes, next_order, input_rect, "exclude.note", app.canvas_dialogs.exclude_with_reason.note.clone());
        }
        ActiveDialog::FormatAxes { .. } => {
            let val = app
                .interaction
                .text_field_buffers
                .get("axes.title")
                .map_or_else(|| app.canvas_dialogs.format_axes.axis_draft.title.clone(), |b| b.text.clone());
            push_modal_input(nodes, next_order, input_rect, "axes.title", val);
        }
        ActiveDialog::EditAnalysisParameters { .. } => {
            let val = app.interaction.text_field_buffers.get("edit.custom_title").map_or_else(String::new, |b| b.text.clone());
            push_modal_input(nodes, next_order, input_rect, "edit.custom_title", val);
        }
        ActiveDialog::Checkpoints => {
            push_modal_input(nodes, next_order, input_rect, "checkpoints.name", app.canvas_dialogs.checkpoints.name_draft.clone());
            let note_rect = LayoutRectPx::new(dx + 16, dy + 82, dw.saturating_sub(32).max(40), 56);
            push_modal_input(nodes, next_order, note_rect, "checkpoints.note", app.canvas_dialogs.checkpoints.note_draft.clone());
        }
        ActiveDialog::ImportReshape => {
            push_modal_input(nodes, next_order, input_rect, "import.title", app.grid.import_reshape.title.clone());
        }
        ActiveDialog::PowerAndSampleSize => {
            let back_rect = LayoutRectPx::new(dx + 16, dy + i32::try_from(dlg_rect.height.saturating_sub(40)).unwrap_or(40), 76, 28);
            let next_rect = LayoutRectPx::new(dx + 100, dy + i32::try_from(dlg_rect.height.saturating_sub(40)).unwrap_or(40), 76, 28);
            nodes.push(WebChromeNode {
                id: String::from("power.back"),
                role: WebChromeRole::ModalButton,
                z_layer: ZLayer::ModalDialog,
                paint_order: next_order(),
                rect_px: back_rect,
                clip_px: back_rect,
                label: String::from("Back"),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::DialogStepBack,
            });
            nodes.push(WebChromeNode {
                id: String::from("power.next"),
                role: WebChromeRole::ModalButton,
                z_layer: ZLayer::ModalDialog,
                paint_order: next_order(),
                rect_px: next_rect,
                clip_px: next_rect,
                label: String::from("Next"),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::DialogStepNext,
            });
        }
        _ => {}
    }

    if let Some(fid) = focused_id
        && !nodes.iter().any(|n| n.id == fid)
    {
        let val = app.interaction.text_field_buffers.get(fid).map_or_else(String::new, |b| b.text.clone());
        push_modal_input(nodes, next_order, input_rect, fid, val);
    }
}

fn append_context_menu_nodes<F>(
    app: &UiApp,
    ctx: &WebContextMenuState,
    vw: u32,
    vh: u32,
    popup_scroll_y: i32,
    next_order: &mut F,
    nodes: &mut Vec<WebChromeNode>,
) where
    F: FnMut() -> u32,
{
    let ctx_items = context_menu_items_for_target(&ctx.target, app);
    let pop_w = 220u32.min(vw);
    let row_h = 22i32;
    let raw_h = u32::try_from(ctx_items.len()).unwrap_or(1).saturating_mul(22).saturating_add(8).max(32);
    let max_h = vh.saturating_sub(28).max(32);
    let pop_h = raw_h.min(max_h);
    let max_x = i32::try_from(vw.saturating_sub(pop_w)).unwrap_or(0);
    let max_y = i32::try_from(vh.saturating_sub(pop_h)).unwrap_or(0);
    let pop_x = ctx.anchor_px[0].clamp(0, max_x);
    let pop_y = ctx.anchor_px[1].clamp(0, max_y);
    let pop_rect = LayoutRectPx::new(pop_x, pop_y, pop_w, pop_h);
    let max_scroll = i32::try_from(raw_h.saturating_sub(pop_h)).unwrap_or(0).max(0);
    let scroll_y = popup_scroll_y.clamp(0, max_scroll);
    let pop_bottom = pop_rect.y.saturating_add(i32::try_from(pop_rect.height).unwrap_or(32));

    nodes.push(WebChromeNode {
        id: String::from("popup.context_menu"),
        role: WebChromeRole::ContextMenu,
        z_layer: ZLayer::ContextMenu,
        paint_order: next_order(),
        rect_px: pop_rect,
        clip_px: pop_rect,
        label: format!("Context Menu ({})", ctx.target),
        shortcut: None,
        enabled: true,
        disabled_reason: None,
        tooltip: None,
        action: WebChromeAction::DismissContextMenu,
    });

    for (idx, (label, cmd_id, enabled)) in ctx_items.into_iter().enumerate() {
        let row_offset = i32::try_from(idx).unwrap_or(i32::MAX / 24).saturating_mul(row_h);
        let iy = pop_rect.y.saturating_add(4).saturating_add(row_offset).saturating_sub(scroll_y);
        let visible_top = iy.max(pop_rect.y);
        let visible_bottom = iy.saturating_add(20).min(pop_bottom);
        if visible_bottom <= visible_top {
            continue;
        }
        let clipped_h = u32::try_from(visible_bottom - visible_top).unwrap_or(0);
        let item_rect = LayoutRectPx::new(pop_rect.x + 4, visible_top, pop_w.saturating_sub(8).max(1), clipped_h);
        nodes.push(WebChromeNode {
            id: format!("context_menu.{cmd_id}"),
            role: WebChromeRole::MenuItem,
            z_layer: ZLayer::ContextMenu,
            paint_order: next_order(),
            rect_px: item_rect,
            clip_px: pop_rect,
            label: label.clone(),
            shortcut: None,
            enabled,
            disabled_reason: (!enabled).then(|| format!("{label} is currently unavailable in this context.")),
            tooltip: None,
            action: if enabled { WebChromeAction::InvokeCommand(cmd_id) } else { WebChromeAction::None },
        });
    }
}

fn build_chrome_a11y_tree(app: &UiApp, bridge: &WebChromeBridge, nodes: &[WebChromeNode], focused_id: Option<&str>, status_summary: &str) -> Value {
    let dir = if is_rtl_locale(&app.locale) { "rtl" } else { "ltr" };
    let (live_announcement, live_politeness) = match &bridge.last_notice {
        Some((lvl, msg)) if !msg.is_empty() => {
            let politeness = match lvl {
                WebNoticeLevel::Error | WebNoticeLevel::Warning => "assertive",
                WebNoticeLevel::Info | WebNoticeLevel::Success => "polite",
            };
            (msg.clone(), politeness)
        }
        _ => (status_summary.to_owned(), "polite"),
    };
    let open_menu = bridge.open_menu_key.as_deref().or(app.interaction.open_menu.as_deref());
    let mut a11y_nodes = Vec::with_capacity(nodes.len());
    for node in nodes {
        let dom_id = chrome_dom_id(&node.id);
        let tag_name = match node.role {
            WebChromeRole::MenuSection
            | WebChromeRole::MenuItem
            | WebChromeRole::ToolbarButton
            | WebChromeRole::ModalButton
            | WebChromeRole::ModalCloseButton
            | WebChromeRole::ModalItem => "button",
            WebChromeRole::ModalInput => {
                if is_web_multiline_textbox(&node.id) || matches!(node.id.as_str(), "exclude.note" | "exclude.note_input") {
                    "textarea"
                } else {
                    "input"
                }
            }
            _ => "div",
        };
        let parent_dom_id = if node.id.starts_with("menu.") && node.id != "menu.bar" && node.role == WebChromeRole::MenuSection {
            Some(chrome_dom_id("menu.bar"))
        } else if node.role == WebChromeRole::MenuItem && node.id.starts_with("context_menu.") {
            Some(chrome_dom_id("popup.context_menu"))
        } else if node.role == WebChromeRole::MenuItem {
            open_menu.map(|m| {
                let norm = if m.starts_with("menu.") { m.to_owned() } else { format!("menu.{m}") };
                chrome_dom_id(&format!("popup.{norm}"))
            })
        } else if node.id == "modal.dialog" {
            Some(chrome_dom_id("modal.backdrop"))
        } else if node.z_layer == ZLayer::ModalDialog && node.id != "modal.dialog" {
            Some(chrome_dom_id("modal.dialog"))
        } else {
            None
        };
        let aria_role = match node.role {
            WebChromeRole::MenuBar => "menubar",
            WebChromeRole::MenuSection => "menuitem",
            WebChromeRole::MenuItem => "menuitem",
            WebChromeRole::ToolbarButton => "button",
            WebChromeRole::DropdownMenu | WebChromeRole::ContextMenu => "menu",
            WebChromeRole::ModalBackdrop => "presentation",
            WebChromeRole::ModalDialog => "dialog",
            WebChromeRole::ModalButton | WebChromeRole::ModalCloseButton => "button",
            WebChromeRole::ModalInput => "textbox",
            WebChromeRole::ModalItem => "option",
            WebChromeRole::StatusBar => "status",
            WebChromeRole::Tooltip => "tooltip",
        };
        let mut attrs = serde_json::Map::new();
        attrs.insert(String::from("role"), Value::String(String::from(aria_role)));
        attrs.insert(String::from("aria-label"), Value::String(node.label.clone()));
        attrs.insert(String::from("data-node-id"), Value::String(node.id.clone()));
        attrs.insert(String::from("tabindex"), Value::String(String::from(if node.enabled && tag_name != "div" { "0" } else { "-1" })));
        if !node.enabled {
            attrs.insert(String::from("aria-disabled"), Value::String(String::from("true")));
        }
        if let Some(ref sc) = node.shortcut {
            attrs.insert(String::from("aria-keyshortcuts"), Value::String(sc.clone()));
        }
        if node.role == WebChromeRole::ModalDialog {
            attrs.insert(String::from("aria-modal"), Value::String(String::from("true")));
        }
        if node.role == WebChromeRole::MenuSection {
            let expanded = open_menu.is_some_and(|m| m == node.id || m == node.id.trim_start_matches("menu."));
            attrs.insert(String::from("aria-expanded"), Value::String(String::from(if expanded { "true" } else { "false" })));
        }
        let value_text = if matches!(tag_name, "input" | "textarea") { Some(node.label.clone()) } else { None };
        a11y_nodes.push(json!({
            "id": node.id,
            "dom_id": dom_id,
            "tag_name": tag_name,
            "parent_dom_id": parent_dom_id,
            "attributes": Value::Object(attrs),
            "label": node.label,
            "value_text": value_text,
            "enabled": node.enabled,
        }));
    }
    let focused_dom_id = focused_id.map(chrome_dom_id);
    json!({
        "dir": dir,
        "locale": app.locale,
        "live_announcement": live_announcement,
        "live_politeness": live_politeness,
        "focused_dom_id": focused_dom_id,
        "nodes": a11y_nodes,
    })
}

fn is_active_data_sheet(app: &UiApp) -> bool {
    let doc = app.document();
    doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some())
}

fn handle_grid_navigation_key(app: &mut UiApp, key: &str, shift: bool) -> Option<WebChromeDispatch> {
    let doc = app.document().clone();
    let ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id))?;
    match key {
        "Delete" | "Backspace" => {
            if let Some(coord) = app.grid.tool.active {
                let cells = if app.grid.selected_coords.is_empty() {
                    vec![json!({ "row": coord.row, "col": coord.col, "subcol": coord.subcol })]
                } else {
                    app.grid.selected_coords.iter().map(|c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol })).collect()
                };
                return Some(WebChromeDispatch::ShortcutTriggered(WebShortcutAction::ExecuteCommand {
                    command_id: String::from("table.clearCells"),
                    params: json!({ "sheet_id": ds.id.as_str(), "cells": cells }),
                }));
            }
            None
        }
        "ArrowUp" | "Up" | "ArrowDown" | "Down" | "ArrowLeft" | "Left" | "ArrowRight" | "Right" | "Tab" | "Enter" => {
            let cur = app.grid.tool.active.unwrap_or_default();
            let max_row = ds.archetype.row_count().saturating_sub(1);
            let max_col = ds.archetype.column_count().saturating_sub(1);
            let mut next = cur;
            match key {
                "ArrowUp" | "Up" => next.row = cur.row.saturating_sub(1),
                "ArrowDown" | "Down" | "Enter" => next.row = (cur.row + 1).min(max_row.max(cur.row + 1)),
                "ArrowLeft" | "Left" => next.col = cur.col.saturating_sub(1),
                "ArrowRight" | "Right" | "Tab" => {
                    next.col = if shift && key == "Tab" { cur.col.saturating_sub(1) } else { (cur.col + 1).min(max_col.max(cur.col)) };
                }
                _ => {}
            }
            if shift && key != "Tab" {
                app.grid.extend_shift_range(ds, next);
            } else {
                app.grid.select_single_sync(next);
            }
            app.grid.refresh_selection_stats(&doc);
            app.sync_brush_from_grid_selection(&ds.id);
            Some(WebChromeDispatch::HandledRedraw { action: format!("grid.navigate:{key}"), command_result: None })
        }
        _ => None,
    }
}

/// Resolved browser command route ensuring zero `Value::Null` engine calls and zero silent no-ops.
#[derive(Debug, Clone, PartialEq)]
pub enum WebCommandRoute {
    OpenFilePicker { accept: String },
    ExportFormat { format: WebExportFormat, sheet_id: Option<String> },
    PrintCurrentSheet,
    OpenEditAnalysisParameters(String),
    UiCommand { command_id: String, params: Value },
    EngineCommand { command_id: String, params: Value },
    NoticeOnly { level: WebNoticeLevel, code: String, message: String },
}

impl WebCommandRoute {
    /// Whether this route only reads/views state or opens a non-mutating picker/export.
    #[must_use]
    pub fn is_read_only_hint(&self) -> bool {
        match self {
            Self::OpenEditAnalysisParameters(_) => false,
            Self::OpenFilePicker { .. } | Self::ExportFormat { .. } | Self::PrintCurrentSheet | Self::NoticeOnly { .. } => true,
            Self::UiCommand { command_id, .. } => is_read_only_ui_command(command_id),
            Self::EngineCommand { command_id, params } => {
                if let Some(spec) = cmd::registry().get(command_id) {
                    spec.effects.read_only_hint()
                        || (matches!(command_id.as_str(), "analysis.exportJson" | "analysis.exportCsv" | "result.generateReport")
                            && !has_non_null_path_param(params))
                } else {
                    is_read_only_control_method(command_id)
                        || (matches!(command_id.as_str(), "doc.export" | "render.pdf") && !has_non_null_path_param(params))
                }
            }
        }
    }
}

/// Resolve any toolbar, dropdown, palette, or shortcut command ID (including `:variant` suffixes) cleanly.
#[must_use]
pub fn resolve_web_command_route(app: &UiApp, raw_command_id: &str) -> WebCommandRoute {
    let doc = app.document();
    let trimmed = raw_command_id.trim();
    if let Some((base, variant)) = trimmed.split_once(':') {
        let active_render = resolve_renderable_sheet_id(doc).map(|id| id.as_str().to_owned());
        let params = match base {
            "ui.theme.set" => json!({ "mode": variant }),
            "ui.locale.set" => json!({ "locale": variant }),
            "ui.previewFilter.set" => json!({ "filter": variant }),
            "ui.canvas.applyPreset" => json!({ "sheet_id": active_render, "preset": variant }),
            "ui.rulers.toggle" => json!({ "unit": variant }),
            _ => json!({ "variant": variant }),
        };
        return WebCommandRoute::UiCommand { command_id: base.to_owned(), params };
    }

    let first_data = || resolve_first_data_sheet_id(doc).map(|id| id.as_str().to_owned());
    let first_renderable = || resolve_renderable_sheet_id(doc).map(|id| id.as_str().to_owned());

    match trimmed {
        "file.open" | "project.open" | "ui.file.open" => {
            WebCommandRoute::OpenFilePicker { accept: String::from(".curvebook,.cbtemplate,.pzfx,.prism,.xlsx,.csv,.tsv,.txt") }
        }
        "table.importCsv" | "table.importPlateMap" | "table.importTidyLong" => {
            WebCommandRoute::OpenFilePicker { accept: String::from(".csv,.tsv,.txt") }
        }
        "table.importPrism" | "table.importPzfx" => WebCommandRoute::OpenFilePicker { accept: String::from(".prism,.pzfx") },
        "table.importXlsx" => WebCommandRoute::OpenFilePicker { accept: String::from(".xlsx") },
        "project.instantiateTemplate" => WebCommandRoute::OpenFilePicker { accept: String::from(".cbtemplate,.curvebook") },
        "project.save" | "file.save" | "project.saveTemplate" => {
            WebCommandRoute::ExportFormat { format: WebExportFormat::CurveBookArchive, sheet_id: None }
        }
        "table.exportCsv" => WebCommandRoute::ExportFormat { format: WebExportFormat::Csv, sheet_id: first_data() },
        "export.svg" | "graph.exportSvg" => WebCommandRoute::ExportFormat { format: WebExportFormat::Svg, sheet_id: first_renderable() },
        "export.pdf" | "graph.exportPdf" => WebCommandRoute::ExportFormat { format: WebExportFormat::Pdf, sheet_id: first_renderable() },
        "export.png" | "graph.exportPng" => WebCommandRoute::ExportFormat { format: WebExportFormat::Png, sheet_id: first_renderable() },
        "export.tiff" => WebCommandRoute::ExportFormat { format: WebExportFormat::Tiff, sheet_id: first_renderable() },
        "file.print" => WebCommandRoute::PrintCurrentSheet,
        "ui.editParams.open" | "analysis.updateParams" => {
            if let Some(rs_id) = resolve_result_sheet_id(doc) {
                WebCommandRoute::OpenEditAnalysisParameters(rs_id.as_str().to_owned())
            } else {
                WebCommandRoute::NoticeOnly {
                    level: WebNoticeLevel::Info,
                    code: String::from("no_result_sheet"),
                    message: String::from("No ResultSheet exists yet; run an analysis first."),
                }
            }
        }
        "ui.event.dispatch" => WebCommandRoute::NoticeOnly {
            level: WebNoticeLevel::Info,
            code: String::from("automation_only"),
            message: String::from("ui.event.dispatch is an automation-only hook omitted from interactive chrome."),
        },
        "ui.result.applyRemediation" => WebCommandRoute::NoticeOnly {
            level: WebNoticeLevel::Info,
            code: String::from("remediation_diagnostic_only"),
            message: String::from("Diagnostic-only (QOL-7): inspect Result Diagnostic Tabs; original data and tests stay unchanged."),
        },
        "ui.shortcut.set" => WebCommandRoute::UiCommand { command_id: String::from("ui.palette.open"), params: json!({}) },
        other => resolve_mapped_ui_or_engine_route(app, other),
    }
}

fn resolve_mapped_ui_or_engine_route(app: &UiApp, cmd_id: &str) -> WebCommandRoute {
    if let Some(ui_cmd) = dialog_ui_command_for_engine_cmd(cmd_id) {
        return WebCommandRoute::UiCommand { command_id: ui_cmd.to_owned(), params: contextual_command_params(app, ui_cmd) };
    }
    if curvebook_ui_egui::UI_COMMANDS.iter().any(|s| s.id == cmd_id)
        || matches!(cmd_id, "ui.print.open" | "app.about" | "app.feedback" | "ui.palette.open" | "ui.editParams.open")
    {
        return WebCommandRoute::UiCommand { command_id: cmd_id.to_owned(), params: contextual_command_params(app, cmd_id) };
    }
    WebCommandRoute::EngineCommand { command_id: cmd_id.to_owned(), params: contextual_command_params(app, cmd_id) }
}

/// Commit the active modal dialog cleanly via `UiApp` state machines (never via `ui.answer_dialog`).
///
/// # Errors
/// Returns a descriptive error message when validation or engine execution fails.
pub fn commit_active_modal_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    host_routing::commit_active_modal_dialog(app, |app, hit_id| match resolve_web_command_route(app, hit_id) {
        WebCommandRoute::EngineCommand { command_id, params } => {
            let out = app.execute_command(&command_id, params).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome {
                handled: true,
                action: format!("palette.engine:{command_id}"),
                command_id: Some(command_id),
                command_result: Some(out.payload),
            })
        }
        WebCommandRoute::UiCommand { command_id, params } => {
            let ui_val = dispatch_ui_command(app, &command_id, &params)?.ok_or_else(|| format!("unhandled palette UI command `{command_id}`"))?;
            Ok(UiEventOutcome {
                handled: true,
                action: format!("palette.ui:{command_id}"),
                command_id: Some(command_id),
                command_result: Some(ui_val),
            })
        }
        WebCommandRoute::OpenEditAnalysisParameters(rs_id) => {
            let doc = app.document().clone();
            let found_id = doc
                .result_sheets
                .iter()
                .find(|s| s.id.as_str() == rs_id)
                .map(|s| s.id.clone())
                .ok_or_else(|| format!("result sheet `{rs_id}` not found"))?;
            if !app.canvas_dialogs.open_edit_parameters(&doc, &found_id) {
                return Err(format!("could not open EditAnalysisParameters for `{rs_id}`"));
            }
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.edit_analysis_parameters"),
                command_id: Some(String::from("ui.editParams.open")),
                command_result: Some(json!({ "sheet_id": rs_id })),
            })
        }
        WebCommandRoute::NoticeOnly { code, message, .. } => Ok(UiEventOutcome {
            handled: true,
            action: format!("palette.notice:{code}"),
            command_id: Some(hit_id.to_owned()),
            command_result: Some(json!({ "notice": message })),
        }),
        WebCommandRoute::OpenFilePicker { .. } | WebCommandRoute::ExportFormat { .. } | WebCommandRoute::PrintCurrentSheet => Ok(UiEventOutcome {
            handled: true,
            action: format!("palette.host:{hit_id}"),
            command_id: Some(hit_id.to_owned()),
            command_result: Some(json!({ "deferred_host_command": hit_id })),
        }),
    })
}

fn build_status_summary(app: &UiApp, bridge: &WebChromeBridge) -> String {
    let doc = app.document();
    let active_label = doc
        .active_sheet_id
        .as_ref()
        .and_then(|id| {
            doc.data_sheet(id)
                .map(|s| s.title.as_str())
                .or_else(|| doc.result_sheet(id).map(|s| s.title()))
                .or_else(|| doc.graph_sheet(id).map(|s| s.title.as_str()))
                .or_else(|| doc.layout_sheet(id).map(|s| s.title.as_str()))
                .or_else(|| doc.info_sheet(id).map(|s| s.title.as_str()))
        })
        .unwrap_or("Empty Project");
    let zoom_pct = (app.canvas_dialogs.pan_zoom.zoom * 100.0).round();
    let notice = bridge.last_notice.as_ref().map_or("", |(_, msg)| msg.as_str());
    if notice.is_empty() {
        format!("{active_label} | {zoom_pct:.0}% | Alt+K Palette | Alt+T Params/New Table")
    } else {
        format!("{active_label} | {zoom_pct:.0}% | {notice}")
    }
}

/// Build contextual right-click menu entries `(label, command_id, enabled)` for `"grid"`, `"navigator"`, or `"canvas"`.
#[must_use]
pub fn context_menu_items_for_target(target: &str, app: &UiApp) -> Vec<(String, String, bool)> {
    let has_data = !app.document().data_sheets.is_empty();
    let has_graph = !app.document().graph_sheets.is_empty() || !app.document().layout_sheets.is_empty();
    match target {
        "grid" => vec![
            (String::from("Clear Selected Cells"), String::from("table.clearCells"), has_data),
            (String::from("Exclude Values"), String::from("table.excludeCells"), has_data),
            (String::from("Exclude With Reason..."), String::from("ui.grid.excludeWithReason"), has_data),
            (String::from("Analyze..."), String::from("ui.analyze.open"), has_data),
        ],
        "navigator" => vec![
            (String::from("New Data Table"), String::from("sheet.createData"), true),
            (String::from("Duplicate Sheet"), String::from("sheet.duplicate"), has_data || has_graph),
            (String::from("Delete Sheet"), String::from("sheet.delete"), has_data || has_graph),
        ],
        _ => vec![
            (String::from("Format Graph..."), String::from("ui.graph.format.open"), has_graph),
            (String::from("Format Axes..."), String::from("ui.graph.axes.open"), has_graph),
            (String::from("Change Graph Type..."), String::from("ui.changeGraphType.open"), has_graph),
            (String::from("Export SVG..."), String::from("export.svg"), has_graph),
            (String::from("Export PDF..."), String::from("export.pdf"), has_graph),
            (String::from("Export PNG..."), String::from("export.png"), has_graph),
        ],
    }
}

/// Inspect active table archetype label for diagnostics.
#[must_use]
pub fn active_table_archetype_label(app: &UiApp) -> Option<String> {
    let doc = app.document();
    let ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id))?;
    Some(format!("{:?}", ds.archetype.kind()))
}

/// Verify all registered engine & UI commands that carry shortcuts have conflict-free Web mappings.
#[must_use]
pub fn all_shortcuts_have_web_policy() -> bool {
    for spec in cmd::registry().specs() {
        if !spec.shortcut.is_empty() && WebShortcutRouter::policy_for_command(spec.id).is_none() && spec.shortcut != "Cmd+V" {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use curvebook_ui_egui::{KeyModifiers, PointerButton, UiApp, ViewportMetrics};

    use super::*;

    fn click_node_press_release(bridge: &mut WebChromeBridge, app: &mut UiApp, viewport: ViewportMetrics, node_id: &str) -> WebChromeDispatch {
        let layout = bridge.build_layout(app, viewport);
        let rect = layout.find_node(node_id).unwrap_or_else(|| panic!("node `{node_id}` must exist")).rect_px;
        let lx = f64::from(rect.x + 4) / viewport.scale_factor;
        let ly = f64::from(rect.y + 4) / viewport.scale_factor;
        let down = bridge.handle_pointer_down(app, &layout, [lx, ly], [rect.x + 4, rect.y + 4], PointerButton::Primary);
        assert!(matches!(down, WebChromeDispatch::HandledRedraw { .. }));
        bridge.handle_pointer_up(app, &layout, [lx, ly], PointerButton::Primary)
    }

    #[test]
    fn browser_reserved_shortcuts_map_cleanly_without_clashes() {
        let mut app = UiApp::new();
        app.execute_command("sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "title": "Groups" })).unwrap();
        for row in 0..3 {
            app.execute_command("table.setCell", json!({ "sheet_id": "d1", "row": row, "col": 0, "value": 10.0 + row as f64 })).unwrap();
            app.execute_command("table.setCell", json!({ "sheet_id": "d1", "row": row, "col": 1, "value": 20.0 + row as f64 })).unwrap();
        }
        app.execute_command("analysis.run", json!({ "source_sheet_id": "d1", "kind": "descriptive_stats" })).unwrap();

        let close_route = WebShortcutRouter::route_key_down(&app, "w", KeyModifiers { alt: true, ..Default::default() });
        assert!(close_route.handled && close_route.prevent_default && close_route.used_web_alias);
        assert_eq!(close_route.action, WebShortcutAction::CloseWorkspacePrompt);

        let new_route = WebShortcutRouter::route_key_down(&app, "n", KeyModifiers { alt: true, shift: true, ..Default::default() });
        assert!(new_route.handled && new_route.used_web_alias);

        let pal_route = WebShortcutRouter::route_key_down(&app, "k", KeyModifiers { alt: true, ..Default::default() });
        assert_eq!(pal_route.action, WebShortcutAction::ExecuteCommand { command_id: String::from("ui.palette.open"), params: json!({}) });

        let rs_id = app.document().result_sheets[0].id.clone();
        app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).unwrap();
        let t_route = WebShortcutRouter::route_key_down(&app, "t", KeyModifiers { alt: true, ..Default::default() });
        assert_eq!(t_route.action, WebShortcutAction::OpenEditAnalysisParameters(rs_id.as_str().to_owned()));

        assert!(all_shortcuts_have_web_policy());
    }

    #[test]
    fn modal_backdrop_blocks_workspace_click_through_and_dropdown_routes_placeholder() {
        let mut app = UiApp::new();
        let mut bridge = WebChromeBridge::new();
        let viewport = ViewportMetrics::new(1024.0, 640.0, 1.0);

        let disp = click_node_press_release(&mut bridge, &mut app, viewport, "menu.file");
        assert_eq!(disp, WebChromeDispatch::InvokeChromeAction(WebChromeAction::ToggleMenuSection(String::from("menu.file"))));
        bridge.open_menu_key = Some(String::from("menu.file"));
        app.interaction.open_menu = Some(String::from("menu.file"));

        let open_layout = bridge.build_layout(&app, viewport);
        assert!(open_layout.find_node("popup.menu.file").is_some());

        app.canvas_dialogs.open_command_palette();
        bridge.close_popups();
        app.interaction.open_menu = None;
        let layout2 = bridge.build_layout(&app, viewport);
        assert!(layout2.find_node("modal.backdrop").is_some());
        assert!(layout2.find_node("palette.query").is_some());
        let hit_corner = bridge.handle_pointer_down(&mut app, &layout2, [5.0, 5.0], [5, 5], PointerButton::Primary);
        assert_eq!(hit_corner, WebChromeDispatch::BlockedByModalBackdrop);
    }

    #[test]
    fn w6_web_commit_active_modal_dialog_commits_all_supported_modals() -> Result<(), String> {
        let mut app = UiApp::new();
        let ds_g = app
            .execute_command(
                "sheet.createData",
                json!({ "sheet_id": "ds-grp", "kind": "grouped", "rows": 3, "cols": 2, "replicates": 2, "auto_family": false }),
            )
            .map_err(|e| e.to_string())?;
        let grp_id = curvebook_ui_egui::serde_json::Value::as_str(&ds_g.payload["sheet_id"]).ok_or("missing grp id")?.to_owned();
        let mut grp_edits = Vec::new();
        for (r, vals) in [[[10.0, 11.0], [20.0, 21.0]], [[15.0, 16.0], [25.0, 26.0]], [[12.0, 13.0], [22.0, 23.0]]].iter().enumerate() {
            for (c, subs) in vals.iter().enumerate() {
                for (s, v) in subs.iter().enumerate() {
                    grp_edits.push(json!({ "row": r, "col": c, "subcol": s, "value": v }));
                }
            }
        }
        app.execute_command("table.setCells", json!({ "sheet_id": grp_id, "edits": grp_edits })).map_err(|e| e.to_string())?;
        let g1 = app.execute_command("graph.create", json!({ "data_sheet_id": grp_id, "kind": "grouped" })).map_err(|e| e.to_string())?;
        let g1_str = g1.payload["sheet_id"].as_str().ok_or("missing g1")?.to_owned();
        let g1_id = app.document().graph_sheets.iter().find(|g| g.id.as_str() == g1_str).ok_or("missing g1 sheet")?.id.clone();
        let g2 = app.execute_command("graph.create", json!({ "data_sheet_id": grp_id, "kind": "grouped" })).map_err(|e| e.to_string())?;
        let g2_str = g2.payload["sheet_id"].as_str().ok_or("missing g2")?.to_owned();
        let g2_id = app.document().graph_sheets.iter().find(|g| g.id.as_str() == g2_str).ok_or("missing g2 sheet")?.id.clone();

        // 1. ChangeGraphType -> grouped.superimposed
        let g1_ref = app.document().graph_sheet(&g1_id).ok_or("missing g1")?.clone();
        let doc = app.document().clone();
        app.canvas_dialogs.change_graph_type.open_for_graph(&g1_ref, &doc);
        app.canvas_dialogs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: g1_id.clone() };
        app.canvas_dialogs.change_graph_type.selected_option_id = String::from("grouped.superimposed");
        let out = commit_active_modal_dialog(&mut app)?;
        assert_eq!(out.action, "dialog.change_graph_type_committed");
        assert_eq!(app.document().graph_sheet(&g1_id).map(|g| format!("{:?}", g.spec.grouped_arrangement)), Some(String::from("Superimposed")));

        // 2. FormatAxes -> sync axes.title & axes.y_thresholds buffers and commit
        let _ = dispatch_ui_command(&mut app, "ui.graph.axes.open", &json!({ "sheet_id": g1_str })).map_err(|e| e.to_string())?;
        app.focus_widget_id = Some(String::from("axes.title"));
        let _ = curvebook_ui_egui::interaction::dispatch_ui_event(
            &mut app,
            UiInputEvent::TextInput { text: String::from("Web Intensity") },
            ViewportMetrics::default(),
        );
        app.focus_widget_id = Some(String::from("axes.y_thresholds"));
        let _ = curvebook_ui_egui::interaction::dispatch_ui_event(
            &mut app,
            UiInputEvent::TextInput { text: String::from("15") },
            ViewportMetrics::default(),
        );
        let out = commit_active_modal_dialog(&mut app)?;
        assert_eq!(out.action, "dialog.format_axes_committed");
        let gs1 = app.document().graph_sheet(&g1_id).ok_or("missing g1")?;
        assert_eq!(gs1.spec.y_axis.title, "Web Intensity");
        assert!(gs1.spec.y_axis.thresholds.iter().any(|t| (t.value - 15.0).abs() < 1e-6));

        // 3. FormatGraph & ApplyStyleFrom & PairwiseComparisons & FormatSelectedPoints
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &g1_id));
        app.canvas_dialogs.format_graph.show_legend = false;
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.format_graph_committed");
        assert_eq!(app.document().graph_sheet(&g1_id).map(|g| g.spec.legend.visible), Some(false));

        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_apply_style_from(&doc, &g2_id));
        app.canvas_dialogs.apply_style.donor_graph_id = Some(g1_id.clone());
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.apply_style_committed");

        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_pairwise_comparisons(&doc, &g1_id));
        app.canvas_dialogs.pairwise.hide_non_significant = true;
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.pairwise_committed");

        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_selected_points(&doc, &g1_id, vec![(0, 0)]));
        app.canvas_dialogs.format_points.color = Some(String::from("#D55E00"));
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.format_points_committed");
        assert!(!app.document().graph_sheet(&g1_id).ok_or("missing g1")?.spec.point_overrides.is_empty());

        // 4. PageLayoutComposer & ChangeDataAnalyzed & BubblePlotInspector & InsertModelEquation
        let lay = app.execute_command("layout.create", json!({ "title": "Pub Fig" })).map_err(|e| e.to_string())?;
        let lay_id = lay.payload["sheet_id"].as_str().ok_or("missing layout")?.into();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_page_layout_composer(&doc, &lay_id));
        app.canvas_dialogs.page_layout.rows = 2;
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.page_layout_committed");

        let an = app.execute_command("analysis.run", json!({ "source_sheet_id": "ds-grp", "kind": "two_way_anova" })).map_err(|e| e.to_string())?;
        let an_id = an.payload["sheet_id"].as_str().ok_or("missing an")?.into();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_change_data_analyzed(&doc, &an_id));
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.change_data_committed");

        // 5. MV sheet -> BubblePlotInspector & XY sheet -> InsertModelEquation
        let ds_mv = app
            .execute_command(
                "sheet.createData",
                json!({ "sheet_id": "ds-mv", "kind": "multiple_variables", "rows": 4, "cols": 2, "auto_family": false }),
            )
            .map_err(|e| e.to_string())?;
        let mv_id = ds_mv.payload["sheet_id"].as_str().ok_or("missing mv")?.to_owned();
        app.execute_command(
            "table.setCells",
            json!({
                "sheet_id": mv_id,
                "edits": [
                    { "row": 0, "col": 0, "value": 1.0 }, { "row": 0, "col": 1, "value": 2.0 },
                    { "row": 1, "col": 0, "value": 2.0 }, { "row": 1, "col": 1, "value": 4.0 },
                    { "row": 2, "col": 0, "value": 3.0 }, { "row": 2, "col": 1, "value": 6.0 },
                    { "row": 3, "col": 0, "value": 4.0 }, { "row": 3, "col": 1, "value": 8.0 }
                ]
            }),
        )
        .map_err(|e| e.to_string())?;
        let gm = app.execute_command("graph.create", json!({ "data_sheet_id": mv_id, "kind": "multiple_variables" })).map_err(|e| e.to_string())?;
        let gm_str = gm.payload["sheet_id"].as_str().ok_or("missing gm")?.to_owned();
        let gm_id = app.document().graph_sheets.iter().find(|g| g.id.as_str() == gm_str).ok_or("missing gm sheet")?.id.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_bubble_plot(&doc, &gm_id));
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.bubble_plot_committed");

        let fit = app.execute_command("analysis.run", json!({ "source_sheet_id": mv_id, "kind": "linear_regression" })).map_err(|e| e.to_string())?;
        let fit_str = fit.payload["sheet_id"].as_str().ok_or("missing fit")?.to_owned();
        let fit_id = app.document().result_sheets.iter().find(|r| r.id.as_str() == fit_str).ok_or("missing fit sheet")?.id.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_insert_model_equation(&doc, &gm_id));
        app.canvas_dialogs.insert_model_equation.result_sheet_id = Some(fit_id);
        assert_eq!(commit_active_modal_dialog(&mut app)?.action, "dialog.insert_model_equation_committed");
        Ok(())
    }
}
