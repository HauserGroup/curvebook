//! Human-equivalent `gui` MCP tool definitions and validated dispatch (`R8.6`).
//!
//! Exposes the 7 semantic GUI tools (`ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`,
//! `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`)
//! across both in-process [`GuiSessionHost`](crate::tools::GuiSessionHost) bindings and
//! loopback `--control` desktop windows without reversing the `L6` layer boundary.

use curvebook_doc::ErrorCode;
use curvebook_engine::cmd::Effects;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::errors::{self, StructuredErrorPayload};
use crate::formats;
use crate::tools::{
    ContentBlock, McpServerState, ToolCallError, ToolCallResult, ToolDefinition, deserialize_args, dispatch_gui_backend, trim_opt_non_empty,
    trim_required_non_empty,
};
use crate::toolsets::ToolAnnotations;

/// Ordered list of the 7 `gui` MCP tool names exposed when the `gui` toolset is enabled.
pub const GUI_TOOL_NAMES: [&str; 7] =
    ["ui_inspect", "ui_screenshot", "ui_pointer_gesture", "ui_type_text", "ui_press_key", "ui_invoke_menu", "ui_switch_tab"];

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

/// Return `true` when `name` identifies one of the 7 `gui` MCP tools.
#[must_use]
pub fn is_gui_tool_name(name: &str) -> bool {
    matches!(name, "ui_inspect" | "ui_screenshot" | "ui_pointer_gesture" | "ui_type_text" | "ui_press_key" | "ui_invoke_menu" | "ui_switch_tab")
}

/// Return all 7 `gui` MCP [`ToolDefinition`] entries with schemas and `Effects`-aligned annotations.
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
    ]
    .into_iter()
    .map(|mut def| {
        def.input_schema = crate::toolsets::compact_json_schema(def.input_schema);
        def
    })
    .collect()
}

fn strict_schema<T: JsonSchema>() -> Value {
    let mut root = schemars::generate::SchemaSettings::draft2020_12().into_generator().into_root_schema_for::<T>().to_value();
    let defs = root
        .as_object_mut()
        .and_then(|obj| {
            obj.remove("$schema");
            obj.remove("title");
            obj.remove("$defs")
        })
        .and_then(|v| match v {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    normalize_gui_schema(&mut root, &defs, 0);
    root
}

fn normalize_gui_schema(schema: &mut Value, defs: &Map<String, Value>, depth: usize) {
    if depth >= 32 {
        return;
    }
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    if let Some(def_name) = obj.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")).map(str::to_owned)
        && let Some(mut resolved) = defs.get(&def_name).cloned()
    {
        normalize_gui_schema(&mut resolved, defs, depth + 1);
        obj.remove("$ref");
        if let Value::Object(resolved_map) = resolved {
            for (k, v) in resolved_map {
                obj.entry(k).or_insert(v);
            }
        }
    }

    if let Some(Value::Array(types)) = obj.get_mut("type") {
        types.retain(|t| t.as_str() != Some("null"));
        if types.len() == 1
            && let Some(single) = types.pop()
        {
            obj.insert(String::from("type"), single);
        }
    }

    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(items)) = obj.get_mut(key) {
            for item in items.iter_mut() {
                normalize_gui_schema(item, defs, depth + 1);
            }
        }
    }

    if let Some(Value::Array(branches)) = obj.get_mut("anyOf") {
        branches.retain(|b| !matches!(b.as_object(), Some(m) if m.len() == 1 && m.get("type").and_then(Value::as_str) == Some("null")));
        if branches.len() == 1
            && let Some(Value::Object(only)) = branches.pop()
        {
            obj.remove("anyOf");
            for (k, v) in only {
                obj.entry(k).or_insert(v);
            }
        }
    }

    if let Some(Value::Object(props)) = obj.get_mut("properties") {
        for prop in props.values_mut() {
            normalize_gui_schema(prop, defs, depth + 1);
        }
        if obj.get("type").and_then(Value::as_str) == Some("object") && !obj.contains_key("additionalProperties") {
            obj.insert(String::from("additionalProperties"), Value::Bool(false));
        }
    }

    for key in ["items", "additionalProperties"] {
        if let Some(child) = obj.get_mut(key) {
            normalize_gui_schema(child, defs, depth + 1);
        }
    }
    if let Some(Value::Array(prefix)) = obj.get_mut("prefixItems") {
        for item in prefix.iter_mut() {
            normalize_gui_schema(item, defs, depth + 1);
        }
    }
}

fn string_enum_schema(variants: &[&str]) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": variants,
    })
}

fn inspect_scope_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    string_enum_schema(&VALID_INSPECT_SCOPES)
}

fn screenshot_target_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    string_enum_schema(&VALID_SCREENSHOT_TARGETS)
}

fn pointer_action_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    string_enum_schema(&["click", "double_click", "right_click", "drag", "hover", "wheel"])
}

fn def_ui_inspect() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_inspect"),
        description: String::from(
            "Inspect the visible CurveBook GUI tree (shell, navigator, menus/toolbars, active modal/wizard with enabled/disabled_reason predicates, visible grid cells, and canvas hit targets).",
        ),
        input_schema: strict_schema::<UiInspectArgs>(),
        annotations: ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT)),
    }
}

fn def_ui_screenshot() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_screenshot"),
        description: String::from(
            "Capture a deterministic PNG screenshot of the live CurveBook UI, returning inline PNG image content alongside pixel/DPI metadata (or atomically writing/overwriting an optional host output path).",
        ),
        input_schema: strict_schema::<UiScreenshotArgs>(),
        annotations: ToolAnnotations::from_effects(Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT)),
    }
}

fn def_ui_pointer_gesture() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_pointer_gesture"),
        description: String::from("Dispatch a visible mouse/trackpad gesture against a widget_id or logical (x, y) viewport coordinates."),
        input_schema: strict_schema::<UiPointerGestureArgs>(),
        annotations: ToolAnnotations::from_effects(GUI_DISPATCH_WORST_CASE_EFFECTS),
    }
}

fn def_ui_type_text() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_type_text"),
        description: String::from(
            "Type or paste UTF-8 text (including multi-cell TSV blocks with tabs/newlines) into the focused cell/field or specified widget_id.",
        ),
        input_schema: strict_schema::<UiTypeTextArgs>(),
        annotations: ToolAnnotations::from_effects(GUI_TYPE_TEXT_WORST_CASE_EFFECTS),
    }
}

fn def_ui_press_key() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_press_key"),
        description: String::from("Press a keyboard key or shortcut (e.g. Enter, Escape, Tab, Delete, Backspace, Cmd/Ctrl+T, Cmd/Ctrl+E)."),
        input_schema: strict_schema::<UiPressKeyArgs>(),
        annotations: ToolAnnotations::from_effects(GUI_DISPATCH_WORST_CASE_EFFECTS),
    }
}

fn def_ui_invoke_menu() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_invoke_menu"),
        description: String::from("Invoke a visible menu bar, toolbar, or context-menu item by path or command_id (fails if disabled)."),
        input_schema: strict_schema::<UiInvokeMenuArgs>(),
        annotations: ToolAnnotations::from_effects(GUI_DISPATCH_WORST_CASE_EFFECTS),
    }
}

fn def_ui_switch_tab() -> ToolDefinition {
    ToolDefinition {
        name: String::from("ui_switch_tab"),
        description: String::from("Switch the active sheet, modal/wizard stage, or ResultSheet sub-tab (fails with disabled_reason if grayed out)."),
        input_schema: strict_schema::<UiSwitchTabArgs>(),
        annotations: ToolAnnotations::from_effects(Effects::MUTATES_DOCUMENT.with(Effects::CHANGES_UI_STATE).with(Effects::IDEMPOTENT)),
    }
}

/// Validate parameters and dispatch one of the 7 `gui` MCP tools against `state`.
pub(crate) fn dispatch_gui_tool(state: &mut McpServerState, name: &str, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    match name {
        "ui_inspect" => handle_ui_inspect(state, args),
        "ui_screenshot" => handle_ui_screenshot(state, args),
        "ui_pointer_gesture" => handle_ui_pointer_gesture(state, args),
        "ui_type_text" => handle_ui_type_text(state, args),
        "ui_press_key" => handle_ui_press_key(state, args),
        "ui_invoke_menu" => handle_ui_invoke_menu(state, args),
        "ui_switch_tab" => handle_ui_switch_tab(state, args),
        _ => Err(ToolCallError::UnknownTool(name.to_owned())),
    }
}

fn dispatch_json_result(state: &mut McpServerState, method: &str, params: Value) -> Result<ToolCallResult, ToolCallError> {
    if state.current_cancel_token().load(std::sync::atomic::Ordering::Acquire) {
        return Ok(errors::result_from_code(ErrorCode::Cancelled, format!("{method} cancelled before execution")));
    }
    match dispatch_gui_backend(state, method, params) {
        Ok(val) => ToolCallResult::ok_json(&val),
        Err(payload) => Ok(errors::result_from_payload(&payload)),
    }
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Optional ViewportMetrics ({logical_width, logical_height, scale_factor}; also accepts width_px/height_px aliases) for HiDPI layout/dispatch."
)]
struct MetricsArgs {
    /// Logical viewport width in points (160..=16384).
    #[schemars(extend("exclusiveMinimum" = 0.0))]
    logical_width: Option<f64>,
    /// Logical viewport height in points (120..=16384).
    #[schemars(extend("exclusiveMinimum" = 0.0))]
    logical_height: Option<f64>,
    /// HiDPI display scale factor (0.5..=8.0, default 1.0).
    #[schemars(extend("exclusiveMinimum" = 0.0))]
    scale_factor: Option<f64>,
    /// Alias for logical_width.
    #[schemars(extend("exclusiveMinimum" = 0.0))]
    width_px: Option<f64>,
    /// Alias for logical_height.
    #[schemars(extend("exclusiveMinimum" = 0.0))]
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ModifierFlagsObj {
    shift: Option<bool>,
    ctrl: Option<bool>,
    alt: Option<bool>,
    meta: Option<bool>,
    cmd: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(description = "Optional modifier keys ({shift, ctrl, alt, meta, cmd} object or string list).")]
enum ModifiersInput {
    Object(ModifierFlagsObj),
    List(Vec<String>),
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum TabTargetInput {
    Index(u64),
    Label(String),
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum MenuPathInput {
    Single(String),
    Segments(Vec<String>),
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiInspectArgs {
    /// Optional subtree scope filter (defaults to 'all').
    #[schemars(schema_with = "inspect_scope_schema")]
    scope: Option<String>,
    /// Include widget/hit-target bounding boxes in the snapshot.
    include_bounds: Option<bool>,
    /// Optional ViewportMetrics ({logical_width, logical_height, scale_factor}; also accepts width_px/height_px aliases) for HiDPI layout/dispatch.
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiScreenshotArgs {
    /// Optional visual surface target (defaults to 'workspace').
    #[schemars(schema_with = "screenshot_target_schema")]
    target: Option<String>,
    /// Image width in pixels (64..=4096).
    #[schemars(range(min = 64, max = 4096))]
    width_px: Option<u32>,
    /// Image height in pixels (64..=4096).
    #[schemars(range(min = 64, max = 4096))]
    height_px: Option<u32>,
    /// Optional render DPI (72..=600).
    #[schemars(range(min = 72, max = 600))]
    dpi: Option<u32>,
    /// Optional host output path (atomically overwrites existing file).
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
        let resolved = match formats::resolve_mcp_path(state, &raw_path) {
            Ok(p) => p,
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
        params.insert(String::from("path"), Value::String(resolved.to_string_lossy().into_owned()));
    }

    if state.current_cancel_token().load(std::sync::atomic::Ordering::Acquire) {
        return Ok(errors::result_from_code(ErrorCode::Cancelled, "ui.screenshot cancelled before execution"));
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

    let mut metadata = obj.clone();
    metadata.remove("data_base64");
    metadata.remove("png_base64");
    metadata.remove("data");
    metadata.entry(String::from("format")).or_insert_with(|| Value::String(String::from("png")));

    let meta_val = Value::Object(metadata);
    let meta_text = serde_json::to_string_pretty(&meta_val).unwrap_or_default();

    if let Some(data) = b64_opt {
        return Ok(ToolCallResult {
            content: vec![ContentBlock::Image { data, mime_type: String::from("image/png") }, ContentBlock::Text { text: meta_text }],
            is_error: false,
            structured_content: Some(meta_val),
        });
    }

    // When the live backend wrote the PNG directly to `path` and omitted inline base64 to respect
    // the 16 MiB control-frame budget (`AUD-13-12`), return metadata cleanly without failing.
    if obj.get("path").and_then(Value::as_str).is_some_and(|p| !p.trim().is_empty()) {
        return Ok(ToolCallResult { content: vec![ContentBlock::Text { text: meta_text }], is_error: false, structured_content: Some(meta_val) });
    }

    Ok(errors::result_from_payload(
        &StructuredErrorPayload::new(ErrorCode::InternalError, "ui.screenshot response did not include base64 PNG data (`data_base64`)")
            .with_details(raw),
    ))
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiPointerGestureArgs {
    /// Pointer gesture kind.
    #[schemars(required, schema_with = "pointer_action_schema")]
    action: Option<String>,
    /// Target widget or canvas hit-target ID from ui_inspect.
    widget_id: Option<String>,
    /// Alias for widget_id.
    target_id: Option<String>,
    /// Logical X coordinate when not addressing by widget_id.
    x: Option<f64>,
    /// Logical Y coordinate when not addressing by widget_id.
    y: Option<f64>,
    /// Destination widget/hit-target ID for 'drag' gestures.
    to_widget_id: Option<String>,
    /// Destination logical X coordinate for 'drag' gestures.
    to_x: Option<f64>,
    /// Destination logical Y coordinate for 'drag' gestures.
    to_y: Option<f64>,
    /// Optional modifier keys ({shift, ctrl, alt, meta, cmd} object or string list).
    modifiers: Option<ModifiersInput>,
    /// Vertical scroll delta for 'wheel' gestures.
    delta_y: Option<f64>,
    /// Optional fractional X coordinate [0.0, 1.0] for power curve hover.
    hover_fraction_x: Option<f64>,
    /// Optional sample-size N coordinate for power curve hover.
    hover_n: Option<f64>,
    /// Optional ViewportMetrics ({logical_width, logical_height, scale_factor}; also accepts width_px/height_px aliases) for HiDPI layout/dispatch.
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiTypeTextArgs {
    /// Exact UTF-8 text or TSV payload to enter (whitespace preserved).
    #[schemars(required)]
    text: Option<String>,
    /// Optional widget ID to focus before entering text.
    widget_id: Option<String>,
    /// Clear existing cell or field content before typing.
    clear_existing: Option<bool>,
    /// Commit the inline edit or field with Enter after typing.
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiPressKeyArgs {
    /// Key identifier such as 'Enter', 'Escape', 'Tab', 'Delete', 'ArrowDown', 't', or 'e'.
    #[schemars(required)]
    key: Option<String>,
    /// Optional modifier keys ({shift, ctrl, alt, meta, cmd} object or string list).
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiInvokeMenuArgs {
    /// Visible menu path (e.g. 'Analyze > Column Analyses > One-way ANOVA') or array of segments.
    path: Option<MenuPathInput>,
    /// Command identifier associated with the menu/toolbar item.
    command_id: Option<String>,
    /// Optional parameter payload for the invoked command or dialog initializer.
    params: Option<Map<String, Value>>,
}

fn handle_ui_invoke_menu(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: UiInvokeMenuArgs = deserialize_args("ui_invoke_menu", args)?;
    let path = parsed.path.map(|p| normalize_menu_path_input(p, "path")).transpose()?;
    let command_id = trim_opt_non_empty(parsed.command_id, "command_id")?;
    if path.is_none() && command_id.is_none() {
        return Err(ToolCallError::InvalidParams(String::from("ui_invoke_menu requires \"path\" or \"command_id\"")));
    }

    let mut params = Map::new();
    if let Some(p) = path {
        params.insert(String::from("path"), p);
    }
    insert_opt_str(&mut params, "command_id", command_id);
    // Preserve omitted/null `params` (do NOT synthesize `{}`) so `dispatch_gui_invoke_menu` runs
    // contextual parameter synthesis and allows parameterless menu clicks in `--human-path-only`
    // mode (`AUD-13-10`). Explicit caller-supplied `params` (including `{}`) are sanitized and forwarded.
    if let Some(map) = parsed.params {
        let mut extra = Value::Object(map);
        if let Err(err) = formats::sanitize_command_params(state, &mut extra) {
            return Ok(errors::result_from_engine_error(&err));
        }
        params.insert(String::from("params"), extra);
    }
    dispatch_json_result(state, "ui.invoke_menu", Value::Object(params))
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct UiSwitchTabArgs {
    /// Tab identifier, visible tab label, or zero-based tab index.
    tab: Option<TabTargetInput>,
    /// Alias for tab.
    tab_id: Option<TabTargetInput>,
    /// Optional sheet identifier to activate before/when selecting a sub-tab.
    sheet_id: Option<String>,
    /// Optional tab strip or widget identifier.
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
        // Normalize zero-based integer tab indices to canonical decimal strings so every GUI
        // backend (`inspect_bridge` and `gui_window`) parses them uniformly (`AUD-13-11`).
        TabTargetInput::Index(idx) => Ok(Value::String(idx.to_string())),
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

fn insert_opt_f64(map: &mut Map<String, Value>, key: &str, value: Option<f64>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), json!(v));
    }
}
