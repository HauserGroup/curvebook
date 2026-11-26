# MCP Server: Source Builds & Internals (`curvebook-mcp`)

> **Who this page is for.** Contributors and people running CurveBook from a source checkout. To connect an AI assistant to an installed release, follow the [beginner's guide](mcp-for-beginners.md). The published [MCP server reference](reference/automation/mcp.md) is the canonical description of transports, attachment modes, tools, resources, and error payloads, and the [troubleshooting page](reference/troubleshooting.md#mcp-connection) covers connection problems. This page adds source builds, client configuration presets, and implementation detail.

`curvebook-cli mcp` runs a stdio [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server (`crates/mcp`) over an in-memory headless `curvebook-engine` session by default, or attached over loopback TCP (`127.0.0.1:<port>`) to a running `curvebook --control <port>` instance via `curvebook-cli mcp --control-port <port>` ([`control-protocol.md`](control-protocol.md)). By default `curvebook --control <port>` runs headless; pass the opt-in `--window` flag (`curvebook --control <port> --window`) to attach a visible desktop window sharing the same live `UiApp` / `Session` ([`R7.3`](history/roadmap/r7.md#r73-mcp-workflow-parity-proof--shared-live-window)).

> Related specifications: [`commands.md`](commands.md), [`gui-workflows.md`](gui-workflows.md), [`control-protocol.md`](control-protocol.md), [`scripting-and-automation.md`](implementation/scripting-and-automation.md) · Beginner guide: [`mcp-for-beginners.md`](mcp-for-beginners.md) · Project overview: [`README.md`](../README.md) · Live test & command counts: [`STATUS.md`](STATUS.md)

## Building & Running from Source

To build the standalone release binary from a checkout:

```sh
cargo build --release -p curvebook-cli
```

This creates `./target/release/curvebook-cli` (`.\target\release\curvebook-cli.exe` on Windows). You can also install onto `PATH` via `cargo install --path apps/curvebook-cli` or run directly via `cargo run -q -p curvebook-cli -- mcp`.

### MCP Client Presets

Always pass the **absolute path** to `curvebook-cli` and an existing `--workdir` folder so file I/O is confined by `WorkdirGuard`:

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/absolute/path/to/curvebook/target/release/curvebook-cli",
      "args": ["mcp", "--workdir", "/path/to/your/data-folder"]
    }
  }
}
```

For live attachment to a shared visible desktop window (`curvebook --control 9123 --window`; omit `--workdir` because `--workdir` and `--control-port` / `CURVEBOOK_CONTROL_PORT` are mutually exclusive and exit with code `2`):

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/absolute/path/to/curvebook-cli",
      "args": ["mcp", "--control-port", "9123"]
    }
  }
}
```

### Quick Smoke Check (stdio JSON-RPC)

Send newline-delimited JSON-RPC over stdin (CurveBook also auto-detects `Content-Length` framing) to verify `initialize`, `ping`, `tools/list`, or an atomic Welch's *t*-test `command_batch`:

```sh
./target/release/curvebook-cli mcp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"ping"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"command_batch","arguments":{"atomic":true,"commands":[{"command_id":"sheet.createData","params":{"sheet_id":"assay","kind":"Column","rows":3,"cols":2}},{"command_id":"table.setTitles","params":{"sheet_id":"assay","titles":["Control","Treated"]}},{"command_id":"table.paste","params":{"sheet_id":"assay","start_row":0,"start_col":0,"text":"10\t20\n11\t22\n12\t24"}},{"command_id":"stats.run","params":{"source_sheet_id":"assay","sheet_id":"ttest","kind":"welch_t"}}]}}}
EOF
```

Expected: `ping` returns `{}` and the `stats.run` step (`"index":3`) reports `"status":"UpToDate"` with `"t": -8.520563361656317` and `"PValue": 0.0036599555836826485`.

## Protocol & Wire Framing

Framing, handshake, and protocol revisions are documented in the [MCP server reference](reference/automation/mcp.md#transports-and-wire-framing). Implementation detail for contributors:

- **Transport (`crates/mcp/src/protocol.rs`):** Default headless `curvebook-cli mcp` opens no network sockets; loopback TCP (`127.0.0.1`) is used only when `--control-port` or `CURVEBOOK_CONTROL_PORT` is supplied.
- **Limits:** `MAX_REQUEST_BYTES = 16 MiB` (`MAX_DRAIN_BYTES = 64 MiB`), `MAX_HEADER_LINE_BYTES = 8 KiB`, `MAX_HEADER_COUNT = 32`, and `MAX_BATCH_LEN = 256`.
- **Handshake & methods:** `initialize` emits `SERVER_INSTRUCTIONS`. Supported methods: `initialize`, `shutdown`, `notifications/initialized`, `notifications/cancelled`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/templates/list`, `resources/read`, `prompts/list`, and `prompts/get`. In single-threaded stdio mode, `notifications/cancelled` applies to queued/batched requests (pre-cancelling matching request IDs processed later in a JSON-RPC batch) and exposes cooperative cancellation via the in-process `CancellationRegistry`.

## Modes & Reach

The desktop application (`curvebook`) provides the declarative `egui 0.37` desktop workspace ([`evidence/user-interface.md`](evidence/user-interface.md); Beta scope and platform notes in [`Beta known issues`](https://curvebook.org/manual/latest/troubleshooting.html#known-issues)), while `curvebook-cli mcp` exposes the full programmatic and AI-assistant automation surface over stdio (the WebAssembly browser bundle `curvebook-web` is deferred to `1.0`, [`R11`](ROADMAP.md#r11-usable-browser-app)).

| Mode | Reach |
|---|---|
| `curvebook-cli mcp` (default, headless) | Every registered engine command in `cmd::registry()` through `command_run`, `command_batch` (with `atomic: true` snapshot rollback), and per-command toolset tools, plus `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `sheet://` / `result://` resources, and guided prompts. UI-only `ui.*` commands report `"available": false`, `"requires_live_attach": true` in `command_list` and return `ErrorCode::Conflict` if called; the opt-in `gui` tools also return `Conflict` unless `--control-port` is active. |
| `curvebook-cli mcp --control-port <port>` | Same toolset surface forwarded over `127.0.0.1:<port>` to a running `curvebook --control <port>` instance (headless by default, or `curvebook --control <port> --window` for a shared visible desktop window), enabling the synced `ui.*` UI commands and the opt-in `gui` tools against the live `UiApp` session. |

## Status & Current Limitations

- **Verified stdio parity (`R7.3`):** Headless `command_run` / `command_batch` / `open_file` / `save_file` / `export` / `render_preview`, `WorkdirGuard` path containment, standalone *P*-value stack adjustment (`stats.pvalueStack`), Compact Letter Display (`graph.setCompactLetterDisplay`), and opt-in `curvebook --control <port> --window` shared desktop sessions are verified end to end over spawned `curvebook-cli mcp` stdio JSON-RPC subprocesses.
- **Desktop GUI & live-attach (`R10`, [`Beta known issues`](https://curvebook.org/manual/latest/troubleshooting.html#known-issues)):** The `curvebook` desktop application renders the declarative `egui 0.37` desktop workspace ([`evidence/user-interface.md`](evidence/user-interface.md)), and `curvebook --control <port> --window` attaches a visible desktop window synchronized with loopback control/MCP requests on the same live `UiApp` / `Session`. In parallel, the opt-in `gui` MCP toolset drives `UiApp` through the semantic harness (`crates/ui-egui/src/{inspect.rs, gui_actions.rs}`). Browser WebAssembly support (`curvebook-web`) is deferred to `1.0` ([`R0.28`](history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files), [`R11`](ROADMAP.md#r11-usable-browser-app)).
- **Headless UI commands:** In default headless `curvebook-cli mcp`, desktop `ui.*` commands report `"available": false` and require `--control-port <port>` attached to a running `curvebook --control <port>` process.
- **Live-attach `command_batch` rollback:** Both headless and `--control-port` live-attach modes support `"atomic": true` snapshot rollback; in `--control-port` live-attach mode, atomic `command_batch` delegates to the desktop host's atomic `command.batch` RPC (restoring the session and UI shell snapshot on failure; earlier filesystem writes cannot be unlinked and report `filesystem_side_effects_persisted: true`).
- **Deferred QoL items:** `QOL-10`, `QOL-11`, `QOL-13`, and `QOL-15` remain deferred without scheduled milestones ([`qol-enhancements.md`](planning/qol-enhancements.md)).

## CLI Flags, `WorkdirGuard` & Live-Attach Routing

```text
curvebook-cli mcp [--control-port <1..=65535>] [--workdir <dir>] [--control-token <token>]
```

- `--workdir <dir>` canonicalizes an existing accessible directory via `WorkdirGuard` (`crates/platform/src/path_guard.rs`) and confines `open_file`, `save_file`, `export`, `ui_screenshot` `path`, `ui_invoke_menu` `params`, recursive `PATH_PARAM_KEYS` inside `command_run` / `command_batch` (leaving inline `csv` text untouched), and stored `ExternalDelimitedFile` link refreshes during `project.recalculate`. Escapes fail with `ErrorCode::PathEscape` (`"path_escape"`, `category: "policy"`).
- **`--workdir` and `--control-port` are mutually exclusive:** combining them exits with code `2`, because `--workdir` cannot confine file operations executed inside an external desktop host process.
- Connection troubleshooting lives once on the published [troubleshooting page](reference/troubleshooting.md#mcp-connection); full tool/resource/prompt tables live in [MCP server reference](reference/automation/mcp.md).

### Live-Attach & Shared-Window Mode (`--control-port` / `--window`)

- Pass `--control-port <port>` (or set `CURVEBOOK_CONTROL_PORT=<port>`, overridden by `--control-port`) to attach MCP to a running `curvebook --control <port>` instance over loopback `127.0.0.1:<port>`.
- **Headless vs. shared visible window (`--window`):**
  - `curvebook --control <port>` (default, `--window` omitted) runs headless for scripts and CI automation.
  - `curvebook --control <port> --window` (also compatible with `CURVEBOOK_CONTROL_PORT=<port>`) opens a visible `eframe`/`egui` desktop window sharing the same live `UiApp` / `Session` and crash-recovery WAL journal between interactive window input and remote MCP/control requests (`127.0.0.1:<port>` loopback-only via `validate_loopback_addr`; cannot be combined with `--headless-once`).
- **Authentication:** Uses `--control-token <token>`, `CURVEBOOK_CONTROL_TOKEN`, or the per-UID `control-<port>.token` file located under `$CURVEBOOK_CONFIG_DIR`, platform config directories (`$HOME/Library/Application Support/CurveBook` on macOS, `$XDG_CONFIG_HOME/curvebook` / `$HOME/.config/curvebook` on Linux/BSD, `%APPDATA%\CurveBook` on Windows), or (when `HOME` is unset on Unix) a validated `0700` non-symlink `$XDG_RUNTIME_DIR/curvebook` or `$TMPDIR/curvebook-uid-<uid>` directory.
- **Timeouts:** Loopback TCP connect uses a fixed 2 s timeout; per-request read/write I/O defaults to 30 s for standard RPCs and 120 s for heavy operations (`doc.export`, `command.batch`, `engine.execute`, `project.open`, `project.save`, `project.instantiateTemplate`, `table.import*`, `render.preview`, `render.pdf`, `render.svg`, `ui.screenshot`). Set `CURVEBOOK_CONTROL_IO_TIMEOUT_MS` (`100..=600_000` ms) to override the read/write I/O timeout.
- **Routing:** When attached, `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `command_list`, `command_run`, `command_batch`, `resources/list`, and `resources/read` operate against the attached `curvebook --control` session rather than a disconnected local `Session`. `--workdir` and `--control-port` (`CURVEBOOK_CONTROL_PORT`) are mutually exclusive and exit with code `2` if combined, because `--workdir` cannot confine file operations executed inside a live-attached desktop host.

## `gui` Toolset & `GuiSessionHost` (`R8.6`, `R7.3`)

Verified in `crates/mcp/src/{gui_tools.rs, tools.rs, protocol.rs}` and `apps/curvebook-cli/src/main.rs` (see [`gui-workflows.md`](gui-workflows.md)):

### 1. Hosts & Live Attachment

- **`GuiSessionHost` (`crates/mcp/src/tools.rs`):** Trait (`call_gui`, plus optional `hosted_session` / `hosted_session_mut` / `sync_after_session_mutation`) implemented outside `crates/mcp` so MCP can drive an embedded `UiApp` without depending on `curvebook-ui-egui`. `McpServerState` routes all `8` `gui` tools either to an attached `GuiSessionHost` (`with_gui_host` / `set_gui_host`) or over loopback TCP via `--control-port <port>` (`with_control_port`, `run_stdio_with_control_port`). Calling a `gui` tool with neither returns `ErrorCode::Conflict`.
- **Live attachment:** Run `curvebook --control <N>` (or `curvebook --control <N> --window` for a visible shared desktop window) and connect with `curvebook-cli mcp --control-port <N>`, then call `enable_toolset` with `{ "group": "gui", "enabled": true }` to expose the `8` `gui` tools alongside the core tools.

### 2. The 8-Tool `gui` Surface (`GUI_TOOL_NAMES`)

Calling `enable_toolset` with `{ "group": "gui" }` (or attaching a `GuiSessionHost`) appends the `8` `gui` tools alongside the core tools above (and any enabled `CommandGroup` tools):

| MCP Tool | Key Inputs | Key Annotations / Behavior |
|---|---|---|
| `ui_inspect` | `scope` (`"all"`, `"workspace"`, `"dialog"`, `"grid"`, `"navigator"`, `"canvas"`, `"menus"`, `"toolbar"`, `"tabs"`), `include_bounds`, `metrics` (`{logical_width, logical_height, scale_factor}`; accepts `width_px`/`height_px` aliases) | `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`; inspects the active sheet (use `ui_switch_tab` to activate another sheet) and returns `UiTreeSnapshot` text + structured JSON. |
| `ui_screenshot` | `target` (`"full_window"`, `"workspace"`, `"viewport"`, `"canvas"`, `"dialog"`, default `"workspace"`), `width_px` (`64..=4096`), `height_px` (`64..=4096`), `dpi` (`72..=600`), optional `path` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: true`; returns inline `image/png` block + metadata (`structuredContent` omits base64 duplication). Optional `path` is checked via `WorkdirGuard` when configured. |
| `ui_pointer_gesture` | `action` (`"click"`, `"double_click"`, `"right_click"`, `"drag"`, `"hover"`, `"wheel"`), `widget_id`/`target_id`, `x`/`y`, `to_widget_id`/`to_x`/`to_y`, `modifiers`, `delta_y`, `hover_fraction_x`, `hover_n`, `metrics` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true` (worst-case GUI dispatch effects). |
| `ui_type_text` | `text`, `widget_id`, `clear_existing`, `commit_enter` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true` (worst-case text/paste effects); supports multi-cell TSV. |
| `ui_press_key` | `key`, `modifiers` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true` (worst-case GUI dispatch effects); dispatches keystrokes/shortcuts (`Enter`, `Escape`, `Tab`, `Cmd/Ctrl+T`, etc.). |
| `ui_invoke_menu` | `path` (string or string array), `command_id`, optional `params` (`WorkdirGuard`-sanitized) | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true` (worst-case GUI dispatch effects); fails with `Conflict` when item is disabled. |
| `ui_switch_tab` | `tab`/`tab_id` (string or non-negative integer index), `sheet_id`, `widget_id` | `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`; honors modal tab graying predicates; `sheet_id` alone selects a sheet. |
| `ui_answer_dialog` | `action` (`ok`, `cancel`, `back`, `next`, `create`, `apply`, `preview`), `fields`/`values`/`patches`, `force_create_new`, `choice` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true` (worst-case modal dialog effects). |

## UI Command Sync

Desktop UI commands declared in `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`; live count in [`STATUS.md`](STATUS.md)) are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (enforced in CI with `cargo xtask ui-commands --check`) so `curvebook-mcp` never depends directly on `curvebook-ui-egui`.
