//! Human-equivalent `gui` MCP tool definitions and validated dispatch (`R8.6`).
//!
//! Exposes the 8 semantic GUI tools (`ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`,
//! `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`)
//! across both in-process [`GuiSessionHost`](crate::tools::GuiSessionHost) bindings and
//! loopback `--control` desktop windows without reversing the `L6` layer boundary.

use curvebook_doc::ErrorCode;
use curvebook_engine::cmd::Effects;
use serde_json::{Map, Value, json};

use crate::errors::{self, StructuredErrorPayload};
use crate::formats;
use crate::tools::{
    ContentBlock, McpServerState, ToolCallError, ToolCallResult, ToolDefinition, dispatch_gui_backend, optional_bool, optional_non_empty_str,
    optional_u32_bounded, required_non_empty_str,
};
use crate::toolsets::ToolAnnotations;

/// Ordered list of the 8 `gui` MCP tool names exposed when `--gui-only` or the `gui` toolset is active.
pub const GUI_TOOL_NAMES: [&str; 8] =
    ["ui_inspect", "ui_screenshot", "ui_pointer_gesture", "ui_type_text", "ui_press_key", "ui_invoke_menu", "ui_switch_tab", "ui_answer_dialog"];

const VALID_INSPECT_SCOPES: [&str; 9] = ["all", "workspace", "dialog", "grid", "navigator", "canvas", "menus", "toolbar", "tabs"];
const VALID_SCREENSHOT_TARGETS: [&str; 5] = ["full_window", "workspace", "viewport", "canvas", "dialog"];

/// Worst-case effects for GUI tools that can dispatch arbitrary menu items, shortcuts, or modal actions.
const GUI_DISPATCH_WORST_CASE_EFFECTS: Effects = Effects::MUTATES_DOCUMENT
    .with(Effects::CHANGES_UI_STATE)
    .with(Effects::MAY_DISCARD_DATA)
    .with(Effects::READS_FILESYSTEM)
    .with(Effects::WRITES_FILESYSTEM)
    .with(Effects::LAUNCHES_PROCESS);

/// Worst-case effects for text entry that can overwrite cells, paste tables, or trigger palette commands.
const GUI_TYPE_TEXT_WORST_CASE_EFFECTS: Effects = Effects::MUTATES_DOCUMENT
    .with(Effects::CHANGES_UI_STATE)
    .with(Effects::MAY_DISCARD_DATA)
    .with(Effects::READS_FILESYSTEM)
    .with(Effects::WRITES_FILESYSTEM);

/// Worst-case effects for modal dialog updates/commits (including lossy conversion, export, or OS print).
const GUI_DIALOG_WORST_CASE_EFFECTS: Effects = Effects::MUTATES_DOCUMENT
    .with(Effects::CHANGES_UI_STATE)
    .with(Effects::MAY_DISCARD_DATA)
    .with(Effects::WRITES_FILESYSTEM)
    .with(Effects::LAUNCHES_PROCESS);

/// Return `true` when `name` identifies one of the 8 `gui` MCP tools.
#[must_use]
pub fn is_gui_tool_name(name: &str) -> bool {
    matches!(
        name,
        "ui_inspect"
            | "ui_screenshot"
            | "ui_pointer_gesture"
            | "ui_type_text"
            | "ui_press_key"
            | "ui_invoke_menu"
            | "ui_switch_tab"
            | "ui_answer_dialog"
    )
}

/// Return all 8 `gui` MCP [`ToolDefinition`] entries with schemas and `Effects`-aligned annotations.
#[must_use]
pub fn list_gui_tools() -> Vec<ToolDefinition> {
    vec![
        def_ui_inspect(),
        def_ui_screenshot(),
        def_ui_pointer_gesture(),
        def_ui_type_text(),
        def_ui_press_key(),
        def_ui_invoke_menu(),
        def_ui_switch_tab(),
        def_ui_answer_dialog(),
    ]
}

fn def_ui_inspect() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_inspect"),
        description: String::from(
            "Inspect the visible CurveBook GUI tree (shell, navigator, menus/toolbars, active modal/wizard with enabled/disabled_reason predicates, visible grid cells, and canvas hit targets).",
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": VALID_INSPECT_SCOPES,
                    "description": "Optional subtree scope filter (defaults to 'all')."
                },
                "include_bounds": {
                    "type": "boolean",
                    "description": "Include widget/hit-target bounding boxes in the snapshot."
                }
            },
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT)),
    }
}

fn def_ui_screenshot() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_screenshot"),
        description: String::from(
            "Capture a deterministic PNG screenshot of the live CurveBook UI, returning inline PNG image content alongside pixel/DPI metadata (or atomically writing/overwriting an optional host output path outside --gui-only).",
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": VALID_SCREENSHOT_TARGETS,
                    "description": "Optional visual surface target (defaults to 'workspace')."
                },
                "width_px": {
                    "type": "integer",
                    "minimum": 64,
                    "maximum": 4096,
                    "description": "Image width in pixels (64..=4096)."
                },
                "height_px": {
                    "type": "integer",
                    "minimum": 64,
                    "maximum": 4096,
                    "description": "Image height in pixels (64..=4096)."
                },
                "dpi": {
                    "type": "integer",
                    "minimum": 72,
                    "maximum": 600,
                    "description": "Optional render DPI (72..=600)."
                },
                "path": {
                    "type": "string",
                    "description": "Optional host output path (overwrites existing file; rejected with gui_only_violation when --gui-only is active)."
                }
            },
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT)),
    }
}

fn modifiers_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "shift": { "type": "boolean" },
                    "ctrl": { "type": "boolean" },
                    "alt": { "type": "boolean" },
                    "meta": { "type": "boolean" },
                    "cmd": { "type": "boolean" }
                },
                "additionalProperties": false
            },
            {
                "type": "array",
                "items": { "type": "string" }
            }
        ],
        "description": "Optional modifier keys ({shift, ctrl, alt, meta, cmd} object or string list)."
    })
}

fn def_ui_pointer_gesture() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_pointer_gesture"),
        description: String::from("Dispatch a visible mouse/trackpad gesture against a widget_id or logical (x, y) viewport coordinates."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["click", "double_click", "right_click", "drag", "hover", "wheel"],
                    "description": "Pointer gesture kind."
                },
                "widget_id": { "type": "string", "description": "Target widget or canvas hit-target ID from ui_inspect." },
                "target_id": { "type": "string", "description": "Alias for widget_id." },
                "x": { "type": "number", "description": "Logical X coordinate when not addressing by widget_id." },
                "y": { "type": "number", "description": "Logical Y coordinate when not addressing by widget_id." },
                "to_widget_id": { "type": "string", "description": "Destination widget/hit-target ID for 'drag' gestures." },
                "to_x": { "type": "number", "description": "Destination logical X coordinate for 'drag' gestures." },
                "to_y": { "type": "number", "description": "Destination logical Y coordinate for 'drag' gestures." },
                "modifiers": modifiers_schema(),
                "delta_y": { "type": "number", "description": "Vertical scroll delta for 'wheel' gestures." },
                "hover_fraction_x": { "type": "number", "description": "Optional fractional X coordinate [0.0, 1.0] for power curve hover." },
                "hover_n": { "type": "number", "description": "Optional sample-size N coordinate for power curve hover." },
                "metrics": { "type": "object", "description": "Optional ViewportMetrics ({width_px, height_px, scale_factor}) for pointer dispatch." }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(GUI_DISPATCH_WORST_CASE_EFFECTS),
    }
}

fn def_ui_type_text() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_type_text"),
        description: String::from(
            "Type or paste UTF-8 text (including multi-cell TSV blocks with tabs/newlines) into the focused cell/field or specified widget_id.",
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Exact UTF-8 text or TSV payload to enter (whitespace preserved)." },
                "widget_id": { "type": "string", "description": "Optional widget ID to focus before entering text." },
                "clear_existing": { "type": "boolean", "description": "Clear existing cell or field content before typing." },
                "commit_enter": { "type": "boolean", "description": "Commit the inline edit or field with Enter after typing." }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(GUI_TYPE_TEXT_WORST_CASE_EFFECTS),
    }
}

fn def_ui_press_key() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_press_key"),
        description: String::from("Press a keyboard key or shortcut (e.g. Enter, Escape, Tab, Delete, Backspace, Cmd/Ctrl+T, Cmd/Ctrl+E)."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key identifier such as 'Enter', 'Escape', 'Tab', 'Delete', 'ArrowDown', 't', or 'e'." },
                "modifiers": modifiers_schema()
            },
            "required": ["key"],
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(GUI_DISPATCH_WORST_CASE_EFFECTS),
    }
}

fn def_ui_invoke_menu() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_invoke_menu"),
        description: String::from("Invoke a visible menu bar, toolbar, or context-menu item by path or command_id (fails if disabled)."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ],
                    "description": "Visible menu path (e.g. 'Analyze > Column Analyses > One-way ANOVA') or array of segments."
                },
                "command_id": { "type": "string", "description": "Command identifier associated with the menu/toolbar item." },
                "params": { "type": "object", "description": "Optional parameter payload for the invoked command or dialog initializer." }
            },
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(GUI_DISPATCH_WORST_CASE_EFFECTS),
    }
}

fn def_ui_switch_tab() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_switch_tab"),
        description: String::from("Switch the active sheet, modal/wizard stage, or ResultSheet sub-tab (fails with disabled_reason if grayed out)."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "tab": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "integer", "minimum": 0 }
                    ],
                    "description": "Tab identifier, visible tab label, or zero-based tab index."
                },
                "tab_id": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "integer", "minimum": 0 }
                    ],
                    "description": "Alias for tab."
                },
                "sheet_id": { "type": "string", "description": "Optional sheet identifier to activate before/when selecting a sub-tab." },
                "widget_id": { "type": "string", "description": "Optional tab strip or widget identifier." }
            },
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(Effects::MUTATES_DOCUMENT.with(Effects::CHANGES_UI_STATE).with(Effects::IDEMPOTENT)),
    }
}

fn def_ui_answer_dialog() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_answer_dialog"),
        description: String::from(
            "Update enabled modal/wizard controls by widget_id (`fields`/`values`/`patches`) and/or click a modal action button (e.g. 'ok', 'cancel', 'next', 'back', 'create', 'apply', 'preview').",
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "description": "Modal button action to activate (e.g. 'ok', 'cancel', 'back', 'next', 'create', 'apply', 'preview')." },
                "fields": { "type": "object", "description": "Map of modal widget_id -> value to set before clicking action." },
                "values": { "type": "object", "description": "Alias for fields mapping widget_id -> value." },
                "patches": { "type": "object", "description": "Alias for fields mapping widget_id -> value." },
                "force_create_new": { "type": "boolean", "description": "Optional flag creating a new ResultSheet directly when committing AnalyzeSheet." },
                "choice": { "type": "string", "description": "Optional selection for prompt dialogs (e.g. 'create_new' or 'modify_existing')." }
            },
            "additionalProperties": false
        }),
        annotations: ToolAnnotations::from_effects(GUI_DIALOG_WORST_CASE_EFFECTS),
    }
}

/// Validate parameters and dispatch one of the 8 `gui` MCP tools against `state`.
pub(crate) fn dispatch_gui_tool(state: &mut McpServerState, name: &str, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    match name {
        "ui_inspect" => handle_ui_inspect(state, args),
        "ui_screenshot" => handle_ui_screenshot(state, args),
        "ui_pointer_gesture" => handle_ui_pointer_gesture(state, args),
        "ui_type_text" => handle_ui_type_text(state, args),
        "ui_press_key" => handle_ui_press_key(state, args),
        "ui_invoke_menu" => handle_ui_invoke_menu(state, args),
        "ui_switch_tab" => handle_ui_switch_tab(state, args),
        "ui_answer_dialog" => handle_ui_answer_dialog(state, args),
        _ => Err(ToolCallError::UnknownTool(name.to_owned())),
    }
}

fn dispatch_json_result(state: &mut McpServerState, method: &str, params: Value) -> Result<ToolCallResult, ToolCallError> {
    match dispatch_gui_backend(state, method, params) {
        Ok(val) => ToolCallResult::ok_json(&val),
        Err(payload) => Ok(errors::result_from_payload(&payload)),
    }
}

fn handle_ui_inspect(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    reject_unknown_args("ui_inspect", args, &["scope", "include_bounds"])?;
    let mut params = Map::new();
    if let Some(raw_scope) = optional_non_empty_str(args, "scope")? {
        let scope = raw_scope.to_ascii_lowercase();
        if !VALID_INSPECT_SCOPES.contains(&scope.as_str()) {
            return Err(ToolCallError::InvalidParams(format!(
                "unsupported ui_inspect scope '{raw_scope}'; expected one of: all, workspace, dialog, grid, navigator, canvas, menus, toolbar, tabs"
            )));
        }
        params.insert(String::from("scope"), Value::String(scope));
    }
    if let Some(include_bounds) = optional_bool(args, "include_bounds")? {
        params.insert(String::from("include_bounds"), Value::Bool(include_bounds));
    }
    dispatch_json_result(state, "ui.inspect", Value::Object(params))
}

fn handle_ui_screenshot(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let mut params = Map::new();
    if let Some(raw_target) = optional_non_empty_str(args, "target")? {
        let target = raw_target.to_ascii_lowercase();
        if !VALID_SCREENSHOT_TARGETS.contains(&target.as_str()) {
            return Err(ToolCallError::InvalidParams(format!(
                "unsupported ui_screenshot target '{raw_target}'; expected full_window, workspace, viewport, canvas, or dialog"
            )));
        }
        params.insert(String::from("target"), Value::String(target));
    }
    if let Some(width_px) = optional_u32_bounded(args, "width_px", 64, 4096)? {
        params.insert(String::from("width_px"), json!(width_px));
    }
    if let Some(height_px) = optional_u32_bounded(args, "height_px", 64, 4096)? {
        params.insert(String::from("height_px"), json!(height_px));
    }
    if let Some(dpi) = optional_u32_bounded(args, "dpi", 72, 600)? {
        params.insert(String::from("dpi"), json!(dpi));
    }
    if let Some(raw_path) = optional_non_empty_str(args, "path")? {
        if state.is_gui_only() {
            return Ok(errors::result_from_payload(
                &StructuredErrorPayload::new(
                    ErrorCode::GuiOnlyViolation,
                    "direct filesystem writes via `ui_screenshot` `path` are blocked under --gui-only",
                )
                .with_remediation("Omit `path` and inspect the returned in-memory `data_base64` PNG instead."),
            ));
        }
        let resolved = match formats::resolve_mcp_path(state, &raw_path) {
            Ok(p) => p,
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
        params.insert(String::from("path"), Value::String(resolved.to_string_lossy().into_owned()));
    }

    match dispatch_gui_backend(state, "ui.screenshot", Value::Object(params)) {
        Ok(raw) => format_screenshot_result(raw),
        Err(payload) => Ok(errors::result_from_payload(&payload)),
    }
}

fn format_screenshot_result(raw: Value) -> Result<ToolCallResult, ToolCallError> {
    let Some(obj) = raw.as_object() else {
        return Ok(errors::result_from_payload(&StructuredErrorPayload::new(
            ErrorCode::InternalError,
            "ui.screenshot backend returned a non-object response",
        )));
    };
    let b64_opt = obj
        .get("data_base64")
        .or_else(|| obj.get("png_base64"))
        .or_else(|| obj.get("data"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let Some(data) = b64_opt else {
        return Ok(errors::result_from_payload(
            &StructuredErrorPayload::new(ErrorCode::InternalError, "ui.screenshot response did not include base64 PNG data (`data_base64`)")
                .with_details(raw),
        ));
    };

    let mut metadata = obj.clone();
    metadata.remove("data_base64");
    metadata.remove("png_base64");
    metadata.remove("data");
    metadata.entry(String::from("format")).or_insert_with(|| Value::String(String::from("png")));

    let meta_val = Value::Object(metadata);
    let meta_text = serde_json::to_string_pretty(&meta_val).unwrap_or_default();
    Ok(ToolCallResult {
        content: vec![ContentBlock::Image { data, mime_type: String::from("image/png") }, ContentBlock::Text { text: meta_text }],
        is_error: false,
        structured_content: Some(meta_val),
    })
}

fn handle_ui_pointer_gesture(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let raw_action = required_non_empty_str(args, "action")?;
    let action = normalize_pointer_action(&raw_action)?;
    let widget_id = optional_non_empty_str(args, "widget_id")?.or(optional_non_empty_str(args, "target_id")?);
    let x = optional_finite_f64(args, "x")?;
    let y = optional_finite_f64(args, "y")?;
    if x.is_some() != y.is_some() {
        return Err(ToolCallError::InvalidParams(String::from("ui_pointer_gesture requires both \"x\" and \"y\" when using coordinates")));
    }
    if widget_id.is_none() && x.is_none() {
        return Err(ToolCallError::InvalidParams(String::from("ui_pointer_gesture requires \"widget_id\" or (\"x\", \"y\") coordinates")));
    }

    let to_widget_id = optional_non_empty_str(args, "to_widget_id")?;
    let to_x = optional_finite_f64(args, "to_x")?;
    let to_y = optional_finite_f64(args, "to_y")?;
    if to_x.is_some() != to_y.is_some() {
        return Err(ToolCallError::InvalidParams(String::from("ui_pointer_gesture requires both \"to_x\" and \"to_y\" for drag destination")));
    }
    if action == "drag" && to_widget_id.is_none() && to_x.is_none() {
        return Err(ToolCallError::InvalidParams(String::from(
            "ui_pointer_gesture with action=\"drag\" requires \"to_widget_id\" or (\"to_x\", \"to_y\")",
        )));
    }

    let delta_y = optional_finite_f64(args, "delta_y")?;
    let hover_fraction_x = optional_finite_f64(args, "hover_fraction_x")?;
    let hover_n = optional_finite_f64(args, "hover_n")?;
    let metrics = optional_object(args, "metrics")?;
    let modifiers = optional_modifiers(args, "modifiers")?;

    let mut params = Map::new();
    params.insert(String::from("action"), Value::String(action.to_owned()));
    insert_opt_str(&mut params, "widget_id", widget_id);
    insert_opt_f64(&mut params, "x", x);
    insert_opt_f64(&mut params, "y", y);
    insert_opt_str(&mut params, "to_widget_id", to_widget_id);
    insert_opt_f64(&mut params, "to_x", to_x);
    insert_opt_f64(&mut params, "to_y", to_y);
    insert_opt_f64(&mut params, "delta_y", delta_y);
    insert_opt_f64(&mut params, "hover_fraction_x", hover_fraction_x);
    insert_opt_f64(&mut params, "hover_n", hover_n);
    if let Some(m) = metrics {
        params.insert(String::from("metrics"), m);
    }
    if let Some(mods) = modifiers {
        params.insert(String::from("modifiers"), mods);
    }

    dispatch_json_result(state, "ui.pointer_gesture", Value::Object(params))
}

fn handle_ui_type_text(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let Some(text) = args.get("text").and_then(Value::as_str) else {
        return Err(ToolCallError::InvalidParams(String::from("ui_type_text requires a string parameter \"text\"")));
    };
    let mut params = Map::new();
    params.insert(String::from("text"), Value::String(text.to_owned()));
    insert_opt_str(&mut params, "widget_id", optional_non_empty_str(args, "widget_id")?);
    if let Some(clear_existing) = optional_bool(args, "clear_existing")? {
        params.insert(String::from("clear_existing"), Value::Bool(clear_existing));
    }
    if let Some(commit_enter) = optional_bool(args, "commit_enter")? {
        params.insert(String::from("commit_enter"), Value::Bool(commit_enter));
    }
    dispatch_json_result(state, "ui.type_text", Value::Object(params))
}

fn handle_ui_press_key(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let key = required_non_empty_str(args, "key")?;
    let mut params = Map::new();
    params.insert(String::from("key"), Value::String(key));
    if let Some(mods) = optional_modifiers(args, "modifiers")? {
        params.insert(String::from("modifiers"), mods);
    }
    dispatch_json_result(state, "ui.press_key", Value::Object(params))
}

fn handle_ui_invoke_menu(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let path = optional_menu_path(args, "path")?;
    let command_id = optional_non_empty_str(args, "command_id")?;
    let extra_params = optional_object(args, "params")?;
    if path.is_none() && command_id.is_none() {
        return Err(ToolCallError::InvalidParams(String::from("ui_invoke_menu requires \"path\" or \"command_id\"")));
    }

    let mut extra = extra_params.unwrap_or_else(|| json!({}));
    if let Err(err) = formats::sanitize_command_params(state, &mut extra) {
        return Ok(errors::result_from_engine_error(&err));
    }
    if state.is_gui_only()
        && let Some(blocked) = check_gui_only_menu_restriction(command_id.as_deref(), path.as_ref(), &extra)
    {
        return Ok(blocked);
    }

    let mut params = Map::new();
    if let Some(p) = path {
        params.insert(String::from("path"), p);
    }
    insert_opt_str(&mut params, "command_id", command_id);
    // Always supply an object for `params` (never `null`/omitted) so standard non-`ui.*` engine
    // menu items pass strict root-object schema validation (`R2-S12-03`).
    params.insert(String::from("params"), extra);
    dispatch_json_result(state, "ui.invoke_menu", Value::Object(params))
}

fn handle_ui_switch_tab(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let tab = optional_tab_target(args, "tab")?.or(optional_tab_target(args, "tab_id")?);
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let widget_id = optional_non_empty_str(args, "widget_id")?;
    if tab.is_none() && sheet_id.is_none() && widget_id.is_none() {
        return Err(ToolCallError::InvalidParams(String::from("ui_switch_tab requires \"tab\" (or \"tab_id\"), \"sheet_id\", or \"widget_id\"")));
    }

    let mut params = Map::new();
    if let Some(t) = tab {
        params.insert(String::from("tab"), t);
    }
    insert_opt_str(&mut params, "sheet_id", sheet_id);
    insert_opt_str(&mut params, "widget_id", widget_id);
    dispatch_json_result(state, "ui.switch_tab", Value::Object(params))
}

fn handle_ui_answer_dialog(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    reject_unknown_args("ui_answer_dialog", args, &["action", "fields", "values", "patches", "force_create_new", "choice"])?;
    let action = optional_non_empty_str(args, "action")?;
    let fields = optional_object(args, "fields")?;
    let values = optional_object(args, "values")?;
    let patches = optional_object(args, "patches")?;
    let force_create_new = optional_bool(args, "force_create_new")?;
    let choice = optional_non_empty_str(args, "choice")?;
    if action.is_none() && fields.is_none() && values.is_none() && patches.is_none() && choice.is_none() {
        return Err(ToolCallError::InvalidParams(String::from(
            "ui_answer_dialog requires \"action\", \"fields\", \"values\", \"patches\", or \"choice\"",
        )));
    }

    if state.is_gui_only() {
        for map_val in [&fields, &values, &patches].into_iter().flatten() {
            if map_requests_os_print_submission(map_val) {
                return Ok(gui_only_subprocess_blocked("ui_answer_dialog.submit_to_os", "OS print spooler subprocess submission (`submit_to_os`)"));
            }
        }
    }

    let mut params = Map::new();
    insert_opt_str(&mut params, "action", action);
    insert_opt_str(&mut params, "choice", choice);
    if let Some(force) = force_create_new {
        params.insert(String::from("force_create_new"), Value::Bool(force));
    }
    if let Some(f) = fields {
        params.insert(String::from("fields"), f);
    }
    if let Some(v) = values {
        params.insert(String::from("values"), v);
    }
    if let Some(p) = patches {
        params.insert(String::from("patches"), p);
    }
    dispatch_json_result(state, "ui.answer_dialog", Value::Object(params))
}

fn normalize_pointer_action(raw: &str) -> Result<&'static str, ToolCallError> {
    match raw.trim().replace(['-', ' '], "_").to_ascii_lowercase().as_str() {
        "click" | "tap" => Ok("click"),
        "double_click" | "doubleclick" => Ok("double_click"),
        "right_click" | "rightclick" | "context_click" | "contextclick" => Ok("right_click"),
        "drag" => Ok("drag"),
        "hover" | "move" => Ok("hover"),
        "wheel" | "scroll" => Ok("wheel"),
        _ => Err(ToolCallError::InvalidParams(format!(
            "unsupported ui_pointer_gesture action '{raw}'; expected click, double_click, right_click, drag, hover, or wheel"
        ))),
    }
}

fn optional_finite_f64(args: &Map<String, Value>, key: &str) -> Result<Option<f64>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(num) = val.as_f64() else {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a number")));
    };
    if !num.is_finite() {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a finite number")));
    }
    Ok(Some(num))
}

fn optional_tab_target(args: &Map<String, Value>, key: &str) -> Result<Option<Value>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    if let Some(idx) = val.as_u64() {
        return Ok(Some(json!(idx)));
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must not be blank")));
        }
        return Ok(Some(Value::String(trimmed.to_owned())));
    }
    Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a non-empty string or non-negative integer")))
}

fn optional_object(args: &Map<String, Value>, key: &str) -> Result<Option<Value>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    if !val.is_object() {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a JSON object")));
    }
    Ok(Some(val.clone()))
}

fn optional_menu_path(args: &Map<String, Value>, key: &str) -> Result<Option<Value>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must not be blank")));
        }
        return Ok(Some(Value::String(trimmed.to_owned())));
    }
    if let Some(items) = val.as_array() {
        if items.is_empty() {
            return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" array must not be empty")));
        }
        let mut segments = Vec::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            let seg = item
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ToolCallError::InvalidParams(format!("{key}[{idx}] must be a non-empty string")))?;
            segments.push(Value::String(seg.to_owned()));
        }
        return Ok(Some(Value::Array(segments)));
    }
    Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a string or non-empty string array")))
}

fn optional_modifiers(args: &Map<String, Value>, key: &str) -> Result<Option<Value>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let (mut shift, mut ctrl, mut alt, mut meta) = (false, false, false, false);
    if let Some(obj) = val.as_object() {
        shift = optional_bool(obj, "shift")?.unwrap_or(false);
        ctrl = optional_bool(obj, "ctrl")?.unwrap_or(false);
        alt = optional_bool(obj, "alt")?.unwrap_or(false);
        meta = optional_bool(obj, "meta")?.unwrap_or(false) || optional_bool(obj, "cmd")?.unwrap_or(false);
        return Ok(Some(json!({ "shift": shift, "ctrl": ctrl, "alt": alt, "meta": meta })));
    }
    if let Some(arr) = val.as_array() {
        for (idx, item) in arr.iter().enumerate() {
            let flag = item.as_str().ok_or_else(|| ToolCallError::InvalidParams(format!("{key}[{idx}] must be a string modifier name")))?;
            match flag.trim().to_ascii_lowercase().as_str() {
                "shift" => shift = true,
                "ctrl" | "control" => ctrl = true,
                "alt" | "option" => alt = true,
                "meta" | "cmd" | "command" | "super" => meta = true,
                other => return Err(ToolCallError::InvalidParams(format!("unknown modifier flag '{other}' in {key}[{idx}]"))),
            }
        }
        return Ok(Some(json!({ "shift": shift, "ctrl": ctrl, "alt": alt, "meta": meta })));
    }
    Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be an object or string array")))
}

fn insert_opt_str(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), Value::String(v));
    }
}

fn reject_unknown_args(tool_name: &str, args: &Map<String, Value>, allowed: &[&str]) -> Result<(), ToolCallError> {
    for key in args.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ToolCallError::InvalidParams(format!("unsupported parameter \"{key}\" for {tool_name}")));
        }
    }
    Ok(())
}

fn normalize_menu_token(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

fn menu_path_matches_tokens(path: Option<&Value>, candidates: &[&str]) -> bool {
    let Some(val) = path else {
        return false;
    };
    let joined = match val {
        Value::String(s) => s.to_owned(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("/"),
        _ => return false,
    };
    let normalized = normalize_menu_token(&joined);
    candidates.iter().any(|c| normalized == *c)
}

fn map_requests_os_print_submission(val: &Value) -> bool {
    val.as_object().is_some_and(|obj| {
        obj.get("submit_to_os").and_then(Value::as_bool) == Some(true) || obj.get("print.submit_to_os").and_then(Value::as_bool) == Some(true)
    })
}

fn gui_only_subprocess_blocked(target: &str, helper_desc: &str) -> ToolCallResult {
    let payload = StructuredErrorPayload::new(ErrorCode::GuiOnlyViolation, format!("{helper_desc} is forbidden under --gui-only policy"))
        .with_remediation("Inspect content inside the window via `ui_inspect` or `ui_screenshot` without invoking host OS helpers.")
        .with_details(json!({ "target": target, "policy": "gui_only" }));
    errors::result_from_payload(&payload)
}

fn check_gui_only_menu_restriction(command_id: Option<&str>, path: Option<&Value>, extra: &Value) -> Option<ToolCallResult> {
    let is_copy_methods = command_id == Some("ui.result.copyMethodsReport")
        || menu_path_matches_tokens(path, &["analyzecopymethodsreport", "analyzecopymethodsstatisticalreporting", "copymethodsreport"]);
    if is_copy_methods {
        return Some(gui_only_subprocess_blocked("ui.result.copyMethodsReport", "OS clipboard helper `ui.result.copyMethodsReport`"));
    }

    if command_id == Some("file.print") {
        return Some(gui_only_subprocess_blocked("file.print", "OS print helper `file.print`"));
    }

    let is_print_open = command_id == Some("ui.print.open") || menu_path_matches_tokens(path, &["fileprint", "print"]);
    if is_print_open && map_requests_os_print_submission(extra) {
        return Some(gui_only_subprocess_blocked("ui.print.open", "OS print spooler subprocess submission (`submit_to_os`)"));
    }

    if command_id == Some("ui.screenshot") && extra.get("path").is_some_and(|v| !v.is_null()) {
        return Some(gui_only_subprocess_blocked("ui.screenshot.path", "ui.screenshot disk `path` output via ui_invoke_menu"));
    }

    None
}

fn insert_opt_f64(map: &mut Map<String, Value>, key: &str, value: Option<f64>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), json!(v));
    }
}
