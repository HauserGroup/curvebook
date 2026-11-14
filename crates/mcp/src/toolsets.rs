//! Progressive MCP toolset discovery (`list_toolsets`, `enable_toolset`), `Effects`-derived tool
//! annotations, and deterministic `[a-z0-9_]` per-command tool mapping over `lumencraft_engine::cmd`.

use std::collections::BTreeSet;

use lumencraft_engine::cmd::{CommandGroup, CommandSpec, Effects, registry};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::tools::{McpServerState, ToolCallError, ToolCallResult, ToolDefinition};

/// MCP tool behavioral annotations derived directly from engine [`Effects`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAnnotations {
    #[serde(rename = "readOnlyHint")]
    pub read_only_hint: bool,
    #[serde(rename = "destructiveHint")]
    pub destructive_hint: bool,
    #[serde(rename = "idempotentHint")]
    pub idempotent_hint: bool,
    #[serde(rename = "openWorldHint")]
    pub open_world_hint: bool,
}

impl ToolAnnotations {
    /// Derive canonical MCP tool annotations from engine [`Effects`] flags.
    #[must_use]
    pub const fn from_effects(effects: Effects) -> Self {
        Self {
            read_only_hint: effects.read_only_hint(),
            destructive_hint: effects.destructive_hint(),
            idempotent_hint: effects.contains(Effects::IDEMPOTENT),
            open_world_hint: effects.intersects(Effects::READS_FILESYSTEM.with(Effects::WRITES_FILESYSTEM).with(Effects::LAUNCHES_PROCESS)),
        }
    }
}

/// Normalize a dotted/camelCase engine command ID (e.g. `sheet.createData`, `table.setXCells`)
/// deterministically into a collision-free MCP tool identifier matching `[a-z0-9_]+`.
#[must_use]
pub fn command_id_to_mcp_tool_name(command_id: &str) -> String {
    let chars: Vec<char> = command_id.chars().collect();
    let mut out = String::with_capacity(command_id.len() + 8);
    let mut prev_kind: Option<CharKind> = None;

    for (idx, &ch) in chars.iter().enumerate() {
        if matches!(ch, '.' | '-' | '_') || !ch.is_ascii_alphanumeric() {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            prev_kind = Some(CharKind::Separator);
            continue;
        }

        if ch.is_ascii_uppercase() {
            let next_is_lower = chars.get(idx + 1).is_some_and(char::is_ascii_lowercase);
            let needs_sep = !out.is_empty()
                && !out.ends_with('_')
                && (matches!(prev_kind, Some(CharKind::LowerOrDigit)) || (matches!(prev_kind, Some(CharKind::Upper)) && next_is_lower));
            if needs_sep {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_kind = Some(CharKind::Upper);
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_kind = Some(CharKind::LowerOrDigit);
        }
    }

    while out.ends_with('_') {
        out.pop();
    }
    out
}

#[derive(Copy, Clone)]
enum CharKind {
    Separator,
    LowerOrDigit,
    Upper,
}

/// Parse a [`CommandGroup`] name case-insensitively (`project`, `edit`, `sheet`, etc.).
pub fn parse_command_group(raw: &str) -> Result<CommandGroup, ToolCallError> {
    let needle = raw.trim();
    CommandGroup::all()
        .iter()
        .copied()
        .find(|group| group.as_str().eq_ignore_ascii_case(needle))
        .ok_or_else(|| ToolCallError::InvalidParams(format!("unknown command group toolset '{raw}'")))
}

/// Convert a static [`CommandSpec`] into a full per-command MCP [`ToolDefinition`].
#[must_use]
pub fn command_spec_to_tool_definition(spec: &'static CommandSpec) -> ToolDefinition {
    let description = if spec.summary.trim().is_empty() {
        format!("[{}] {}", spec.id, spec.label)
    } else {
        format!("[{}] {} — {}", spec.id, spec.label, spec.summary)
    };
    ToolDefinition {
        name: command_id_to_mcp_tool_name(spec.id),
        description,
        input_schema: (spec.input_schema)(),
        annotations: ToolAnnotations::from_effects(spec.effects),
    }
}

/// Return all per-command [`ToolDefinition`] entries belonging to `group`.
#[must_use]
pub fn list_tools_for_group(group: CommandGroup) -> Vec<ToolDefinition> {
    registry().specs_by_group(group).into_iter().map(command_spec_to_tool_definition).collect()
}

/// Look up an engine [`CommandSpec`] by its normalized `[a-z0-9_]` MCP tool name.
#[must_use]
pub fn find_command_spec_by_tool_name(tool_name: &str) -> Option<&'static CommandSpec> {
    registry().specs().find(|spec| command_id_to_mcp_tool_name(spec.id) == tool_name)
}

/// JSON Schema array of valid [`CommandGroup`] identifiers for tool definitions.
#[must_use]
pub fn group_enum_json() -> Value {
    let values: Vec<Value> = CommandGroup::all().iter().map(|g| Value::String(g.as_str().to_owned())).collect();
    Value::Array(values)
}

/// Build the structured response for the `list_toolsets` MCP tool.
pub fn handle_list_toolsets(state: &McpServerState) -> Result<ToolCallResult, ToolCallError> {
    let reg = registry();
    let enabled = state.enabled_toolsets();
    let mut toolsets = Vec::with_capacity(CommandGroup::all().len());

    for &group in CommandGroup::all() {
        let specs = reg.specs_by_group(group);
        let tools: Vec<Value> = specs
            .iter()
            .map(|spec| {
                json!({
                    "name": command_id_to_mcp_tool_name(spec.id),
                    "command_id": spec.id,
                    "label": spec.label,
                    "summary": spec.summary,
                    "annotations": ToolAnnotations::from_effects(spec.effects),
                })
            })
            .collect();
        toolsets.push(json!({
            "group": group.as_str(),
            "title": group.title(),
            "enabled": enabled.contains(&group),
            "command_count": tools.len(),
            "tools": tools,
        }));
    }

    let enabled_groups: Vec<&'static str> = enabled.iter().copied().map(CommandGroup::as_str).collect();
    ToolCallResult::ok_json(&json!({
        "toolsets": toolsets,
        "enabled_groups": enabled_groups,
        "total_engine_commands": reg.len(),
    }))
}

/// Handle the `enable_toolset` MCP tool, emitting `notifications/tools/list_changed` when enabled
/// groups change.
pub fn handle_enable_toolset(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let enable_flag = crate::tools::optional_bool(args, "enabled")?.unwrap_or(true);
    let requested = collect_requested_groups(args)?;

    let mut any_changed = false;
    let mut target_names = Vec::with_capacity(requested.len());
    for group in requested {
        if state.set_toolset_enabled(group, enable_flag) {
            any_changed = true;
        }
        target_names.push(group.as_str());
    }

    if any_changed {
        state.queue_tools_list_changed();
    }

    let enabled_groups: Vec<&'static str> = state.enabled_toolsets().iter().copied().map(CommandGroup::as_str).collect();
    ToolCallResult::ok_json(&json!({
        "changed": any_changed,
        "enabled": enable_flag,
        "requested_groups": target_names,
        "enabled_groups": enabled_groups,
    }))
}

fn collect_requested_groups(args: &Map<String, Value>) -> Result<Vec<CommandGroup>, ToolCallError> {
    let mut selected = BTreeSet::new();

    if let Some(single) = crate::tools::optional_non_empty_str(args, "group")? {
        if single.eq_ignore_ascii_case("all") {
            for &group in CommandGroup::all() {
                selected.insert(group);
            }
        } else {
            selected.insert(parse_command_group(&single)?);
        }
    }

    if let Some(raw_arr) = args.get("groups")
        && !raw_arr.is_null()
    {
        let arr = raw_arr.as_array().ok_or_else(|| ToolCallError::InvalidParams(String::from("parameter \"groups\" must be an array")))?;
        for (idx, item) in arr.iter().enumerate() {
            let s = item.as_str().ok_or_else(|| ToolCallError::InvalidParams(format!("groups[{idx}] must be a string")))?;
            if s.trim().eq_ignore_ascii_case("all") {
                for &group in CommandGroup::all() {
                    selected.insert(group);
                }
            } else {
                selected.insert(parse_command_group(s)?);
            }
        }
    }

    if selected.is_empty() {
        return Err(ToolCallError::InvalidParams(String::from("enable_toolset requires \"group\" or non-empty \"groups\"")));
    }

    Ok(selected.into_iter().collect())
}
