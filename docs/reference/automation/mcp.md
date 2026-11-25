# MCP Server Reference

CurveBook exposes its calculation, graphing, inspection, and UI automation surface over the Model Context Protocol (`MCP`, JSON-RPC 2.0). The stdio transport works identically with headless documents or attached to a live desktop window started with `--control <port>`.

For first-time client configuration snippets and verification steps, see [MCP setup for beginners](../../mcp-for-beginners.md). For complete machine-generated tool schemas, annotations, prompts, and resources, see the [MCP Tools & Surface Registry](../registry/mcp-tools.html) and [Engine Command Registry](../registry/commands.html).

## Transports and Wire Framing

`curvebook-cli mcp` reads and writes JSON-RPC 2.0 messages on standard input and standard output:

- **NDJSON framing:** One compact UTF-8 JSON object per line, terminated by `\n`.
- **Content-Length framing:** `Content-Length: <n>\r\n\r\n` header followed by `<n>` UTF-8 JSON payload bytes.

Both framing modes are auto-detected per incoming request. Payloads larger than `MAX_REQUEST_BYTES` (`16 MiB`) return JSON-RPC error `-32600` (`Invalid Request`), and the server safely drains up to `MAX_DRAIN_BYTES` (`64 MiB`) to recover framing boundaries. JSON-RPC batch request arrays accept up to `MAX_BATCH_LEN` (`256`) elements (empty or oversized batches return `-32600`). Protocol revisions `2024-11-05` and `2025-03-26` are negotiated on `initialize`.

Supported JSON-RPC methods: `initialize`, `shutdown` (marks the connection shutting down so subsequent operational requests return `-32600`), `ping`, `notifications/initialized`, `notifications/cancelled`, `tools/list`, `tools/call`, `resources/list`, `resources/templates/list`, `resources/read`, `prompts/list`, and `prompts/get`. In single-threaded stdio mode, `notifications/cancelled` applies to queued/batched requests (pre-cancelling matching request IDs processed later in a JSON-RPC batch) and cooperates with in-process `CancellationRegistry` handles.

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

Pass `--workdir <dir>` to confine all filesystem reads, saves, exports, and external delimited-file links inside `<dir>` using canonicalized containment checks (`ErrorCode::PathEscape` if violated). `--workdir` and `--control-port` (including `CURVEBOOK_CONTROL_PORT`) are mutually exclusive and fail immediately with exit code `2`, because `--workdir` cannot confine file operations executed inside a live-attached desktop host process.

Environment variables for live-attach connections:

- `CURVEBOOK_CONTROL_PORT`: Loopback TCP control port (`1..=65535`); overridden by `--control-port`.
- `CURVEBOOK_CONTROL_TOKEN`: Bearer authentication token; overridden by `--control-token`, or auto-discovered from the per-port `control-<port>.token` file written by the desktop listener.
- `CURVEBOOK_CONFIG_DIR`: Overrides the platform configuration directory searched for `control-<port>.token`.
- `CURVEBOOK_CONTROL_IO_TIMEOUT_MS`: Loopback TCP read/write I/O timeout in milliseconds (`100..=600_000` ms; defaults to 30 s for standard RPCs and 120 s for heavy operations such as `doc.export`, `command.batch`, `engine.execute`, imports, and renders; TCP connect uses a fixed 2 s timeout).

## Core Tools and Dynamic Toolsets

On startup in normal mode, `tools/list` exposes the core MCP tools immediately (`doc_inspect`, `open_file`, `save_file`, `command_list`, `command_run`, `command_batch`, `render_preview`, `export`, `list_toolsets`, `enable_toolset`). `command_batch` accepts up to `MAX_BATCH_COMMANDS` (`1024`) command steps per call. Call `list_toolsets` and `enable_toolset` to load per-command tools grouped by engine `CommandGroup` or to toggle the `gui` toolset dynamically (which emits `notifications/tools/list_changed`).

Each tool carries standard MCP `annotations` derived from its typed `Effects` mask:

- `readOnlyHint`: `true` when the operation neither mutates the document nor alters UI/toolset state (`CHANGES_UI_STATE`), discards data, writes the filesystem, or launches subprocesses.
- `destructiveHint`: `true` when `MAY_DISCARD_DATA` or `WRITES_FILESYSTEM` is set (file-writing tools can overwrite existing targets on disk).
- `idempotentHint`: `true` when repeated calls with identical parameters produce the same state (`IDEMPOTENT`).
- `openWorldHint`: `true` when `Effects` includes `READS_FILESYSTEM`, `WRITES_FILESYSTEM`, or `LAUNCHES_PROCESS`, and `false` for pure in-memory tools (CurveBook performs zero outbound network requests).

Single-call `tools/call` request and response syntax (`doc_inspect`):

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"doc_inspect","arguments":{"include_cells":false}}}
```

```json
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"title\":\"Untitled Project\",\"schema_version\":7,\"file_path\":null,\"dirty\":false,\"total_sheet_count\":0,\"active_sheet_id\":null,\"family_anchor_sheet_id\":null,\"sheets\":{\"data\":[],\"info\":[],\"analyses\":[],\"graphs\":[],\"layouts\":[]},\"variables\":{\"info_constants\":[],\"result_scalars\":[]},\"hooks\":[],\"analyses\":[],\"graphs\":[],\"layouts\":[]}"}],"isError":false,"structuredContent":{"title":"Untitled Project","schema_version":7,"file_path":null,"dirty":false,"total_sheet_count":0,"active_sheet_id":null,"family_anchor_sheet_id":null,"sheets":{"data":[],"info":[],"analyses":[],"graphs":[],"layouts":[]},"variables":{"info_constants":[],"result_scalars":[]},"hooks":[],"analyses":[],"graphs":[],"layouts":[]}}}
```

In `--control-port` live-attach mode, calling `doc_inspect` without `sheet_id` always returns `mode: "live_attach"`, `control_port`, the live desktop `snapshot`, and top-level project summary fields (regardless of `include_cells`); supplying `sheet_id` returns the targeted sheet detail (with optional `include_cells` previews) plus `mode` and `control_port`.

Exact schemas, annotations, and single-call JSON request/response examples for every core tool, every dynamic `CommandGroup` toolset, and the `gui` toolset are published in the [MCP Tools & Surface Registry](../registry/mcp-tools.html).

## Resources and Prompts

- **Resources (`resources/list`, `resources/templates/list`, `resources/read`):** Exposes `sheet://<id>` (`application/json`) for every sheet in the active project, `result://<id>` (`application/json`) for computed `ResultSheet` outputs, and URI templates (`sheet://{id}`, `result://{id}`) via `resources/templates/list`.
- **Prompts (`prompts/list`, `prompts/get`):** Built-in structured prompts (`dose_response_curve_fit`, `choose_two_group_test`, `publication_figure_checklist`) provide parameterised scientific analysis checklists.

Single-call `resources/read` syntax:

```json
{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"sheet://data-1"}}
```

## Structured Error Payloads

Tool execution failures set `result.isError: true` and emit a structured JSON object matching `StructuredErrorPayload`: `code`, `category`, `message`, `retryable`, `remediation`, and `details` (`command_batch` adds `failed_index`, `failed_command_id`, and `error`). For the full catalog of `ErrorCode` variants, see [Control Protocol & Batch Reference](control-and-batch.md#error-codes).
