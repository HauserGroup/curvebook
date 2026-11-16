//! Web chrome parity (`R10.7`): shared menu bar, contextual toolbar, interactive modals,
//! context menus, browser-reserved shortcut routing, and unified input dispatch.
#![forbid(unsafe_code)]

use curvebook_engine::cmd;
use curvebook_ui_egui::compositor::WorkspaceFrameLayout;
use curvebook_ui_egui::menus::normalize_shortcut_combo;
use curvebook_ui_egui::serde_json::{Value, json};
use curvebook_ui_egui::{
    ActiveDialog, ControlEnablement, KeyModifiers, PointerButton, UiApp, UiEventOutcome, UiInputEvent, UiThemeSpec, ViewportMetrics, command_tooltip,
    dispatch_gui_action, dispatch_ui_command, menu_bar_tooltips, toolbar_tooltips,
};

use crate::text_bridge::{
    CSS_REFERENCE_DPI, LayoutRectPx, UiTextStyle, WebRectPrimitive, WebTextAlign, WebTextDirection, WebTextDrawOp, WebTextMeasurer, ZLayer,
    canvas_font_css,
};

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
        }
    }
}

/// Active right-click context menu target & origin in physical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebContextMenuState {
    pub target: String,
    pub anchor_px: [i32; 2],
}

/// Full computed chrome layout for one rendered web frame.
#[derive(Debug, Clone)]
pub struct WebChromeLayout {
    pub nodes: Vec<WebChromeNode>,
    pub rects: Vec<WebRectPrimitive>,
    pub text_ops: Vec<WebTextDrawOp>,
    pub snapshot: WebChromeSnapshot,
}

impl WebChromeLayout {
    /// Shared single-source hit test walking `nodes` in reverse `(z_layer, paint_order)`.
    ///
    /// An open [`ZLayer::ModalBackdrop`] blocks click-through to workspace and dropdown nodes below it.
    #[must_use]
    pub fn hit_test_physical(&self, px: i32, py: i32) -> Option<&WebChromeNode> {
        self.nodes.iter().rev().find(|n| n.clip_px.contains(px, py) && n.rect_px.contains(px, py))
    }

    /// Look up a node by stable `id`.
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

/// Stateful browser chrome controller owning open menu, hover/focus, context menu, and IME state.
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

    /// Build the unified chrome [`WebChromeLayout`] using `measurer` for every string (`R10.1`, `R10.7`).
    #[must_use]
    pub fn build_layout(
        &self,
        app: &UiApp,
        viewport: ViewportMetrics,
        frame: &WorkspaceFrameLayout,
        measurer: &dyn WebTextMeasurer,
        dpi: u16,
    ) -> WebChromeLayout {
        let mut builder = ChromeLayoutBuilder::new(app, viewport, frame, measurer, dpi, self);
        builder.build()
    }

    /// Handle pointer movement across chrome layers and update hover tooltip state.
    pub fn handle_pointer_move(&mut self, layout: &WebChromeLayout, phys_x: i32, phys_y: i32) -> bool {
        let next_hover = layout.hit_test_physical(phys_x, phys_y).map(|n| n.id.clone());
        if next_hover != self.hovered_node_id {
            self.hovered_node_id = next_hover;
            return true;
        }
        false
    }

    /// Route a physical pointer press through Z-ordered chrome layers before workspace hit testing.
    pub fn handle_pointer_down(
        &mut self,
        app: &mut UiApp,
        layout: &WebChromeLayout,
        logical_pos: [f64; 2],
        phys_pos: [i32; 2],
        button: PointerButton,
    ) -> WebChromeDispatch {
        let [px, py] = phys_pos;

        if button == PointerButton::Primary
            && let Some(node) = layout.hit_test_physical(px, py)
        {
            match node.role {
                WebChromeRole::ModalBackdrop => {
                    self.open_menu_key = None;
                    self.context_menu = None;
                    return WebChromeDispatch::BlockedByModalBackdrop;
                }
                WebChromeRole::MenuSection
                | WebChromeRole::MenuItem
                | WebChromeRole::ToolbarButton
                | WebChromeRole::ModalCloseButton
                | WebChromeRole::ModalInput
                | WebChromeRole::ModalItem
                | WebChromeRole::ModalButton => {
                    self.focused_node_id = Some(node.id.clone());
                    if !node.enabled {
                        if let Some(reason) = &node.disabled_reason {
                            self.last_notice = Some((WebNoticeLevel::Warning, reason.clone()));
                        }
                        return WebChromeDispatch::HandledRedraw {
                            action: format!("chrome.disabled:{}", node.id),
                            command_result: Some(json!({ "disabled_reason": node.disabled_reason })),
                        };
                    }
                    return self.activate_node_action(app, node.action.clone());
                }
                WebChromeRole::MenuBar => {}
                WebChromeRole::StatusBar
                | WebChromeRole::DropdownMenu
                | WebChromeRole::ModalDialog
                | WebChromeRole::ContextMenu
                | WebChromeRole::Tooltip => {
                    return WebChromeDispatch::HandledRedraw { action: format!("chrome.surface:{}", node.id), command_result: None };
                }
            }
        }

        // Clicking workspace dismisses open dropdown or context menu without click-through.
        if button == PointerButton::Primary && (self.open_menu_key.is_some() || self.context_menu.is_some()) {
            self.close_popups();
            return WebChromeDispatch::HandledRedraw { action: String::from("chrome.popup_dismissed"), command_result: None };
        }

        if button == PointerButton::Secondary {
            if app.canvas_dialogs.active_dialog != ActiveDialog::None {
                return WebChromeDispatch::BlockedByModalBackdrop;
            }
            self.open_menu_key = None;
            let target = classify_context_target(app, frame_rect_hit(app, logical_pos));
            self.context_menu = Some(WebContextMenuState { target: target.to_owned(), anchor_px: [px, py] });
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown {
                x: logical_pos[0],
                y: logical_pos[1],
                button: PointerButton::Secondary,
                modifiers: KeyModifiers::default(),
            });
        }

        WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown {
            x: logical_pos[0],
            y: logical_pos[1],
            button,
            modifiers: KeyModifiers::default(),
        })
    }

    fn activate_node_action(&mut self, app: &mut UiApp, action: WebChromeAction) -> WebChromeDispatch {
        match action {
            WebChromeAction::ToggleMenuSection(ref key) => {
                self.context_menu = None;
                if self.open_menu_key.as_deref() == Some(key.as_str()) {
                    self.open_menu_key = None;
                } else {
                    self.open_menu_key = Some(key.clone());
                }
                WebChromeDispatch::HandledRedraw { action: format!("menu.toggle:{key}"), command_result: None }
            }
            WebChromeAction::InvokeCommand(ref cmd_id) => {
                self.close_popups();
                WebChromeDispatch::InvokeChromeAction(WebChromeAction::InvokeCommand(cmd_id.clone()))
            }
            WebChromeAction::ExportFormat(fmt) => {
                self.close_popups();
                WebChromeDispatch::InvokeChromeAction(WebChromeAction::ExportFormat(fmt))
            }
            WebChromeAction::OpenFilePicker => {
                self.close_popups();
                WebChromeDispatch::ShortcutTriggered(WebShortcutAction::OpenFilePicker)
            }
            WebChromeAction::OpenDirectoryPicker => {
                self.close_popups();
                WebChromeDispatch::InvokeChromeAction(WebChromeAction::OpenDirectoryPicker)
            }
            WebChromeAction::SelectDialogItem(idx) => apply_dialog_item_selection(app, idx),
            WebChromeAction::DialogCancel | WebChromeAction::CloseModalBackdrop => {
                app.canvas_dialogs.close_dialog();
                WebChromeDispatch::HandledRedraw { action: String::from("dialog.cancelled"), command_result: None }
            }
            WebChromeAction::DialogStepBack => {
                let _ = app.canvas_dialogs.power_wizard.go_back();
                WebChromeDispatch::HandledRedraw { action: String::from("dialog.back"), command_result: None }
            }
            WebChromeAction::DialogStepNext => {
                let _ = app.canvas_dialogs.power_wizard.advance();
                WebChromeDispatch::HandledRedraw { action: String::from("dialog.next"), command_result: None }
            }
            WebChromeAction::DialogConfirm => WebChromeDispatch::InvokeChromeAction(WebChromeAction::DialogConfirm),
            WebChromeAction::FocusModalInput => WebChromeDispatch::HandledRedraw { action: String::from("dialog.focus_input"), command_result: None },
            WebChromeAction::DismissContextMenu => {
                self.context_menu = None;
                WebChromeDispatch::HandledRedraw { action: String::from("context_menu.dismissed"), command_result: None }
            }
            WebChromeAction::None => WebChromeDispatch::Ignored,
        }
    }

    /// Route mouse-wheel / trackpad scroll over DataSheet/ResultSheet grids vs Graph/Layout canvases.
    #[must_use]
    pub fn route_wheel(
        &self,
        app: &UiApp,
        viewport: ViewportMetrics,
        pos_logical: [f64; 2],
        delta_px: [f64; 2],
        modifiers: KeyModifiers,
    ) -> WebChromeDispatch {
        let [x_logical, y_logical] = pos_logical;
        let [delta_x, delta_y] = delta_px;
        if app.canvas_dialogs.active_dialog != ActiveDialog::None {
            return WebChromeDispatch::BlockedByModalBackdrop;
        }
        if modifiers.ctrl || modifiers.meta {
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::WheelZoom { x: x_logical, y: y_logical, delta_y, modifiers });
        }
        let doc = app.document();
        let active_is_grid = doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some());
        if active_is_grid {
            let layout = app.layout_map(viewport);
            let [px, py] = viewport.logical_to_physical_px([x_logical, y_logical]);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (px_u, py_u) = (px.max(0.0).round() as u32, py.max(0.0).round() as u32);
            if layout.frame.canvas.contains(px_u, py_u) {
                return WebChromeDispatch::WheelScrollGrid { dx_px: delta_x, dy_px: delta_y };
            }
        }
        WebChromeDispatch::PassToWorkspace(UiInputEvent::InertialPan { delta_x: -delta_x, delta_y: -delta_y, dt_sec: 1.0 / 60.0, step_coast: false })
    }

    /// Route keyboard `KeyDown` events through popups, modals, inline edit, and Web shortcuts.
    pub fn handle_key_down(&mut self, app: &mut UiApp, key: &str, modifiers: KeyModifiers) -> WebChromeDispatch {
        let trimmed = key.trim();

        if matches!(trimmed, "Escape" | "Esc") {
            if self.context_menu.is_some() || self.open_menu_key.is_some() {
                self.close_popups();
                return WebChromeDispatch::HandledRedraw { action: String::from("chrome.escape_popup"), command_result: None };
            }
            if app.canvas_dialogs.active_dialog != ActiveDialog::None && !app.inline_editor.active {
                app.canvas_dialogs.close_dialog();
                return WebChromeDispatch::HandledRedraw { action: String::from("dialog.escape_close"), command_result: None };
            }
        }

        if app.canvas_dialogs.active_dialog != ActiveDialog::None {
            return self.handle_modal_key_down(app, trimmed, modifiers);
        }

        if app.inline_editor.active {
            return WebChromeDispatch::PassToWorkspace(UiInputEvent::KeyDown { key: key.to_owned(), modifiers });
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

    /// Route IME committed text into active modal search fields or workspace inline editor.
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

fn frame_rect_hit(app: &UiApp, logical_pos: [f64; 2]) -> String {
    let layout = app.layout_map(ViewportMetrics::new(1280.0, 800.0, 1.0));
    match layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
        curvebook_ui_egui::WorkspaceHitTarget::GridCell { .. } | curvebook_ui_egui::WorkspaceHitTarget::GridXCell { .. } => String::from("grid"),
        curvebook_ui_egui::WorkspaceHitTarget::NavigatorEntry { .. } => String::from("navigator"),
        _ => String::from("canvas"),
    }
}

fn classify_context_target(app: &UiApp, guessed: String) -> &'static str {
    let doc = app.document();
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if doc.data_sheet(active).is_some() && guessed != "navigator" {
            return "grid";
        }
        if doc.graph_sheet(active).is_some() || doc.layout_sheet(active).is_some() {
            return if guessed == "navigator" { "navigator" } else { "canvas" };
        }
    }
    "canvas"
}

fn apply_dialog_item_selection(app: &mut UiApp, idx: usize) -> WebChromeDispatch {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::AnalyzeSheet { .. } => {
            let entries = app.canvas_dialogs.analyze.compatible_entries();
            if let Some(item) = entries.get(idx) {
                app.canvas_dialogs.analyze.selected_entry_id = Some(item.id.to_owned());
            }
            WebChromeDispatch::HandledRedraw { action: format!("dialog.select_analyze:{idx}"), command_result: None }
        }
        ActiveDialog::CommandPalette => {
            app.canvas_dialogs.command_palette.selected_index = idx;
            WebChromeDispatch::InvokeChromeAction(WebChromeAction::DialogConfirm)
        }
        ActiveDialog::ExportFigure { .. } => {
            let fmt = match idx {
                0 => curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Svg,
                1 => curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Pdf,
                2 => curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Png,
                _ => curvebook_ui_egui::serde_json::from_value(json!({ "Tiff": "Deflate" }))
                    .unwrap_or(curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Png),
            };
            app.canvas_dialogs.export_figure.format = fmt;
            WebChromeDispatch::HandledRedraw { action: format!("dialog.select_export_fmt:{idx}"), command_result: None }
        }
        ActiveDialog::ChangeGraphType { .. } => {
            let opts = app.canvas_dialogs.change_graph_type.recommended_options();
            if let Some(opt) = opts.get(idx) {
                app.canvas_dialogs.change_graph_type.selected_option_id = opt.id.to_owned();
            }
            WebChromeDispatch::HandledRedraw { action: format!("dialog.select_graph_type:{idx}"), command_result: None }
        }
        _ => WebChromeDispatch::HandledRedraw { action: format!("dialog.select_item:{idx}"), command_result: None },
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
            let intent = app.canvas_dialogs.edit_params.commit_intent().ok_or_else(|| String::from("no draft parameters ready to commit"))?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.edit_params_committed"),
                command_id: Some(intent.command_id),
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
        _ => {
            app.canvas_dialogs.close_dialog();
            Ok(UiEventOutcome { handled: true, action: String::from("dialog.closed"), ..Default::default() })
        }
    }
}

struct ChromeColors {
    panel: [u8; 4],
    surface: [u8; 4],
    header: [u8; 4],
    border: [u8; 4],
    text: [u8; 4],
    muted: [u8; 4],
    accent: [u8; 4],
    hover_bg: [u8; 4],
}

impl ChromeColors {
    fn from_theme(theme: &UiThemeSpec) -> Self {
        let dark = theme.surface_bg.relative_luminance() < 0.25;
        let to_arr = |c: (u8, u8, u8, u8)| -> [u8; 4] { [c.0, c.1, c.2, c.3] };
        let shift = |base: [u8; 4], delta: i16| -> [u8; 4] {
            let ch = |c: u8| -> u8 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = i16::from(c).saturating_add(delta).clamp(0, 255) as u8;
                v
            };
            [ch(base[0]), ch(base[1]), ch(base[2]), base[3]]
        };
        let panel = to_arr((theme.panel_bg.r, theme.panel_bg.g, theme.panel_bg.b, theme.panel_bg.a));
        Self {
            panel,
            surface: to_arr((theme.surface_bg.r, theme.surface_bg.g, theme.surface_bg.b, theme.surface_bg.a)),
            header: shift(panel, if dark { 12 } else { -10 }),
            border: to_arr((theme.border.r, theme.border.g, theme.border.b, theme.border.a)),
            text: to_arr((theme.text_primary.r, theme.text_primary.g, theme.text_primary.b, theme.text_primary.a)),
            muted: to_arr((theme.text_muted.r, theme.text_muted.g, theme.text_muted.b, theme.text_muted.a)),
            accent: to_arr((theme.accent.r, theme.accent.g, theme.accent.b, theme.accent.a)),
            hover_bg: shift(panel, if dark { 24 } else { -20 }),
        }
    }
}

struct ChromeLayoutBuilder<'a> {
    app: &'a UiApp,
    viewport: ViewportMetrics,
    frame: &'a WorkspaceFrameLayout,
    measurer: &'a dyn WebTextMeasurer,
    dpi: u16,
    bridge: &'a WebChromeBridge,
    colors: ChromeColors,
    is_rtl: bool,
    dir: WebTextDirection,
    nodes: Vec<WebChromeNode>,
    rects: Vec<WebRectPrimitive>,
    text_ops: Vec<WebTextDrawOp>,
    paint_seq: u32,
}

impl<'a> ChromeLayoutBuilder<'a> {
    fn new(
        app: &'a UiApp,
        viewport: ViewportMetrics,
        frame: &'a WorkspaceFrameLayout,
        measurer: &'a dyn WebTextMeasurer,
        dpi: u16,
        bridge: &'a WebChromeBridge,
    ) -> Self {
        let is_rtl = curvebook_ui_egui::i18n::is_rtl_locale(&app.locale);
        Self {
            app,
            viewport,
            frame,
            measurer,
            dpi: dpi.max(72),
            bridge,
            colors: ChromeColors::from_theme(&app.theme),
            is_rtl,
            dir: if is_rtl { WebTextDirection::Rtl } else { WebTextDirection::Ltr },
            nodes: Vec::new(),
            rects: Vec::new(),
            text_ops: Vec::new(),
            paint_seq: 0,
        }
    }

    fn next_order(&mut self) -> u32 {
        let cur = self.paint_seq;
        self.paint_seq = self.paint_seq.saturating_add(1);
        cur
    }

    fn push_rect(&mut self, z_layer: ZLayer, rect_px: LayoutRectPx, fill: Option<[u8; 4]>, stroke: Option<[u8; 4]>) {
        self.rects.push(WebRectPrimitive { z_layer, rect_px, clip_px: None, fill, stroke, stroke_width_px: 1 });
    }

    fn push_text(&mut self, z_layer: ZLayer, rect_px: LayoutRectPx, text: String, style: UiTextStyle, color: [u8; 4], node_id: Option<&str>) {
        self.push_text_with_dpi(z_layer, rect_px, text, style, (color, CSS_REFERENCE_DPI), node_id);
    }

    fn push_text_scaled(&mut self, z_layer: ZLayer, rect_px: LayoutRectPx, text: String, style: UiTextStyle, color: [u8; 4], node_id: Option<&str>) {
        self.push_text_with_dpi(z_layer, rect_px, text, style, (color, self.dpi), node_id);
    }

    fn push_text_with_dpi(
        &mut self,
        z_layer: ZLayer,
        rect_px: LayoutRectPx,
        text: String,
        style: UiTextStyle,
        color_and_dpi: ([u8; 4], u16),
        node_id: Option<&str>,
    ) {
        let (color, font_dpi) = color_and_dpi;
        if text.is_empty() || rect_px.width == 0 || rect_px.height == 0 {
            return;
        }
        let metrics = self.measurer.measure(&style, &text, font_dpi);
        let pad_x = 4.0;
        let x_px = if self.is_rtl { f64::from(rect_px.right()) - pad_x } else { f64::from(rect_px.x) + pad_x };
        let baseline_y_px = f64::from(rect_px.y) + ((f64::from(rect_px.height) + metrics.ascent_px - metrics.descent_px) * 0.5);
        let paint_order = self.next_order();
        self.text_ops.push(WebTextDrawOp {
            z_layer,
            z_band: z_layer.band(),
            paint_order,
            clip_px: Some(rect_px),
            x_px,
            y_px: f64::from(rect_px.y),
            baseline_y_px,
            max_width_px: Some((f64::from(rect_px.width) - 8.0).max(4.0)),
            text,
            font_css: canvas_font_css(&style, font_dpi),
            style,
            color_rgba: color,
            align: if self.is_rtl { WebTextAlign::Right } else { WebTextAlign::Left },
            direction: self.dir,
            ellipsized: metrics.width_px > f64::from(rect_px.width),
            node_id: node_id.map(str::to_owned),
        });
    }

    fn build(&mut self) -> WebChromeLayout {
        let menu_sections = menu_bar_tooltips(self.app);
        let menu_titles: Vec<String> = menu_sections.iter().map(|s| s.title.to_owned()).collect();
        let mut toolbar_ids = Vec::new();

        if !self.app.presentation_mode && self.frame.toolbar.width > 0 && self.frame.toolbar.height > 0 {
            self.build_top_bar(&menu_sections, &mut toolbar_ids);
            self.build_status_bar();
            self.build_open_dropdown(&menu_sections);
        }

        if self.app.canvas_dialogs.active_dialog != ActiveDialog::None {
            self.build_modal_dialog();
        } else if let Some(ref ctx) = self.bridge.context_menu {
            self.build_context_menu(ctx);
        }

        self.build_hover_tooltip();

        let status_summary = build_status_summary(self.app, self.bridge);
        let snapshot = WebChromeSnapshot {
            menu_bar_visible: !self.app.presentation_mode,
            open_menu_key: self.bridge.open_menu_key.clone(),
            context_menu_open: self.bridge.context_menu.is_some() && self.app.canvas_dialogs.active_dialog == ActiveDialog::None,
            active_dialog: format!("{:?}", self.app.canvas_dialogs.active_dialog),
            hovered_node_id: self.bridge.hovered_node_id.clone(),
            focused_node_id: self.bridge.focused_node_id.clone(),
            ime_preedit: self.bridge.ime_preedit.clone(),
            drag_active: self.bridge.drag_active,
            node_count: self.nodes.len(),
            menu_titles,
            toolbar_ids,
            status_summary,
        };

        self.nodes.sort_by(|a, b| a.z_layer.cmp(&b.z_layer).then_with(|| a.paint_order.cmp(&b.paint_order)));
        WebChromeLayout {
            nodes: std::mem::take(&mut self.nodes),
            rects: std::mem::take(&mut self.rects),
            text_ops: std::mem::take(&mut self.text_ops),
            snapshot,
        }
    }

    fn build_top_bar(&mut self, menu_sections: &[curvebook_ui_egui::MenuSectionTooltips], toolbar_ids: &mut Vec<String>) {
        let tb = self.frame.toolbar;
        #[allow(clippy::cast_possible_wrap)]
        let bar_rect = LayoutRectPx::new(tb.x as i32, tb.y as i32, tb.width, tb.height);
        self.push_rect(ZLayer::Workspace, bar_rect, Some(self.colors.panel), Some(self.colors.border));

        let split_rows = tb.height >= 34;
        let menu_h = if split_rows { tb.height / 2 } else { tb.height };
        let tool_h = if split_rows { tb.height.saturating_sub(menu_h) } else { tb.height };
        #[allow(clippy::cast_possible_truncation)]
        let menu_font_px = (f64::from(menu_h) * 0.62).clamp(9.0, 13.0) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let tool_font_px = (f64::from(tool_h) * 0.58).clamp(9.0, 12.5) as f32;
        let menu_style = UiTextStyle::ui_sans(menu_font_px);
        let tool_style = UiTextStyle::ui_sans(tool_font_px);

        let mut cursor_x = bar_rect.x + 6;
        for sec in menu_sections {
            let label_w = self.measurer.measure(&menu_style, sec.title, CSS_REFERENCE_DPI).width_px.ceil();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let btn_w = (label_w as u32).saturating_add(14).min(tb.width / 3);
            if cursor_x + btn_w as i32 > bar_rect.right() - 4 {
                break;
            }
            let r = LayoutRectPx::new(cursor_x, bar_rect.y + 1, btn_w, menu_h.saturating_sub(2));
            let active = self.bridge.open_menu_key.as_deref() == Some(sec.menu_key) || self.bridge.hovered_node_id.as_deref() == Some(sec.menu_key);
            if active {
                self.push_rect(ZLayer::Workspace, r, Some(self.colors.hover_bg), Some(self.colors.border));
            }
            self.push_text(ZLayer::Workspace, r, sec.title.to_owned(), menu_style.clone(), self.colors.text, Some(sec.menu_key));
            let order = self.next_order();
            self.nodes.push(WebChromeNode {
                id: sec.menu_key.to_owned(),
                role: WebChromeRole::MenuSection,
                z_layer: ZLayer::Workspace,
                paint_order: order,
                rect_px: r,
                clip_px: bar_rect,
                label: sec.title.to_owned(),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: Some(format!("{} menu ({} items)", sec.title, sec.items.len())),
                action: WebChromeAction::ToggleMenuSection(sec.menu_key.to_owned()),
            });
            cursor_x = r.right() + 4;
        }

        let tool_y = if split_rows { bar_rect.y + menu_h as i32 } else { bar_rect.y };
        let mut tx = if split_rows { bar_rect.x + 6 } else { cursor_x + 12 };
        let buttons = build_contextual_toolbar_specs(self.app);
        for spec in buttons {
            let measured = self.measurer.measure(&tool_style, &spec.caption, CSS_REFERENCE_DPI).width_px.ceil();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let btn_w = (measured as u32).saturating_add(14);
            if tx + btn_w as i32 > bar_rect.right() - 4 {
                break;
            }
            let r = LayoutRectPx::new(tx, tool_y + 1, btn_w, tool_h.saturating_sub(2));
            let hovered = self.bridge.hovered_node_id.as_deref() == Some(&spec.id);
            let fill = if !spec.enabled {
                Some(self.colors.panel)
            } else if hovered {
                Some(self.colors.hover_bg)
            } else {
                Some(self.colors.surface)
            };
            self.push_rect(ZLayer::Workspace, r, fill, Some(self.colors.border));
            let text_col = if spec.enabled { self.colors.text } else { self.colors.muted };
            self.push_text(ZLayer::Workspace, r, spec.caption.clone(), tool_style.clone(), text_col, Some(&spec.id));
            let order = self.next_order();
            toolbar_ids.push(spec.id.clone());
            self.nodes.push(WebChromeNode {
                id: spec.id,
                role: WebChromeRole::ToolbarButton,
                z_layer: ZLayer::Workspace,
                paint_order: order,
                rect_px: r,
                clip_px: bar_rect,
                label: spec.caption,
                shortcut: spec.shortcut,
                enabled: spec.enabled,
                disabled_reason: spec.disabled_reason,
                tooltip: spec.tooltip,
                action: spec.action,
            });
            tx = r.right() + 4;
        }
    }

    fn build_status_bar(&mut self) {
        let sb = self.frame.status_bar;
        if sb.width == 0 || sb.height == 0 {
            return;
        }
        #[allow(clippy::cast_possible_wrap)]
        let rect = LayoutRectPx::new(sb.x as i32, sb.y as i32, sb.width, sb.height);
        self.push_rect(ZLayer::Workspace, rect, Some(self.colors.header), Some(self.colors.border));
        #[allow(clippy::cast_possible_truncation)]
        let font_px = (f64::from(sb.height) * 0.58).clamp(9.0, 12.0) as f32;
        let summary = build_status_summary(self.app, self.bridge);
        self.push_text(ZLayer::Workspace, rect, summary.clone(), UiTextStyle::ui_sans(font_px), self.colors.muted, Some("status_bar"));
        let order = self.next_order();
        self.nodes.push(WebChromeNode {
            id: String::from("status_bar"),
            role: WebChromeRole::StatusBar,
            z_layer: ZLayer::Workspace,
            paint_order: order,
            rect_px: rect,
            clip_px: rect,
            label: summary,
            shortcut: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            action: WebChromeAction::None,
        });
    }

    fn build_open_dropdown(&mut self, sections: &[curvebook_ui_egui::MenuSectionTooltips]) {
        let Some(open_key) = self.bridge.open_menu_key.as_deref() else {
            return;
        };
        let Some(section) = sections.iter().find(|s| s.menu_key == open_key) else {
            return;
        };
        let anchor_x = self.nodes.iter().find(|n| n.id == open_key).map_or(8, |n| n.rect_px.x);
        let anchor_y = self.nodes.iter().find(|n| n.id == open_key).map_or(24, |n| n.rect_px.bottom());

        let style = UiTextStyle::ui_sans(12.0);
        let row_h = ((22.0 * self.viewport.clamped_scale_factor()).round() as u32).max(20);
        let max_items = ((self.frame.total_height.saturating_sub(anchor_y.max(0) as u32 + 12)) / row_h).max(1) as usize;
        let visible_items = build_dropdown_entries_for_section(open_key, section, max_items.min(28));

        let mut max_w = 220u32;
        for item in &visible_items {
            let web_sc = WebShortcutRouter::display_shortcut(&item.command_id, &item.shortcut);
            let full = if web_sc.is_empty() { item.label.clone() } else { format!("{}  ({web_sc})", item.label) };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let w = (self.measurer.measure(&style, &full, self.dpi).width_px.ceil() as u32).saturating_add(24);
            max_w = max_w.max(w);
        }
        let panel_w = max_w.min(self.frame.total_width.saturating_sub(16).max(160));
        #[allow(clippy::cast_possible_truncation)]
        let panel_h = (visible_items.len() as u32).saturating_mul(row_h).saturating_add(8).max(row_h);
        let clamped_x = anchor_x.min((self.frame.total_width.saturating_sub(panel_w)) as i32).max(4);
        let panel_rect = LayoutRectPx::new(clamped_x, anchor_y + 2, panel_w, panel_h);
        self.push_rect(ZLayer::DropdownMenu, panel_rect, Some(self.colors.surface), Some(self.colors.border));

        for (idx, item) in visible_items.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let iy = panel_rect.y + 4 + (idx as i32) * (row_h as i32);
            let item_rect = LayoutRectPx::new(panel_rect.x + 4, iy, panel_w.saturating_sub(8), row_h.saturating_sub(2));
            let node_id = format!("menu.item.{}", item.command_id);
            let hovered = self.bridge.hovered_node_id.as_deref() == Some(&node_id);
            if hovered {
                self.push_rect(ZLayer::DropdownMenu, item_rect, Some(self.colors.hover_bg), None);
            }
            let web_sc = WebShortcutRouter::display_shortcut(&item.command_id, &item.shortcut);
            let text = if web_sc.is_empty() { item.label.clone() } else { format!("{}  [{}]", item.label, web_sc) };
            let col = if item.enabled { self.colors.text } else { self.colors.muted };
            self.push_text_scaled(ZLayer::DropdownMenu, item_rect, text, style.clone(), col, Some(&node_id));

            let action = match item.command_id.as_str() {
                "project.open" | "file.open" => WebChromeAction::OpenFilePicker,
                "export.svg" | "graph.exportSvg" => WebChromeAction::ExportFormat(WebExportFormat::Svg),
                "export.pdf" | "graph.exportPdf" => WebChromeAction::ExportFormat(WebExportFormat::Pdf),
                "export.png" | "graph.exportPng" => WebChromeAction::ExportFormat(WebExportFormat::Png),
                "export.tiff" => WebChromeAction::ExportFormat(WebExportFormat::Tiff),
                "table.exportCsv" => WebChromeAction::ExportFormat(WebExportFormat::Csv),
                "project.save" => WebChromeAction::ExportFormat(WebExportFormat::CurveBookArchive),
                _ => WebChromeAction::InvokeCommand(item.command_id.clone()),
            };
            let order = self.next_order();
            self.nodes.push(WebChromeNode {
                id: node_id,
                role: WebChromeRole::MenuItem,
                z_layer: ZLayer::DropdownMenu,
                paint_order: order,
                rect_px: item_rect,
                clip_px: panel_rect,
                label: item.label.clone(),
                shortcut: (!web_sc.is_empty()).then(|| web_sc.to_owned()),
                enabled: item.enabled,
                disabled_reason: item.disabled_reason.clone(),
                tooltip: Some(item.tooltip.clone()),
                action,
            });
        }
    }

    fn build_modal_dialog(&mut self) {
        let full = LayoutRectPx::new(0, 0, self.frame.total_width, self.frame.total_height);
        self.push_rect(ZLayer::ModalBackdrop, full, Some([14, 18, 26, 130]), None);
        let backdrop_order = self.next_order();
        self.nodes.push(WebChromeNode {
            id: String::from("modal.backdrop"),
            role: WebChromeRole::ModalBackdrop,
            z_layer: ZLayer::ModalBackdrop,
            paint_order: backdrop_order,
            rect_px: full,
            clip_px: full,
            label: String::from("Modal Backdrop"),
            shortcut: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            action: WebChromeAction::CloseModalBackdrop,
        });

        let mw = (self.frame.total_width * 3 / 5).max(280).min(self.frame.total_width.saturating_sub(24).max(160));
        let mh = (self.frame.total_height * 3 / 5).max(200).min(self.frame.total_height.saturating_sub(24).max(120));
        let mx = ((self.frame.total_width.saturating_sub(mw)) / 2) as i32;
        let my = ((self.frame.total_height.saturating_sub(mh)) / 2) as i32;
        let modal = LayoutRectPx::new(mx, my, mw, mh);

        self.push_rect(ZLayer::ModalDialog, modal, Some(self.colors.surface), Some(self.colors.accent));
        let header_h = 30u32.min(mh);
        let header_rect = LayoutRectPx::new(modal.x, modal.y, modal.width, header_h);
        self.push_rect(ZLayer::ModalDialog, header_rect, Some(self.colors.accent), None);
        self.push_text_scaled(
            ZLayer::ModalDialog,
            LayoutRectPx::new(modal.x + 8, modal.y + 2, modal.width.saturating_sub(40), header_h.saturating_sub(4)),
            modal_title(&self.app.canvas_dialogs.active_dialog).to_owned(),
            UiTextStyle::ui_sans_bold(13.0),
            [255, 255, 255, 255],
            Some("modal.title"),
        );

        let close_rect = LayoutRectPx::new(modal.right() - 28, modal.y + 4, 22, header_h.saturating_sub(8));
        self.push_rect(ZLayer::ModalDialog, close_rect, Some([0, 0, 0, 45]), None);
        self.push_text_scaled(
            ZLayer::ModalDialog,
            close_rect,
            String::from("X"),
            UiTextStyle::ui_sans_bold(11.0),
            [255, 255, 255, 255],
            Some("modal.close"),
        );
        let close_order = self.next_order();
        self.nodes.push(WebChromeNode {
            id: String::from("modal.close"),
            role: WebChromeRole::ModalCloseButton,
            z_layer: ZLayer::ModalDialog,
            paint_order: close_order,
            rect_px: close_rect,
            clip_px: modal,
            label: String::from("Close"),
            shortcut: Some(String::from("Escape")),
            enabled: true,
            disabled_reason: None,
            tooltip: Some(String::from("Close dialog (Escape)")),
            action: WebChromeAction::DialogCancel,
        });

        self.build_modal_body(modal, header_h);
        self.build_modal_footer(modal);
    }

    fn build_modal_body(&mut self, modal: LayoutRectPx, header_h: u32) {
        let mut cy = modal.y + header_h as i32 + 10;
        let body_bottom = modal.bottom() - 42;
        let body_w = modal.width.saturating_sub(24);
        let item_style = UiTextStyle::ui_sans(12.5);
        let row_h = 24i32;

        if let Some((input_label, input_val)) = active_modal_input_text(self.app)
            && cy + 28 < body_bottom
        {
            let input_rect = LayoutRectPx::new(modal.x + 12, cy, body_w, 26);
            self.push_rect(ZLayer::ModalDialog, input_rect, Some(self.colors.panel), Some(self.colors.accent));
            self.push_text_scaled(
                ZLayer::ModalDialog,
                input_rect,
                format!("{input_label}: {input_val}"),
                item_style.clone(),
                self.colors.text,
                Some("modal.input"),
            );
            let order = self.next_order();
            self.nodes.push(WebChromeNode {
                id: String::from("modal.input"),
                role: WebChromeRole::ModalInput,
                z_layer: ZLayer::ModalDialog,
                paint_order: order,
                rect_px: input_rect,
                clip_px: modal,
                label: input_label.to_owned(),
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::FocusModalInput,
            });
            cy += 34;
        }

        let items = active_modal_items(self.app);
        for (idx, (label, selected)) in items.into_iter().enumerate() {
            if cy + row_h > body_bottom {
                break;
            }
            let r = LayoutRectPx::new(modal.x + 12, cy, body_w, 22);
            let id = format!("modal.item.{idx}");
            let hovered = self.bridge.hovered_node_id.as_deref() == Some(&id);
            let fill = if selected {
                Some([self.colors.accent[0], self.colors.accent[1], self.colors.accent[2], 42])
            } else if hovered {
                Some(self.colors.hover_bg)
            } else {
                None
            };
            if fill.is_some() || selected {
                self.push_rect(ZLayer::ModalDialog, r, fill, selected.then_some(self.colors.accent));
            }
            self.push_text_scaled(ZLayer::ModalDialog, r, label.clone(), item_style.clone(), self.colors.text, Some(&id));
            let order = self.next_order();
            self.nodes.push(WebChromeNode {
                id,
                role: WebChromeRole::ModalItem,
                z_layer: ZLayer::ModalDialog,
                paint_order: order,
                rect_px: r,
                clip_px: modal,
                label,
                shortcut: None,
                enabled: true,
                disabled_reason: None,
                tooltip: None,
                action: WebChromeAction::SelectDialogItem(idx),
            });
            cy += row_h + 2;
        }
    }

    fn build_modal_footer(&mut self, modal: LayoutRectPx) {
        let cancel_rect = LayoutRectPx::new(modal.right() - 180, modal.bottom() - 34, 78, 24);
        let ok_rect = LayoutRectPx::new(modal.right() - 94, modal.bottom() - 34, 82, 24);
        let btn_style = UiTextStyle::ui_sans_bold(12.0);

        self.push_rect(ZLayer::ModalDialog, cancel_rect, Some(self.colors.panel), Some(self.colors.border));
        self.push_text_scaled(ZLayer::ModalDialog, cancel_rect, String::from("Cancel"), btn_style.clone(), self.colors.text, Some("modal.cancel"));
        let cancel_order = self.next_order();
        self.nodes.push(WebChromeNode {
            id: String::from("modal.cancel"),
            role: WebChromeRole::ModalButton,
            z_layer: ZLayer::ModalDialog,
            paint_order: cancel_order,
            rect_px: cancel_rect,
            clip_px: modal,
            label: String::from("Cancel"),
            shortcut: Some(String::from("Escape")),
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            action: WebChromeAction::DialogCancel,
        });

        let ok_enable = curvebook_ui_egui::modal_predicates::evaluate_dialog_button(&self.app.canvas_dialogs, self.app.document(), "ok")
            .unwrap_or_else(ControlEnablement::enabled);
        let ok_bg = if ok_enable.enabled { self.colors.accent } else { self.colors.panel };
        let ok_fg = if ok_enable.enabled { [255, 255, 255, 255] } else { self.colors.muted };
        self.push_rect(ZLayer::ModalDialog, ok_rect, Some(ok_bg), Some(self.colors.border));
        self.push_text_scaled(ZLayer::ModalDialog, ok_rect, String::from("OK"), btn_style, ok_fg, Some("modal.ok"));
        let ok_order = self.next_order();
        self.nodes.push(WebChromeNode {
            id: String::from("modal.ok"),
            role: WebChromeRole::ModalButton,
            z_layer: ZLayer::ModalDialog,
            paint_order: ok_order,
            rect_px: ok_rect,
            clip_px: modal,
            label: String::from("OK"),
            shortcut: Some(String::from("Enter")),
            enabled: ok_enable.enabled,
            disabled_reason: ok_enable.disabled_reason.clone(),
            tooltip: ok_enable.disabled_reason,
            action: WebChromeAction::DialogConfirm,
        });
    }

    fn build_context_menu(&mut self, ctx: &WebContextMenuState) {
        let items = context_menu_items_for_target(&ctx.target, self.app);
        if items.is_empty() {
            return;
        }
        let row_h = 24u32;
        let menu_w = 210u32.min(self.frame.total_width.saturating_sub(16).max(140));
        #[allow(clippy::cast_possible_truncation)]
        let menu_h = (items.len() as u32).saturating_mul(row_h).saturating_add(8);
        let x = ctx.anchor_px[0].min((self.frame.total_width.saturating_sub(menu_w)) as i32).max(4);
        let y = ctx.anchor_px[1].min((self.frame.total_height.saturating_sub(menu_h)) as i32).max(4);
        let menu_rect = LayoutRectPx::new(x, y, menu_w, menu_h);
        self.push_rect(ZLayer::ContextMenu, menu_rect, Some(self.colors.surface), Some(self.colors.border));

        let style = UiTextStyle::ui_sans(12.0);
        for (idx, (label, cmd_id, enabled)) in items.into_iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let r = LayoutRectPx::new(menu_rect.x + 4, menu_rect.y + 4 + (idx as i32) * (row_h as i32), menu_w.saturating_sub(8), row_h - 2);
            let node_id = format!("ctx.{}", cmd_id);
            if self.bridge.hovered_node_id.as_deref() == Some(&node_id) {
                self.push_rect(ZLayer::ContextMenu, r, Some(self.colors.hover_bg), None);
            }
            let fg = if enabled { self.colors.text } else { self.colors.muted };
            self.push_text_scaled(ZLayer::ContextMenu, r, label.clone(), style.clone(), fg, Some(&node_id));
            let action = match cmd_id.as_str() {
                "export.svg" => WebChromeAction::ExportFormat(WebExportFormat::Svg),
                "export.pdf" => WebChromeAction::ExportFormat(WebExportFormat::Pdf),
                "export.png" => WebChromeAction::ExportFormat(WebExportFormat::Png),
                _ => WebChromeAction::InvokeCommand(cmd_id),
            };
            let order = self.next_order();
            self.nodes.push(WebChromeNode {
                id: node_id,
                role: WebChromeRole::MenuItem,
                z_layer: ZLayer::ContextMenu,
                paint_order: order,
                rect_px: r,
                clip_px: menu_rect,
                label,
                shortcut: None,
                enabled,
                disabled_reason: (!enabled).then(|| String::from("Unavailable in current sheet")),
                tooltip: None,
                action,
            });
        }
    }

    fn build_hover_tooltip(&mut self) {
        let Some(hovered_id) = self.bridge.hovered_node_id.as_deref() else {
            return;
        };
        let Some(hovered_node) = self.nodes.iter().find(|n| n.id == hovered_id).cloned() else {
            return;
        };
        let Some(tip_text) = hovered_node.disabled_reason.as_ref().or(hovered_node.tooltip.as_ref()) else {
            return;
        };
        let first_line = tip_text.lines().next().unwrap_or(tip_text.as_str()).to_owned();
        if first_line.is_empty() {
            return;
        }
        let style = UiTextStyle::ui_sans(11.5);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tw = (self.measurer.measure(&style, &first_line, self.dpi).width_px.ceil() as u32)
            .saturating_add(16)
            .min(self.frame.total_width.saturating_sub(16).max(80));
        let th = 22u32;
        let tx = hovered_node.rect_px.x.min((self.frame.total_width.saturating_sub(tw)) as i32).max(4);
        let ty = (hovered_node.rect_px.bottom() + 4).min((self.frame.total_height.saturating_sub(th)) as i32).max(4);
        let tip_rect = LayoutRectPx::new(tx, ty, tw, th);
        self.push_rect(ZLayer::Tooltip, tip_rect, Some([20, 24, 34, 242]), Some(self.colors.accent));
        self.push_text_scaled(ZLayer::Tooltip, tip_rect, first_line.clone(), style, [255, 255, 255, 255], Some("tooltip.active"));
        let order = self.next_order();
        self.nodes.push(WebChromeNode {
            id: String::from("tooltip.active"),
            role: WebChromeRole::Tooltip,
            z_layer: ZLayer::Tooltip,
            paint_order: order,
            rect_px: tip_rect,
            clip_px: tip_rect,
            label: first_line,
            shortcut: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            action: WebChromeAction::None,
        });
    }
}

struct ToolbarItemSpec {
    id: String,
    caption: String,
    shortcut: Option<String>,
    enabled: bool,
    disabled_reason: Option<String>,
    tooltip: Option<String>,
    action: WebChromeAction,
}

fn build_contextual_toolbar_specs(app: &UiApp) -> Vec<ToolbarItemSpec> {
    let mut out = Vec::new();
    let doc = app.document();
    let active_is_result = doc.active_sheet_id.as_ref().is_some_and(|id| doc.result_sheet(id).is_some());
    let active_is_graph = doc.active_sheet_id.as_ref().is_some_and(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some());

    for entry in toolbar_tooltips(app) {
        let web_sc = WebShortcutRouter::display_shortcut(entry.button.command_id, &entry.tooltip.shortcut);
        let action = match entry.button.command_id {
            "project.open" => WebChromeAction::OpenFilePicker,
            "project.save" => WebChromeAction::ExportFormat(WebExportFormat::CurveBookArchive),
            "export.svg" => WebChromeAction::ExportFormat(WebExportFormat::Svg),
            "graph.setAxes" => WebChromeAction::InvokeCommand(String::from("ui.graph.axes.open")),
            other => WebChromeAction::InvokeCommand(other.to_owned()),
        };
        out.push(ToolbarItemSpec {
            id: entry.button.id.to_owned(),
            caption: entry.caption.to_owned(),
            shortcut: (!web_sc.is_empty()).then(|| web_sc.to_owned()),
            enabled: entry.tooltip.enabled,
            disabled_reason: (!entry.tooltip.enabled).then(|| format!("{} is unavailable in the current workspace state", entry.caption)),
            tooltip: Some(entry.tooltip.format_tooltip()),
            action,
        });
    }

    if active_is_result {
        out.push(ToolbarItemSpec {
            id: String::from("toolbar.edit_params"),
            caption: String::from("Params (Alt+T)"),
            shortcut: Some(String::from("Alt+T")),
            enabled: true,
            disabled_reason: None,
            tooltip: Some(String::from("Edit Analysis Parameters for active ResultSheet (Alt+T)")),
            action: WebChromeAction::InvokeCommand(String::from("ui.editParams.open")),
        });
    }
    if active_is_graph && let Some(tip) = command_tooltip(app, "export.png") {
        out.push(ToolbarItemSpec {
            id: String::from("toolbar.export_png"),
            caption: String::from("PNG"),
            shortcut: None,
            enabled: tip.enabled,
            disabled_reason: (!tip.enabled).then(|| String::from("No graph or layout sheet to export")),
            tooltip: Some(tip.format_tooltip()),
            action: WebChromeAction::ExportFormat(WebExportFormat::Png),
        });
    }
    out
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

fn modal_title(dialog: &ActiveDialog) -> &'static str {
    match dialog {
        ActiveDialog::None => "",
        ActiveDialog::AnalyzeSheet { .. } => "Analyze...",
        ActiveDialog::DuplicateAnalysisPrompt { .. } => "Analysis Already Exists",
        ActiveDialog::EditAnalysisParameters { .. } => "Edit Analysis Parameters (Alt+T)",
        ActiveDialog::ChangeDataAnalyzed { .. } => "Change Data Analyzed",
        ActiveDialog::ResultViewer { .. } => "Result Sheet Viewer",
        ActiveDialog::ConstantHookPicker { .. } => "Insert Hooked Constant",
        ActiveDialog::ChangeGraphType { .. } => "Change Graph Type",
        ActiveDialog::FormatGraph { .. } => "Format Graph",
        ActiveDialog::FormatAxes { .. } => "Format Axes",
        ActiveDialog::PairwiseComparisons { .. } => "Pairwise Comparisons",
        ActiveDialog::BubblePlotInspector { .. } => "Bubble Plot Channels",
        ActiveDialog::ApplyStyleFrom { .. } => "Apply Style From...",
        ActiveDialog::PageLayoutComposer { .. } => "Page Layout Composer",
        ActiveDialog::ExportFigure { .. } => "Export Publication Figure",
        ActiveDialog::ConvertArchetype { .. } => "Convert Table Archetype...",
        ActiveDialog::ImportReshape => "Import Reshape",
        ActiveDialog::ExcludeWithReason { .. } => "Exclude Selection with Reason",
        ActiveDialog::FormatSelectedPoints { .. } => "Format Selected Points...",
        ActiveDialog::PowerAndSampleSize => "Power and Sample Size",
        ActiveDialog::Print { .. } => "Print...",
        ActiveDialog::RandomizeGroups => "Randomize Subjects to Groups...",
        ActiveDialog::RandomSampleRows { .. } => "Random Sample Rows...",
        ActiveDialog::SimulateDistributions => "Simulate Distributions...",
        ActiveDialog::InsertModelEquation { .. } => "Insert Model Equation...",
        ActiveDialog::CommandPalette => "Command Palette (Alt+K / Cmd+K)",
        ActiveDialog::About => "About CurveBook",
    }
}

fn active_modal_input_text(app: &UiApp) -> Option<(&'static str, String)> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::CommandPalette => Some(("Filter", app.canvas_dialogs.command_palette.query.clone())),
        ActiveDialog::AnalyzeSheet { .. } => Some(("Search", app.canvas_dialogs.analyze.search_query.clone())),
        ActiveDialog::ExcludeWithReason { .. } => Some(("Reason Note", app.canvas_dialogs.exclude_with_reason.note.clone())),
        _ => None,
    }
}

fn active_modal_items(app: &UiApp) -> Vec<(String, bool)> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::CommandPalette => app
            .canvas_dialogs
            .command_palette
            .matches_for_session(app.session())
            .into_iter()
            .take(10)
            .enumerate()
            .map(|(i, e)| {
                let sc = WebShortcutRouter::display_shortcut(e.id, &e.shortcut);
                let label = if sc.is_empty() { e.label.to_owned() } else { format!("{} ({sc})", e.label) };
                (label, i == app.canvas_dialogs.command_palette.selected_index)
            })
            .collect(),
        ActiveDialog::AnalyzeSheet { .. } => app
            .canvas_dialogs
            .analyze
            .compatible_entries()
            .into_iter()
            .take(10)
            .map(|item| {
                let sel = app.canvas_dialogs.analyze.selected_entry_id.as_deref() == Some(item.id);
                (item.label.to_owned(), sel)
            })
            .collect(),
        ActiveDialog::EditAnalysisParameters { .. } => {
            if let Some(draft) = &app.canvas_dialogs.edit_params.draft {
                vec![
                    (format!("Analysis: {}", draft.effective_title()), true),
                    (format!("Confidence: {:.1}%", draft.confidence_level.percent()), false),
                    (format!("Source: {}", draft.source_sheet_id.as_str()), false),
                ]
            } else {
                vec![(String::from("Edit Analysis Parameters ready"), true)]
            }
        }
        ActiveDialog::ExportFigure { .. } => {
            let cur = app.canvas_dialogs.export_figure.format;
            vec![
                (String::from("SVG Vector"), matches!(cur, curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Svg)),
                (String::from("PDF 1.7 Vector"), matches!(cur, curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Pdf)),
                (String::from("PNG Raster (300 DPI)"), matches!(cur, curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Png)),
                (String::from("TIFF Raster (Deflate)"), matches!(cur, curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Tiff(_))),
            ]
        }
        ActiveDialog::ChangeGraphType { .. } => app
            .canvas_dialogs
            .change_graph_type
            .recommended_options()
            .into_iter()
            .take(8)
            .map(|opt| {
                let sel = opt.id == app.canvas_dialogs.change_graph_type.selected_option_id;
                (opt.label.to_owned(), sel)
            })
            .collect(),
        ActiveDialog::About => vec![(String::from("CurveBook Scientific Graphing & Biostatistics (WebAssembly)"), true)],
        _ => vec![(format!("{} ready", modal_title(&app.canvas_dialogs.active_dialog)), true)],
    }
}

fn context_menu_items_for_target(target: &str, app: &UiApp) -> Vec<(String, String, bool)> {
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

struct DropdownEntrySpec {
    command_id: String,
    label: String,
    shortcut: String,
    enabled: bool,
    disabled_reason: Option<String>,
    tooltip: String,
}

fn append_inline_variants(out: &mut Vec<DropdownEntrySpec>, base_id: &str, limit: usize) {
    let variants: &[(&str, &str, &str)] = match base_id {
        "ui.theme.set" => &[
            ("ui.theme.set:dark", "  Theme: Dark", "Switch interface to Dark theme"),
            ("ui.theme.set:light", "  Theme: Light", "Switch interface to Light theme"),
        ],
        "ui.previewFilter.set" => &[("ui.previewFilter.set:grayscale", "  Preview: Grayscale", "Toggle print grayscale canvas preview")],
        "ui.locale.set" => &[("ui.locale.set:de", "  Locale: Deutsch (de)", "Switch UI and decimal formatting to German")],
        _ => return,
    };
    for &(cmd_id, label, tip) in variants {
        if out.len() >= limit {
            break;
        }
        out.push(DropdownEntrySpec {
            command_id: String::from(cmd_id),
            label: String::from(label),
            shortcut: String::new(),
            enabled: true,
            disabled_reason: None,
            tooltip: String::from(tip),
        });
    }
}

fn build_dropdown_entries_for_section(_open_key: &str, section: &curvebook_ui_egui::MenuSectionTooltips, limit: usize) -> Vec<DropdownEntrySpec> {
    let mut out = Vec::new();
    for item in &section.items {
        // Automation-only hook must never surface in interactive user menus (`R10.6` / `R10.7`).
        if item.command_id == "ui.event.dispatch" {
            continue;
        }
        let is_remediation = item.command_id == "ui.result.applyRemediation";
        let enabled = if is_remediation { false } else { item.enabled };
        let disabled_reason = if is_remediation {
            Some(String::from("Diagnostic-only (QOL-7): inspect Result Diagnostic Tabs instead"))
        } else if !item.enabled {
            Some(format!("{} is disabled in current context", item.localized_label))
        } else {
            None
        };
        out.push(DropdownEntrySpec {
            command_id: item.command_id.to_owned(),
            label: item.localized_label.to_owned(),
            shortcut: item.shortcut.clone(),
            enabled,
            disabled_reason,
            tooltip: item.format_tooltip(),
        });
        append_inline_variants(&mut out, item.command_id, limit);
        if out.len() >= limit {
            break;
        }
    }
    out
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
    use crate::text_bridge::BrowserCanvasTextMeasurer;

    #[test]
    fn browser_reserved_shortcuts_map_cleanly_without_clashes() {
        let mut app = UiApp::new();
        app.execute_command("sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "title": "Groups" })).unwrap();
        for row in 0..3 {
            app.execute_command("table.setCell", json!({ "sheet_id": "d1", "row": row, "col": 0, "value": 10.0 + row as f64 })).unwrap();
            app.execute_command("table.setCell", json!({ "sheet_id": "d1", "row": row, "col": 1, "value": 20.0 + row as f64 })).unwrap();
        }
        app.execute_command("analysis.run", json!({ "source_sheet_id": "d1", "kind": "descriptive_stats" })).unwrap();

        // `Alt+W` maps browser-reserved `Cmd/Ctrl+W` cleanly.
        let close_route = WebShortcutRouter::route_key_down(&app, "w", KeyModifiers { alt: true, ..Default::default() });
        assert!(close_route.handled && close_route.prevent_default && close_route.used_web_alias);
        assert_eq!(close_route.action, WebShortcutAction::CloseWorkspacePrompt);

        // `Alt+Shift+N` maps browser-reserved `Cmd/Ctrl+N`.
        let new_route = WebShortcutRouter::route_key_down(&app, "n", KeyModifiers { alt: true, shift: true, ..Default::default() });
        assert!(new_route.handled && new_route.used_web_alias);

        // `Alt+K` maps Command Palette cleanly without browser search-bar clash.
        let pal_route = WebShortcutRouter::route_key_down(&app, "k", KeyModifiers { alt: true, ..Default::default() });
        assert_eq!(pal_route.action, WebShortcutAction::ExecuteCommand { command_id: String::from("ui.palette.open"), params: json!({}) });

        // On an active ResultSheet, `Alt+T` (and canvas `Cmd+T`) opens EditAnalysisParameters (`R10.5`).
        let rs_id = app.document().result_sheets[0].id.clone();
        app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).unwrap();
        let t_route = WebShortcutRouter::route_key_down(&app, "t", KeyModifiers { alt: true, ..Default::default() });
        assert_eq!(t_route.action, WebShortcutAction::OpenEditAnalysisParameters(rs_id.as_str().to_owned()));

        assert!(all_shortcuts_have_web_policy());
    }

    #[test]
    fn modal_backdrop_blocks_workspace_click_through_and_dropdown_routes_commands() {
        let mut app = UiApp::new();
        let mut bridge = WebChromeBridge::new();
        let measurer = BrowserCanvasTextMeasurer::new();
        let viewport = ViewportMetrics::new(1024.0, 640.0, 1.0);

        let frame1 = curvebook_ui_egui::compute_workspace_layout(&app, 1024, 640);
        let layout1 = bridge.build_layout(&app, viewport, &frame1, &measurer, 96);
        let file_btn = layout1.find_node("menu.file").expect("File menu node exists").rect_px;
        let disp = bridge.handle_pointer_down(
            &mut app,
            &layout1,
            [f64::from(file_btn.x + 4), f64::from(file_btn.y + 4)],
            [file_btn.x + 4, file_btn.y + 4],
            PointerButton::Primary,
        );
        assert!(matches!(disp, WebChromeDispatch::HandledRedraw { .. }));
        assert_eq!(bridge.open_menu_key.as_deref(), Some("menu.file"));

        // Open Command Palette modal; clicking outside modal dialog hits `ModalBackdrop` and blocks workspace.
        app.canvas_dialogs.open_command_palette();
        bridge.close_popups();
        let frame2 = curvebook_ui_egui::compute_workspace_layout(&app, 1024, 640);
        let layout2 = bridge.build_layout(&app, viewport, &frame2, &measurer, 96);
        let hit_corner = bridge.handle_pointer_down(&mut app, &layout2, [5.0, 5.0], [5, 5], PointerButton::Primary);
        assert_eq!(hit_corner, WebChromeDispatch::BlockedByModalBackdrop);
    }
}
