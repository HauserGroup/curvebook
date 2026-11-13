# Live TCP Control Protocol (`--control`)

When launched with `lumencraft --control <port>`, the desktop application binds strictly to loopback (`127.0.0.1:<port>`) and serves a line-delimited JSON (`JSONL`) automation channel backed by the same command registry as `lumencraft-cli` and `lumencraft-mcp`.

## Framing

Each client request is a single UTF-8 JSON object terminated by `\n`:

```json
{"id": 1, "op": "command.list"}
{"id": 2, "op": "command.run", "command": "sheet.createData", "params": {"kind": "xy", "replicates": 3, "title": "Dose Response"}}
{"id": 3, "op": "doc.inspect"}
{"id": 4, "op": "render.svg", "sheetId": "graph-1"}
```

Each response is a single UTF-8 JSON object terminated by `\n`:

```json
{"id": 1, "ok": true, "result": {"commands": [...]}}
```

Errors return `ok: false` without disconnecting the socket or crashing the session:

```json
{"id": 99, "ok": false, "error": {"code": "unknown_command", "message": "unknown command `foo.bar`"}}
```

## Security Invariants

- Never binds to `0.0.0.0` or `::`; loopback (`127.0.0.1`) only.
- Disabled by default unless `--control <port>` (`1..=65535`) is explicitly passed on the command line.
