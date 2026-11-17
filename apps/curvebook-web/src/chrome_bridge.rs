//! Web chrome parity (`R10.7`): shared menu bar, contextual toolbar, interactive modals,
//! context menus, browser-reserved shortcut routing, and unified input dispatch.
#![forbid(unsafe_code)]

use curvebook_engine::cmd;
use curvebook_ui_egui::compositor::FramePalette;
use curvebook_ui_egui::layout::a11y::project_accessible_tree_with_announcement;
use curvebook_ui_egui::layout::{
    AccessibilityMeta, ClipId, FocusScope, LayoutRectPx as TreeLayoutRectPx, LayoutTextRun, LayoutTree, NodeBounds, PaintOp, PaintOpKind,
    WidgetAction, WidgetId, WidgetNode, WidgetRole, WidgetVisualState, ZLayer as TreeZLayer, cycle_focus_in_scope, find_scroll_container_at,
    hit_test_logical, hit_test_physical,
};
use curvebook_ui_egui::menus::normalize_shortcut_combo;
use curvebook_ui_egui::serde_json::{Value, json};
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle as TreeUiTextStyle, ellipsize_text};
use curvebook_ui_egui::views::a11y::resolve_live_announcement;
use curvebook_ui_egui::views::build_workspace_layout_tree;
use curvebook_ui_egui::{
    ActiveDialog, ControlEnablement, KeyModifiers, PointerButton, UiApp, UiEventOutcome, UiInputEvent, ViewportMetrics, command_tooltip,
    dispatch_gui_action, dispatch_ui_command,
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

/// Hit-testable chrome widget node sharing geometry with [`WebTextDrawOp`]s.
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

/// Full computed chrome layout backed directly by shared [`LayoutTree`].
#[derive(Debug, Clone)]
pub struct WebChromeLayout {
    pub tree: LayoutTree,
    pub nodes: Vec<WebChromeNode>,
    pub snapshot: WebChromeSnapshot,
}

impl WebChromeLayout {
    /// Shared single-source hit test consulting [`LayoutTree`] in reverse `(z_layer, paint_order)`.
    ///
    /// An open [`ZLayer::ModalBackdrop`] blocks click-through to workspace and dropdown nodes below it.
    #[must_use]
    pub fn hit_test_physical(&self, px: i32, py: i32) -> Option<&WebChromeNode> {
        let hit = hit_test_physical(&self.tree, px, py)?;
        self.find_node(hit.id.as_str())
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
    ExecuteWidget { widget_id: String, action: WidgetAction },
    BlockedByModalBackdrop,
    Ignored,
}

/// Stateful browser chrome controller synchronized with [`UiApp`] and shared [`LayoutTree`].
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

    /// Synchronize mirrored chrome state fields from [`UiApp`] after unified interaction dispatch.
    pub fn sync_from_app(&mut self, app: &UiApp) {
        self.open_menu_key = app.interaction.open_menu.clone();
        self.hovered_node_id = app.interaction.hovered_widget_id.clone();
        self.focused_node_id = app.focus_widget_id.clone();
        if app.canvas_dialogs.active_dialog != ActiveDialog::None || app.interaction.context_menu_target.is_none() {
            self.context_menu = None;
        }
    }

    /// Build and augment the canonical shared [`LayoutTree`] for browser chrome, dialogs, and context menus.
    #[must_use]
    pub fn build_layout_tree(&self, app: &UiApp, viewport: ViewportMetrics) -> LayoutTree {
        let palette = FramePalette::from_theme(&app.theme);
        let mut tree = build_workspace_layout_tree(app, viewport, DesktopTextMeasurer::global(), &palette);
        apply_web_tree_rules(&mut tree, app, self, &palette);
        let _ = tree.finalize();
        tree
    }

    /// Build the unified chrome [`WebChromeLayout`] directly from the shared [`LayoutTree`] (`R10.1`, `R10.7`).
    #[must_use]
    pub fn build_layout(&self, app: &UiApp, viewport: ViewportMetrics) -> WebChromeLayout {
        let tree = self.build_layout_tree(app, viewport);
        let nodes = project_chrome_nodes(&tree, app);
        let mut menu_titles = Vec::new();
        let mut toolbar_ids = Vec::new();
        for node in &tree.nodes {
            if matches!(node.action, WidgetAction::MenuOpen(_)) {
                menu_titles.push(node.a11y.label.clone());
            } else if node.id.as_str().starts_with("toolbar.") {
                toolbar_ids.push(node.id.0.clone());
            }
        }
        let status_summary = build_status_summary(app, self);
        let focused_id = app.focus_widget_id.clone().or_else(|| self.focused_node_id.clone());
        let (announcement, politeness) = resolve_live_announcement(app, &tree);
        let a11y_update = project_accessible_tree_with_announcement(&tree, focused_id.as_deref(), announcement.as_deref(), politeness);
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
            a11y_tree: Some(Box::new(a11y_update.to_aria_json_value())),
        };
        WebChromeLayout { tree, nodes, snapshot }
    }

    /// Update hover state against shared [`LayoutTree`] nodes and return whether visual hover changed,
    /// plus whether workspace pointer-move should be suppressed (popup/modal/context-menu occlusion).
    pub fn handle_pointer_move(&mut self, app: &mut UiApp, layout: &WebChromeLayout, logical_pos: [f64; 2]) -> (bool, bool) {
        let hit = hit_test_logical(&layout.tree, logical_pos[0], logical_pos[1]);
        let next_hover = hit.map(|n| n.id.0.clone());
        let next_cmd = hit.and_then(command_id_for_hover);
        let changed = next_hover != app.interaction.hovered_widget_id || next_cmd != app.hover_command_id || next_hover != self.hovered_node_id;
        app.interaction.hovered_widget_id = next_hover.clone();
        app.hover_command_id = next_cmd;
        self.hovered_node_id = next_hover;
        let occluded_above_workspace = hit.is_some_and(|n| n.z_layer >= TreeZLayer::DropdownMenu);
        (changed, occluded_above_workspace)
    }

    /// Route a pointer press through Z-ordered [`LayoutTree`] layers before workspace hit testing.
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
            let target = classify_tree_context_target(app, &layout.tree, logical_pos);
            app.interaction.context_menu_target = Some(target.clone());
            app.view_shards.toolbar_menus.active_context_menu_id = Some(target.clone());
            app.view_shards.toolbar_menus.context_menu_anchor_px = Some([px, py]);
            self.context_menu = Some(WebContextMenuState { target: target.clone(), anchor_px: [px, py] });
            return WebChromeDispatch::HandledRedraw { action: format!("context_menu.open:{target}"), command_result: None };
        }

        if button != PointerButton::Primary {
            return WebChromeDispatch::Ignored;
        }

        let hit = hit_test_logical(&layout.tree, logical_pos[0], logical_pos[1]);

        if self.context_menu.is_some() {
            if let Some(node) = hit.filter(|n| n.z_layer == TreeZLayer::ContextMenu) {
                return self.arm_pressed_chrome_node(app, node);
            }
            self.context_menu = None;
            app.interaction.context_menu_target = None;
            app.view_shards.toolbar_menus.active_context_menu_id = None;
            app.view_shards.toolbar_menus.context_menu_anchor_px = None;
            app.interaction.pressed_widget_id = None;
            return WebChromeDispatch::HandledRedraw { action: String::from("context_menu.dismissed"), command_result: None };
        }

        if hit.is_some_and(|n| n.id.as_str() == "modal.backdrop") {
            app.interaction.open_menu = None;
            app.interaction.open_dropdown = None;
            app.interaction.pressed_widget_id = None;
            self.open_menu_key = None;
            return WebChromeDispatch::BlockedByModalBackdrop;
        }

        let popup_open = self.open_menu_key.is_some() || app.interaction.open_menu.is_some() || app.interaction.open_dropdown.is_some();
        let on_menu_header = hit.is_some_and(|n| matches!(n.action, WidgetAction::MenuOpen(_)));
        if popup_open && hit.is_none_or(|n| n.z_layer < TreeZLayer::DropdownMenu && !on_menu_header) {
            self.close_popups();
            app.interaction.open_menu = None;
            app.interaction.open_dropdown = None;
            app.interaction.pressed_widget_id = None;
            return WebChromeDispatch::HandledRedraw { action: String::from("chrome.popup_dismissed"), command_result: None };
        }

        if let Some(node) = hit.filter(|n| is_web_chrome_widget(n)) {
            return self.arm_pressed_chrome_node(app, node);
        }

        WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown {
            x: logical_pos[0],
            y: logical_pos[1],
            button,
            modifiers: KeyModifiers::default(),
        })
    }

    fn arm_pressed_chrome_node(&mut self, app: &mut UiApp, node: &WidgetNode) -> WebChromeDispatch {
        let nid = node.id.as_str();
        if nid == "formula_bar.input" {
            app.grid.formula_bar.active = true;
        }
        if node.state.enablement.enabled && (node.a11y.tab_order.is_some() || node.a11y.role == WidgetRole::TextBox) {
            app.focus_widget_id = Some(node.id.0.clone());
            self.focused_node_id = app.focus_widget_id.clone();
            if node.a11y.role == WidgetRole::TextBox {
                let initial = node.a11y.value_text.as_deref().unwrap_or("");
                let multiline = nid == "info.notes";
                app.interaction
                    .text_field_buffers
                    .entry(node.id.0.clone())
                    .or_insert_with(|| curvebook_ui_egui::layout::widgets::TextFieldEditState::new(initial, multiline));
            }
        }
        if !node.state.enablement.enabled {
            let reason = node.state.enablement.disabled_reason.clone();
            if let Some(ref msg) = reason {
                self.last_notice = Some((WebNoticeLevel::Warning, msg.clone()));
            }
            return WebChromeDispatch::HandledRedraw {
                action: format!("chrome.disabled:{nid}"),
                command_result: Some(json!({ "disabled_reason": reason })),
            };
        }
        if node.action != WidgetAction::NoOp {
            app.interaction.pressed_widget_id = Some(node.id.0.clone());
            return WebChromeDispatch::HandledRedraw { action: format!("chrome.pressed:{nid}"), command_result: None };
        }
        WebChromeDispatch::HandledRedraw { action: format!("chrome.surface:{nid}"), command_result: None }
    }

    /// Complete a pointer release against pressed [`LayoutTree`] chrome widgets or workspace drags.
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
        let hit = hit_test_logical(&layout.tree, logical_pos[0], logical_pos[1]);
        if let Some(pressed_id) = app.interaction.pressed_widget_id.take() {
            let Some(node) = hit.filter(|n| n.id.as_str() == pressed_id) else {
                return WebChromeDispatch::HandledRedraw { action: String::from("chrome.release_cancelled"), command_result: None };
            };
            if node.z_layer == TreeZLayer::ContextMenu {
                self.context_menu = None;
                app.interaction.context_menu_target = None;
                app.view_shards.toolbar_menus.active_context_menu_id = None;
                app.view_shards.toolbar_menus.context_menu_anchor_px = None;
            }
            if !node.state.enablement.enabled {
                let reason = node.state.enablement.disabled_reason.clone();
                if let Some(ref msg) = reason {
                    self.last_notice = Some((WebNoticeLevel::Warning, msg.clone()));
                }
                return WebChromeDispatch::HandledRedraw {
                    action: format!("chrome.disabled:{}", node.id.as_str()),
                    command_result: Some(json!({ "disabled_reason": reason })),
                };
            }
            return WebChromeDispatch::ExecuteWidget { widget_id: node.id.0.clone(), action: node.action.clone() };
        }

        let workspace_drag_active = app.interaction.dragging_split_divider
            || app.interaction.dragging_navigator_sheet.is_some()
            || app.interaction.dragging_grid_sheet.is_some()
            || app.interaction.canvas_drag.is_some();
        if !workspace_drag_active {
            if let Some(node) = hit {
                if !node.state.enablement.enabled && (node.z_layer >= TreeZLayer::DropdownMenu || is_web_chrome_widget(node)) {
                    let reason = node.state.enablement.disabled_reason.clone();
                    return WebChromeDispatch::HandledRedraw {
                        action: format!("chrome.disabled:{}", node.id.as_str()),
                        command_result: Some(json!({ "disabled_reason": reason })),
                    };
                }
                if node.z_layer >= TreeZLayer::DropdownMenu {
                    return WebChromeDispatch::HandledRedraw { action: format!("chrome.surface_release:{}", node.id.as_str()), command_result: None };
                }
            }
            if self.context_menu.is_some() {
                return WebChromeDispatch::BlockedByModalBackdrop;
            }
        }

        WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerUp {
            x: logical_pos[0],
            y: logical_pos[1],
            button,
            modifiers: KeyModifiers::default(),
        })
    }

    /// Route mouse-wheel / trackpad scroll through [`LayoutTree`] scrollable containers first,
    /// falling back to grid scroll or canvas pan/zoom only when not occluded by popups/modals.
    pub fn route_wheel(
        &self,
        app: &mut UiApp,
        layout: &WebChromeLayout,
        pos_logical: [f64; 2],
        delta_px: [f64; 2],
        modifiers: KeyModifiers,
    ) -> WebChromeDispatch {
        let [x_logical, y_logical] = pos_logical;
        let [delta_x, delta_y] = delta_px;
        let [px_f, py_f] = layout.tree.metrics.logical_to_physical_px([x_logical, y_logical]);
        let modal_open = app.canvas_dialogs.active_dialog != ActiveDialog::None;

        let min_layer = if self.context_menu.is_some() {
            TreeZLayer::ContextMenu
        } else if modal_open {
            TreeZLayer::ModalDialog
        } else if self.open_menu_key.is_some() || app.interaction.open_menu.is_some() || app.interaction.open_dropdown.is_some() {
            TreeZLayer::DropdownMenu
        } else {
            TreeZLayer::Workspace
        };

        #[allow(clippy::cast_possible_truncation)]
        let (px_i, py_i) = (px_f.round() as i32, py_f.round() as i32);
        if let Some(scroll_meta) = find_scroll_container_at(&layout.tree, px_i, py_i)
            && (scroll_meta.has_vertical_overflow() || scroll_meta.has_horizontal_overflow())
        {
            let owner_layer = layout.tree.node(scroll_meta.container_id.as_str()).map_or_else(
                || {
                    layout
                        .tree
                        .nodes
                        .iter()
                        .find(|n| n.scroll.as_ref().is_some_and(|s| s.container_id == scroll_meta.container_id))
                        .map_or(TreeZLayer::Workspace, |n| n.z_layer)
                },
                |n| n.z_layer,
            );
            if owner_layer >= min_layer {
                app.interaction.scroll_offsets.apply_wheel(scroll_meta, delta_x, delta_y);
                if owner_layer == TreeZLayer::Workspace
                    && !(modifiers.ctrl || modifiers.meta)
                    && scroll_meta.container_id.as_str().contains("grid.sheet.")
                {
                    return WebChromeDispatch::WheelScrollGrid { dx_px: delta_x, dy_px: delta_y };
                }
                return WebChromeDispatch::HandledRedraw {
                    action: format!("scroll.wheel:{}", scroll_meta.container_id.as_str()),
                    command_result: None,
                };
            }
        }

        if modal_open {
            return WebChromeDispatch::BlockedByModalBackdrop;
        }
        if min_layer > TreeZLayer::Workspace {
            return WebChromeDispatch::HandledRedraw { action: String::from("popup.wheel_ignored"), command_result: None };
        }
        if modifiers.ctrl || modifiers.meta {
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::WheelZoom { x: x_logical, y: y_logical, delta_y, modifiers });
        }
        let doc = app.document();
        let active_is_grid = doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some());
        if active_is_grid {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (px_u, py_u) = (px_f.max(0.0).round() as u32, py_f.max(0.0).round() as u32);
            if layout.tree.frame.canvas.contains(px_u, py_u) {
                return WebChromeDispatch::WheelScrollGrid { dx_px: delta_x, dy_px: delta_y };
            }
        }
        WebChromeDispatch::PassToWorkspace(UiInputEvent::InertialPan { delta_x: -delta_x, delta_y: -delta_y, dt_sec: 1.0 / 60.0, step_coast: false })
    }

    /// Route keyboard `KeyDown` events through popups, modals, focus traversal, inline edit, and Web shortcuts.
    pub fn handle_key_down(&mut self, app: &mut UiApp, layout: &WebChromeLayout, key: &str, modifiers: KeyModifiers) -> WebChromeDispatch {
        let trimmed = key.trim();

        if matches!(trimmed, "Escape" | "Esc") {
            if self.context_menu.is_some()
                || self.open_menu_key.is_some()
                || app.interaction.open_menu.is_some()
                || app.interaction.open_dropdown.is_some()
            {
                self.close_popups();
                app.interaction.open_menu = None;
                app.interaction.open_dropdown = None;
                app.interaction.context_menu_target = None;
                app.view_shards.toolbar_menus.active_context_menu_id = None;
                app.view_shards.toolbar_menus.context_menu_anchor_px = None;
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

        if trimmed.eq_ignore_ascii_case("tab") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
            if app.canvas_dialogs.active_dialog == ActiveDialog::None && app.grid.formula_bar.active {
                return WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers });
            }
            let trap_or_focused = app.canvas_dialogs.active_dialog != ActiveDialog::None
                || self.context_menu.is_some()
                || self.open_menu_key.is_some()
                || app.interaction.open_menu.is_some()
                || app.interaction.open_dropdown.is_some()
                || app.focus_widget_id.is_some()
                || !is_active_data_sheet(app);
            if trap_or_focused
                && let Some(next_id) =
                    cycle_focus_in_scope(&layout.tree.nodes, layout.tree.active_focus_scope, app.focus_widget_id.as_deref(), modifiers.shift)
            {
                app.focus_widget_id = Some(next_id.clone());
                self.focused_node_id = Some(next_id.clone());
                if let Some(node) = layout.tree.node(&next_id)
                    && node.a11y.role == WidgetRole::TextBox
                {
                    let initial = node.a11y.value_text.as_deref().unwrap_or("");
                    app.interaction
                        .text_field_buffers
                        .entry(next_id.clone())
                        .or_insert_with(|| curvebook_ui_egui::layout::widgets::TextFieldEditState::new(initial, next_id == "info.notes"));
                }
                return WebChromeDispatch::HandledRedraw { action: format!("focus.cycle:{next_id}"), command_result: None };
            }
        }

        if app.canvas_dialogs.active_dialog != ActiveDialog::None {
            return self.handle_modal_key_down(app, layout, trimmed, modifiers);
        }

        if let Some(fid) = app.focus_widget_id.as_deref()
            && is_text_input_widget_id(fid)
            && !modifiers.ctrl
            && !modifiers.meta
            && !modifiers.alt
        {
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers });
        }

        if (trimmed == " " || trimmed.eq_ignore_ascii_case("space"))
            && let Some(fid) = app.focus_widget_id.clone()
            && let Some(node) = layout.tree.node(&fid)
            && node.state.enablement.enabled
            && node.action != WidgetAction::NoOp
        {
            return WebChromeDispatch::ExecuteWidget { widget_id: fid, action: node.action.clone() };
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

    fn handle_modal_key_down(&mut self, app: &mut UiApp, layout: &WebChromeLayout, key: &str, modifiers: KeyModifiers) -> WebChromeDispatch {
        if matches!(key, "Enter" | "Return") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
            return WebChromeDispatch::InvokeChromeAction(WebChromeAction::DialogConfirm);
        }
        if (key == " " || key.eq_ignore_ascii_case("space"))
            && let Some(fid) = app.focus_widget_id.clone()
            && let Some(node) = layout.tree.node(&fid)
            && node.a11y.role != WidgetRole::TextBox
            && node.state.enablement.enabled
            && node.action != WidgetAction::NoOp
        {
            return WebChromeDispatch::ExecuteWidget { widget_id: fid, action: node.action.clone() };
        }
        match &app.canvas_dialogs.active_dialog {
            ActiveDialog::CommandPalette => {
                let hits_len = app.canvas_dialogs.command_palette.matches_for_session(app.session()).len();
                match key {
                    "Backspace" => {
                        app.canvas_dialogs.command_palette.query.pop();
                        app.canvas_dialogs.command_palette.selected_index = 0;
                        sync_palette_query_buffer(app);
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
                        sync_palette_query_buffer(app);
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
            _ => {
                if app.focus_widget_id.as_deref().is_some_and(is_text_input_widget_id) {
                    WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers })
                } else {
                    WebChromeDispatch::BlockedByModalBackdrop
                }
            }
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
                sync_palette_query_buffer(app);
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

fn sync_palette_query_buffer(app: &mut UiApp) {
    app.interaction.text_field_buffers.insert(
        String::from("palette.query"),
        curvebook_ui_egui::layout::widgets::TextFieldEditState::new(&app.canvas_dialogs.command_palette.query, false),
    );
}

fn is_text_input_widget_id(wid: &str) -> bool {
    matches!(wid, "palette.query" | "formula_bar.input" | "info.notes") || wid.contains("search") || wid.contains("input")
}

fn is_web_chrome_widget(node: &WidgetNode) -> bool {
    if node.z_layer >= TreeZLayer::DropdownMenu {
        return true;
    }
    let id = node.id.as_str();
    if id == "result.diagnostic_banner"
        || id.starts_with("canvas.surface.")
        || matches!(node.action, WidgetAction::SwitchSheet { .. } | WidgetAction::SelectGridCell { .. } | WidgetAction::SwapSplitPanes)
    {
        return false;
    }
    id == "menu.bar"
        || id == "status.bar"
        || id.starts_with("menu.")
        || id.starts_with("toolbar.")
        || id.starts_with("nav.folder.")
        || id == "navigator.family_toggle"
        || id.starts_with("result.")
        || id.starts_with("inspector.")
        || id.starts_with("scrollbar.")
        || id == "formula_bar.input"
        || id.starts_with("info.")
        || (id.starts_with("canvas.") && node.action != WidgetAction::NoOp)
        || node.action != WidgetAction::NoOp
}

fn command_id_for_hover(node: &WidgetNode) -> Option<String> {
    if matches!(node.action, WidgetAction::MenuOpen(_)) {
        return None;
    }
    if let WidgetAction::InvokeCommand { command_id, .. } = &node.action {
        return Some(command_id.clone());
    }
    let id = node.id.as_str();
    id.strip_prefix("menu.").or_else(|| id.strip_prefix("ctx.")).filter(|s| !s.is_empty() && *s != "bar").map(str::to_owned)
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

fn classify_tree_context_target(app: &UiApp, tree: &LayoutTree, logical_pos: [f64; 2]) -> String {
    if let Some(hit) = hit_test_logical(tree, logical_pos[0], logical_pos[1]) {
        let id = hit.id.as_str();
        if let Some(sid) = id.strip_prefix("nav.sheet.") {
            return format!("navigator:{sid}");
        }
        if id.starts_with("nav.") || id == "navigator.pane" || id == "navigator.family_toggle" {
            return String::from("navigator");
        }
        if id.starts_with("grid.header.") || id.starts_with("grid.col_hdr.") || id.starts_with("grid.x_hdr.") {
            return String::from("column_header");
        }
        if id.starts_with("grid.") {
            return String::from("grid");
        }
    }
    let layout = app.layout_map(tree.metrics);
    match layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
        curvebook_ui_egui::WorkspaceHitTarget::NavigatorEntry { sheet_id, .. } => format!("navigator:{}", sheet_id.as_str()),
        curvebook_ui_egui::WorkspaceHitTarget::GridCell { .. } | curvebook_ui_egui::WorkspaceHitTarget::GridXCell { .. } => String::from("grid"),
        _ => {
            let doc = app.document();
            if doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some()) {
                String::from("grid")
            } else {
                String::from("graph_object")
            }
        }
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

#[derive(Clone, Copy)]
enum PatchBrush {
    Surface,
    Panel,
    Header,
    Accent,
    Text,
    Muted,
}

struct TreePatchWriter<'a> {
    tree: &'a mut LayoutTree,
    palette: &'a FramePalette,
    next_paint_order: u32,
}

impl<'a> TreePatchWriter<'a> {
    fn new(tree: &'a mut LayoutTree, palette: &'a FramePalette) -> Self {
        let next_paint_order = tree.paint_ops.iter().map(|op| op.paint_order).max().unwrap_or(0).saturating_add(1);
        Self { tree, palette, next_paint_order }
    }

    fn next_order(&mut self) -> u32 {
        let order = self.next_paint_order;
        self.next_paint_order = self.next_paint_order.saturating_add(1);
        order
    }

    fn push_clip(&mut self, rect: TreeLayoutRectPx) -> ClipId {
        let idx = self.tree.clips.len();
        self.tree.clips.push(rect);
        ClipId(idx)
    }

    fn push_node(&mut self, node: WidgetNode) {
        self.tree.nodes.push(node);
    }

    fn paint_fill(&mut self, z_layer: TreeZLayer, clip: ClipId, owner: &WidgetId, rect: TreeLayoutRectPx, brush: PatchBrush) {
        self.push_rect_op(z_layer, clip, owner, rect, brush, None);
    }

    fn paint_stroke(&mut self, z_layer: TreeZLayer, clip: ClipId, owner: &WidgetId, rect: TreeLayoutRectPx, brush: PatchBrush, thickness: u32) {
        self.push_rect_op(z_layer, clip, owner, rect, brush, Some(thickness));
    }

    fn push_rect_op(
        &mut self,
        z_layer: TreeZLayer,
        clip: ClipId,
        owner: &WidgetId,
        rect: TreeLayoutRectPx,
        brush: PatchBrush,
        thickness: Option<u32>,
    ) {
        let order = self.next_order();
        let color = match brush {
            PatchBrush::Surface => self.palette.surface,
            PatchBrush::Panel => self.palette.panel,
            PatchBrush::Header => self.palette.header,
            PatchBrush::Accent => self.palette.accent,
            PatchBrush::Text => self.palette.text,
            PatchBrush::Muted => self.palette.muted,
        };
        let kind = match thickness {
            Some(t) => PaintOpKind::StrokeRect { rect, color, thickness: t },
            None => PaintOpKind::FillRect { rect, color },
        };
        self.tree.paint_ops.push(PaintOp { z_layer, paint_order: order, clip: Some(clip), owner_widget_id: Some(owner.clone()), kind });
    }

    fn paint_text_line(&mut self, spec: WebTreeTextSpec<'_>) {
        let shaped = DesktopTextMeasurer::global().shape_line(spec.text, spec.style, &self.tree.locale);
        let baseline_y = spec.pos_px[1] + shaped.metrics.ascent_px;
        #[allow(clippy::cast_possible_truncation)]
        let bx = spec.pos_px[0].floor() as i32;
        #[allow(clippy::cast_possible_truncation)]
        let by = spec.pos_px[1].floor() as i32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bw = shaped.metrics.width_px.ceil().max(1.0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bh = shaped.metrics.line_height_px.ceil().max(1.0) as u32;
        if !spec.text.is_empty() {
            self.tree.text_runs.push(LayoutTextRun {
                owner_widget_id: spec.owner.clone(),
                text: spec.text.to_owned(),
                style: spec.style.clone(),
                z_layer: spec.z_layer,
                clip: Some(spec.clip),
                scroll_container_id: None,
                origin_px: [spec.pos_px[0], baseline_y],
                baseline_y_px: baseline_y,
                bounds_px: TreeLayoutRectPx::new(bx, by, bw, bh),
                shaped: shaped.clone(),
                ellipsized: spec.ellipsized,
            });
        }
        let order = self.next_order();
        let color = match spec.brush {
            PatchBrush::Surface => self.palette.surface,
            PatchBrush::Panel => self.palette.panel,
            PatchBrush::Header => self.palette.header,
            PatchBrush::Accent => self.palette.accent,
            PatchBrush::Text => self.palette.text,
            PatchBrush::Muted => self.palette.muted,
        };
        self.tree.paint_ops.push(PaintOp {
            z_layer: spec.z_layer,
            paint_order: order,
            clip: Some(spec.clip),
            owner_widget_id: Some(spec.owner.clone()),
            kind: PaintOpKind::TextRun {
                text: spec.text.to_owned(),
                style: spec.style.clone(),
                locale: self.tree.locale.clone(),
                origin_px: [spec.pos_px[0], baseline_y],
                color,
                shaped,
            },
        });
    }
}

struct WebTreeTextSpec<'a> {
    z_layer: TreeZLayer,
    clip: ClipId,
    owner: &'a WidgetId,
    pos_px: [f64; 2],
    text: &'a str,
    style: &'a TreeUiTextStyle,
    brush: PatchBrush,
    ellipsized: bool,
}

fn apply_web_tree_rules(tree: &mut LayoutTree, app: &UiApp, bridge: &WebChromeBridge, palette: &FramePalette) {
    remove_menu_item_and_compact(tree, "menu.ui.event.dispatch");
    enforce_qol7_remediation(tree, palette);
    append_context_menu_to_tree(tree, app, bridge, palette);
    emit_web_tooltip_if_needed(tree, app, palette);
}

fn remove_menu_item_and_compact(tree: &mut LayoutTree, target_id: &str) {
    let Some(removed) = tree.nodes.iter().find(|n| n.id.as_str() == target_id).cloned() else {
        return;
    };
    let dy = i32::try_from(removed.bounds.rect_px.height).unwrap_or(0);
    let scale = tree.metrics.scale_factor;
    tree.nodes.retain(|n| n.id.as_str() != target_id);
    tree.paint_ops.retain(|op| op.owner_widget_id.as_ref().is_none_or(|id| id.as_str() != target_id));
    tree.text_runs.retain(|run| run.owner_widget_id.as_str() != target_id);

    let Some(parent_id) = removed.parent else {
        return;
    };
    let mut shifted_ids = Vec::new();
    for node in &mut tree.nodes {
        if node.parent.as_ref() == Some(&parent_id) && node.bounds.rect_px.y > removed.bounds.rect_px.y && dy > 0 {
            let mut r = node.bounds.rect_px;
            r.y = r.y.saturating_sub(dy);
            let clip_r = node.clip.and_then(|c| tree.clips.get(c.0).copied());
            node.bounds = NodeBounds::from_physical(r, clip_r, scale);
            shifted_ids.push(node.id.0.clone());
        } else if node.id == parent_id
            && let Some(scroll) = &mut node.scroll
        {
            scroll.content_size_px[1] = scroll.content_size_px[1].saturating_sub(removed.bounds.rect_px.height);
        }
    }
    if dy <= 0 || shifted_ids.is_empty() {
        return;
    }
    for op in &mut tree.paint_ops {
        if op.owner_widget_id.as_ref().is_some_and(|id| shifted_ids.iter().any(|s| s == id.as_str())) {
            shift_paint_op_y(&mut op.kind, -dy);
        }
    }
    for run in &mut tree.text_runs {
        if shifted_ids.iter().any(|s| s == run.owner_widget_id.as_str()) {
            run.origin_px[1] -= f64::from(dy);
            run.baseline_y_px -= f64::from(dy);
            run.bounds_px.y = run.bounds_px.y.saturating_sub(dy);
        }
    }
}

fn shift_paint_op_y(kind: &mut PaintOpKind, dy: i32) {
    match kind {
        PaintOpKind::FillRect { rect, .. }
        | PaintOpKind::StrokeRect { rect, .. }
        | PaintOpKind::DimRect { rect, .. }
        | PaintOpKind::BlitImage { dst: rect, .. } => {
            rect.y = rect.y.saturating_add(dy);
        }
        PaintOpKind::LineSegment { p0, p1, .. } => {
            p0[1] = p0[1].saturating_add(dy);
            p1[1] = p1[1].saturating_add(dy);
        }
        PaintOpKind::CircleIndicator { center, .. } => {
            center[1] = center[1].saturating_add(dy);
        }
        PaintOpKind::TextRun { origin_px, .. } => {
            origin_px[1] += f64::from(dy);
        }
    }
}

fn enforce_qol7_remediation(tree: &mut LayoutTree, palette: &FramePalette) {
    let target_id = "menu.ui.result.applyRemediation";
    let Some(node) = tree.nodes.iter_mut().find(|n| n.id.as_str() == target_id) else {
        return;
    };
    node.state.enablement = ControlEnablement::disabled(String::from("Diagnostic-only (QOL-7): inspect Result Diagnostic Tabs instead"));
    node.action = WidgetAction::NoOp;
    for op in &mut tree.paint_ops {
        if op.owner_widget_id.as_ref().is_some_and(|id| id.as_str() == target_id)
            && let PaintOpKind::TextRun { color, .. } = &mut op.kind
        {
            *color = palette.muted;
        }
    }
}

fn append_context_menu_to_tree(tree: &mut LayoutTree, app: &UiApp, bridge: &WebChromeBridge, palette: &FramePalette) {
    if app.canvas_dialogs.active_dialog != ActiveDialog::None || tree.node("popup.context_menu").is_some() {
        return;
    }
    let ctx_state = if let Some(ctx) = &bridge.context_menu {
        Some(ctx.clone())
    } else {
        app.interaction.context_menu_target.as_ref().map(|target| WebContextMenuState {
            target: target.clone(),
            anchor_px: [tree.frame.canvas.x as i32 + 24, tree.frame.canvas.y as i32 + 24],
        })
    };
    let Some(ctx) = ctx_state else {
        return;
    };
    let items = context_menu_items_for_target(&ctx.target, app);
    if items.is_empty() {
        return;
    }
    let scale = tree.metrics.scale_factor;
    let vw = i32::try_from(tree.metrics.physical_width_px()).unwrap_or(1024);
    let vh = i32::try_from(tree.metrics.physical_height_px()).unwrap_or(640);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let row_h = (22.0 * scale).round().clamp(18.0, 44.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pop_w = (224.0 * scale).round().max(120.0).min(f64::from(vw.saturating_sub(8)).max(120.0)) as u32;
    let pad = 4u32;
    let pop_h = (u32::try_from(items.len()).unwrap_or(1).saturating_mul(row_h).saturating_add(pad.saturating_mul(2)))
        .min(tree.metrics.physical_height_px().saturating_sub(8).max(row_h));
    let max_x = vw.saturating_sub(i32::try_from(pop_w).unwrap_or(0)).saturating_sub(4).max(4);
    let max_y = vh.saturating_sub(i32::try_from(pop_h).unwrap_or(0)).saturating_sub(4).max(4);
    let pop_rect = TreeLayoutRectPx::new(ctx.anchor_px[0].clamp(4, max_x), ctx.anchor_px[1].clamp(4, max_y), pop_w, pop_h);

    tree.active_focus_scope = FocusScope::PopupMenu;
    let hovered_id = app.interaction.hovered_widget_id.clone();
    let focused_id = app.focus_widget_id.clone();
    let pressed_id = app.interaction.pressed_widget_id.clone();
    let locale = tree.locale.clone();
    #[allow(clippy::cast_possible_truncation)]
    let size_px = (12.0 * scale).clamp(9.0, 22.0);
    let style = TreeUiTextStyle::ui_sans(size_px);

    let mut writer = TreePatchWriter::new(tree, palette);
    let pop_clip = writer.push_clip(pop_rect);
    let pop_wid = WidgetId::from("popup.context_menu");
    writer.paint_fill(TreeZLayer::ContextMenu, pop_clip, &pop_wid, pop_rect, PatchBrush::Surface);
    writer.paint_stroke(TreeZLayer::ContextMenu, pop_clip, &pop_wid, pop_rect, PatchBrush::Accent, 1);
    let pop_order = writer.next_order();
    let mut pop_a11y = AccessibilityMeta::new(WidgetRole::ContextMenu, format!("Context Menu ({})", ctx.target), FocusScope::PopupMenu);
    pop_a11y.keyboard_path = Some(String::from("context_menu"));
    writer.push_node(WidgetNode {
        id: pop_wid.clone(),
        parent: None,
        z_layer: TreeZLayer::ContextMenu,
        paint_order: pop_order,
        clip: Some(pop_clip),
        scroll: None,
        bounds: NodeBounds::from_physical(pop_rect, Some(pop_rect), scale),
        a11y: pop_a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
        drawn_in_frame: true,
    });

    for (idx, (label, cmd_id, enabled)) in items.iter().enumerate() {
        let ry = pop_rect.y.saturating_add(i32::try_from(pad).unwrap_or(4)).saturating_add(i32::try_from(idx as u32 * row_h).unwrap_or(0));
        if ry + i32::try_from(row_h).unwrap_or(22) > pop_rect.bottom() {
            break;
        }
        let item_rect = TreeLayoutRectPx::new(pop_rect.x + 2, ry, pop_rect.width.saturating_sub(4), row_h);
        let item_id = WidgetId::from(format!("ctx.{cmd_id}"));
        let is_hovered = hovered_id.as_deref() == Some(item_id.as_str());
        let is_focused = focused_id.as_deref() == Some(item_id.as_str());
        let is_active = pressed_id.as_deref() == Some(item_id.as_str());
        let enablement =
            if *enabled { ControlEnablement::enabled() } else { ControlEnablement::disabled(format!("{label} is unavailable for current sheet")) };
        if *enabled && (is_hovered || is_active) {
            writer.paint_fill(TreeZLayer::ContextMenu, pop_clip, &item_id, item_rect, PatchBrush::Header);
        }
        if is_focused {
            writer.paint_stroke(TreeZLayer::ContextMenu, pop_clip, &item_id, item_rect, PatchBrush::Accent, 1);
        }
        let max_text_w = f64::from(item_rect.width.saturating_sub(12));
        let (display, ellipsized) = ellipsize_text(DesktopTextMeasurer::global(), label, &style, &locale, max_text_w);
        let metrics = DesktopTextMeasurer::global().measure(&display, &style, &locale);
        let ty = f64::from(item_rect.y) + ((f64::from(item_rect.height) - metrics.line_height_px) * 0.5).max(1.0);
        let brush = if *enabled { PatchBrush::Text } else { PatchBrush::Muted };
        writer.paint_text_line(WebTreeTextSpec {
            z_layer: TreeZLayer::ContextMenu,
            clip: pop_clip,
            owner: &item_id,
            pos_px: [f64::from(item_rect.x + 6), ty],
            text: &display,
            style: &style,
            brush,
            ellipsized,
        });
        let mut a11y = AccessibilityMeta::new(WidgetRole::MenuItem, label.clone(), FocusScope::PopupMenu);
        a11y.keyboard_path = Some(format!("ctx.{cmd_id}"));
        a11y.tab_order = Some(900 + u32::try_from(idx).unwrap_or(0));
        let item_order = writer.next_order();
        let params = contextual_command_params(app, cmd_id);
        writer.push_node(WidgetNode {
            id: item_id,
            parent: Some(pop_wid.clone()),
            z_layer: TreeZLayer::ContextMenu,
            paint_order: item_order,
            clip: Some(pop_clip),
            scroll: None,
            bounds: NodeBounds::from_physical(item_rect, Some(pop_rect), scale),
            a11y,
            state: WidgetVisualState { hovered: is_hovered, active: is_active, focused: is_focused, enablement, ellipsized },
            action: if *enabled { WidgetAction::InvokeCommand { command_id: cmd_id.clone(), params } } else { WidgetAction::NoOp },
            drawn_in_frame: true,
        });
    }
}

fn emit_web_tooltip_if_needed(tree: &mut LayoutTree, app: &UiApp, palette: &FramePalette) {
    if tree.nodes.iter().any(|n| n.id.as_str() == "overlay.tooltip") {
        return;
    }
    let Some(hovered_id) = &app.interaction.hovered_widget_id else {
        return;
    };
    let hovered_node = tree.nodes.iter().find(|n| n.id.as_str() == hovered_id).cloned();
    let tip_text = hovered_node
        .as_ref()
        .and_then(|n| n.state.enablement.disabled_reason.clone())
        .or_else(|| app.hover_command_id.as_deref().and_then(|c| command_tooltip(app, c)).map(|t| t.format_inline_tooltip()));
    let Some(text) = tip_text.filter(|s| !s.is_empty()) else {
        return;
    };
    let vw = tree.metrics.physical_width_px();
    let vh = tree.metrics.physical_height_px();
    if vw < 120 || vh < 60 {
        return;
    }
    let scale = tree.metrics.scale_factor;
    let size_px = (11.5 * scale).clamp(9.0, 22.0);
    let style = TreeUiTextStyle::ui_sans(size_px);
    let max_w = vw.saturating_sub(24).min(360);
    let locale = tree.locale.clone();
    let (display, ellipsized) = ellipsize_text(DesktopTextMeasurer::global(), &text, &style, &locale, f64::from(max_w.saturating_sub(12)));
    let measured = DesktopTextMeasurer::global().measure(&display, &style, &locale);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tw = ((measured.width_px.ceil() as u32) + 14).clamp(60, max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let th = (size_px * 1.6).ceil() as u32;
    let rect = TreeLayoutRectPx::new(12, i32::try_from(vh).unwrap_or(640) - i32::try_from(th + 8).unwrap_or(28), tw, th);
    let focus_scope = tree.active_focus_scope;
    let mut writer = TreePatchWriter::new(tree, palette);
    let clip = writer.push_clip(rect);
    let wid = WidgetId::from("overlay.tooltip");
    writer.paint_fill(TreeZLayer::Tooltip, clip, &wid, rect, PatchBrush::Panel);
    writer.paint_stroke(TreeZLayer::Tooltip, clip, &wid, rect, PatchBrush::Accent, 1);
    let ty = f64::from(rect.y) + ((f64::from(rect.height) - measured.line_height_px) * 0.5).max(1.0);
    writer.paint_text_line(WebTreeTextSpec {
        z_layer: TreeZLayer::Tooltip,
        clip,
        owner: &wid,
        pos_px: [f64::from(rect.x + 6), ty],
        text: &display,
        style: &style,
        brush: PatchBrush::Text,
        ellipsized,
    });
    let order = writer.next_order();
    writer.push_node(WidgetNode {
        id: wid,
        parent: None,
        z_layer: TreeZLayer::Tooltip,
        paint_order: order,
        clip: Some(clip),
        scroll: None,
        bounds: NodeBounds::from_physical(rect, Some(rect), scale),
        a11y: AccessibilityMeta::new(WidgetRole::Tooltip, display, focus_scope),
        state: WidgetVisualState { ellipsized, ..Default::default() },
        action: WidgetAction::NoOp,
        drawn_in_frame: true,
    });
}

fn project_chrome_nodes(tree: &LayoutTree, app: &UiApp) -> Vec<WebChromeNode> {
    let mut nodes: Vec<WebChromeNode> = tree
        .nodes
        .iter()
        .map(|node| {
            let role = project_chrome_role(node);
            let tooltip = if !node.state.enablement.enabled {
                node.state.enablement.disabled_reason.clone()
            } else if let Some(cmd_id) = command_id_for_hover(node) {
                command_tooltip(app, &cmd_id).map(|t| t.format_inline_tooltip())
            } else if node.a11y.role == WidgetRole::Tooltip {
                Some(node.a11y.label.clone())
            } else {
                None
            };
            WebChromeNode {
                id: node.id.0.clone(),
                role,
                z_layer: ZLayer::from(node.z_layer),
                paint_order: node.paint_order,
                rect_px: LayoutRectPx::from(node.bounds.rect_px),
                clip_px: LayoutRectPx::from(node.bounds.visible_rect_px),
                label: node.a11y.label.clone(),
                shortcut: node.a11y.shortcut.clone(),
                enabled: node.state.enablement.enabled,
                disabled_reason: node.state.enablement.disabled_reason.clone(),
                tooltip,
                action: project_chrome_action(&node.action),
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.z_layer.cmp(&b.z_layer).then_with(|| a.paint_order.cmp(&b.paint_order)));
    nodes
}

fn project_chrome_role(node: &WidgetNode) -> WebChromeRole {
    if node.id.as_str() == "modal.backdrop" {
        return WebChromeRole::ModalBackdrop;
    }
    if node.id.as_str() == "modal.close" {
        return WebChromeRole::ModalCloseButton;
    }
    if matches!(node.action, WidgetAction::MenuOpen(_)) {
        return WebChromeRole::MenuSection;
    }
    match node.a11y.role {
        WidgetRole::MenuBar => WebChromeRole::MenuBar,
        WidgetRole::MenuItem => WebChromeRole::MenuItem,
        WidgetRole::ContextMenu => {
            if node.z_layer == TreeZLayer::DropdownMenu || node.z_layer == TreeZLayer::ModalDropdown {
                WebChromeRole::DropdownMenu
            } else {
                WebChromeRole::ContextMenu
            }
        }
        WidgetRole::Dialog => WebChromeRole::ModalDialog,
        WidgetRole::Tooltip => WebChromeRole::Tooltip,
        WidgetRole::Banner => WebChromeRole::StatusBar,
        WidgetRole::TextBox if node.z_layer >= TreeZLayer::ModalDialog => WebChromeRole::ModalInput,
        WidgetRole::RadioButton if node.z_layer >= TreeZLayer::ModalDialog => WebChromeRole::ModalItem,
        _ if node.z_layer >= TreeZLayer::ModalDialog => WebChromeRole::ModalButton,
        _ => WebChromeRole::ToolbarButton,
    }
}

fn project_chrome_action(action: &WidgetAction) -> WebChromeAction {
    match action {
        WidgetAction::MenuOpen(key) => WebChromeAction::ToggleMenuSection(key.clone()),
        WidgetAction::InvokeCommand { command_id, .. } => match command_id.as_str() {
            "project.open" | "file.open" => WebChromeAction::OpenFilePicker,
            "export.svg" | "graph.exportSvg" => WebChromeAction::ExportFormat(WebExportFormat::Svg),
            "export.pdf" | "graph.exportPdf" => WebChromeAction::ExportFormat(WebExportFormat::Pdf),
            "export.png" | "graph.exportPng" => WebChromeAction::ExportFormat(WebExportFormat::Png),
            "export.tiff" => WebChromeAction::ExportFormat(WebExportFormat::Tiff),
            "table.exportCsv" => WebChromeAction::ExportFormat(WebExportFormat::Csv),
            "project.save" => WebChromeAction::ExportFormat(WebExportFormat::CurveBookArchive),
            _ => WebChromeAction::InvokeCommand(command_id.clone()),
        },
        WidgetAction::DialogConfirm => WebChromeAction::DialogConfirm,
        WidgetAction::DialogCancel => WebChromeAction::DialogCancel,
        WidgetAction::InvokeDialogButton(btn) if btn == "back" => WebChromeAction::DialogStepBack,
        WidgetAction::InvokeDialogButton(btn) if btn == "next" => WebChromeAction::DialogStepNext,
        WidgetAction::SelectAnalyzeCategory(_) | WidgetAction::SelectAnalyzeEntry(_) | WidgetAction::SelectDropdownOption { .. } => {
            WebChromeAction::SelectDialogItem(0)
        }
        WidgetAction::FocusWidget(_) => WebChromeAction::FocusModalInput,
        _ => WebChromeAction::None,
    }
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
    use curvebook_ui_egui::layout::verify::verify_tree_invariants;
    use curvebook_ui_egui::{KeyModifiers, PointerButton, UiApp, ViewportMetrics, execute_widget_action};

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
    fn modal_backdrop_blocks_workspace_click_through_and_dropdown_routes_shared_tree() {
        let mut app = UiApp::new();
        let mut bridge = WebChromeBridge::new();
        let viewport = ViewportMetrics::new(1024.0, 640.0, 1.0);

        let disp = click_node_press_release(&mut bridge, &mut app, viewport, "menu.file");
        let WebChromeDispatch::ExecuteWidget { action, .. } = disp else {
            panic!("expected ExecuteWidget on menu.file release, got {disp:?}");
        };
        execute_widget_action(&mut app, &action, viewport).unwrap();
        bridge.sync_from_app(&app);
        assert_eq!(bridge.open_menu_key.as_deref(), Some("menu.file"));

        let open_layout = bridge.build_layout(&app, viewport);
        assert!(open_layout.find_node("popup.menu.file").is_some());
        let report = verify_tree_invariants(&open_layout.tree);
        assert!(report.is_clean(), "open menu LayoutTree parity issues: {report:?}");

        app.canvas_dialogs.open_command_palette();
        bridge.close_popups();
        app.interaction.open_menu = None;
        let layout2 = bridge.build_layout(&app, viewport);
        assert!(layout2.find_node("modal.backdrop").is_some());
        assert!(layout2.find_node("palette.query").is_some());
        let hit_corner = bridge.handle_pointer_down(&mut app, &layout2, [5.0, 5.0], [5, 5], PointerButton::Primary);
        assert_eq!(hit_corner, WebChromeDispatch::BlockedByModalBackdrop);
        let modal_report = verify_tree_invariants(&layout2.tree);
        assert!(modal_report.is_clean(), "modal LayoutTree parity issues: {modal_report:?}");
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
