# Model Context Protocol Server (`lumencraft-mcp`)

`lumencraft-cli mcp` runs a stdio [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server (`crates/mcp`) over an in-memory headless `lumencraft-engine` session by default, or attached over loopback TCP (`127.0.0.1:<port>`) to a running `lumencraft --control <port>` instance via `lumencraft-cli mcp --control-port <port>` ([`control-protocol.md`](control-protocol.md)). By default `lumencraft --control <port>` runs headless; pass the opt-in `--window` flag (`lumencraft --control <port> --window`) to attach a visible desktop window sharing the same live `UiApp` / `Session` ([`R7.3`](ROADMAP.md#r73-mcp-workflow-parity-proof--shared-live-window)).

> Related specifications: [`commands.md`](commands.md), [`gui-workflows.md`](gui-workflows.md), [`control-protocol.md`](control-protocol.md), [`scripting-and-automation.md`](scripting-and-automation.md) · Beginner quick start: [`README.md`](../README.md#model-context-protocol-mcp) · Live test & command counts: [`STATUS.md`](STATUS.md)

## Protocol & Wire Framing

- **Transport (`crates/mcp/src/protocol.rs`):** JSON-RPC 2.0 over `stdin` / `stdout`. Default headless `lumencraft-cli mcp` opens no network sockets; loopback TCP (`127.0.0.1`) is used only when `--control-port` or `LUMENCRAFT_CONTROL_PORT` is supplied.
- **Framing:** Auto-detects and mirrors both standard `Content-Length: N\r\n\r\n` framing and newline-delimited JSON-RPC (`<json>\n`), enforcing `MAX_REQUEST_BYTES = 16 MiB` (`MAX_DRAIN_BYTES = 64 MiB`), `MAX_HEADER_LINE_BYTES = 8 KiB`, `MAX_HEADER_COUNT = 32`, and `MAX_BATCH_LEN = 256`.
- **Handshake & Capabilities:** `initialize` negotiates `protocolVersion` `"2024-11-05"` (and accepts `"2025-03-26"` when requested), identifies `serverInfo.name = "lumencraft"`, advertises `capabilities.tools.listChanged: true`, `resources` (`subscribe: false`, `listChanged: false`), and `prompts` (`listChanged: false`), and emits `SERVER_INSTRUCTIONS` (or `GUI_ONLY_SERVER_INSTRUCTIONS` under `--gui-only`). Supports `notifications/initialized`, `notifications/cancelled`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, and `prompts/get`.

## Modes & Reach

MCP is currently the most complete way to drive LumenCraft end to end; the rendered desktop and WebAssembly GUI chrome is pre-alpha and not yet human-operable ([`R10`](ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion)).

| Mode | Reach |
|---|---|
| `lumencraft-cli mcp` (default, headless) | Every registered engine command in `cmd::registry()` through `command_run`, `command_batch` (with `atomic: true` snapshot rollback), and per-command toolset tools, plus `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `sheet://` / `result://` resources, and guided prompts. UI-only `ui.*` commands report `"available": false`, `"requires_live_attach": true` in `command_list` and return `ErrorCode::Conflict` if called; the `8` `gui` tools also return `Conflict` unless `--control-port` or `--gui-only` is active. |
| `lumencraft-cli mcp --control-port <port>` | Same toolset surface forwarded over `127.0.0.1:<port>` to a running `lumencraft --control <port>` instance (headless by default, or `lumencraft --control <port> --window` for a shared visible desktop window), enabling the synced `ui.*` UI commands and the `8` `gui` tools against the live `UiApp` session. |
| `lumencraft-cli mcp --gui-only` (or `--control-port <port> --gui-only`) | Exposes strictly the `8` `gui` tools, driving `UiApp` through the semantic agent harness verified across `WF-01`–`WF-50` over stdio JSON-RPC (`R7.3`). |

## Status & Current Limitations

- **Verified stdio parity (`R7.3`):** `WF-01`–`WF-50` (including standalone *P*-value stack adjustment `stats.pvalueStack` and Compact Letter Display `graph.setCompactLetterDisplay`) are verified end to end over spawned `lumencraft-cli mcp --gui-only` stdio JSON-RPC subprocesses, alongside headless `command_run` / `command_batch` / `open_file` / `save_file` / `export` / `render_preview`, `WorkdirGuard` path containment, and opt-in `lumencraft --control <port> --window` shared desktop sessions.
- **Human-operable GUI (`R10`):** Even with `lumencraft --control <port> --window`, the rendered `eframe`/`egui` desktop window draws a non-interactive RGBA chrome below the native menu bar (`0/50` human-operable workflows until [`R10`](ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion) lands). The `8` `gui` MCP tools drive the live `UiApp` state machine through the semantic harness (`crates/ui-egui/src/{inspect.rs, gui_actions.rs}`).
- **Headless UI commands:** In default headless `lumencraft-cli mcp`, desktop `ui.*` commands report `"available": false` and require `--control-port <port>` attached to a running `lumencraft --control <port>` process.
- **Live-attach `command_batch` rollback:** In headless mode, `command_batch` with `"atomic": true` restores an exact session snapshot on failure; in `--control-port` live-attach mode, rollback replays `edit.undo` for completed mutating steps (`ping` and read-only `ui.*` commands excluded), so remote rollback follows the desktop session's undo stack.
- **Deferred QoL items:** `QOL-10`, `QOL-11`, `QOL-13`, and `QOL-15` remain deferred without scheduled milestones ([`qol-beyond-parity.md`](qol-beyond-parity.md)).

## CLI Flags & Advanced Configuration

```text
lumencraft-cli mcp [--control-port <1..=65535>] [--gui-only] [--workdir <dir>] [--control-token <token>]
```

- Always use the **absolute path** to `lumencraft-cli` in GUI MCP client configs so the host application does not depend on shell `PATH` or working-directory lookup.
- `--workdir <dir>` canonicalizes an existing accessible directory via `WorkdirGuard` (`crates/platform/src/path_guard.rs`) and confines `open_file`, `save_file`, `export`, non-`--gui-only` `ui_screenshot` `path`, `ui_invoke_menu` `params`, recursive `PATH_PARAM_KEYS` inside `command_run` / `command_batch` (leaving inline `csv` text untouched), and stored `ExternalDelimitedFile` link refreshes during `project.recalculate`. Escapes (`..`, symlinks pointing outside, or external absolute paths) fail with `ErrorCode::PathEscape` (`"path_escape"`, `category: "policy"`). When `--gui-only --workdir <dir>` is passed, `InProcessUiGuiHost` also applies `ExternalPathPolicy::workdir` to the in-process `Session`.

### Example Configurations

Headless with strict `--workdir` confinement (recommended starting configuration):

```json
{
  "mcpServers": {
    "lumencraft": {
      "command": "/absolute/path/to/lumencraft-cli",
      "args": ["mcp", "--workdir", "/path/to/workspace"]
    }
  }
}
```

Live-attached to a shared visible desktop window (`lumencraft --control 9123 --window`):

```json
{
  "mcpServers": {
    "lumencraft": {
      "command": "/absolute/path/to/lumencraft-cli",
      "args": ["mcp", "--control-port", "9123", "--workdir", "/path/to/workspace"]
    }
  }
}
```

Strict `--gui-only` agent-harness mode (in-process `UiApp`):

```json
{
  "mcpServers": {
    "lumencraft-gui": {
      "command": "/absolute/path/to/lumencraft-cli",
      "args": ["mcp", "--gui-only", "--workdir", "/path/to/workspace"]
    }
  }
}
```

## Shipped Surface (`R7.1`, `R0.13`, `R7.3`)

Every handler executes inside `run_guarded` with structured error payloads (`code`, `category`, `message`, `retryable`, `remediation`, `details`), `Effects`-derived MCP annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`), and optional `WorkdirGuard` path containment:

| Surface | Purpose |
|---|---|
| `doc_inspect` / `open_file` / `save_file` | Inspect project sheets (`Data`, `Info`, `Analyses`, `Graphs`, `Layouts`) or live `--control` snapshots (including per-sheet previews when `sheet_id` or `include_cells` is given), open `.lumencraft` / `.lumencraft.d` / `.lctemplate` / `.prism` / `.pzfx` / `.csv` / `.tsv`, and atomically save `.lumencraft`, `.lumencraft.d`, or `.lctemplate` projects — forwarding to the attached session when `--control-port` is configured. |
| `command_list` / `command_run` / `command_batch` | Discover and execute typed engine commands ([`commands.md`](commands.md)) plus the synced UI commands (`crates/mcp/src/ui_catalog.rs`), up to `1024` steps per `command_batch` with optional `atomic: true` rollback. `command_list` reports `"live_attached"`, `"mode"` (`"live_attach"` vs `"headless"`), and per-command `"available"` (`ui.*` commands report `"available": false`, `"requires_live_attach": true` when headless). |
| `list_toolsets` / `enable_toolset` | Progressively enable or disable per-command MCP tools grouped by `CommandGroup` (`project`, `edit`, `sheet`, `table`, `info`, `analysis`, `graph`, `layout`, `export`, or `"all"`), plus the `"gui"` toolset (`8` `GUI Automation` tools), emitting `notifications/tools/list_changed` whenever the active set changes. |
| `render_preview` / `export` | Compile and rasterize any `GraphSheet` or `LayoutSheet` to inline PNG preview bytes, or export `SVG` / `%PDF-1.7` / `PNG` / `TIFF` (`none` / `deflate` / `lzw`) / `CSV` / `JSON` inline or atomically to disk. For CSV exports on a `ResultSheet`, supplying `tab_id` defaults `tidy` to `false` so the requested tab exports directly, while explicit `tidy: true` + `tab_id` fails with `InvalidParams`. |
| MCP Resources & Prompts | Read `sheet://<id>` and `result://<id>` resources (from the attached desktop session when `--control-port` is set, or the local session in headless mode) and guided workflow prompts (`dose_response_curve_fit`, `choose_two_group_test`, `publication_figure_checklist`). |

### Live-Attach & Shared-Window Mode (`--control-port` / `--window`)

- Pass `--control-port <port>` (or set `LUMENCRAFT_CONTROL_PORT=<port>`, overridden by `--control-port`) to attach MCP to a running `lumencraft --control <port>` instance over loopback `127.0.0.1:<port>`.
- **Headless vs. shared visible window (`--window`):**
  - `lumencraft --control <port>` (default, `--window` omitted) runs headless for scripts and CI automation.
  - `lumencraft --control <port> --window` (also compatible with `LUMENCRAFT_CONTROL_PORT=<port>`) opens a visible `eframe`/`egui` desktop window sharing the same live `UiApp` / `Session` and crash-recovery WAL journal between interactive window input and remote MCP/control requests (`127.0.0.1:<port>` loopback-only via `validate_loopback_addr`; cannot be combined with `--headless-once`).
  - Add `--gui-only` (`lumencraft --control <port> --window --gui-only`) to enforce `Caller::Control` `--gui-only` restrictions on the control socket while keeping the shared window live.
- **Authentication:** Uses `--control-token <token>`, `LUMENCRAFT_CONTROL_TOKEN`, or the per-UID `control-<port>.token` file located under `$LUMENCRAFT_CONFIG_DIR`, platform config directories (`$HOME/Library/Application Support/LumenCraft` on macOS, `$XDG_CONFIG_HOME/lumencraft` / `$HOME/.config/lumencraft` on Linux/BSD, `%APPDATA%\LumenCraft` on Windows), or (when `HOME` is unset on Unix) a validated `0700` non-symlink `$XDG_RUNTIME_DIR/lumencraft` or `$TMPDIR/lumencraft-uid-<uid>` directory.
- **Routing:** When attached, `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `command_list`, `command_run`, `command_batch`, `resources/list`, and `resources/read` operate against the attached `lumencraft --control` session rather than a disconnected local `Session`, while local `--workdir` containment continues to be enforced on file paths.

## `gui` Toolset, `GuiSessionHost` & `--gui-only` Mode (`R8.6`–`R8.8`, `R7.3`)

Verified in `crates/mcp/src/{gui_tools.rs, tools.rs, protocol.rs}` and `apps/lumencraft-cli/src/main.rs` (see [`gui-workflows.md`](gui-workflows.md)):

### 1. Hosts & CLI Modes

- **`GuiSessionHost` (`crates/mcp/src/tools.rs`):** Trait (`call_gui`, `set_gui_only`, `is_gui_only`) implemented outside `crates/mcp` so MCP can drive a live `UiApp` without depending on `lumencraft-ui-egui`. `McpServerState` routes all `8` `gui` tools either to an attached `GuiSessionHost` (`with_gui_host` / `set_gui_host`) or over loopback TCP via `--control-port <port>` (`with_control_port` / `with_control_port_and_gui_only`, `run_stdio_with_control_port` / `run_stdio_with_control_port_gui_only`). Calling a `gui` tool with neither returns `ErrorCode::Conflict`.
- **`InProcessUiGuiHost` (`apps/lumencraft-cli/src/main.rs`):** Wraps an in-process `UiApp` for `lumencraft-cli mcp --gui-only` (propagating `ExternalPathPolicy::workdir` when `--workdir` is set), normalizing `"workspace"`/`"viewport"` screenshot targets to `"full_window"`, `"wheel"` gestures to `"scroll"`, and array menu `path`s into `"/"`-joined strings before delegating to `lumencraft_ui_egui::dispatch_gui_action`.
- **Two `--gui-only` launch modes:**
  - `lumencraft-cli mcp --gui-only` — in-process `UiApp` via `InProcessUiGuiHost` with `GuiOnlyPolicy::Strict`.
  - `lumencraft-cli mcp --control-port <N> --gui-only` — attaches over loopback `127.0.0.1:<N>` to a running `lumencraft --control <N> --gui-only` instance (or `lumencraft --control <N> --window --gui-only` for a visible shared desktop window).

### 2. The 8-Tool `gui` Surface (`GUI_TOOL_NAMES`)

In normal mode, `enable_toolset` with `{ "group": "gui" }` (or attaching a `GuiSessionHost`) appends the `8` `gui` tools alongside the core tools above (and any enabled `CommandGroup` tools). Under `--gui-only`, `list_tools_for_state` shrinks `tools/list` strictly to these `8` tools:

| MCP Tool | Key Inputs | Key Annotations / Behavior |
|---|---|---|
| `ui_inspect` | `scope` (`"all"`, `"workspace"`, `"dialog"`, `"grid"`, `"navigator"`, `"canvas"`, `"menus"`, `"toolbar"`, `"tabs"`), `include_bounds`, `sheet_id` | `readOnlyHint: true`, `idempotentHint: true`; returns `UiTreeSnapshot` text + structured JSON. |
| `ui_screenshot` | `target` (`"full_window"`, `"workspace"`, `"viewport"`, `"canvas"`, `"dialog"`, default `"workspace"`), `width_px` (`64..=4096`), `height_px` (`64..=4096`), `dpi` (`72..=600`), optional `path` | `readOnlyHint: true`, `idempotentHint: true`; returns inline `image/png` block + metadata (`structuredContent` omits base64 duplication). Supplying `path` is rejected with `gui_only_violation` when `--gui-only` is active, and checked via `WorkdirGuard` otherwise. |
| `ui_pointer_gesture` | `action` (`"click"`, `"double_click"`, `"right_click"`, `"drag"`, `"hover"`, `"wheel"`), `widget_id`/`target_id`, `x`/`y`, `to_widget_id`/`to_x`/`to_y`, `modifiers`, `delta_y`, `hover_fraction_x`, `hover_n`, `metrics` | `readOnlyHint: true`, `idempotentHint: false` (`Effects::CHANGES_UI_STATE`). |
| `ui_type_text` | `text`, `widget_id`, `clear_existing`, `commit_enter` | `readOnlyHint: false` (`MUTATES_DOCUMENT \| CHANGES_UI_STATE`); supports multi-cell TSV. |
| `ui_press_key` | `key`, `modifiers` | `Effects::CHANGES_UI_STATE`; dispatches keystrokes/shortcuts (`Enter`, `Escape`, `Tab`, `Cmd/Ctrl+T`, etc.). |
| `ui_invoke_menu` | `path` (string or string array), `command_id`, optional `params` (`WorkdirGuard`-sanitized) | `Effects::CHANGES_UI_STATE`; fails with `Conflict` when item is disabled. |
| `ui_switch_tab` | `tab`/`tab_id` (string or non-negative integer index), `sheet_id`, `widget_id` | `Effects::CHANGES_UI_STATE \| IDEMPOTENT`; honors modal tab graying predicates; `sheet_id` alone selects a sheet. |
| `ui_answer_dialog` | `action` (`ok`, `cancel`, `back`, `next`, `create`, `apply`, `preview`), `fields`/`values`/`patches`, `dialog_id`, `force_create_new`, `choice` | `readOnlyHint: false` (`MUTATES_DOCUMENT \| CHANGES_UI_STATE`). |

### 3. `--gui-only` Protocol Behavior

- **`initialize`:** Advertises `GUI_ONLY_SERVER_INSTRUCTIONS` instead of `SERVER_INSTRUCTIONS`.
- **Blocked tools:** Calling any non-`gui` tool (`command_run`, `command_batch`, `command_list`, `doc_inspect`, `open_file`, `save_file`, `export`, `render_preview`, `list_toolsets`, `enable_toolset`, dynamic command tools, or unknown tools) returns `isError: true` with `structuredContent.code == "gui_only_violation"` (`ErrorCode::GuiOnlyViolation`, `category: "policy"`). Calling `ui_screenshot` with `path` also returns `isError: true` with `gui_only_violation`.
- **Resources:** `resources/list` returns `{"resources": []}`, and `resources/read` returns JSON-RPC error `-32600` (`ERR_INVALID_REQUEST`) with `data.code == "gui_only_violation"`.
- **Contrast with raw `--control --gui-only` TCP:** On the direct loopback TCP socket (`lumencraft --control <port> --gui-only`, [`control-protocol.md`](control-protocol.md)), read-only inspection (`ping`, `doc.inspect`, `ui.state` / `state.snapshot`, `command.list`, `ui.inspect`, `ui.canvas.preflight`) remains allowed while mutating/L5 calls and disk-writing paths (`ui.screenshot` / `render.pdf` with `path`) are rejected with `gui_only_violation`.

## UI Command Sync

Desktop UI commands declared in `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`; live count in [`STATUS.md`](STATUS.md)) are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (enforced in CI with `cargo xtask ui-commands --check`) so `lumencraft-mcp` never depends directly on `lumencraft-ui-egui`.
