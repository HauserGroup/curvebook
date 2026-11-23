# Control Protocol, Batch Format & Engine Errors

CurveBook supports three scriptable interfaces alongside MCP: a loopback JSON-lines `--control` channel attached to a desktop window, standalone `run` command scripts, and `curvebook.batch.v1` manifests. Every engine command identifier follows the stability contract documented in [Command IDs](../../command-ids.md).

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
{"id":1,"ok":true,"result":{"command_id":"sheet.createData","mutated":true,"message":"created data table sheet","payload":{"sheet_id":"data-1","title":"Data 1","kind":"xy","info_sheet_id":"info-1","graph_sheet_id":"graph-1"}},"error":null}
```

Built-in control methods: `"auth"`, `"ping"`, `"command.list"`, `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"doc.export"`, `"render.svg"`, `"render.pdf"`, `"render.preview"`, `"ui.state"` (alias `"state.snapshot"`), `"app.quit"`, the interactive `ui.*` GUI actions (`ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, `ui.switch_tab`, `ui.answer_dialog`), plus any registered engine or `ui.*` command ID passed directly as `"method"`/`"op"`.

When started with `--gui-only`, direct engine commands (`command.run`, `command.batch`, `engine.execute`), `doc.inspect`, `doc.export`, and `render.*` are rejected with `gui_only_violation` so external agents drive the desktop exclusively through the `ui.*` surface.

## Batch Manifest Format (`curvebook.batch.v1`) {#batch-manifest}

A `curvebook.batch.v1` manifest is a deterministic JSON document validated strictly against the JSON Schema (`additionalProperties: false`) published in the [Formats & Schemas Registry](../registry/formats.html#schema-batch-v1) before any operation runs. Execute a manifest with `curvebook-cli batch <manifest> [--workdir <dir>] [--json]`.

Single-call manifest structure (top-level keys):

```json
{"schema":"curvebook.batch.v1","project":"in.curvebook","inputs":[{"csv":"assay.csv","sheet":"data-2","kind":"xy"}],"commands":[{"cmd":"fit.run","id":"step-1","params":{"source_sheet_id":"data-2","sheet_id":"fit-1","equation_id":"log_agonist_vs_response_4pl"}}],"exports":[{"out":"out.curvebook","format":"curvebook"},{"sheet":"graph-1","out":"out.svg","format":"svg"}]}
```

- `schema`: Required constant `"curvebook.batch.v1"`.
- `project` / `template`: Mutually exclusive optional source project path or `.cbtemplate` template path (omit both to start from a blank session).
- `inputs`: Optional ordered list of delimited imports using canonical keys (`path`/`file` or inline `csv`/`tsv`; optional `sheet`, `kind`, `subcolumn_mode`, `delimiter`, and `decimal_separator`).
- `commands`: Optional ordered list of command steps, each naming the command via `cmd` (accepted aliases: `command`, `method`), with optional `id` and canonical `snake_case` `params` object.
- `exports`: Optional ordered list of project saves or table/figure exports using canonical keys (`out`/`path`; optional `format`, `sheet`, `dpi`, `pretty`, `tab_index`, `pdf_version`, `ttf_font`). Pass `format: "curvebook"` or `"curvebook.d"` to write the resulting project archive or unpacked directory.

## `run` Script Format

`curvebook-cli run <script> [--workdir <dir>] [--json]` accepts either newline-delimited JSON objects (NDJSON) or a JSON array of command step objects (`{"cmd":"...","id":"...","params":{...}}`, accepting `cmd`, `command`, or `method`). Passing `-` reads the script from `stdin`.

## Engine Error Codes and Categories {#error-codes}

Engine, MCP, and CLI errors carry stable machine-readable `ErrorCode` variants classified into seven `ErrorCategory` values (`input`, `state`, `format`, `io`, `policy`, `compute`, `internal`) and an explicit `retryable` boolean flag (`StructuredErrorPayload` / `StructuredCliError`). See the complete generated `ErrorCode` table in the [Tables & Error Codes Registry](../registry/tables.html#error-codes).

## Command ID Stability Rules

Command IDs used in scripts, batch manifests, and MCP `command_run` calls remain stable across minor releases. See [Command IDs](../../command-ids.md) for naming, command aliases, and canonical `snake_case` parameter schema compatibility guarantees.
