# Model Context Protocol Server (`lumencraft-mcp`)

`lumencraft-cli mcp` runs a stdio [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server (`crates/mcp`) over an in-memory `lumencraft-engine` session, or attached to a running `lumencraft --control <port>` instance via `lumencraft-cli mcp --control-port <port>` ([`control-protocol.md`](control-protocol.md)). By default `lumencraft --control <port>` runs headless; pass the opt-in `--window` flag (`lumencraft --control <port> --window`) to attach a visible desktop window sharing the same live `UiApp` session ([`R7.3`](ROADMAP.md#r73-mcp-workflow-parity-proof--shared-live-window)).

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

## What MCP Can and Cannot Do Today

MCP is currently the most complete way to drive LumenCraft; the rendered GUI is not yet operable by a person ([`R10`](ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion)).

| Mode | Reach |
|---|---|
| `lumencraft-cli mcp` (in-process, headless) | Every engine command in `cmd::registry()` through `command_run`, `command_batch`, and per-command toolset tools, plus open/save/export and previews. All document work is reachable. `ui.*` UI commands report `"available": false`, `"requires_live_attach": true` in `command_list` and are not executable in headless mode. |
| `--control-port <port>` | Same, forwarded to a running `lumencraft --control <port>` (headless by default, or add `--window` for a shared visible desktop window); adds the UI-only `ui.*` commands. |
| `--gui-only` | Only the 8 `gui` tools, driving `UiApp` through the semantic agent harness used by the `WF-01`–`WF-49` tests. |

Limits: "Analyze a stack of *P* values" ([`R10.10`](ROADMAP.md#r1010-standalone-analyze-a-stack-of-p-values-analysis)) and Compact Letter Display ([`R10.11`](ROADMAP.md#r1011-compact-letter-display-cld)) are not implemented by any route; `WF-01`–`WF-49` are verified in-process, not yet over MCP stdio (tracked in [`R7.3`](ROADMAP.md#r73-mcp-workflow-parity-proof--shared-live-window); shared visible desktop window is opt-in via `lumencraft --control <port> --window`).

**Roadmap priority:** [`R7.3`](ROADMAP.md#r73-mcp-workflow-parity-proof--shared-live-window) is the next feature priority after the Rust 1.99.1 upgrade (`R0.18`) and before new human-operable GUI work (`R10`). Completion requires real MCP transport coverage of existing engine capabilities and an opt-in shared visible session. Tests for later engine features are added when those features land and do not postpone this gate. This is a sequencing decision, not a claim that the remaining MCP gaps are fixed. After implementation is verified, [`R7.4`](ROADMAP.md#r74-accessible-mcp-guide-and-minimal-readme) adds a short beginner guide to a simplified README; this document remains the detailed reference.

Pass `--workdir <dir>` to confine `open_file`, `save_file`, `export`, engine command path parameters, and stored `ExternalDelimitedFile` link refreshes strictly inside a canonicalized workspace root (propagated to `McpServerState` and the underlying `engine::Session::path_policy()`).

## Shipped Surface (`R7.1`, `R0.13`)

Every handler executes inside `run_guarded` with structured error payloads (`ErrorCode`, `category`, `retryable`, `remediation`, `details`), `Effects`-derived MCP annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`), and optional `WorkdirGuard` path containment:

| Surface | Purpose |
|---|---|
| `doc_inspect` / `open_file` / `save_file` | Inspect project sheets (`Data`, `Info`, `Results`, `Graphs`, `Layouts`) or live `--control` state snapshots (including per-sheet cell previews when `sheet_id` or `include_cells` is given), open `.lumencraft` / `.lumencraft.d` / `.lctemplate` / `.prism` / `.pzfx` / `.csv` / `.tsv`, and atomically save `.lumencraft`, `.lumencraft.d`, or `.lctemplate` projects — forwarding directly to the attached desktop session when `--control-port` is configured. |
| `command_list` / `command_run` / `command_batch` | Discover and execute typed engine commands ([`commands.md`](commands.md)) plus the `45` synced UI commands (generated `crates/mcp/src/ui_catalog.rs`), optionally with `atomic: true` batch rollback. `command_list` reports `"live_attached"`, `"mode"` (`"live_attach"` vs `"headless"`), and per-command `"available"` (`ui.*` commands report `"available": false`, `"requires_live_attach": true` when headless). |
| `list_toolsets` / `enable_toolset` | Progressively enable or disable per-command MCP tools grouped by `CommandGroup` — plus the `"gui"` toolset (`gui_toolset` in `list_toolsets`: 8 `GUI Automation` tools) — emitting `notifications/tools/list_changed`. |
| `render_preview` / `export` | Compile and rasterize any `GraphSheet` or `LayoutSheet` to PNG preview bytes, or export `SVG` / `%PDF-1.7` / `PNG` / `TIFF` / `CSV` / `JSON` (forwarded to the live window when attached; for CSV, specifying `tab_id` defaults `tidy` to `false` so the requested sheet is exported, while explicit `tidy: true` + `tab_id` fails with `InvalidParams`). |
| MCP Resources & Prompts | Read `sheet://<id>` and `result://<id>` resources (from the live desktop session when `--control-port` is attached, or the local session in headless mode) and guided statistical/curve-fitting workflow prompts. |

### Live-Attach Mode (`--control-port` / `LUMENCRAFT_CONTROL_PORT`)
- Pass `--control-port <port>` (or set `LUMENCRAFT_CONTROL_PORT=<port>`, overridden by `--control-port`) to attach MCP to a running `lumencraft --control <port>` instance over loopback `127.0.0.1:<port>`.
- **Headless vs. shared visible window (`--window`):**
  - `lumencraft --control <port>` (default, `--window` omitted) runs headless for scripts and CI automation.
  - `lumencraft --control <port> --window` (also compatible with `LUMENCRAFT_CONTROL_PORT=<port>`) opens a visible `eframe`/`egui` desktop window sharing the same live `UiApp` / `Session` and crash-recovery WAL journal between interactive window input and remote MCP/control requests (`127.0.0.1:<port>` loopback-only via `validate_loopback_addr`; cannot be combined with `--headless-once`).
  - Add `--gui-only` (`lumencraft --control <port> --window --gui-only`) to enforce `Caller::Control` `--gui-only` restrictions on the control socket while keeping the shared window live.
- Authentication uses `--control-token <token>`, `LUMENCRAFT_CONTROL_TOKEN`, or the per-UID `control-<port>.token` file located under `$LUMENCRAFT_CONFIG_DIR`, `$XDG_CONFIG_HOME/lumencraft` / `$HOME` config directories, or (when `HOME` is unset on Unix) a validated `0700` non-symlink `$XDG_RUNTIME_DIR/lumencraft` or `$TMPDIR/lumencraft-uid-<uid>` directory.
- When attached, `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `command_list`, `command_run`, `command_batch`, `resources/list`, and `resources/read` operate against the live desktop window session rather than a disconnected local `Session`, while local `--workdir` containment is still enforced for file paths.

## `gui` Toolset, `GuiSessionHost` & `--gui-only` Mode (`R8.6`–`R8.8`)

Verified in `crates/mcp/src/{gui_tools.rs, tools.rs, protocol.rs}` and `apps/lumencraft-cli/src/main.rs` (see [`gui-workflows.md`](gui-workflows.md)):

### 1. Hosts & CLI Modes
- **`GuiSessionHost` (`crates/mcp/src/tools.rs`):** Trait (`call_gui`, `set_gui_only`, `is_gui_only`) implemented outside `crates/mcp` so MCP can drive a live `UiApp` without depending on `lumencraft-ui-egui`. `McpServerState` routes all 8 `gui` tools either to an attached `GuiSessionHost` (`with_gui_host` / `set_gui_host`) or over loopback TCP via `--control-port <port>` (`with_control_port` / `with_control_port_and_gui_only`, `run_stdio_with_control_port` / `run_stdio_with_control_port_gui_only`). Calling a `gui` tool with neither returns `ErrorCode::Conflict`.
- **`InProcessUiGuiHost` (`apps/lumencraft-cli/src/main.rs`):** Wraps an in-process `UiApp` for `lumencraft-cli mcp --gui-only`, normalizing `"workspace"`/`"viewport"` screenshot targets to `"full_window"`, `"wheel"` gestures to `"scroll"`, and array menu `path`s into `"/"`-joined strings before delegating to `lumencraft_ui_egui::dispatch_gui_action`.
- **Two `--gui-only` launch modes:**
  - `lumencraft-cli mcp --gui-only` — in-process `UiApp` via `InProcessUiGuiHost` with `GuiOnlyPolicy::Strict`.
  - `lumencraft-cli mcp --control-port <N> --gui-only` — attaches over loopback `127.0.0.1:<N>` to a running `lumencraft --control <N> --gui-only` instance (or `lumencraft --control <N> --window --gui-only` for a visible shared desktop window).

### 2. The 8-Tool `gui` Surface (`GUI_TOOL_NAMES`)
In normal mode, `enable_toolset` with `{ "group": "gui" }` (or attaching a `GuiSessionHost`) appends the 8 `gui` tools alongside the core tools in the table above (and any enabled `CommandGroup` tools). Under `--gui-only`, `list_tools_for_state` shrinks `tools/list` strictly to these 8 tools:

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

Desktop UI commands declared in `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`, including `ui.powerWizard.open` and `ui.formatPoints.open`; live count in [`STATUS.md`](STATUS.md)) are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (enforced in CI with `cargo xtask ui-commands --check`) so `lumencraft-mcp` never depends directly on `lumencraft-ui-egui`.
