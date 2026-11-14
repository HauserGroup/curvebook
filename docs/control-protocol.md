# Live TCP Control Protocol (`--control`)

When launched with `lumencraft --control <port>`, the desktop application binds strictly to loopback (`127.0.0.1:<port>`) and serves a line-delimited JSON (`JSONL`) automation channel backed by the same command registry as `lumencraft-cli` and `lumencraft-mcp`.

## Framing

Each client request is a single UTF-8 JSON object terminated by `\n`, using either `"method"` or `"op"` (`ControlRequest` aliases `"op"` to `"method"`):

```json
{"id": 1, "op": "command.list"}
{"id": 2, "op": "command.run", "command": "sheet.createData", "params": {"kind": "xy", "replicates": 3, "title": "Dose Response"}}
{"id": 3, "op": "command.batch", "params": {"atomic": true, "commands": [{"command_id": "sheet.reorder", "params": {"order": ["data-1"]}}]}}
{"id": 4, "op": "doc.inspect"}
{"id": 5, "op": "ui.state"}
{"id": 6, "op": "render.svg", "sheet_id": "graph-1"}
{"id": 7, "op": "render.pdf", "sheet_id": "graph-1", "color_space": "srgb"}
```

Supported built-in methods are `"ping"`, `"command.list"` (`75` engine commands + `17` `UI_COMMANDS`), `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"render.svg"`, `"render.pdf"`, `"ui.state"` (alias `"state.snapshot"`), `"app.quit"`, or any registered engine / `ui.*` command ID passed directly as `"method"`/`"op"`.

Each response (`ControlResponse`) is a single UTF-8 JSON object terminated by `\n`:

```json
{"id": 1, "ok": true, "result": {"commands": [...], "engine_count": 75, "ui_count": 17}, "error": null}
```

Errors return `ok: false` with `"result": null` and `"error"` set to a plain string without disconnecting the socket or crashing the session:

```json
{"id": 99, "ok": false, "result": null, "error": "unknown command `foo.bar`"}
```

## Security Invariants

- Never binds to `0.0.0.0` or `::`; loopback (`127.0.0.1`) only.
- Disabled by default unless `--control <port>` (`1..=65535`) is explicitly passed on the command line.
