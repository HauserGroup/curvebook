# Model Context Protocol Server (`lumencraft-mcp`)

`lumencraft-cli mcp` runs a stdio [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server (`crates/mcp`) over an in-memory `lumencraft-engine` session, or attached to a live desktop window via `lumencraft-cli mcp --control-port <port>` ([`control-protocol.md`](control-protocol.md)).

> Related specifications: [`commands.md`](commands.md), [`gui-workflows.md`](gui-workflows.md), [`control-protocol.md`](control-protocol.md) · Live test & command counts: [`STATUS.md`](STATUS.md)

## Client Configuration

```json
{
  "mcpServers": {
    "lumencraft": {
      "command": "lumencraft-cli",
      "args": ["mcp", "--workdir", "/path/to/workspace"]
    }
  }
}
```

Pass `--workdir <dir>` to confine `open_file`, `save_file`, `export`, engine command path parameters, and stored `ExternalDelimitedFile` link refreshes strictly inside a canonicalized workspace root (propagated to `McpServerState` and the underlying `engine::Session::path_policy()`).

## Shipped Surface (`R7.1`, `R0.13`)

Every handler executes inside `run_guarded` with structured error payloads (`ErrorCode`, `category`, `retryable`, `remediation`, `details`), `Effects`-derived MCP annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`), and optional `WorkdirGuard` path containment:

| Surface | Purpose |
|---|---|
| `doc_inspect` / `open_file` / `save_file` | Inspect project sheets (`Data`, `Info`, `Results`, `Graphs`, `Layouts`) or live `--control` state snapshots, open `.lumencraft` / `.lumencraft.d` / `.lctemplate` / `.prism` / `.pzfx` / `.csv` / `.tsv`, and atomically save `.lumencraft`, `.lumencraft.d`, or `.lctemplate` projects. |
| `command_list` / `command_run` / `command_batch` | Discover and execute typed engine commands ([`commands.md`](commands.md)) plus the `32` synced UI commands (`crates/mcp/src/ui_catalog.rs`), optionally with `atomic: true` batch rollback. |
| `list_toolsets` / `enable_toolset` | Progressively enable or disable per-command MCP tools grouped by `CommandGroup` — plus the `"gui"` toolset (`gui_toolset` in `list_toolsets`: 8 `GUI Automation` tools) — emitting `notifications/tools/list_changed`. |
| `render_preview` / `export` | Compile and rasterize any `GraphSheet` or `LayoutSheet` to PNG preview bytes, or export `SVG` / `%PDF-1.7` / `PNG` / `TIFF` / `CSV` / `JSON`. |
| MCP Resources & Prompts | Read `sheet://<id>` and `result://<id>` resources and guided statistical/curve-fitting workflow prompts. |

## `gui` Toolset, `GuiSessionHost` & `--gui-only` Mode (`R8.6`–`R8.8`)

Verified in `crates/mcp/src/{gui_tools.rs, tools.rs, protocol.rs}` and `apps/lumencraft-cli/src/main.rs` (see [`gui-workflows.md`](gui-workflows.md)):

### 1. Hosts & CLI Modes
- **`GuiSessionHost` (`crates/mcp/src/tools.rs`):** Trait (`call_gui`, `set_gui_only`, `is_gui_only`) implemented outside `crates/mcp` so MCP can drive a live `UiApp` without depending on `lumencraft-ui-egui`. `McpServerState` routes all 8 `gui` tools either to an attached `GuiSessionHost` (`with_gui_host` / `set_gui_host`) or over loopback TCP via `--control-port <port>` (`with_control_port` / `with_control_port_and_gui_only`, `run_stdio_with_control_port` / `run_stdio_with_control_port_gui_only`). Calling a `gui` tool with neither returns `ErrorCode::Conflict`.
- **`InProcessUiGuiHost` (`apps/lumencraft-cli/src/main.rs`):** Wraps an in-process `UiApp` for `lumencraft-cli mcp --gui-only`, normalizing `"workspace"`/`"viewport"` screenshot targets to `"full_window"`, `"wheel"` gestures to `"scroll"`, and array menu `path`s into `"/"`-joined strings before delegating to `lumencraft_ui_egui::dispatch_gui_action`.
- **Two `--gui-only` launch modes:**
  - `lumencraft-cli mcp --gui-only` — in-process `UiApp` via `InProcessUiGuiHost` with `GuiOnlyPolicy::Strict`.
  - `lumencraft-cli mcp --control-port <N> --gui-only` — attaches over loopback `127.0.0.1:<N>` to a running desktop window (typically launched with `lumencraft --control <N> --gui-only`).

### 2. The 8-Tool `gui` Surface (`GUI_TOOL_NAMES`)
In normal mode, `enable_toolset` with `{ "group": "gui" }` (or attaching a `GuiSessionHost`) appends the 8 `gui` tools alongside the 10 core tools (and any enabled `CommandGroup` tools). Under `--gui-only`, `list_tools_for_state` shrinks `tools/list` strictly to these 8 tools:

| MCP Tool | Key Inputs | Key Annotations / Return |
|---|---|---|
| `ui_inspect` | `scope` (`"all"`, `"dialog"`, `"grid"`, `"navigator"`, `"canvas"`, `"menus"`), `include_bounds`, `sheet_id` | `readOnlyHint: true`, `idempotentHint: true`; returns `UiTreeSnapshot` text + structured JSON. |
| `ui_screenshot` | `target` (`"workspace"`, `"viewport"`, `"canvas"`, `"dialog"`), `width_px` (`64..=4096`), `height_px` (`64..=4096`), `dpi` (`72..=600`) | `readOnlyHint: true`, `idempotentHint: true`; no `path` param; returns inline `image/png` block + metadata (`structured_content` omits base64 duplication). |
| `ui_pointer_gesture` | `action` (`"click"`, `"double_click"`, `"right_click"`, `"drag"`, `"hover"`, `"wheel"`), `widget_id`/`target_id`, `x`/`y`, `to_widget_id`/`to_x`/`to_y`, `modifiers`, `delta_y` | `readOnlyHint: true`, `idempotentHint: false` (`Effects::CHANGES_UI_STATE`). |
| `ui_type_text` | `text`, `widget_id`, `clear_existing`, `commit_enter` | `readOnlyHint: false` (`MUTATES_DOCUMENT \| CHANGES_UI_STATE`); supports multi-cell TSV. |
| `ui_press_key` | `key`, `modifiers` | `Effects::CHANGES_UI_STATE`; dispatches keystrokes/shortcuts (`Enter`, `Escape`, `Cmd/Ctrl+T`, etc.). |
| `ui_invoke_menu` | `path` (string or string array), `command_id` | `Effects::CHANGES_UI_STATE`; fails with `Conflict` when item is disabled. |
| `ui_switch_tab` | `tab`/`tab_id`, `sheet_id`, `widget_id` | `Effects::CHANGES_UI_STATE \| IDEMPOTENT`; honors modal tab graying predicates. |
| `ui_answer_dialog` | `action` (`ok`, `cancel`, `back`, `next`, `create`, `apply`), `fields`/`values`, `dialog_id` | `readOnlyHint: false` (`MUTATES_DOCUMENT \| CHANGES_UI_STATE`). |

### 3. `--gui-only` Protocol Behavior
- **`initialize`:** Advertises `GUI_ONLY_SERVER_INSTRUCTIONS` instead of `SERVER_INSTRUCTIONS`.
- **Blocked tools:** Calling any non-`gui` tool (`command_run`, `command_batch`, `doc_inspect`, `open_file`, `save_file`, `export`, `render_preview`, `list_toolsets`, `enable_toolset`, dynamic command tools, or unknown tools) returns `isError: true` with `structuredContent.code == "gui_only_violation"` (`ErrorCode::GuiOnlyViolation`, `category: "policy"`).
- **Resources:** `resources/list` returns `{"resources": []}`, and `resources/read` returns JSON-RPC error `-32600` (`ERR_INVALID_REQUEST`) with `data.code == "gui_only_violation"`.

## UI Command Sync

Desktop UI commands declared in `crates/ui-egui/src/menus/ui_commands.rs` (`32` entries in `UI_COMMANDS`, including `ui.powerWizard.open` and `ui.formatPoints.open`) are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (enforced in CI with `cargo xtask ui-commands --check`) so `lumencraft-mcp` never depends directly on `lumencraft-ui-egui`.
