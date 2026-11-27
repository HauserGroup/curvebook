# Control protocol, batch format, and engine errors

<!-- Audience: Tier 2 Technical Reference — loopback --control wire protocol, curvebook.batch.v1 manifest keys, run scripts, and engine error categories belong here; keep ../get-started.md and ../../mcp-for-beginners.md concise for scientists. -->

Besides MCP, CurveBook offers three ways to script its work:

- a `--control` channel, which sends JSON lines over a loopback connection to the running desktop app;
- standalone `run` scripts;
- `curvebook.batch.v1` batch manifests.

For a standard installation or to set up an AI assistant, start with [Install CurveBook](../get-started.md) or [MCP setup for beginners](../../mcp-for-beginners.md). Every engine command follows the stability rules in [Command IDs](../../command-ids.md).

## Loopback desktop control protocol (`--control`)

Start the desktop app with `--control <port>` to open a loopback TCP listener on `127.0.0.1:<port>` that accepts JSON lines. You can also add `--window` or set `CURVEBOOK_CONTROL_TOKEN`. CurveBook never listens on a non-loopback network address.

- Framing: Send one UTF-8 JSON object per line, ending with `\n`. Each line can be up to 16 MiB.
- Authentication: Supply the bearer token either in an `"auth"` handshake or in a `"token"` field on each request. The token comes from `CURVEBOOK_CONTROL_TOKEN`, `--control-token`, or the `control-<port>.token` file.
- Method name: Each request names its method in `"method"` or in the alias `"op"`.

An example request and its response:

```json
{"id":1,"op":"command.run","command":"sheet.createData","params":{"kind":"xy","replicates":3,"title":"Data 1"}}
```

```json
{"id":1,"ok":true,"result":{"command_id":"sheet.createData","mutated":true,"message":"created data table sheet","payload":{"sheet_id":"data-1","title":"Data 1","kind":"xy","info_sheet_id":"info-1","graph_sheet_id":"graph-1"}},"error":null}
```

The built-in control methods are `"auth"`, `"ping"`, `"command.list"`, `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"doc.export"`, `"render.svg"`, `"render.pdf"`, `"render.preview"`, `"ui.state"` (alias `"state.snapshot"`), and `"app.quit"`. For interactive UI actions, the protocol adds `ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, and `ui.switch_tab`. You can also pass the ID of any registered engine command or `ui.*` command directly as `"method"` or `"op"`.

The `doc.export` method accepts an optional `transparent_bg: true`, which works only for `svg`, `png`, and `tiff`. For opaque formats such as `pdf` or `emf`, `transparent_bg: true` fails with `bad_input`.

## Batch manifest format (`curvebook.batch.v1`) {#batch-manifest}

A `curvebook.batch.v1` manifest is a JSON document. Before anything runs, CurveBook validates it strictly against the schema (`additionalProperties: false`) in the [Formats & Schemas Registry](../registry/formats.html#schema-batch-v1). To run a manifest, use `curvebook-cli batch <manifest> [--workdir <dir>] [--json]`.

An example manifest:

```json
{"schema":"curvebook.batch.v1","project":"in.curvebook","inputs":[{"csv":"assay.csv","sheet":"data-2","kind":"xy"}],"commands":[{"cmd":"fit.run","id":"step-1","params":{"source_sheet_id":"data-2","sheet_id":"fit-1","equation_id":"log_agonist_vs_response_4pl"}}],"exports":[{"out":"out.curvebook","format":"curvebook"},{"sheet":"graph-1","out":"out.svg","format":"svg"}]}
```

The manifest has these top-level keys:

- `schema`: Required. Must be the constant `"curvebook.batch.v1"`.
- `project` / `template`: Optional. The path of a source project or `.cbtemplate` template. Use at most one of these two keys. If you omit both, the batch starts from a blank session.
- `inputs`: Optional. An ordered list of delimited-file imports. Each import names a file with `path` or `file`, or gives the data inline with `csv` or `tsv`. It can also set `sheet`, `kind`, `subcolumn_mode`, `delimiter`, and `decimal_separator`.
- `commands`: Optional. An ordered list of steps. Each step names its command in `cmd` (or the aliases `command` and `method`), and can include an `id` and a `params` object with `snake_case` keys.
- `exports`: Optional. An ordered list of project saves or table and figure exports. Each entry names its output file with `out` or `path`, and can also set `format`, `sheet`, `dpi`, `pretty`, `tab_index`, `pdf_version`, and `ttf_font`. To save the resulting project, use `format: "curvebook"` or `"curvebook.d"`.

## `run` script format

`curvebook-cli run <script> [--workdir <dir>] [--json]` reads a script of command steps, written either as newline-delimited JSON objects (NDJSON) or as a JSON array. Each step has the form `{"cmd":"...","id":"...","params":{...}}`, and the command can be named with `cmd`, `command`, or `method`. To read the script from standard input, pass `-` as the script name.

## Engine error codes and categories {#error-codes}

When the engine, the MCP server, or the CLI fails, it returns a stable `snake_case` error code and a `retryable` boolean flag. The codes fall into seven categories: `input`, `state`, `format`, `io`, `policy`, `compute`, and `internal`. The [Tables & Error Codes Registry](../registry/tables.html#error-codes) contains the complete generated table.

## Command ID stability rules

The command IDs that you use in scripts, batch manifests, and MCP `command_run` calls stay the same across minor releases. See [Command IDs](../../command-ids.md) for the naming conventions, aliases, and compatibility rules for `snake_case` parameters.
