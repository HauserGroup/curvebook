# MCP Server: Source Builds & Internals (`curvebook-mcp`)

> **Who this page is for.** Contributors and people running CurveBook from a source checkout. To connect an AI assistant to an installed release, follow the [beginner's guide](mcp-for-beginners.md). The published [MCP server reference](reference/automation/mcp.md) is the canonical description of transports, attachment modes, tools, resources, and error payloads, and the [troubleshooting page](reference/troubleshooting.md#mcp-connection) covers connection problems. This page adds source builds, client configuration presets, and implementation detail.

`curvebook-cli mcp` runs a stdio [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server (`crates/mcp`) over an in-memory headless `curvebook-engine` session by default, or attached over loopback TCP (`127.0.0.1:<port>`) to a running `curvebook --control <port>` instance via `curvebook-cli mcp --control-port <port>` ([`control-protocol.md`](control-protocol.md)). By default `curvebook --control <port>` runs headless; pass the opt-in `--window` flag (`curvebook --control <port> --window`) to attach a visible desktop window sharing the same live `UiApp` / `Session` ([`R7.3`](history/roadmap/r7.md#r73-mcp-workflow-parity-proof--shared-live-window)).

> Related specifications: [`commands.md`](commands.md), [`gui-workflows.md`](gui-workflows.md), [`control-protocol.md`](control-protocol.md), [`scripting-and-automation.md`](implementation/scripting-and-automation.md) · Beginner guide: [`mcp-for-beginners.md`](mcp-for-beginners.md) · Project overview: [`README.md`](../README.md) · Live test & command counts: [`STATUS.md`](STATUS.md)

## Quick Start: Build, Configure & Verify

The **Model Context Protocol (MCP)** lets an AI assistant or script talk to CurveBook's statistical and graphing engine over standard input/output (stdio) using structured JSON messages.

Key terms in this guide:
- **stdio:** Your MCP client launches `curvebook-cli mcp` as a local subprocess and exchanges JSON messages through its standard input and output streams—no background daemon or open network port is started.
- **Headless mode:** Runs calculations, file imports, and figure exports directly in memory with no GUI window (the default for `curvebook-cli mcp`).
- **`--workdir`:** Sandboxes all file reads, saves, and exports inside one chosen folder on disk so paths outside it or using `..` are rejected.

### 1. Build or Run the Server

If you installed a release package, `curvebook-cli` is included alongside the desktop app (see the [Get Started guide](https://hausergroup.github.io/curvebook/manual/v0.1/get-started.html) for per-platform paths).

To build the standalone release binary from a source checkout (recommended for MCP clients):

```sh
cargo build --release -p curvebook-cli
```

This creates `./target/release/curvebook-cli` (`.\target\release\curvebook-cli.exe` on Windows). You can also install onto `PATH` with `cargo install --path apps/curvebook-cli` (verify with `which curvebook-cli`), or run directly from a repository checkout without a separate build step via `cargo run -q -p curvebook-cli -- mcp`.

### 2. Configure Your MCP Client

Create an existing folder for your data and exports (for example `mkdir -p /path/to/your/data-folder`), then point your MCP client to the **full absolute path** of `curvebook-cli` so it does not rely on shell `PATH` or working-directory lookup:

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

- `curvebook-cli mcp` starts in **default headless mode**.
- `--workdir` must name an existing directory; CurveBook resolves relative paths beneath it and blocks escapes (`path_escape`). Omit `--workdir` only when testing pure in-memory calculations from a terminal without disk I/O. (See [Example Configurations](#example-configurations) below for `--control-port` live-attach and `--gui-only` presets.)

### 3. Verify the Connection

Send `initialize`, `notifications/initialized`, `ping`, and `tools/list` as newline-delimited JSON-RPC requests over stdin (one JSON object per line; CurveBook also auto-detects standard `Content-Length` MCP framing):

```sh
./target/release/curvebook-cli mcp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"quickstart","version":"0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"ping"}
{"jsonrpc":"2.0","id":3,"method":"tools/list"}
EOF
```

Expected output (three JSON lines on stdout; `notifications/initialized` produces no reply):
1. Line 1 (`"id":1`): `"protocolVersion":"2024-11-05"` and `"serverInfo":{"name":"curvebook","version":"<package version>"}`.
2. Line 2 (`"id":2`): exact pong `{"id":2,"jsonrpc":"2.0","result":{}}`.
3. Line 3 (`"id":3`): `"tools"` array containing the default core tools: `doc_inspect`, `open_file`, `save_file`, `command_list`, `command_run`, `command_batch`, `render_preview`, `export`, `list_toolsets`, and `enable_toolset`.

### 4. End-to-End Example (Welch's *t*-test)

Create a two-group table (`Control`: `10, 11, 12`; `Treated`: `20, 22, 24`) and run an unpaired Welch's *t*-test in one all-or-nothing (`"atomic": true`) `command_batch` call:

```sh
./target/release/curvebook-cli mcp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_batch","arguments":{"atomic":true,"commands":[{"command_id":"sheet.createData","params":{"sheet_id":"assay","kind":"Column","rows":3,"cols":2}},{"command_id":"table.setTitles","params":{"sheet_id":"assay","titles":["Control","Treated"]}},{"command_id":"table.paste","params":{"sheet_id":"assay","start_row":0,"start_col":0,"text":"10\t20\n11\t22\n12\t24"}},{"command_id":"stats.run","params":{"source_sheet_id":"assay","sheet_id":"ttest","kind":"welch_t"}}]}}}
EOF
```

Expected output:
1. Line 1 (`"id":1`): `initialize` response (`"serverInfo":{"name":"curvebook","version":"<package version>"}`).
2. Line 2 (`"id":2`): `tools/call` response with `"isError":false` whose final `stats.run` step (`"index":3`) reports `"message":"ran Welch's t test"`, `"status":"UpToDate"`, and these exact scalar results:
   - `"t"`: `-8.520563361656317`
   - `"PValue"`: `0.0036599555836826485`
   - `"CohensD"`: `-6.957010852370434`
   - `"HedgesG"`: `-5.550891548444351`

## Protocol & Wire Framing

Framing, handshake, and protocol revisions are documented in the [MCP server reference](reference/automation/mcp.md#transports-and-wire-framing). Implementation detail for contributors:

- **Transport (`crates/mcp/src/protocol.rs`):** Default headless `curvebook-cli mcp` opens no network sockets; loopback TCP (`127.0.0.1`) is used only when `--control-port` or `CURVEBOOK_CONTROL_PORT` is supplied.
- **Limits:** `MAX_REQUEST_BYTES = 16 MiB` (`MAX_DRAIN_BYTES = 64 MiB`), `MAX_HEADER_LINE_BYTES = 8 KiB`, `MAX_HEADER_COUNT = 32`, and `MAX_BATCH_LEN = 256`.
- **Handshake:** `initialize` emits `SERVER_INSTRUCTIONS` (or `GUI_ONLY_SERVER_INSTRUCTIONS` under `--gui-only`). Supported methods: `notifications/initialized`, `notifications/cancelled`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, and `prompts/get`.

## Modes & Reach

The desktop application (`curvebook`) provides the interactive `LayoutTree` GUI ([`evidence/user-interface.md`](evidence/user-interface.md); Beta scope and platform notes in [`Beta known issues`](https://hausergroup.github.io/curvebook/manual/v0.1/troubleshooting.html#known-issues)), while `curvebook-cli mcp` exposes the full programmatic and AI-assistant automation surface over stdio (the WebAssembly browser bundle `curvebook-web` is deferred to `1.0`, [`R11`](ROADMAP.md#r11-usable-browser-app)).

| Mode | Reach |
|---|---|
| `curvebook-cli mcp` (default, headless) | Every registered engine command in `cmd::registry()` through `command_run`, `command_batch` (with `atomic: true` snapshot rollback), and per-command toolset tools, plus `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `sheet://` / `result://` resources, and guided prompts. UI-only `ui.*` commands report `"available": false`, `"requires_live_attach": true` in `command_list` and return `ErrorCode::Conflict` if called; the `gui` tools also return `Conflict` unless `--control-port` or `--gui-only` is active. |
| `curvebook-cli mcp --control-port <port>` | Same toolset surface forwarded over `127.0.0.1:<port>` to a running `curvebook --control <port>` instance (headless by default, or `curvebook --control <port> --window` for a shared visible desktop window), enabling the synced `ui.*` UI commands and the `gui` tools against the live `UiApp` session. |
| `curvebook-cli mcp --gui-only` (or `--control-port <port> --gui-only`) | Exposes strictly the `gui` tools, driving `UiApp` through the semantic agent harness over stdio JSON-RPC (`R7.3`). |

## Status & Current Limitations

- **Verified stdio parity (`R7.3`):** Interactive workflow parity (including standalone *P*-value stack adjustment `stats.pvalueStack` and Compact Letter Display `graph.setCompactLetterDisplay`) is verified end to end over spawned `curvebook-cli mcp --gui-only` stdio JSON-RPC subprocesses, alongside headless `command_run` / `command_batch` / `open_file` / `save_file` / `export` / `render_preview`, `WorkdirGuard` path containment, and opt-in `curvebook --control <port> --window` shared desktop sessions.
- **Desktop GUI & live-attach (`R10`, [`Beta known issues`](https://hausergroup.github.io/curvebook/manual/v0.1/troubleshooting.html#known-issues)):** The `curvebook` desktop application renders the interactive `LayoutTree` workspace ([`evidence/user-interface.md`](evidence/user-interface.md)), and `curvebook --control <port> --window` attaches a visible desktop window synchronized with loopback control/MCP requests on the same live `UiApp` / `Session`. In parallel, the `gui` MCP toolset drives `UiApp` through the semantic harness (`crates/ui-egui/src/{inspect.rs, gui_actions.rs}`). Browser WebAssembly support (`curvebook-web`) is deferred to `1.0` ([`R0.28`](history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files), [`R11`](ROADMAP.md#r11-usable-browser-app)).
- **Headless UI commands:** In default headless `curvebook-cli mcp`, desktop `ui.*` commands report `"available": false` and require `--control-port <port>` attached to a running `curvebook --control <port>` process.
- **Live-attach `command_batch` rollback:** In headless mode, `command_batch` with `"atomic": true` restores an exact session snapshot on failure; in `--control-port` live-attach mode, rollback replays `edit.undo` for completed mutating steps (`ping` and read-only `ui.*` commands excluded), so remote rollback follows the desktop session's undo stack.
- **Deferred QoL items:** `QOL-10`, `QOL-11`, `QOL-13`, and `QOL-15` remain deferred without scheduled milestones ([`qol-enhancements.md`](planning/qol-enhancements.md)).

## CLI Flags & Advanced Configuration

```text
curvebook-cli mcp [--control-port <1..=65535>] [--gui-only] [--workdir <dir>] [--control-token <token>]
```

- Always use the **absolute path** to `curvebook-cli` in GUI MCP client configs so the host application does not depend on shell `PATH` or working-directory lookup.
- `--workdir <dir>` canonicalizes an existing accessible directory via `WorkdirGuard` (`crates/platform/src/path_guard.rs`) and confines `open_file`, `save_file`, `export`, non-`--gui-only` `ui_screenshot` `path`, `ui_invoke_menu` `params`, recursive `PATH_PARAM_KEYS` inside `command_run` / `command_batch` (leaving inline `csv` text untouched), and stored `ExternalDelimitedFile` link refreshes during `project.recalculate`. Escapes (`..`, symlinks pointing outside, or external absolute paths) fail with `ErrorCode::PathEscape` (`"path_escape"`, `category: "policy"`). When `--gui-only --workdir <dir>` is passed, `InProcessUiGuiHost` also applies `ExternalPathPolicy::workdir` to the in-process `Session`.

### Example Configurations

The headless configuration with `--workdir` is shown in [Configure Your MCP Client](#2-configure-your-mcp-client). Two further presets:

Live-attached to a shared visible desktop window (`curvebook --control 9123 --window`):

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/absolute/path/to/curvebook-cli",
      "args": ["mcp", "--control-port", "9123", "--workdir", "/path/to/workspace"]
    }
  }
}
```

Strict `--gui-only` agent-harness mode (in-process `UiApp`):

```json
{
  "mcpServers": {
    "curvebook-gui": {
      "command": "/absolute/path/to/curvebook-cli",
      "args": ["mcp", "--gui-only", "--workdir", "/path/to/workspace"]
    }
  }
}
```

## Troubleshooting the connection

Connection problems (paths, `PATH`, `"type": "stdio"`, clean stdout, and the errors that appear after connecting) are covered once, on the published [troubleshooting page](reference/troubleshooting.md#mcp-connection).

## Shipped Surface (`R7.1`, `R0.13`, `R7.3`)

Every handler executes inside `run_guarded` with structured error payloads (`code`, `category`, `message`, `retryable`, `remediation`, `details`), `Effects`-derived MCP annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`), and optional `WorkdirGuard` path containment:

| Surface | Purpose |
|---|---|
| `doc_inspect` / `open_file` / `save_file` | Inspect project sheets (`Data`, `Info`, `Analyses`, `Graphs`, `Layouts`) or live `--control` snapshots (including per-sheet previews when `sheet_id` or `include_cells` is given), open `.curvebook` / `.curvebook.d` / `.cbtemplate` / `.prism` / `.pzfx` / `.csv` / `.tsv`, and atomically save `.curvebook`, `.curvebook.d`, or `.cbtemplate` projects — forwarding to the attached session when `--control-port` is configured. |
| `command_list` / `command_run` / `command_batch` | Discover and execute typed engine commands ([`commands.md`](commands.md)) plus the synced UI commands (`crates/mcp/src/ui_catalog.rs`), up to `1024` steps per `command_batch` with optional `atomic: true` rollback. `command_list` reports `"live_attached"`, `"mode"` (`"live_attach"` vs `"headless"`), and per-command `"available"` (`ui.*` commands report `"available": false`, `"requires_live_attach": true` when headless). During the one-release parameter-deprecation window (`R16.5`), calls supplying retired `x-deprecated-aliases` parameter keys continue to succeed and return `"warnings": [...]` on `command_run` payloads and affected `command_batch` step entries (canonical `snake_case` calls omit `"warnings"`). |
| `list_toolsets` / `enable_toolset` | Progressively enable or disable per-command MCP tools grouped by `CommandGroup` (`project`, `edit`, `sheet`, `table`, `info`, `analysis`, `graph`, `layout`, `export`, or `"all"`), plus the `"gui"` toolset (`8` `GUI Automation` tools), emitting `notifications/tools/list_changed` whenever the active set changes. |
| `render_preview` / `export` | Compile and rasterize any `GraphSheet` or `LayoutSheet` to inline PNG preview bytes, or export `SVG` / `%PDF-1.7` / `PNG` / `TIFF` (`none` / `deflate` / `lzw`) / `CSV` / `JSON` inline or atomically to disk. For CSV exports on a `ResultSheet`, supplying `tab_id` defaults `tidy` to `false` so the requested tab exports directly, while explicit `tidy: true` + `tab_id` fails with `InvalidParams`. |
| MCP Resources & Prompts | Read `sheet://<id>` and `result://<id>` resources (from the attached desktop session when `--control-port` is set, or the local session in headless mode) and guided workflow prompts (`dose_response_curve_fit`, `choose_two_group_test`, `publication_figure_checklist`). |

### Live-Attach & Shared-Window Mode (`--control-port` / `--window`)

- Pass `--control-port <port>` (or set `CURVEBOOK_CONTROL_PORT=<port>`, overridden by `--control-port`) to attach MCP to a running `curvebook --control <port>` instance over loopback `127.0.0.1:<port>`.
- **Headless vs. shared visible window (`--window`):**
  - `curvebook --control <port>` (default, `--window` omitted) runs headless for scripts and CI automation.
  - `curvebook --control <port> --window` (also compatible with `CURVEBOOK_CONTROL_PORT=<port>`) opens a visible `eframe`/`egui` desktop window sharing the same live `UiApp` / `Session` and crash-recovery WAL journal between interactive window input and remote MCP/control requests (`127.0.0.1:<port>` loopback-only via `validate_loopback_addr`; cannot be combined with `--headless-once`).
  - Add `--gui-only` (`curvebook --control <port> --window --gui-only`) to enforce `Caller::Control` `--gui-only` restrictions on the control socket while keeping the shared window live.
- **Authentication:** Uses `--control-token <token>`, `CURVEBOOK_CONTROL_TOKEN`, or the per-UID `control-<port>.token` file located under `$CURVEBOOK_CONFIG_DIR`, platform config directories (`$HOME/Library/Application Support/CurveBook` on macOS, `$XDG_CONFIG_HOME/curvebook` / `$HOME/.config/curvebook` on Linux/BSD, `%APPDATA%\CurveBook` on Windows), or (when `HOME` is unset on Unix) a validated `0700` non-symlink `$XDG_RUNTIME_DIR/curvebook` or `$TMPDIR/curvebook-uid-<uid>` directory.
- **Routing:** When attached, `doc_inspect`, `open_file`, `save_file`, `render_preview`, `export`, `command_list`, `command_run`, `command_batch`, `resources/list`, and `resources/read` operate against the attached `curvebook --control` session rather than a disconnected local `Session`, while local `--workdir` containment continues to be enforced on file paths.

## `gui` Toolset, `GuiSessionHost` & `--gui-only` Mode (`R8.6`–`R8.8`, `R7.3`)

Verified in `crates/mcp/src/{gui_tools.rs, tools.rs, protocol.rs}` and `apps/curvebook-cli/src/main.rs` (see [`gui-workflows.md`](gui-workflows.md)):

### 1. Hosts & CLI Modes

- **`GuiSessionHost` (`crates/mcp/src/tools.rs`):** Trait (`call_gui`, `set_gui_only`, `is_gui_only`) implemented outside `crates/mcp` so MCP can drive a live `UiApp` without depending on `curvebook-ui-egui`. `McpServerState` routes all `8` `gui` tools either to an attached `GuiSessionHost` (`with_gui_host` / `set_gui_host`) or over loopback TCP via `--control-port <port>` (`with_control_port` / `with_control_port_and_gui_only`, `run_stdio_with_control_port` / `run_stdio_with_control_port_gui_only`). Calling a `gui` tool with neither returns `ErrorCode::Conflict`.
- **`InProcessUiGuiHost` (`apps/curvebook-cli/src/main.rs`):** Wraps an in-process `UiApp` for `curvebook-cli mcp --gui-only` (propagating `ExternalPathPolicy::workdir` when `--workdir` is set), normalizing `"workspace"`/`"viewport"` screenshot targets to `"full_window"`, `"wheel"` gestures to `"scroll"`, and array menu `path`s into `"/"`-joined strings before delegating to `curvebook_ui_egui::dispatch_gui_action`.
- **Two `--gui-only` launch modes:**
  - `curvebook-cli mcp --gui-only` — in-process `UiApp` via `InProcessUiGuiHost` with `GuiOnlyPolicy::Strict`.
  - `curvebook-cli mcp --control-port <N> --gui-only` — attaches over loopback `127.0.0.1:<N>` to a running `curvebook --control <N> --gui-only` instance (or `curvebook --control <N> --window --gui-only` for a visible shared desktop window).

### 2. The 8-Tool `gui` Surface (`GUI_TOOL_NAMES`)

In normal mode, `enable_toolset` with `{ "group": "gui" }` (or attaching a `GuiSessionHost`) appends the `8` `gui` tools alongside the core tools above (and any enabled `CommandGroup` tools). Under `--gui-only`, `list_tools_for_state` shrinks `tools/list` strictly to these `8` tools:

| MCP Tool | Key Inputs | Key Annotations / Behavior |
|---|---|---|
| `ui_inspect` | `scope` (`"all"`, `"workspace"`, `"dialog"`, `"grid"`, `"navigator"`, `"canvas"`, `"menus"`, `"toolbar"`, `"tabs"`), `include_bounds`, `metrics` (`{logical_width, logical_height, scale_factor}`; accepts `width_px`/`height_px` aliases) | `readOnlyHint: true`, `idempotentHint: true`; inspects the active sheet (use `ui_switch_tab` to activate another sheet) and returns `UiTreeSnapshot` text + structured JSON. |
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
- **Contrast with raw `--control --gui-only` TCP:** On the direct loopback TCP socket (`curvebook --control <port> --gui-only`, [`control-protocol.md`](control-protocol.md)), read-only inspection (`ping`, `doc.inspect`, `ui.state` / `state.snapshot`, `command.list`, `ui.inspect`, `ui.canvas.preflight`) remains allowed while mutating/L5 calls and disk-writing paths (`ui.screenshot` / `render.pdf` with `path`) are rejected with `gui_only_violation`.

## UI Command Sync

Desktop UI commands declared in `crates/ui-egui/src/menus/ui_commands.rs` (`UI_COMMANDS`; live count in [`STATUS.md`](STATUS.md)) are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (enforced in CI with `cargo xtask ui-commands --check`) so `curvebook-mcp` never depends directly on `curvebook-ui-egui`.
