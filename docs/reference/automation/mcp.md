# MCP server reference

<!-- Audience: Tier 2 Technical Reference — JSON-RPC wire framing, attachment flags, tool annotations, resource URIs, and error payloads belong here; keep ../../mcp-for-beginners.md concise for scientists. -->

CurveBook's MCP server lets an MCP client, such as an AI assistant, run calculations, build graphs, inspect projects, and automate the user interface through the Model Context Protocol (MCP, JSON-RPC 2.0). The server communicates over stdio. It can work on files without a desktop window, or attach to a running desktop window that was started with `--control <port>`.

For client setup snippets and checks, see [MCP setup for beginners](../../mcp-for-beginners.md). The generated [MCP Tools & Surface Registry](../registry/mcp-tools.html) and [Engine Command Registry](../registry/commands.html) contain the complete tool schemas, annotations, prompts, and resources.

## Transports and wire framing

`curvebook-cli mcp` reads and writes JSON-RPC 2.0 messages on standard input and standard output. It accepts two framing modes:

- NDJSON framing: One compact UTF-8 JSON object per line, ending with `\n`.
- Content-Length framing: A `Content-Length: <n>\r\n\r\n` header, followed by `<n>` bytes of UTF-8 JSON.

The server detects the framing mode of each incoming request automatically. A message larger than 16 MiB returns JSON-RPC error `-32600` (invalid request); the reader then discards up to 64 MiB of input to find the next message boundary. A batch request array must contain 1 to 256 elements, and an empty or larger batch also returns `-32600`. The client and server agree on protocol revision `2024-11-05` or `2025-03-26` during `initialize`.

The server supports these JSON-RPC methods: `initialize`, `shutdown`, `ping`, `notifications/initialized`, `notifications/cancelled`, `tools/list`, `tools/call`, `resources/list`, `resources/templates/list`, `resources/read`, `prompts/list`, and `prompts/get`. After `shutdown`, further operational requests return `-32600`. In stdio mode, which is single-threaded, `notifications/cancelled` cancels requests that are queued later in the same batch and signals operations that are already running in the process.

An example `initialize` request and its response:

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

In headless mode, pass `--workdir <dir>` to keep all reads, saves, exports, and links to external delimited files inside `<dir>`. Any path that leads outside `<dir>` fails with `path_escape`.

Do not combine `--workdir` with `--control-port` (or `CURVEBOOK_CONTROL_PORT`). `--workdir` cannot restrict the file operations of a live-attached desktop process, so the combination exits immediately with code 2.

Live-attach mode uses these flags and environment variables:

- `--control-port <port>` / `CURVEBOOK_CONTROL_PORT`: The loopback TCP control port, from `1` to `65535`. The command-line flag takes precedence over the environment variable.
- `--control-token <token>` / `CURVEBOOK_CONTROL_TOKEN`: The bearer token for authentication. Prefer the environment variable, or let the server find the token automatically in the `control-<port>.token` file that `curvebook --control <port>` writes for each port.
- `CURVEBOOK_CONFIG_DIR`: Changes the configuration folder in which the server looks for `control-<port>.token`.
- `CURVEBOOK_CONTROL_IO_TIMEOUT_MS`: The read and write timeout for the loopback TCP connection, in milliseconds (`100` to `600000`). By default, the timeout is 30 s for standard requests and 120 s for heavy commands (`doc.export`, `command.batch`, `engine.execute`, imports, and renders). Opening the TCP connection always uses a fixed 2 s timeout.

## Core tools and dynamic toolsets

When the server starts in normal mode, `tools/list` immediately offers the core MCP tools: `doc_inspect`, `open_file`, `save_file`, `command_list`, `command_run`, `command_batch`, `render_preview`, `export`, `list_toolsets`, and `enable_toolset`. Each `command_batch` call accepts up to 1,000 command steps. To load the engine tools for one command group, or to switch the `gui` toolset on or off, call `list_toolsets` and `enable_toolset`. The server then sends `notifications/tools/list_changed`.

Every tool carries the standard MCP annotations, which CurveBook derives from the effects of the underlying command:

- `readOnlyHint`: `true` when the tool leaves documents, UI state, toolsets, files, and processes unchanged.
- `destructiveHint`: `true` when the tool may discard data or overwrite files on disk.
- `idempotentHint`: `true` when repeating the call with identical parameters produces the same state.
- `openWorldHint`: `true` when the tool reads or writes files or launches a process, and `false` for tools that work only in memory. CurveBook never makes outbound network requests.

An example `tools/call` request for `doc_inspect` and its response:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"doc_inspect","arguments":{"include_cells":false}}}
```

```json
{"id":2,"jsonrpc":"2.0","result":{"content":[{"text":"{\"analyses\":[],\"dirty\":false,\"file_path\":null,\"graphs\":[],\"layouts\":[],\"schema_version\":9,\"sheets\":{\"data\":[],\"info\":[]},\"title\":\"Untitled Project\",\"total_sheet_count\":0,\"variables\":{\"result_scalars\":[]}}","type":"text"}],"isError":false,"structuredContent":{"analyses":[],"dirty":false,"file_path":null,"graphs":[],"layouts":[],"schema_version":9,"sheets":{"data":[],"info":[]},"title":"Untitled Project","total_sheet_count":0,"variables":{"result_scalars":[]}}}}
```

In `--control-port` live-attach mode, `doc_inspect` without a `sheet_id` returns `mode: "live_attach"`, `control_port`, a `snapshot` of the live desktop, and the project summary fields. This response ignores `include_cells`. With a `sheet_id`, `doc_inspect` returns the details of that sheet, including optional `include_cells` previews, together with `mode` and `control_port`.

The [MCP Tools & Surface Registry](../registry/mcp-tools.html) lists the exact schemas, annotations, and request and response examples for every core tool, every command-group toolset, and every `gui` tool.

## Files: open, save, and export

- `open_file` opens or imports `.curvebook`, `.curvebook.d`, `.cbtemplate`, `.pzfx files`, `.prism files`, `.xlsx`, `.csv`, and `.tsv`. By default it replaces the open project. Pass `append: true` to merge the incoming sheets into the open project instead; this is the same merge as **File › Merge Project…**.
- `save_file` writes `.curvebook`, `.curvebook.d`, or `.cbtemplate`. It can also write a data-only `.pzfx` file (`format: "pzfx"`, which requires `path`). Writing a `.pzfx` file does not change the project's own file path or its unsaved-changes state. `.prism files` are import-only at Beta.
- `export` writes a sheet or the whole project as `svg`, `pdf`, `emf`, `png`, `tiff`, `csv`, `tsv`, or `json`, either to disk or inline in the response. It offers the same data-only `.pzfx` option as `save_file`.

A `.pzfx` export contains only data tables and info sheets; analyses, graphs, and layouts stay in the `.curvebook` project. The export follows the structure of real `.pzfx files`, but it has not been tested in other applications. A request to write a `.prism` file fails with `unsupported_format`.

## Resources and prompts

- Resources (`resources/list`, `resources/templates/list`, `resources/read`): The server offers a `sheet://<id>` resource (`application/json`) for each project sheet, a `result://<id>` resource (`application/json`) for each computed results sheet, and the URI templates `sheet://{id}` and `result://{id}`.
  - To keep responses small, `sheet://` URIs accept `?max_rows=N`, `?tab_id=<id>`, and `?detail=summary|full`.
  - `result://` URIs accept `?include_curves=false` or `?detail=summary`, which leave out the dense arrays of fitted-curve points.
- Prompts (`prompts/list`, `prompts/get`): The built-in prompts `dose_response_curve_fit`, `choose_two_group_test`, and `publication_figure_checklist` provide structured scientific checklists.

An example `resources/read` request:

```json
{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"sheet://data-1"}}
```

## Structured error payloads

When a tool call fails, the server sets `result.isError: true` and returns a structured JSON error object with `code`, `category`, `message`, `retryable`, `remediation`, and `details`. A failure inside `command_batch` also includes `failed_index`, `failed_command_id`, and `error`. For the full list of error codes, see the [control protocol and batch reference](control-and-batch.md#error-codes).
