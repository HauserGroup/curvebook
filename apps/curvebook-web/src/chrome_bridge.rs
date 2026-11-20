//! Minimal strict-CSP web chrome placeholder (`R10.7` / `R15.5`): command/shortcut routing,
//! modal commits, context menu projection, and semantic placeholder nodes until post-Beta `R11.11`.
#![forbid(unsafe_code)]

use curvebook_engine::cmd;
use curvebook_ui_egui::menus::normalize_shortcut_combo;
use curvebook_ui_egui::serde_json::{Value, json};
use curvebook_ui_egui::{
    ActiveDialog, KeyModifiers, PointerButton, UiApp, UiEventOutcome, UiInputEvent, ViewportMetrics, dispatch_gui_action, dispatch_ui_command,
    menu_bar_tooltips, menus::toolbar_tooltips,
};

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
        command_id: "ui.presentation.toggle",
        desktop_combo: "Cmd+Shift+P",
        web_combo: "Alt+Shift+P",
        browser_reserved: true,
        prevent_default_on_canvas: true,
        reservation_reason: Some("Browsers reserve Cmd/Ctrl+Shift+P for system print"),
        menu_fallback_path: "View/Toggle Presentation Mode",
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
            ("p", true) => ("ui.presentation.toggle", "Alt+Shift+P mapped from browser-reserved Cmd/Ctrl+Shift+P"),
            ("l", false) => ("sheet.freeze", "Alt+L mapped from browser-reserved Ctrl+L"),
            ("k", false) => ("ui.palette.open", "Alt+K opens Command Palette without browser address-bar clash"),
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

/// Accessible summary of current Web chrome state mirrored to `RenderRgbaFrame.chrome`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebChromeSnapshot {
    pub menu_bar_visible: bool,
    pub open_menu_key: Option<String>,
    pub context_menu_open: bool,
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
        json!({
            "menu_bar_visible": self.menu_bar_visible,
            "open_menu_key": self.open_menu_key,
            "context_menu_open": self.context_menu_open,
            "active_dialog": self.active_dialog,
            "hovered_node_id": self.hovered_node_id,
            "focused_node_id": self.focused_node_id,
            "ime_preedit": self.ime_preedit,
            "drag_active": self.drag_active,
            "node_count": self.node_count,
            "menu_titles": self.menu_titles,
            "toolbar_ids": self.toolbar_ids,
            "status_summary": self.status_summary,
            "dirty": self.dirty,
            "a11y_tree": self.a11y_tree,
        })
    }

    /// Decode a [`WebChromeSnapshot`] from JSON, tolerating missing optional fields.
    #[must_use]
    pub fn from_json_value(value: &Value) -> Self {
        let Some(obj) = value.as_object() else {
            return Self::default();
        };
        let str_list = |key: &str| -> Vec<String> {
            obj.get(key).and_then(Value::as_array).map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
        };
        Self {
            menu_bar_visible: obj.get("menu_bar_visible").and_then(Value::as_bool).unwrap_or(false),
            open_menu_key: obj.get("open_menu_key").and_then(Value::as_str).map(str::to_owned),
            context_menu_open: obj.get("context_menu_open").and_then(Value::as_bool).unwrap_or(false),
            active_dialog: obj.get("active_dialog").and_then(Value::as_str).unwrap_or("none").to_owned(),
            hovered_node_id: obj.get("hovered_node_id").and_then(Value::as_str).map(str::to_owned),
            focused_node_id: obj.get("focused_node_id").and_then(Value::as_str).map(str::to_owned),
            ime_preedit: obj.get("ime_preedit").and_then(Value::as_str).map(str::to_owned),
            drag_active: obj.get("drag_active").and_then(Value::as_bool).unwrap_or(false),
            node_count: usize::try_from(obj.get("node_count").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0),
            menu_titles: str_list("menu_titles"),
            toolbar_ids: str_list("toolbar_ids"),
            status_summary: obj.get("status_summary").and_then(Value::as_str).unwrap_or_default().to_owned(),
            dirty: obj.get("dirty").and_then(Value::as_bool).unwrap_or(false),
            a11y_tree: obj.get("a11y_tree").filter(|v| !v.is_null()).cloned().map(Box::new),
        }
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
        self.context_menu = None;
    }

    /// Synchronize mirrored chrome state fields from [`UiApp`].
    pub fn sync_from_app(&mut self, app: &UiApp) {
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
                #[allow(clippy::cast_possible_truncation)]
                let row_h = 14i32;
                let pop_h = u32::try_from(items.len()).unwrap_or(1).saturating_mul(14).saturating_add(8).max(28);
                let pop_rect = LayoutRectPx::new(mx, 28, 220, pop_h);
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
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let iy = 32 + (idx as i32) * row_h;
                    let item_rect = LayoutRectPx::new(mx + 4, iy, 212, 12);
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
            let enabled = btn.tooltip.enabled || btn.button.id == "toolbar.axes";
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
            if app.canvas_dialogs.active_dialog == ActiveDialog::CommandPalette {
                let qrect = LayoutRectPx::new(dx + 16, dy + 44, dw.saturating_sub(32), 28);
                nodes.push(WebChromeNode {
                    id: String::from("palette.query"),
                    role: WebChromeRole::ModalInput,
                    z_layer: ZLayer::ModalDialog,
                    paint_order: next_order(),
                    rect_px: qrect,
                    clip_px: qrect,
                    label: app.canvas_dialogs.command_palette.query.clone(),
                    shortcut: None,
                    enabled: true,
                    disabled_reason: None,
                    tooltip: None,
                    action: WebChromeAction::FocusModalInput,
                });
            }
        }

        if let Some(ctx) = &self.context_menu {
            let pop_rect = LayoutRectPx::new(ctx.anchor_px[0], ctx.anchor_px[1], 200, 120);
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

        let focused_id = app.focus_widget_id.clone().or_else(|| self.focused_node_id.clone());
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
            a11y_tree: Some(Box::new(json!({
                "nodes": nodes.iter().map(|n| json!({ "id": n.id, "label": n.label, "enabled": n.enabled })).collect::<Vec<_>>(),
            }))),
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

    /// Route mouse-wheel / trackpad scroll over placeholder layout and active sheet.
    pub fn route_wheel(
        &self,
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
            return WebChromeDispatch::HandledRedraw { action: String::from("popup.wheel_ignored"), command_result: None };
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
        if matches!(key, "Enter" | "Return") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
            return WebChromeDispatch::InvokeChromeAction(WebChromeAction::DialogConfirm);
        }
        match &app.canvas_dialogs.active_dialog {
            ActiveDialog::CommandPalette => {
                let hits_len = app.canvas_dialogs.command_palette.matches_for_session(app.session()).len();
                match key {
                    "Backspace" => {
                        app.canvas_dialogs.command_palette.query.pop();
                        app.canvas_dialogs.command_palette.selected_index = 0;
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
                    _ if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && key.chars().count() == 1 => {
                        app.canvas_dialogs.command_palette.query.push_str(key);
                        app.canvas_dialogs.command_palette.selected_index = 0;
                        WebChromeDispatch::HandledRedraw { action: String::from("palette.type"), command_result: None }
                    }
                    _ => WebChromeDispatch::BlockedByModalBackdrop,
                }
            }
            ActiveDialog::AnalyzeSheet { .. } => {
                let hits_len = app.canvas_dialogs.analyze.compatible_entries().len();
                match key {
                    "Backspace" => {
                        app.canvas_dialogs.analyze.search_query.pop();
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.backspace"), command_result: None }
                    }
                    "ArrowDown" | "Down" if hits_len > 0 => {
                        let entries = app.canvas_dialogs.analyze.compatible_entries();
                        let cur = entries.iter().position(|e| Some(e.id) == app.canvas_dialogs.analyze.selected_entry_id.as_deref()).unwrap_or(0);
                        let next = (cur + 1).min(entries.len() - 1);
                        app.canvas_dialogs.analyze.selected_entry_id = Some(entries[next].id.to_owned());
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.next"), command_result: None }
                    }
                    "ArrowUp" | "Up" if hits_len > 0 => {
                        let entries = app.canvas_dialogs.analyze.compatible_entries();
                        let cur = entries.iter().position(|e| Some(e.id) == app.canvas_dialogs.analyze.selected_entry_id.as_deref()).unwrap_or(0);
                        let prev = cur.saturating_sub(1);
                        app.canvas_dialogs.analyze.selected_entry_id = Some(entries[prev].id.to_owned());
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.prev"), command_result: None }
                    }
                    _ if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && key.chars().count() == 1 => {
                        app.canvas_dialogs.analyze.search_query.push_str(key);
                        if let Some(first) = app.canvas_dialogs.analyze.compatible_entries().first() {
                            app.canvas_dialogs.analyze.selected_entry_id = Some(first.id.to_owned());
                        }
                        WebChromeDispatch::HandledRedraw { action: String::from("analyze.type"), command_result: None }
                    }
                    _ => WebChromeDispatch::BlockedByModalBackdrop,
                }
            }
            ActiveDialog::ExcludeWithReason { .. } => match key {
                "Backspace" => {
                    app.canvas_dialogs.exclude_with_reason.note.pop();
                    WebChromeDispatch::HandledRedraw { action: String::from("exclude.backspace"), command_result: None }
                }
                _ if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && key.chars().count() == 1 => {
                    app.canvas_dialogs.exclude_with_reason.note.push_str(key);
                    WebChromeDispatch::HandledRedraw { action: String::from("exclude.type"), command_result: None }
                }
                _ => WebChromeDispatch::BlockedByModalBackdrop,
            },
            _ => WebChromeDispatch::BlockedByModalBackdrop,
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
                WebChromeDispatch::HandledRedraw { action: String::from("palette.ime_commit"), command_result: None }
            }
            ActiveDialog::AnalyzeSheet { .. } => {
                app.canvas_dialogs.analyze.search_query.push_str(text);
                if let Some(first) = app.canvas_dialogs.analyze.compatible_entries().first() {
                    app.canvas_dialogs.analyze.selected_entry_id = Some(first.id.to_owned());
                }
                WebChromeDispatch::HandledRedraw { action: String::from("analyze.ime_commit"), command_result: None }
            }
            ActiveDialog::ExcludeWithReason { .. } => {
                app.canvas_dialogs.exclude_with_reason.note.push_str(text);
                WebChromeDispatch::HandledRedraw { action: String::from("exclude.ime_commit"), command_result: None }
            }
            _ => WebChromeDispatch::PassToWorkspace(UiInputEvent::TextInput { text: text.to_owned() }),
        }
    }
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
            Self::OpenFilePicker { .. } | Self::OpenEditAnalysisParameters(_) => false,
            Self::ExportFormat { .. } | Self::PrintCurrentSheet | Self::NoticeOnly { .. } => true,
            Self::UiCommand { command_id, .. } => !matches!(command_id.as_str(), "ui.grid.excludeWithReason" | "ui.result.appendClusterColumn"),
            Self::EngineCommand { command_id, .. } => cmd::registry().get(command_id).is_some_and(|spec| spec.effects.read_only_hint()),
        }
    }
}

/// Resolve any toolbar, dropdown, palette, or shortcut command ID (including `:variant` suffixes) cleanly.
#[must_use]
pub fn resolve_web_command_route(app: &UiApp, raw_command_id: &str) -> WebCommandRoute {
    let trimmed = raw_command_id.trim();
    if let Some((base, variant)) = trimmed.split_once(':') {
        let active_render = first_renderable_sheet_id(app);
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

    match trimmed {
        "file.open" | "project.open" | "ui.file.open" => {
            WebCommandRoute::OpenFilePicker { accept: String::from(".curvebook,.cbtemplate,.pzfx,.prism,.csv,.tsv,.txt") }
        }
        "table.importCsv" | "table.importPlateMap" | "table.importTidyLong" => {
            WebCommandRoute::OpenFilePicker { accept: String::from(".csv,.tsv,.txt") }
        }
        "table.importPrism" | "table.importPzfx" => WebCommandRoute::OpenFilePicker { accept: String::from(".prism,.pzfx") },
        "project.instantiateTemplate" => WebCommandRoute::OpenFilePicker { accept: String::from(".cbtemplate,.curvebook") },
        "project.save" | "file.save" | "project.saveTemplate" => {
            WebCommandRoute::ExportFormat { format: WebExportFormat::CurveBookArchive, sheet_id: None }
        }
        "table.exportCsv" => WebCommandRoute::ExportFormat { format: WebExportFormat::Csv, sheet_id: first_data_sheet_id(app) },
        "export.svg" | "graph.exportSvg" => WebCommandRoute::ExportFormat { format: WebExportFormat::Svg, sheet_id: first_renderable_sheet_id(app) },
        "export.pdf" | "graph.exportPdf" => WebCommandRoute::ExportFormat { format: WebExportFormat::Pdf, sheet_id: first_renderable_sheet_id(app) },
        "export.png" | "graph.exportPng" => WebCommandRoute::ExportFormat { format: WebExportFormat::Png, sheet_id: first_renderable_sheet_id(app) },
        "export.tiff" => WebCommandRoute::ExportFormat { format: WebExportFormat::Tiff, sheet_id: first_renderable_sheet_id(app) },
        "file.print" => WebCommandRoute::PrintCurrentSheet,
        "ui.editParams.open" | "analysis.updateParams" => {
            if let Some(rs_id) = active_or_first_result_sheet_id(app) {
                WebCommandRoute::OpenEditAnalysisParameters(rs_id)
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
    let dialog_route = match cmd_id {
        "toolbar.axes" | "graph.setAxes" => Some("ui.graph.axes.open"),
        "graph.setKind" => Some("ui.changeGraphType.open"),
        "graph.setSpec" | "graph.setSeries" | "graph.setAdditionalTicks" | "graph.setRegressionOverlay" | "graph.applyStyleFrom" => {
            Some("ui.graph.format.open")
        }
        "graph.formatSelectedPoints" => Some("ui.formatPoints.open"),
        "graph.addComparisonBrackets" | "graph.setCompactLetterDisplay" => Some("ui.graph.pairwise.open"),
        "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => Some("ui.canvas.applyPreset"),
        "analysis.run" | "stats.run" | "fit.run" | "transform.run" | "analysis.add" | "stats.pvalueStack" => Some("ui.analyze.open"),
        "analysis.retargetData" => Some("ui.analyze.open"),
        "table.convertArchetype" => Some("ui.table.convertArchetype.open"),
        "table.excludeCellsWithReason" => Some("ui.grid.excludeWithReason"),
        "table.randomizeGroups" => Some("ui.table.randomizeGroups"),
        "table.randomSampleRows" => Some("ui.table.randomSampleRows"),
        "table.simulateDistributions" => Some("ui.table.simulateDistributions"),
        "table.setCalculatedVariable" => Some("ui.table.openFormulaBar"),
        "table.pasteLink" => Some("ui.import.reshape.open"),
        "sheet.clonePipeline" => Some("ui.clonePipeline.open"),
        "layout.setSpec" | "layout.addPanel" => Some("ui.layout.composer.open"),
        _ => None,
    };
    if let Some(ui_cmd) = dialog_route {
        return WebCommandRoute::UiCommand { command_id: ui_cmd.to_owned(), params: contextual_command_params(app, ui_cmd) };
    }
    if curvebook_ui_egui::UI_COMMANDS.iter().any(|s| s.id == cmd_id)
        || matches!(cmd_id, "ui.print.open" | "app.about" | "ui.palette.open" | "ui.editParams.open")
    {
        return WebCommandRoute::UiCommand { command_id: cmd_id.to_owned(), params: contextual_command_params(app, cmd_id) };
    }
    WebCommandRoute::EngineCommand { command_id: cmd_id.to_owned(), params: contextual_command_params(app, cmd_id) }
}

fn first_data_sheet_id(app: &UiApp) -> Option<String> {
    let doc = app.document();
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.data_sheet(id).is_some())
        .or_else(|| doc.data_sheets.first().map(|s| &s.id))
        .map(|id| id.as_str().to_owned())
}

fn first_renderable_sheet_id(app: &UiApp) -> Option<String> {
    let doc = app.document();
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some())
        .or_else(|| doc.graph_sheets.first().map(|s| &s.id))
        .or_else(|| doc.layout_sheets.first().map(|s| &s.id))
        .map(|id| id.as_str().to_owned())
}

fn active_or_first_result_sheet_id(app: &UiApp) -> Option<String> {
    let doc = app.document();
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.result_sheet(id).is_some())
        .or_else(|| doc.result_sheets.first().map(|s| &s.id))
        .map(|id| id.as_str().to_owned())
}

fn next_cycle_theme_mode(app: &UiApp) -> &'static str {
    let lum = app.theme.surface_bg.relative_luminance();
    if lum > 0.5 {
        "dark"
    } else if app.theme.border.r >= 240 && app.theme.border.g >= 240 {
        "light"
    } else {
        "high_contrast"
    }
}

fn next_cycle_preview_filter(filter: curvebook_ui_egui::CanvasPreviewFilter) -> &'static str {
    use curvebook_ui_egui::CanvasPreviewFilter;
    match filter {
        CanvasPreviewFilter::Normal => "deuteranopia",
        CanvasPreviewFilter::Deuteranopia => "protanopia",
        CanvasPreviewFilter::Protanopia => "tritanopia",
        CanvasPreviewFilter::Tritanopia => "grayscale",
        CanvasPreviewFilter::GrayscalePrint => "normal",
    }
}

/// Build contextual default parameters for menu/toolbar/palette commands (always returns a JSON object, never `Value::Null`).
#[must_use]
pub fn contextual_command_params(app: &UiApp, command_id: &str) -> Value {
    let doc = app.document();
    let active_sheet = doc.active_sheet_id.as_ref().map(|id| id.as_str());
    let active_data = first_data_sheet_id(app);
    let active_graph = doc
        .active_sheet_id
        .as_ref()
        .filter(|id| doc.graph_sheet(id).is_some())
        .or_else(|| doc.graph_sheets.first().map(|s| &s.id))
        .map(|id| id.as_str().to_owned());
    let active_layout = doc
        .active_sheet_id
        .as_ref()
        .filter(|id| doc.layout_sheet(id).is_some())
        .or_else(|| doc.layout_sheets.first().map(|s| &s.id))
        .map(|id| id.as_str().to_owned());
    let active_render = first_renderable_sheet_id(app);
    let active_result = active_or_first_result_sheet_id(app);

    match command_id {
        "sheet.createData" => json!({ "kind": "xy", "title": "Data Table", "auto_family": true }),
        "sheet.rename" => json!({ "sheet_id": active_sheet, "title": "Renamed Sheet" }),
        "sheet.duplicate" | "sheet.duplicateFamily" | "sheet.clonePipeline" | "sheet.delete" | "sheet.freeze" | "sheet.unfreeze" | "sheet.select" => {
            json!({ "sheet_id": active_sheet })
        }
        "sheet.extractResultTab" => json!({ "result_sheet_id": active_result }),
        "graph.create" => json!({ "source_sheet_id": active_data, "title": "Graph" }),
        "graph.initialize" | "graph.preflightCheck" | "graph.setAxes" | "graph.setKind" | "graph.setSpec" | "graph.setSeries" => {
            json!({ "sheet_id": active_graph })
        }
        "graph.addAnnotation" => json!({ "sheet_id": active_graph, "text": "Annotation", "x_pt": 72.0, "y_pt": 72.0 }),
        "graph.applyStyleFrom" => json!({
            "target_sheet_id": active_graph,
            "source_sheet_id": doc.graph_sheets.first().map(|s| s.id.as_str()),
        }),
        "graph.applyCanvasPreset" | "ui.canvas.applyPreset" => json!({ "sheet_id": active_render, "preset": "single_column_88mm" }),
        "layout.create" => json!({ "title": "Layout" }),
        "layout.alignOrigins" => json!({ "sheet_id": active_layout, "align": true, "normalize_bounds": true }),
        "layout.applyCanvasPreset" => json!({ "sheet_id": active_layout, "preset": "double_column_180mm" }),
        "layout.addPanel" | "layout.setSpec" => json!({ "sheet_id": active_layout }),
        "table.clearCells" | "table.excludeCells" => {
            let coords = app.grid.selected_coords.iter().map(|c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol })).collect::<Vec<_>>();
            if coords.is_empty() {
                if let Some(c) = app.grid.tool.active {
                    json!({ "sheet_id": active_data, "cells": [{ "row": c.row, "col": c.col, "subcol": c.subcol }] })
                } else {
                    json!({ "sheet_id": active_data })
                }
            } else {
                json!({ "sheet_id": active_data, "cells": coords })
            }
        }
        "table.exclusionAuditReport" | "table.exportCsv" => json!({ "sheet_id": active_data }),
        "export.svg" | "graph.exportSvg" | "export.pdf" | "graph.exportPdf" | "export.png" | "graph.exportPng" | "export.tiff"
        | "graph.exportEmf" | "layout.exportEmf" | "ui.print.open" | "file.print" => json!({ "sheet_id": active_render }),
        "analysis.run" | "stats.run" | "fit.run" | "transform.run" | "analysis.add" => {
            json!({ "source_sheet_id": active_data, "kind": "descriptive_stats" })
        }
        "analysis.exportJson"
        | "analysis.exportCsv"
        | "ui.result.copyMethodsReport"
        | "ui.result.inspectDiagnostics"
        | "ui.result.appendClusterColumn" => json!({ "sheet_id": active_result }),
        "result.generateReport" => json!({ "result_sheet_id": active_result, "format": "markdown" }),
        "project.createCheckpoint" => json!({ "label": "Checkpoint" }),
        "ui.theme.set" => json!({ "mode": next_cycle_theme_mode(app) }),
        "ui.locale.set" => json!({ "locale": if app.locale == "en" { "de" } else { "en" } }),
        "ui.previewFilter.set" => json!({ "filter": next_cycle_preview_filter(app.preview_filter) }),
        "ui.brush.point" => {
            if let (Some(d), Some(c)) = (active_data, app.grid.tool.active) {
                json!({ "sheet_id": d, "row": c.row, "col": c.col, "subcol": c.subcol })
            } else {
                json!({ "clear": true })
            }
        }
        _ => json!({}),
    }
}

/// Commit the active modal dialog cleanly via `UiApp` state machines (never via `ui.answer_dialog`).
///
/// # Errors
/// Returns a descriptive error message when validation or engine execution fails.
pub fn commit_active_modal_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    let dialog = app.canvas_dialogs.active_dialog.clone();
    match dialog {
        ActiveDialog::None => Ok(UiEventOutcome { handled: false, action: String::from("dialog.none"), ..Default::default() }),
        ActiveDialog::AnalyzeSheet { .. } => {
            let intent = match app.canvas_dialogs.analyze.try_build_run_intent(app.document()) {
                Ok(intent) => intent,
                Err(existing_ids) => {
                    let entry_id = app.canvas_dialogs.analyze.selected_entry_id.clone().unwrap_or_default();
                    let Some(src) =
                        app.canvas_dialogs.analyze.source_sheet_id.clone().or_else(|| app.document().data_sheets.first().map(|ds| ds.id.clone()))
                    else {
                        return Err(String::from("no source data sheet selected"));
                    };
                    app.canvas_dialogs.active_dialog =
                        ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id: src, existing_result_ids: existing_ids, pending_entry_id: entry_id };
                    return Ok(UiEventOutcome { handled: true, action: String::from("dialog.duplicate_prompt"), ..Default::default() });
                }
            };
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.analyze_committed"),
                command_id: Some(intent.command_id),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::DuplicateAnalysisPrompt { .. } => {
            let intent = app
                .canvas_dialogs
                .analyze
                .build_create_new_intent()
                .ok_or_else(|| String::from("no selected analysis for duplicate confirmation"))?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.duplicate_created_new"),
                command_id: Some(intent.command_id),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::EditAnalysisParameters { .. } => {
            curvebook_ui_egui::interaction::sync_edit_params_text_buffers(app);
            let doc = app.document().clone();
            let intents = app.canvas_dialogs.edit_params.commit_intents(Some(&doc));
            let mut iter = intents.into_iter();
            let primary = iter.next().ok_or_else(|| String::from("no draft parameters ready to commit"))?;
            let primary_cmd = primary.command_id.clone();
            let out = app.execute_command(&primary.command_id, primary.params).map_err(|e| e.to_string())?;
            for extra in iter {
                let _ = app.execute_command(&extra.command_id, extra.params).map_err(|e| e.to_string())?;
            }
            app.canvas_dialogs.close_dialog();
            curvebook_ui_egui::interaction::clear_modal_text_buffers(app);
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.edit_params_committed"),
                command_id: Some(primary_cmd),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::CommandPalette => {
            let hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
            let selected = hits.get(app.canvas_dialogs.command_palette.selected_index).or_else(|| hits.first()).cloned();
            app.canvas_dialogs.close_dialog();
            let Some(hit) = selected else {
                return Ok(UiEventOutcome { handled: true, action: String::from("palette.no_match"), ..Default::default() });
            };
            match resolve_web_command_route(app, hit.id) {
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
                    let ui_val =
                        dispatch_ui_command(app, &command_id, &params)?.ok_or_else(|| format!("unhandled palette UI command `{command_id}`"))?;
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
                    command_id: Some(hit.id.to_owned()),
                    command_result: Some(json!({ "notice": message })),
                }),
                WebCommandRoute::OpenFilePicker { .. } | WebCommandRoute::ExportFormat { .. } | WebCommandRoute::PrintCurrentSheet => {
                    Ok(UiEventOutcome {
                        handled: true,
                        action: format!("palette.host:{}", hit.id),
                        command_id: Some(hit.id.to_owned()),
                        command_result: Some(json!({ "deferred_host_command": hit.id })),
                    })
                }
            }
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            let intent = app.canvas_dialogs.exclude_with_reason.commit_intent()?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.exclude_committed"),
                command_id: Some(intent.command_id),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::ExportFigure { sheet_id } => {
            let intent = app.canvas_dialogs.export_figure.commit_intent(&sheet_id);
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.export_committed"),
                command_id: Some(intent.command_id),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::Print { .. } => {
            let out = app.execute_print_dialog_spool(false)?;
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.print_committed"),
                command_id: Some(String::from("ui.print.open")),
                command_result: Some(json!({ "page_count": out.page_count })),
            })
        }
        ActiveDialog::PowerAndSampleSize => {
            if app.canvas_dialogs.power_wizard.stage != curvebook_ui_egui::PowerWizardStage::Results {
                let next = app.canvas_dialogs.power_wizard.advance()?;
                return Ok(UiEventOutcome { handled: true, action: format!("dialog.power_advance:{}", next.label()), ..Default::default() });
            }
            let payload = dispatch_gui_action(app, "ui.answer_dialog", json!({ "action": "create" })).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.power_created"),
                command_id: Some(String::from("ui.powerWizard.create")),
                command_result: Some(payload),
            })
        }
        ActiveDialog::ConvertArchetype { .. } => {
            let out = app.commit_convert_archetype()?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.convert_archetype"),
                command_id: Some(out.command_id),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::ChangeGraphType { .. } => {
            let intents = app.canvas_dialogs.change_graph_type.commit_intents(app.document());
            commit_web_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.change_graph_type_committed")
        }
        ActiveDialog::FormatAxes { .. } => {
            curvebook_ui_egui::interaction::sync_format_axes_text_buffers(app);
            let intent = app.canvas_dialogs.format_axes.commit_intent().ok_or_else(|| String::from("invalid Format Axes settings"))?;
            let res = commit_web_intent_single(app, &intent.command_id, intent.params, "dialog.format_axes_committed")?;
            curvebook_ui_egui::interaction::clear_modal_text_buffers(app);
            Ok(res)
        }
        ActiveDialog::FormatGraph { .. } => {
            let intents = app.canvas_dialogs.format_graph.commit_intents();
            commit_web_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.format_graph_committed")
        }
        ActiveDialog::PairwiseComparisons { .. } => {
            let intents = app.canvas_dialogs.pairwise.commit_intents();
            commit_web_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.pairwise_committed")
        }
        ActiveDialog::BubblePlotInspector { .. } => {
            let intent = app.canvas_dialogs.bubble_plot.commit_intent().ok_or_else(|| String::from("no bubble plot graph target selected"))?;
            commit_web_intent_single(app, &intent.command_id, intent.params, "dialog.bubble_plot_committed")
        }
        ActiveDialog::InsertModelEquation { .. } => {
            let doc = app.document().clone();
            let intent = app
                .canvas_dialogs
                .insert_model_equation
                .commit_intent(Some(&doc))
                .ok_or_else(|| String::from("no model equation result sheet selected"))?;
            commit_web_intent_single(app, &intent.command_id, intent.params, "dialog.insert_model_equation_committed")
        }
        ActiveDialog::ApplyStyleFrom { .. } => {
            let intent =
                app.canvas_dialogs.apply_style.commit_intent().ok_or_else(|| String::from("no donor graph selected for Apply Style From"))?;
            commit_web_intent_single(app, &intent.command_id, intent.params, "dialog.apply_style_committed")
        }
        ActiveDialog::ChangeDataAnalyzed { .. } => {
            let intent =
                app.canvas_dialogs.change_data.commit_intent().ok_or_else(|| String::from("no compatible replacement data sheet selected"))?;
            commit_web_intent_single(app, &intent.command_id, intent.params, "dialog.change_data_committed")
        }
        ActiveDialog::FormatSelectedPoints { .. } => {
            if app.canvas_dialogs.format_points.selected_points.is_empty()
                && let Some(active_ds_id) = app.brushed_points.first().map(|p| p.sheet_id.clone())
                && let Some(ds) = app.document().data_sheet(&active_ds_id).cloned()
            {
                let targets = app.grid.selected_graph_point_targets(&ds, usize::MAX);
                app.canvas_dialogs.format_points.set_selected_targets(targets);
            }
            let mut pairs: Vec<(String, Value)> = Vec::new();
            if let Some(intent) = app.canvas_dialogs.format_points.build_commit_intent() {
                pairs.push((intent.command_id, intent.params));
            }
            let scene = app.canvas_dialogs.format_points.graph_sheet_id.as_ref().and_then(|gid| app.session().render_sheet_scene(gid).ok());
            let doc = app.document().clone();
            for intent in app.canvas_dialogs.format_points.build_row_label_intents_for_scene(&doc, scene.as_ref()) {
                pairs.push((intent.command_id, intent.params));
            }
            commit_web_intent_batch(app, pairs, "dialog.format_points_committed")
        }
        ActiveDialog::PageLayoutComposer { .. } => {
            let intents = app.canvas_dialogs.page_layout.commit_intents();
            commit_web_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.page_layout_committed")
        }
        _ => {
            app.canvas_dialogs.close_dialog();
            curvebook_ui_egui::interaction::clear_modal_text_buffers(app);
            Ok(UiEventOutcome { handled: true, action: String::from("dialog.closed"), ..Default::default() })
        }
    }
}

fn commit_web_intent_single(app: &mut UiApp, command_id: &str, params: Value, action: &str) -> Result<UiEventOutcome, String> {
    let out = app.execute_command(command_id, params).map_err(|e| e.to_string())?;
    app.canvas_dialogs.close_dialog();
    Ok(UiEventOutcome { handled: true, action: action.to_owned(), command_id: Some(command_id.to_owned()), command_result: Some(out.payload) })
}

fn commit_web_intent_batch<I>(app: &mut UiApp, intents: I, action: &str) -> Result<UiEventOutcome, String>
where
    I: IntoIterator<Item = (String, Value)>,
{
    let mut last_cmd = None;
    let mut last_payload = None;
    for (command_id, params) in intents {
        let out = app.execute_command(&command_id, params).map_err(|e| e.to_string())?;
        last_cmd = Some(command_id);
        last_payload = Some(out.payload);
    }
    app.canvas_dialogs.close_dialog();
    Ok(UiEventOutcome { handled: true, action: action.to_owned(), command_id: last_cmd, command_result: last_payload })
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
                json!({ "sheet_id": "ds-grp", "kind": "grouped", "rows": 3, "cols": 2, "subcolumns": 2, "auto_family": false }),
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
