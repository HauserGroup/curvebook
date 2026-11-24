//! Human-equivalent `gui` MCP tool definitions and validated dispatch (`R8.6`).
//!
//! Exposes the 8 semantic GUI tools (`ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`,
//! `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`)
//! across both in-process [`GuiSessionHost`](crate::tools::GuiSessionHost) bindings and
//! loopback `--control` desktop windows without reversing the `L6` layer boundary.

use curvebook_doc::ErrorCode;
use curvebook_engine::cmd::Effects;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::errors::{self, StructuredErrorPayload};
use crate::formats;
use crate::tools::{
    ContentBlock, McpServerState, ToolCallError, ToolCallResult, ToolDefinition, deserialize_args, dispatch_gui_backend, trim_opt_non_empty,
    trim_required_non_empty,
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
    .into_iter()
    .map(|mut def| {
        def.input_schema = crate::toolsets::compact_json_schema(def.input_schema);
        def
    })
    .collect()
}

fn metrics_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "logical_width": { "type": "number", "exclusiveMinimum": 0.0, "description": "Logical viewport width in points (160..=16384)." },
            "logical_height": { "type": "number", "exclusiveMinimum": 0.0, "description": "Logical viewport height in points (120..=16384)." },
            "scale_factor": { "type": "number", "exclusiveMinimum": 0.0, "description": "HiDPI display scale factor (0.5..=8.0, default 1.0)." },
            "width_px": { "type": "number", "exclusiveMinimum": 0.0, "description": "Alias for logical_width." },
            "height_px": { "type": "number", "exclusiveMinimum": 0.0, "description": "Alias for logical_height." }
        },
        "additionalProperties": false,
        "description": "Optional ViewportMetrics ({logical_width, logical_height, scale_factor}; also accepts width_px/height_px aliases) for HiDPI layout/dispatch."
    })
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
                },
                "metrics": metrics_schema()
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
                "metrics": metrics_schema()
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

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MetricsArgs {
    logical_width: Option<f64>,
    logical_height: Option<f64>,
    scale_factor: Option<f64>,
    width_px: Option<f64>,
    height_px: Option<f64>,
}

#[derive(Serialize)]
struct NormalizedMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale_factor: Option<f64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ModifierFlagsObj {
    shift: Option<bool>,
    ctrl: Option<bool>,
    alt: Option<bool>,
    meta: Option<bool>,
    cmd: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ModifiersInput {
    Object(ModifierFlagsObj),
    List(Vec<String>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TabTargetInput {
    Index(u64),
    Label(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MenuPathInput {
    Single(String),
    Segments(Vec<String>),
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiInspectArgs {
    scope: Option<String>,
    include_bounds: Option<bool>,
    metrics: Option<MetricsArgs>,
}

fn handle_ui_inspect(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiInspectArgs = deserialize_args("ui_inspect", args)?;
    let scope = match trim_opt_non_empty(parsed.scope, "scope")? {
        Some(raw) => {
            let lower = raw.to_ascii_lowercase();
            if !VALID_INSPECT_SCOPES.contains(&lower.as_str()) {
                return Err(ToolCallError::InvalidParams(format!(
                    "unsupported ui_inspect scope '{raw}'; expected one of: all, workspace, dialog, grid, navigator, canvas, menus, toolbar, tabs"
                )));
            }
            Some(lower)
        }
        None => None,
    };
    let metrics = parsed.metrics.map(|m| normalize_metrics(m, "ui_inspect")).transpose()?;
    let mut params = Map::new();
    insert_opt_str(&mut params, "scope", scope);
    if let Some(include_bounds) = parsed.include_bounds {
        params.insert(String::from("include_bounds"), Value::Bool(include_bounds));
    }
    if let Some(m) = metrics {
        params.insert(String::from("metrics"), serde_json::to_value(m).unwrap_or(Value::Null));
    }
    dispatch_json_result(state, "ui.inspect", Value::Object(params))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiScreenshotArgs {
    target: Option<String>,
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: Option<u32>,
    path: Option<String>,
}

fn handle_ui_screenshot(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiScreenshotArgs = deserialize_args("ui_screenshot", args)?;
    let mut params = Map::new();
    if let Some(raw_target) = trim_opt_non_empty(parsed.target, "target")? {
        let target = raw_target.to_ascii_lowercase();
        if !VALID_SCREENSHOT_TARGETS.contains(&target.as_str()) {
            return Err(ToolCallError::InvalidParams(format!(
                "unsupported ui_screenshot target '{raw_target}'; expected full_window, workspace, viewport, canvas, or dialog"
            )));
        }
        params.insert(String::from("target"), Value::String(target));
    }
    for (key, value, min, max) in [("width_px", parsed.width_px, 64, 4096), ("height_px", parsed.height_px, 64, 4096), ("dpi", parsed.dpi, 72, 600)] {
        if let Some(v) = value {
            if v < min || v > max {
                return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be in [{min}, {max}]")));
            }
            params.insert(String::from(key), json!(v));
        }
    }
    if let Some(raw_path) = trim_opt_non_empty(parsed.path, "path")? {
        if state.is_gui_only() {
            return Ok(errors::result_from_payload(
                &StructuredErrorPayload::new(
                    ErrorCode::GuiOnlyViolation,
                    "direct filesystem writes via `ui_screenshot` `path` are blocked under --gui-only",
                )
                .with_remediation("Omit `path` and inspect the returned inline `image/png` content block instead."),
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

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiPointerGestureArgs {
    action: Option<String>,
    widget_id: Option<String>,
    target_id: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    to_widget_id: Option<String>,
    to_x: Option<f64>,
    to_y: Option<f64>,
    modifiers: Option<ModifiersInput>,
    delta_y: Option<f64>,
    hover_fraction_x: Option<f64>,
    hover_n: Option<f64>,
    metrics: Option<MetricsArgs>,
}

fn handle_ui_pointer_gesture(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiPointerGestureArgs = deserialize_args("ui_pointer_gesture", args)?;
    let raw_action = trim_required_non_empty(
        parsed.action.ok_or_else(|| ToolCallError::InvalidParams(String::from("missing or empty string parameter \"action\"")))?,
        "action",
    )?;
    let action = normalize_pointer_action(&raw_action)?;
    let widget_id = trim_opt_non_empty(parsed.widget_id, "widget_id")?.or(trim_opt_non_empty(parsed.target_id, "target_id")?);
    let x = check_opt_finite_f64(parsed.x, "x")?;
    let y = check_opt_finite_f64(parsed.y, "y")?;
    if x.is_some() != y.is_some() {
        return Err(ToolCallError::InvalidParams(String::from("ui_pointer_gesture requires both \"x\" and \"y\" when using coordinates")));
    }
    if widget_id.is_none() && x.is_none() {
        return Err(ToolCallError::InvalidParams(String::from("ui_pointer_gesture requires \"widget_id\" or (\"x\", \"y\") coordinates")));
    }

    let to_widget_id = trim_opt_non_empty(parsed.to_widget_id, "to_widget_id")?;
    let to_x = check_opt_finite_f64(parsed.to_x, "to_x")?;
    let to_y = check_opt_finite_f64(parsed.to_y, "to_y")?;
    if to_x.is_some() != to_y.is_some() {
        return Err(ToolCallError::InvalidParams(String::from("ui_pointer_gesture requires both \"to_x\" and \"to_y\" for drag destination")));
    }
    if action == "drag" && to_widget_id.is_none() && to_x.is_none() {
        return Err(ToolCallError::InvalidParams(String::from(
            "ui_pointer_gesture with action=\"drag\" requires \"to_widget_id\" or (\"to_x\", \"to_y\")",
        )));
    }

    let delta_y = check_opt_finite_f64(parsed.delta_y, "delta_y")?;
    let hover_fraction_x = check_opt_finite_f64(parsed.hover_fraction_x, "hover_fraction_x")?;
    let hover_n = check_opt_finite_f64(parsed.hover_n, "hover_n")?;
    let metrics = parsed.metrics.map(|m| normalize_metrics(m, "ui_pointer_gesture")).transpose()?;
    let modifiers = parsed.modifiers.map(|m| normalize_modifiers(m, "modifiers")).transpose()?;

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
        params.insert(String::from("metrics"), serde_json::to_value(m).unwrap_or(Value::Null));
    }
    if let Some(mods) = modifiers {
        params.insert(String::from("modifiers"), mods);
    }

    dispatch_json_result(state, "ui.pointer_gesture", Value::Object(params))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiTypeTextArgs {
    text: Option<String>,
    widget_id: Option<String>,
    clear_existing: Option<bool>,
    commit_enter: Option<bool>,
}

fn handle_ui_type_text(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiTypeTextArgs = deserialize_args("ui_type_text", args)?;
    let Some(text) = parsed.text else {
        return Err(ToolCallError::InvalidParams(String::from("ui_type_text requires a string parameter \"text\"")));
    };
    let mut params = Map::new();
    params.insert(String::from("text"), Value::String(text));
    insert_opt_str(&mut params, "widget_id", trim_opt_non_empty(parsed.widget_id, "widget_id")?);
    if let Some(clear_existing) = parsed.clear_existing {
        params.insert(String::from("clear_existing"), Value::Bool(clear_existing));
    }
    if let Some(commit_enter) = parsed.commit_enter {
        params.insert(String::from("commit_enter"), Value::Bool(commit_enter));
    }
    dispatch_json_result(state, "ui.type_text", Value::Object(params))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiPressKeyArgs {
    key: Option<String>,
    modifiers: Option<ModifiersInput>,
}

fn handle_ui_press_key(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiPressKeyArgs = deserialize_args("ui_press_key", args)?;
    let key = trim_required_non_empty(
        parsed.key.ok_or_else(|| ToolCallError::InvalidParams(String::from("missing or empty string parameter \"key\"")))?,
        "key",
    )?;
    let mut params = Map::new();
    params.insert(String::from("key"), Value::String(key));
    if let Some(mods) = parsed.modifiers.map(|m| normalize_modifiers(m, "modifiers")).transpose()? {
        params.insert(String::from("modifiers"), mods);
    }
    dispatch_json_result(state, "ui.press_key", Value::Object(params))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiInvokeMenuArgs {
    path: Option<MenuPathInput>,
    command_id: Option<String>,
    params: Option<Map<String, Value>>,
}

fn handle_ui_invoke_menu(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiInvokeMenuArgs = deserialize_args("ui_invoke_menu", args)?;
    let path = parsed.path.map(|p| normalize_menu_path_input(p, "path")).transpose()?;
    let command_id = trim_opt_non_empty(parsed.command_id, "command_id")?;
    if path.is_none() && command_id.is_none() {
        return Err(ToolCallError::InvalidParams(String::from("ui_invoke_menu requires \"path\" or \"command_id\"")));
    }

    let mut extra = parsed.params.map_or_else(|| json!({}), Value::Object);
    if let Err(err) = formats::sanitize_command_params(state, &mut extra) {
        return Ok(errors::result_from_engine_error(&err));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let has_workdir = state.workdir_guard().is_some();
    #[cfg(target_arch = "wasm32")]
    let has_workdir = false;
    let path_constrained = has_workdir || state.session().path_policy().is_constrained();
    if state.is_gui_only()
        && let Some(blocked) = check_gui_only_menu_restriction(command_id.as_deref(), path.as_ref(), &extra, path_constrained)
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

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiSwitchTabArgs {
    tab: Option<TabTargetInput>,
    tab_id: Option<TabTargetInput>,
    sheet_id: Option<String>,
    widget_id: Option<String>,
}

fn handle_ui_switch_tab(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiSwitchTabArgs = deserialize_args("ui_switch_tab", args)?;
    let tab =
        parsed.tab.map(|t| normalize_tab_target(t, "tab")).transpose()?.or(parsed.tab_id.map(|t| normalize_tab_target(t, "tab_id")).transpose()?);
    let sheet_id = trim_opt_non_empty(parsed.sheet_id, "sheet_id")?;
    let widget_id = trim_opt_non_empty(parsed.widget_id, "widget_id")?;
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

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiAnswerDialogArgs {
    action: Option<String>,
    fields: Option<Map<String, Value>>,
    values: Option<Map<String, Value>>,
    patches: Option<Map<String, Value>>,
    force_create_new: Option<bool>,
    choice: Option<String>,
}

fn handle_ui_answer_dialog(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiAnswerDialogArgs = deserialize_args("ui_answer_dialog", args)?;
    let action = trim_opt_non_empty(parsed.action, "action")?;
    let fields = parsed.fields.map(Value::Object);
    let values = parsed.values.map(Value::Object);
    let patches = parsed.patches.map(Value::Object);
    let choice = trim_opt_non_empty(parsed.choice, "choice")?;
    if action.is_none() && fields.is_none() && values.is_none() && patches.is_none() && choice.is_none() {
        return Err(ToolCallError::InvalidParams(String::from(
            "ui_answer_dialog requires \"action\", \"fields\", \"values\", \"patches\", or \"choice\"",
        )));
    }

    if state.is_gui_only() {
        for map_val in [&fields, &values, &patches].into_iter().flatten() {
            if has_print_submit_to_os_true(map_val) {
                return Ok(gui_only_subprocess_blocked("ui_answer_dialog.submit_to_os", "OS print spooler subprocess submission (`submit_to_os`)"));
            }
        }
    }

    let mut params = Map::new();
    insert_opt_str(&mut params, "action", action);
    insert_opt_str(&mut params, "choice", choice);
    if let Some(force) = parsed.force_create_new {
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

fn check_opt_finite_f64(value: Option<f64>, key: &str) -> Result<Option<f64>, ToolCallError> {
    let Some(num) = value else {
        return Ok(None);
    };
    if !num.is_finite() {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a finite number")));
    }
    Ok(Some(num))
}

fn check_positive_finite_f64(value: Option<f64>, key: &str, context: &str) -> Result<Option<f64>, ToolCallError> {
    let Some(num) = check_opt_finite_f64(value, key)? else {
        return Ok(None);
    };
    if num <= 0.0 {
        return Err(ToolCallError::InvalidParams(format!("{context} parameter \"{key}\" must be positive (> 0)")));
    }
    Ok(Some(num))
}

fn normalize_tab_target(input: TabTargetInput, key: &str) -> Result<Value, ToolCallError> {
    match input {
        TabTargetInput::Index(idx) => Ok(json!(idx)),
        TabTargetInput::Label(s) => Ok(Value::String(trim_required_non_empty(s, key)?)),
    }
}

fn normalize_menu_path_input(input: MenuPathInput, key: &str) -> Result<Value, ToolCallError> {
    match input {
        MenuPathInput::Single(s) => Ok(Value::String(trim_required_non_empty(s, key)?)),
        MenuPathInput::Segments(items) => {
            if items.is_empty() {
                return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" array must not be empty")));
            }
            let mut segments = Vec::with_capacity(items.len());
            for (idx, item) in items.into_iter().enumerate() {
                let trimmed = item.trim();
                if trimmed.is_empty() {
                    return Err(ToolCallError::InvalidParams(format!("{key}[{idx}] must be a non-empty string")));
                }
                segments.push(Value::String(trimmed.to_owned()));
            }
            Ok(Value::Array(segments))
        }
    }
}

fn normalize_modifiers(input: ModifiersInput, key: &str) -> Result<Value, ToolCallError> {
    let (mut shift, mut ctrl, mut alt, mut meta) = (false, false, false, false);
    match input {
        ModifiersInput::Object(obj) => {
            shift = obj.shift.unwrap_or(false);
            ctrl = obj.ctrl.unwrap_or(false);
            alt = obj.alt.unwrap_or(false);
            meta = obj.meta.unwrap_or(false) || obj.cmd.unwrap_or(false);
        }
        ModifiersInput::List(items) => {
            for (idx, flag) in items.into_iter().enumerate() {
                match flag.trim().to_ascii_lowercase().as_str() {
                    "shift" => shift = true,
                    "ctrl" | "control" => ctrl = true,
                    "alt" | "option" => alt = true,
                    "meta" | "cmd" | "command" | "super" => meta = true,
                    other => return Err(ToolCallError::InvalidParams(format!("unknown modifier flag '{other}' in {key}[{idx}]"))),
                }
            }
        }
    }
    Ok(json!({ "shift": shift, "ctrl": ctrl, "alt": alt, "meta": meta }))
}

fn resolve_metric_dimension(
    canonical: Option<f64>,
    alias: Option<f64>,
    canonical_key: &str,
    alias_key: &str,
    context: &str,
) -> Result<Option<f64>, ToolCallError> {
    let canonical = check_positive_finite_f64(canonical, canonical_key, context)?;
    let alias = check_positive_finite_f64(alias, alias_key, context)?;
    match (canonical, alias) {
        (Some(c), Some(a)) if (c - a).abs() > 1e-9 => {
            Err(ToolCallError::InvalidParams(format!("{context} cannot specify conflicting \"{canonical_key}\" ({c}) and \"{alias_key}\" ({a})")))
        }
        (Some(c), _) => Ok(Some(c)),
        (None, Some(a)) => Ok(Some(a)),
        (None, None) => Ok(None),
    }
}

fn normalize_metrics(raw: MetricsArgs, tool_name: &str) -> Result<NormalizedMetrics, ToolCallError> {
    let context = format!("{tool_name}.metrics");
    let logical_width = resolve_metric_dimension(raw.logical_width, raw.width_px, "logical_width", "width_px", &context)?;
    let logical_height = resolve_metric_dimension(raw.logical_height, raw.height_px, "logical_height", "height_px", &context)?;
    let scale_factor = check_positive_finite_f64(raw.scale_factor, "scale_factor", &context)?;
    if logical_width.is_none() && logical_height.is_none() && scale_factor.is_none() {
        return Err(ToolCallError::InvalidParams(format!(
            "{tool_name} \"metrics\" must specify at least one of logical_width, logical_height, or scale_factor"
        )));
    }
    Ok(NormalizedMetrics { logical_width, logical_height, scale_factor })
}

fn insert_opt_str(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), Value::String(v));
    }
}

pub(crate) fn reject_unknown_args(tool_name: &str, args: &Map<String, Value>, allowed: &[&str]) -> Result<(), ToolCallError> {
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

fn has_print_submit_to_os_true(val: &Value) -> bool {
    match val {
        Value::Object(map) => {
            if map.get("submit_to_os").and_then(Value::as_bool) == Some(true)
                || map.get("print.submit_to_os").and_then(Value::as_bool) == Some(true)
                || map.get("print").and_then(|v| v.get("submit_to_os")).and_then(Value::as_bool) == Some(true)
            {
                return true;
            }
            map.values().any(has_print_submit_to_os_true)
        }
        Value::Array(arr) => arr.iter().any(has_print_submit_to_os_true),
        _ => false,
    }
}

fn canonical_menu_command_from_path(path: Option<&Value>) -> Option<&'static str> {
    const TOKEN_TO_CMD: [(&str, &str); 21] = [
        ("fileopen", "project.open"),
        ("open", "project.open"),
        ("fileopenproject", "project.open"),
        ("fileopenunpackeddir", "file.open_unpacked_dir"),
        ("filenewfromtemplate", "project.instantiateTemplate"),
        ("newfromtemplate", "project.instantiateTemplate"),
        ("fileimportexternalarchive", "table.importPrism"),
        ("importexternalarchive", "table.importPrism"),
        ("fileimportxmlproject", "table.importPzfx"),
        ("importxmlproject", "table.importPzfx"),
        ("fileimportcsvortsv", "table.importCsv"),
        ("importcsvortsv", "table.importCsv"),
        ("fileimporttidylongtable", "table.importTidyLong"),
        ("importtidylongtable", "table.importTidyLong"),
        ("editpastelink", "table.pasteLink"),
        ("pastelink", "table.pasteLink"),
        ("analyzerecalculateall", "project.recalculate"),
        ("recalculateall", "project.recalculate"),
        ("recalculate", "project.recalculate"),
        ("filesave", "project.save"),
        ("filesaveas", "project.saveAs"),
    ];
    let norm = normalized_menu_path(path)?;
    TOKEN_TO_CMD.iter().find_map(|(token, cmd)| (norm == *token).then_some(*cmd))
}

fn normalized_menu_path(path: Option<&Value>) -> Option<String> {
    let joined = match path? {
        Value::String(s) => s.to_owned(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("/"),
        _ => return None,
    };
    Some(normalize_menu_token(&joined))
}

fn gui_only_subprocess_blocked(target: &str, helper_desc: &str) -> ToolCallResult {
    let payload = StructuredErrorPayload::new(ErrorCode::GuiOnlyViolation, format!("{helper_desc} is forbidden under --gui-only policy"))
        .with_remediation("Inspect content inside the window via `ui_inspect` or `ui_screenshot` without invoking host OS helpers.")
        .with_details(json!({ "target": target, "policy": "gui_only" }));
    errors::result_from_payload(&payload)
}

fn check_gui_only_menu_restriction(command_id: Option<&str>, path: Option<&Value>, extra: &Value, path_constrained: bool) -> Option<ToolCallResult> {
    let is_copy_methods = command_id == Some("ui.result.copyMethodsReport")
        || menu_path_matches_tokens(path, &["analyzecopymethodsreport", "analyzecopymethodsstatisticalreporting", "copymethodsreport"]);
    if is_copy_methods {
        return Some(gui_only_subprocess_blocked("ui.result.copyMethodsReport", "OS clipboard helper `ui.result.copyMethodsReport`"));
    }

    if command_id == Some("file.print") {
        return Some(gui_only_subprocess_blocked("file.print", "OS print helper `file.print`"));
    }

    if let Some(cid) = command_id
        && matches!(cid, "table.setCell" | "ui.event.dispatch" | "ui.shortcut.set" | "ui.shortcut.reset" | "ui.brush.point" | "app.quit")
    {
        return Some(gui_only_subprocess_blocked(cid, &format!("menu command `{cid}`")));
    }
    if menu_path_matches_tokens(path, &["graphbrushpoint", "brushpoint"]) {
        return Some(gui_only_subprocess_blocked("ui.brush.point", "menu path `Brush Point`"));
    }

    let is_print_open = command_id == Some("ui.print.open") || menu_path_matches_tokens(path, &["fileprint", "print"]);
    if is_print_open && has_print_submit_to_os_true(extra) {
        return Some(gui_only_subprocess_blocked("ui.print.open", "OS print spooler subprocess submission (`submit_to_os`)"));
    }

    if command_id == Some("ui.screenshot") && extra.get("path").is_some_and(|v| !v.is_null()) {
        return Some(gui_only_subprocess_blocked("ui.screenshot.path", "ui.screenshot disk `path` output via ui_invoke_menu"));
    }

    if path_constrained {
        return None;
    }

    let path_cmd = canonical_menu_command_from_path(path);
    for candidate in [command_id, path_cmd] {
        if let Err(err) = curvebook_engine::enforce_unconstrained_gui_only_menu_params(candidate, extra) {
            return Some(errors::result_from_engine_error(&err));
        }
    }
    None
}

fn insert_opt_f64(map: &mut Map<String, Value>, key: &str, value: Option<f64>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), json!(v));
    }
}
