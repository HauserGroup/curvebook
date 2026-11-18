# Control Protocol, Batch Format & Engine Errors

CurveBook supports three scriptable interfaces alongside MCP: a loopback JSON-lines `--control` channel attached to a desktop window, standalone `run` command scripts, and `curvebook.batch.v1` manifests. Every engine command identifier follows the stability contract documented in [Command IDs](../contribute/command-ids.html).

## Loopback Desktop Control Protocol (`--control`)

Launch the desktop GUI with `--control <port>` (optionally paired with `--gui-only` or `--control-token <token>`) to bind a loopback TCP JSON-lines listener on `127.0.0.1:<port>`. Non-loopback bindings are never opened.

- **Framing:** One UTF-8 JSON object per line terminated by `\n`, bounded by `MAX_CONTROL_LINE_BYTES` (`16 MiB`).
- **Authentication:** Supply the bearer token via `"auth"` handshake or per-request `"token"` field (`--control-token`, `CURVEBOOK_CONTROL_TOKEN`, or the `control-<port>.token` file).
- **Dispatch key:** Requests accept `"method"` or its alias `"op"`.

Single-call request and response syntax:

```json
{"id":1,"op":"command.run","command":"sheet.createData","params":{"kind":"xy","replicates":3,"title":"Data 1"}}
```

```json
{"id":1,"ok":true,"result":{"created":["data-1"]},"error":null}
```

Built-in control methods: `"auth"`, `"ping"`, `"command.list"`, `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"doc.export"`, `"render.svg"`, `"render.pdf"`, `"render.preview"`, `"ui.state"` (alias `"state.snapshot"`), `"app.quit"`, the interactive `ui.*` GUI actions (`ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, `ui.switch_tab`, `ui.answer_dialog`), plus any registered engine or `ui.*` command ID passed directly as `"method"`/`"op"`.

When started with `--gui-only`, direct engine commands (`command.run`, `command.batch`, `engine.execute`), `doc.inspect`, `doc.export`, and `render.*` are rejected with `gui_only_violation` so external agents drive the desktop exclusively through the `ui.*` surface.

## Batch Manifest Format (`curvebook.batch.v1`) {#batch-manifest}

A `curvebook.batch.v1` manifest is a deterministic JSON document validated against the JSON Schema published in the [Formats & Schemas Registry](../registry/formats.html#schema-batch-v1) before any operation runs. Execute a manifest with `curvebook-cli batch <manifest> [--workdir <dir>] [--json]`.

Single-call manifest structure (all top-level keys):

```json
{"schema":"curvebook.batch.v1","input":"in.curvebook","output":"out.curvebook","atomic":true,"operations":[{"command_id":"sheet.createData","params":{"kind":"xy"}}],"exports":[{"sheet_id":"graph-1","path":"out.svg","format":"svg"}]}
```

- `schema`: Must equal `"curvebook.batch.v1"`.
- `input` / `output`: Optional source project/interchange path and destination `.curvebook` / `.curvebook.d` / `.cbtemplate` output path.
- `atomic`: When `true`, rolls back in-memory session mutations if any operation step fails.
- `operations`: Ordered list of `{command_id, params}` objects (or supported `command.batch` steps) validated against [Engine Command Registry](../registry/commands.html).
- `exports`: Optional list of post-execution table/figure export targets (`sheet_id`, `path`, `format`, `dpi`, `pdf_version`, `ttf_font`).

## `run` Script Format

`curvebook-cli run <script> [--workdir <dir>] [--json]` accepts either newline-delimited JSON objects (NDJSON) or a JSON array of command step objects (`{"command_id":"...","params":{...}}`). Passing `-` reads the script from `stdin`.

## Engine Error Codes and Categories {#error-codes}

Engine, MCP, and CLI errors carry stable machine-readable `ErrorCode` variants classified into categories (`bad_input`, `not_found`, `policy_violation`, `io`, `domain_math`, `internal`) and an explicit `recoverable` flag. See the complete generated `ErrorCode` table in the [Tables & Error Codes Registry](../registry/tables.html#error-codes).

## Command ID Stability Rules

Command IDs used in scripts, batch manifests, and MCP `command_run` calls remain stable across minor releases. See [Command IDs](../contribute/command-ids.html) for naming, deprecation aliases, and parameter schema compatibility guarantees.
