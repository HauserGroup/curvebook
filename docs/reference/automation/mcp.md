# MCP server reference

CurveBook exposes calculation, graphing, inspection, and UI automation over the Model Context Protocol (MCP, JSON-RPC 2.0). The stdio server works with headless files or attaches to a live desktop window started with `--control <port>`.

For client setup snippets and checks, see [MCP setup for beginners](../../mcp-for-beginners.md). For complete generated tool schemas, annotations, prompts, and resources, see the [MCP Tools & Surface Registry](../registry/mcp-tools.html) and [Engine Command Registry](../registry/commands.html).

## Transports and wire framing

`curvebook-cli mcp` reads and writes JSON-RPC 2.0 messages on standard input and standard output:

- **NDJSON framing:** One compact UTF-8 JSON object per line, terminated by `\n`.
- **Content-Length framing:** `Content-Length: <n>\r\n\r\n` header followed by `<n>` UTF-8 JSON bytes.

The server auto-detects both framing modes on each incoming request. Payloads over 16 MiB return JSON-RPC error `-32600` (invalid request), and the reader drains up to 64 MiB to recover framing boundaries. Batch request arrays accept 1 to 256 elements; empty or oversized batches return `-32600`. Protocol revisions `2024-11-05` and `2025-03-26` are negotiated during `initialize`.

Supported JSON-RPC methods are `initialize`, `shutdown`, `ping`, `notifications/initialized`, `notifications/cancelled`, `tools/list`, `tools/call`, `resources/list`, `resources/templates/list`, `resources/read`, `prompts/list`, and `prompts/get`. After `shutdown`, operational requests return `-32600`. In single-threaded stdio mode, `notifications/cancelled` cancels queued requests later in the same batch and signals active in-process operations.

Single-call `initialize` request and response syntax:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"client","version":"1.0.0"}}}
```

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":true},"resources":{"subscribe":false,"listChanged":false},"prompts":{"listChanged":false}},"serverInfo":{"name":"curvebook","version":"<version>"},"instructions":"..."}}
```

## Attachment modes and filesystem policy

| Mode | Command syntax | Session backing |
|---|---|---|
| Headless stdio | `curvebook-cli mcp [--workdir <dir>]` | In-process engine session |
| Live-attach stdio | `curvebook-cli mcp --control-port <port>` | Attached desktop on `127.0.0.1:<port>` |

Pass `--workdir <dir>` in headless mode to confine all reads, saves, exports, and external delimited-file links inside `<dir>`. Escapes outside `<dir>` fail with `path_escape`.

Do not combine `--workdir` with `--control-port` (or `CURVEBOOK_CONTROL_PORT`). Combining them exits immediately with code 2 because `--workdir` cannot confine file operations inside a live-attached desktop process.

Environment variables and flags for live-attach mode:

- `--control-port <port>` / `CURVEBOOK_CONTROL_PORT`: Loopback TCP control port from `1` to `65535`; the CLI flag overrides the environment variable.
- `--control-token <token>` / `CURVEBOOK_CONTROL_TOKEN`: Bearer authentication token. Prefer the environment variable or automatic discovery from the per-port `control-<port>.token` file written by `curvebook --control <port>`.
- `CURVEBOOK_CONFIG_DIR`: Overrides the configuration folder searched for `control-<port>.token`.
- `CURVEBOOK_CONTROL_IO_TIMEOUT_MS`: Loopback TCP read/write timeout in milliseconds (`100` to `600000`). Defaults are 30 s for standard requests and 120 s for heavy commands (`doc.export`, `command.batch`, `engine.execute`, imports, and renders); TCP connect uses a fixed 2 s timeout.

## Core tools and dynamic toolsets

On startup in normal mode, `tools/list` exposes the core MCP tools immediately: `doc_inspect`, `open_file`, `save_file`, `command_list`, `command_run`, `command_batch`, `render_preview`, `export`, `list_toolsets`, and `enable_toolset`. Each `command_batch` call accepts up to 1,024 command steps. Call `list_toolsets` and `enable_toolset` to load per-group engine tools or toggle the `gui` toolset dynamically (which emits `notifications/tools/list_changed`).

Every tool carries standard MCP annotations derived from its command effects:

- `readOnlyHint`: `true` when the tool leaves documents, UI state, toolsets, files, and processes unchanged.
- `destructiveHint`: `true` when the tool may discard data or overwrite files on disk.
- `idempotentHint`: `true` when repeating the call with identical parameters yields the same state.
- `openWorldHint`: `true` when the tool reads or writes files or launches a process, and `false` for pure in-memory tools. CurveBook never makes outbound network requests.

Single-call `tools/call` request and response syntax (`doc_inspect`):

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"doc_inspect","arguments":{"include_cells":false}}}
```

```json
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"title\":\"Untitled Project\",\"schema_version\":7,\"file_path\":null,\"dirty\":false,\"total_sheet_count\":0,\"active_sheet_id\":null,\"family_anchor_sheet_id\":null,\"sheets\":{\"data\":[],\"info\":[],\"analyses\":[],\"graphs\":[],\"layouts\":[]},\"variables\":{\"info_constants\":[],\"result_scalars\":[]},\"hooks\":[],\"analyses\":[],\"graphs\":[],\"layouts\":[]}"}],"isError":false,"structuredContent":{"title":"Untitled Project","schema_version":7,"file_path":null,"dirty":false,"total_sheet_count":0,"active_sheet_id":null,"family_anchor_sheet_id":null,"sheets":{"data":[],"info":[],"analyses":[],"graphs":[],"layouts":[]},"variables":{"info_constants":[],"result_scalars":[]},"hooks":[],"analyses":[],"graphs":[],"layouts":[]}}}
```

In `--control-port` live-attach mode, calling `doc_inspect` without `sheet_id` returns `mode: "live_attach"`, `control_port`, the live desktop `snapshot`, and project summary fields regardless of `include_cells`. Supplying `sheet_id` returns that sheet's detail (with optional `include_cells` previews) alongside `mode` and `control_port`.

Exact schemas, annotations, and request/response examples for every core tool, dynamic group toolset, and `gui` tool appear in the [MCP Tools & Surface Registry](../registry/mcp-tools.html).

## Resources and prompts

- **Resources (`resources/list`, `resources/templates/list`, `resources/read`):** Exposes `sheet://<id>` (`application/json`) for each project sheet, `result://<id>` (`application/json`) for computed results sheets, and URI templates (`sheet://{id}`, `result://{id}`).
- **Prompts (`prompts/list`, `prompts/get`):** Built-in prompts (`dose_response_curve_fit`, `choose_two_group_test`, `publication_figure_checklist`) provide structured scientific checklists.

Single-call `resources/read` syntax:

```json
{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"sheet://data-1"}}
```

## Structured error payloads

Failed tool calls set `result.isError: true` and return a structured JSON error payload with `code`, `category`, `message`, `retryable`, `remediation`, and `details`. Failures inside `command_batch` also include `failed_index`, `failed_command_id`, and `error`. For the full catalog of error codes, see [Control protocol & batch reference](control-and-batch.md#error-codes).
