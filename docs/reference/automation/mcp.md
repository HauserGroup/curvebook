# MCP Server Reference

CurveBook exposes its calculation, graphing, inspection, and UI automation surface over the Model Context Protocol (`MCP`, JSON-RPC 2.0). The stdio transport works identically with headless documents or attached to a live desktop window started with `--control <port>`.

For first-time client configuration snippets and verification steps, see [MCP setup for beginners](../../mcp-for-beginners.md). For complete machine-generated tool schemas, annotations, prompts, and resources, see the [MCP Tools & Surface Registry](../registry/mcp-tools.html) and [Engine Command Registry](../registry/commands.html).

## Transports and Wire Framing

`curvebook-cli mcp` reads and writes JSON-RPC 2.0 messages on standard input and standard output:

- **NDJSON framing:** One compact UTF-8 JSON object per line, terminated by `\n`.
- **Content-Length framing:** `Content-Length: <n>\r\n\r\n` header followed by `<n>` UTF-8 JSON payload bytes.

Both framing modes are auto-detected per incoming request. Payloads larger than `MAX_REQUEST_BYTES` (`16 MiB`) return JSON-RPC error `-32600` (`Invalid Request`), and the server safely drains up to `MAX_DRAIN_BYTES` (`64 MiB`) to recover framing boundaries. Protocol revisions `2024-11-05` and `2025-03-26` are negotiated on `initialize`.

Single-call `initialize` request and response syntax:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"client","version":"1.0.0"}}}
```

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":true},"resources":{"subscribe":false,"listChanged":false},"prompts":{"listChanged":false}},"serverInfo":{"name":"curvebook","version":"<version>"},"instructions":"..."}}
```

## Attachment Modes and Filesystem Policy

| Mode | Command syntax | Session backing |
|---|---|---|
| Headless stdio | `curvebook-cli mcp [--workdir <dir>]` | In-process `engine::Session` |
| Live-attach stdio | `curvebook-cli mcp --control-port <port> [--control-token <token>]` | Forwarded over loopback `127.0.0.1:<port>` to running desktop `curvebook --control <port>` |
| Strict GUI-only (in-process) | `curvebook-cli mcp --gui-only [--workdir <dir>]` | In-process `UiApp` exposing only the `gui` toolset |
| Strict GUI-only (live-attach) | `curvebook-cli mcp --control-port <port> --gui-only` | Attached to a `--gui-only` desktop control listener |

Pass `--workdir <dir>` to confine all filesystem reads, saves, exports, and external delimited-file links inside `<dir>` using canonicalized containment checks (`ErrorCode::PathEscape` if violated). Control tokens may also be supplied via `CURVEBOOK_CONTROL_TOKEN` or auto-discovered from the per-port local token file written by the desktop listener.

## Core Tools and Dynamic Toolsets

On startup in normal mode, `tools/list` exposes the core MCP tools immediately (`doc_inspect`, `open_file`, `save_file`, `command_list`, `command_run`, `command_batch`, `render_preview`, `export`, `list_toolsets`, `enable_toolset`). Call `list_toolsets` and `enable_toolset` to load per-command tools grouped by engine `CommandGroup` or to toggle the `gui` toolset dynamically (which emits `notifications/tools/list_changed`).

Each tool carries standard MCP `annotations` derived from its typed `Effects` mask:

- `readOnlyHint`: `true` when the operation neither mutates the document nor writes to the filesystem or UI state.
- `destructiveHint`: `true` when `MAY_DISCARD_DATA` or filesystem overwrites can discard or alter existing work.
- `idempotentHint`: `true` when repeated calls with identical parameters produce the same state (`IDEMPOTENT`).
- `openWorldHint`: always `false`; CurveBook performs zero outbound network requests.

Single-call `tools/call` request and response syntax (`doc_inspect`):

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"doc_inspect","arguments":{"include_cells":false}}}
```

```json
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"title\":\"Untitled\",\"schema_version\":7,\"file_path\":null,\"dirty\":false,\"total_sheet_count\":0,\"active_sheet_id\":null,\"family_anchor_sheet_id\":null,\"sheets\":{\"data\":[],\"info\":[],\"analyses\":[],\"graphs\":[],\"layouts\":[]},\"variables\":[],\"hooks\":[],\"analyses\":[],\"graphs\":[],\"layouts\":[]}"}],"isError":false}}
```

Exact schemas, annotations, and single-call JSON request/response examples for every core tool, every dynamic `CommandGroup` toolset, and the `gui` toolset are published in the [MCP Tools & Surface Registry](../registry/mcp-tools.html).

## Resources and Prompts

- **Resources (`resources/list`, `resources/read`):** Exposes `sheet://<id>` (`application/json`) for every sheet in the active project and `result://<id>` (`application/json`) for computed `ResultSheet` outputs. Disabled under `--gui-only` (`gui_only_violation`).
- **Prompts (`prompts/list`, `prompts/get`):** Built-in structured prompts (`dose_response_curve_fit`, `choose_two_group_test`, `publication_figure_checklist`) provide parameterised analysis checklists tailored to either standard mode or `--gui-only` mode.

Single-call `resources/read` syntax:

```json
{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"sheet://data-1"}}
```

## Strict `--gui-only` Policy {#gui-only}

Under `--gui-only` (`GuiOnlyPolicy::Strict`), `tools/list` exposes only the interactive `gui` toolset (`ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`, `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`). Calling `resources/read` returns JSON-RPC error `-32600` (`Invalid Request`) with `data.code: "gui_only_violation"`. Calling any non-`gui` tool (`command_run`, `command_batch`, `enable_toolset`, `doc_inspect`, `open_file`, `save_file`, `export`, `render_preview`) returns a `tools/call` result with `isError: true` and `structuredContent.code: "gui_only_violation"` (or JSON-RPC `-32600` if surfaced as a protocol-level error).

## Structured Error Payloads

Tool execution failures set `result.isError: true` and emit a structured JSON object matching `StructuredErrorPayload`: `code`, `category`, `message`, `retryable`, `remediation`, and `details` (`command_batch` adds `failed_index`, `failed_command_id`, and `error`). For the full catalog of `ErrorCode` variants, see [Control Protocol & Batch Reference](control-and-batch.md#error-codes).
