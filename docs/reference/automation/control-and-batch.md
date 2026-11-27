# Control protocol, batch format, and engine errors

<!-- Audience: Tier 2 Technical Reference — loopback --control wire protocol, curvebook.batch.v1 manifest keys, run scripts, and engine error categories belong here; keep ../get-started.md and ../../mcp-for-beginners.md concise for scientists. -->

CurveBook supports three scriptable interfaces alongside MCP: a loopback JSON-lines `--control` channel attached to the desktop app, standalone `run` scripts, and `curvebook.batch.v1` batch manifests. For standard installation or AI assistant setup, start with [Install CurveBook](../get-started.md) or [MCP setup for beginners](../../mcp-for-beginners.md). Every engine command follows the stability contract in [Command IDs](../../command-ids.md).

## Loopback desktop control protocol (`--control`)

Launch the desktop app with `--control <port>` (and optionally `--window` or `CURVEBOOK_CONTROL_TOKEN`) to bind a loopback TCP JSON-lines listener on `127.0.0.1:<port>`. CurveBook never opens non-loopback network bindings.

- **Framing:** Send one UTF-8 JSON object per line terminated by `\n`, up to 16 MiB per line.
- **Authentication:** Provide the bearer token via an `"auth"` handshake or a per-request `"token"` field (`CURVEBOOK_CONTROL_TOKEN`, `--control-token`, or the `control-<port>.token` file).
- **Dispatch key:** Requests accept `"method"` or its alias `"op"`.

Single-call request and response syntax:

```json
{"id":1,"op":"command.run","command":"sheet.createData","params":{"kind":"xy","replicates":3,"title":"Data 1"}}
```

```json
{"id":1,"ok":true,"result":{"command_id":"sheet.createData","mutated":true,"message":"created data table sheet","payload":{"sheet_id":"data-1","title":"Data 1","kind":"xy","info_sheet_id":"info-1","graph_sheet_id":"graph-1"}},"error":null}
```

Built-in control methods include `"auth"`, `"ping"`, `"command.list"`, `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"doc.export"`, `"render.svg"`, `"render.pdf"`, `"render.preview"`, `"ui.state"` (alias `"state.snapshot"`), and `"app.quit"`. Interactive UI actions add `ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, and `ui.switch_tab`. You can also pass any registered engine or `ui.*` command ID directly as `"method"` or `"op"`.

When calling `doc.export`, optional `transparent_bg: true` works only for `svg`, `png`, and `tiff`. Passing `transparent_bg: true` for opaque formats such as `pdf` or `emf` fails with `bad_input`.

## Batch manifest format (`curvebook.batch.v1`) {#batch-manifest}

A `curvebook.batch.v1` manifest is a JSON document validated strictly against the schema (`additionalProperties: false`) in the [Formats & Schemas Registry](../registry/formats.html#schema-batch-v1) before anything runs. Run a manifest with `curvebook-cli batch <manifest> [--workdir <dir>] [--json]`.

Single-call manifest structure:

```json
{"schema":"curvebook.batch.v1","project":"in.curvebook","inputs":[{"csv":"assay.csv","sheet":"data-2","kind":"xy"}],"commands":[{"cmd":"fit.run","id":"step-1","params":{"source_sheet_id":"data-2","sheet_id":"fit-1","equation_id":"log_agonist_vs_response_4pl"}}],"exports":[{"out":"out.curvebook","format":"curvebook"},{"sheet":"graph-1","out":"out.svg","format":"svg"}]}
```

Top-level manifest keys:

- `schema`: Required constant `"curvebook.batch.v1"`.
- `project` / `template`: Optional source project or `.cbtemplate` path. These two keys are mutually exclusive; omit both to start from a blank session.
- `inputs`: Optional ordered list of delimited imports using `path` or `file` (or inline `csv` / `tsv`), plus optional `sheet`, `kind`, `subcolumn_mode`, `delimiter`, and `decimal_separator`.
- `commands`: Optional ordered list of steps, each naming `cmd` (aliases: `command`, `method`) with an optional `id` and canonical `snake_case` `params` object.
- `exports`: Optional ordered list of project saves or table/figure exports using `out` or `path`. Optional keys are `format`, `sheet`, `dpi`, `pretty`, `tab_index`, `pdf_version`, and `ttf_font`. Use `format: "curvebook"` or `"curvebook.d"` to save the resulting project.

## `run` script format

`curvebook-cli run <script> [--workdir <dir>] [--json]` accepts newline-delimited JSON objects (NDJSON) or a JSON array of command steps. Each step uses `{"cmd":"...","id":"...","params":{...}}` (accepting `cmd`, `command`, or `method`). Pass `-` to read the script from standard input.

## Engine error codes and categories {#error-codes}

Engine, MCP, and CLI failures return stable `snake_case` error codes across seven categories (`input`, `state`, `format`, `io`, `policy`, `compute`, `internal`) plus a `retryable` boolean flag. See the complete generated table in the [Tables & Error Codes Registry](../registry/tables.html#error-codes).

## Command ID stability rules

Command IDs used in scripts, batch manifests, and MCP `command_run` calls stay stable across minor releases. See [Command IDs](../../command-ids.md) for naming conventions, aliases, and `snake_case` parameter compatibility rules.
