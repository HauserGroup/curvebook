//! Progressive MCP toolset discovery (`list_toolsets`, `enable_toolset`), `Effects`-derived tool
//! annotations, and deterministic `[a-z0-9_]` per-command tool mapping over `curvebook_engine::cmd`.

use std::collections::BTreeSet;

use curvebook_engine::cmd::{CommandGroup, CommandSpec, Effects, registry};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::tools::{McpServerState, ToolCallError, ToolCallResult, ToolDefinition};

/// MCP tool behavioral annotations derived directly from engine [`Effects`].
///
/// Matches engine [`Effects`] semantics cleanly:
/// - `readOnlyHint`: `false` whenever the tool can mutate the document, alter UI/toolset state
///   (`CHANGES_UI_STATE`), discard data, write the filesystem, or launch processes.
/// - `destructiveHint`: `true` whenever `MAY_DISCARD_DATA` or `WRITES_FILESYSTEM` is set, because
///   file-writing tools can overwrite existing targets on disk.
/// - `idempotentHint`: mirrors `Effects::IDEMPOTENT`.
/// - `openWorldHint`: `true` when touching external filesystem paths or launching subprocesses.
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
    ///
    /// Both `CHANGES_UI_STATE` (`readOnlyHint: false`) and `WRITES_FILESYSTEM`
    /// (`destructiveHint: true` for potential overwrites) are propagated automatically via
    /// [`Effects::read_only_hint`] and [`Effects::destructive_hint`].
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
pub fn handle_list_toolsets(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    crate::gui_tools::reject_unknown_args("list_toolsets", args, &[])?;
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

    let gui_list = crate::gui_tools::list_gui_tools();
    let gui_summaries: Vec<Value> = gui_list
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "annotations": tool.annotations,
            })
        })
        .collect();

    let mut enabled_groups: Vec<&'static str> = enabled.iter().copied().map(CommandGroup::as_str).collect();
    if state.is_gui_toolset_enabled() {
        enabled_groups.push("gui");
    }
    ToolCallResult::ok_json(&json!({
        "toolsets": toolsets,
        "gui_toolset": {
            "group": "gui",
            "title": "GUI Automation",
            "enabled": state.is_gui_toolset_enabled(),
            "tool_count": gui_summaries.len(),
            "tools": gui_summaries,
        },
        "enabled_groups": enabled_groups,
        "total_engine_commands": reg.len(),
    }))
}

/// Handle the `enable_toolset` MCP tool, emitting `notifications/tools/list_changed` when enabled
/// groups change.
pub fn handle_enable_toolset(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    crate::gui_tools::reject_unknown_args("enable_toolset", args, &["group", "groups", "enabled"])?;
    let enable_flag = crate::tools::optional_bool(args, "enabled")?.unwrap_or(true);
    let requested = collect_requested_groups(args)?;

    let mut any_changed = false;
    let mut target_names = Vec::with_capacity(requested.groups.len() + usize::from(requested.include_gui));
    for group in requested.groups {
        if state.set_toolset_enabled(group, enable_flag) {
            any_changed = true;
        }
        target_names.push(group.as_str());
    }
    if requested.include_gui {
        if state.set_gui_toolset_enabled(enable_flag) {
            any_changed = true;
        }
        target_names.push("gui");
    }

    if any_changed {
        state.queue_tools_list_changed();
    }

    let mut enabled_groups: Vec<&'static str> = state.enabled_toolsets().iter().copied().map(CommandGroup::as_str).collect();
    if state.is_gui_toolset_enabled() {
        enabled_groups.push("gui");
    }
    ToolCallResult::ok_json(&json!({
        "changed": any_changed,
        "enabled": enable_flag,
        "requested_groups": target_names,
        "enabled_groups": enabled_groups,
    }))
}

struct RequestedToolsets {
    groups: Vec<CommandGroup>,
    include_gui: bool,
}

fn push_group_token(raw: &str, selected: &mut BTreeSet<CommandGroup>, include_gui: &mut bool) -> Result<(), ToolCallError> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        for &group in CommandGroup::all() {
            selected.insert(group);
        }
    } else if trimmed.eq_ignore_ascii_case("gui") {
        *include_gui = true;
    } else {
        selected.insert(parse_command_group(trimmed)?);
    }
    Ok(())
}

fn collect_requested_groups(args: &Map<String, Value>) -> Result<RequestedToolsets, ToolCallError> {
    let mut selected = BTreeSet::new();
    let mut include_gui = false;

    if let Some(single) = crate::tools::optional_non_empty_str(args, "group")? {
        push_group_token(&single, &mut selected, &mut include_gui)?;
    }

    if let Some(raw_arr) = args.get("groups")
        && !raw_arr.is_null()
    {
        let arr = raw_arr.as_array().ok_or_else(|| ToolCallError::InvalidParams(String::from("parameter \"groups\" must be an array")))?;
        for (idx, item) in arr.iter().enumerate() {
            let s = item.as_str().ok_or_else(|| ToolCallError::InvalidParams(format!("groups[{idx}] must be a string")))?;
            push_group_token(s, &mut selected, &mut include_gui)?;
        }
    }

    if selected.is_empty() && !include_gui {
        return Err(ToolCallError::InvalidParams(String::from("enable_toolset requires \"group\" or non-empty \"groups\"")));
    }

    Ok(RequestedToolsets { groups: selected.into_iter().collect(), include_gui })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_effects_maps_ui_state_and_filesystem_writes_to_worst_case_annotations() {
        let read_only = ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT));
        assert!(read_only.read_only_hint);
        assert!(!read_only.destructive_hint);
        assert!(read_only.idempotent_hint);
        assert!(!read_only.open_world_hint);

        let ui_only = ToolAnnotations::from_effects(Effects::CHANGES_UI_STATE.with(Effects::IDEMPOTENT));
        assert!(!ui_only.read_only_hint);
        assert!(!ui_only.destructive_hint);
        assert!(ui_only.idempotent_hint);
        assert!(!ui_only.open_world_hint);

        let fs_write = ToolAnnotations::from_effects(Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT));
        assert!(!fs_write.read_only_hint);
        assert!(fs_write.destructive_hint);
        assert!(fs_write.idempotent_hint);
        assert!(fs_write.open_world_hint);
    }

    #[test]
    fn list_toolsets_exposes_accurate_worst_case_gui_tool_annotations() {
        let state = McpServerState::new();
        let res = handle_list_toolsets(&state, &Map::new()).expect("list_toolsets");
        let text = match res.content.first() {
            Some(crate::tools::ContentBlock::Text { text }) => text,
            other => panic!("unexpected list_toolsets content: {other:?}"),
        };
        let parsed: Value = serde_json::from_str(text).expect("list_toolsets JSON");
        let gui_tools = parsed["gui_toolset"]["tools"].as_array().expect("gui tools array");
        assert_eq!(gui_tools.len(), 8);

        let by_name: std::collections::BTreeMap<&str, &Value> =
            gui_tools.iter().map(|entry| (entry["name"].as_str().expect("gui tool name"), &entry["annotations"])).collect();

        assert_eq!(by_name["ui_inspect"], &json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }));
        assert_eq!(
            by_name["ui_screenshot"],
            &json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": true })
        );
        assert_eq!(
            by_name["ui_pointer_gesture"],
            &json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
        );
        assert_eq!(
            by_name["ui_type_text"],
            &json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
        );
        assert_eq!(
            by_name["ui_press_key"],
            &json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
        );
        assert_eq!(
            by_name["ui_invoke_menu"],
            &json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
        );
        assert_eq!(
            by_name["ui_switch_tab"],
            &json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
        );
        assert_eq!(
            by_name["ui_answer_dialog"],
            &json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
        );
    }
}
