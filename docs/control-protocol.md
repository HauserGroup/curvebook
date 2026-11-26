# Live TCP Control Protocol (`--control`)

> **Who this page is for.** This is the contributor specification, and its strings are asserted by tests. The published [control protocol and batch reference](reference/automation/control-and-batch.md) is the user-facing page; the documentation site links there in place of this file.

When launched with `curvebook --control <port>` (or with `CURVEBOOK_CONTROL_PORT=<port>`), the desktop application binds strictly to loopback (`127.0.0.1:<port>`) and serves a line-delimited JSON (`JSONL`) automation channel (`crates/ui-egui/src/control_io.rs`) backed by the same typed command registries as `curvebook-cli` and `curvebook-mcp` ([`commands.md`](commands.md), [`mcp.md`](mcp.md), [`STATUS.md`](STATUS.md)). By default both `--control <port>` and `CURVEBOOK_CONTROL_PORT=<port>` run headless without opening a GUI window; pass the opt-in `--window` flag (`curvebook --control <port> --window` or `CURVEBOOK_CONTROL_PORT=<port> curvebook --window`) to attach a visible `eframe`/`egui` desktop window sharing the same live `UiApp` / `Session` (`apps/curvebook/src/main.rs`; cannot be combined with `--headless-once`).

## Framing

Each client request is a single UTF-8 JSON object terminated by `\n` (bounded by `MAX_CONTROL_LINE_BYTES = 16 MiB`), using either `"method"` or `"op"` (`ControlRequest` aliases `"op"` to `"method"`):

```json
{"id": 1, "op": "command.list"}
{"id": 2, "op": "command.run", "command": "sheet.createData", "params": {"kind": "xy", "replicates": 3, "title": "Dose Response"}}
{"id": 3, "op": "command.batch", "params": {"atomic": true, "commands": [{"command_id": "sheet.reorder", "params": {"order": ["data-1"]}}]}}
{"id": 4, "op": "doc.inspect"}
{"id": 5, "op": "ui.state"}
{"id": 6, "op": "render.svg", "sheet_id": "graph-1"}
{"id": 7, "op": "render.pdf", "sheet_id": "graph-1", "color_space": "srgb"}
```

Supported built-in methods today are `"auth"`, `"ping"`, `"command.list"` (all registered engine commands in [`commands.md`](commands.md) plus all synced `UI_COMMANDS` in the generated `crates/mcp/src/ui_catalog.rs`), `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"doc.export"`, `"render.svg"`, `"render.pdf"`, `"render.preview"`, `"ui.state"` (alias `"state.snapshot"`), `"app.quit"`, the 7 interactive GUI actions (`"ui.inspect"`, `"ui.screenshot"`, `"ui.pointer_gesture"`, `"ui.type_text"`, `"ui.press_key"`, `"ui.invoke_menu"`, `"ui.switch_tab"`), or any registered engine / `ui.*` command ID (`"ui.canvas.preflight"`, `"ui.event.dispatch"`, etc.) passed directly as `"method"`/`"op"` (`MAX_CONTROL_LINE_BYTES = 16 MiB` in `crates/ui-egui/src/control_io.rs`).

`render.pdf` accepts:
- `sheet_id` / `sheetId`: optional `SheetId` (defaults to the active or first graph/layout sheet).
- `all`: optional `bool` (default `false`); when `true`, renders all graph and layout sheets into a multi-page PDF.
- `color_space`: optional `"srgb"` | `"rgb"` | `"cmyk"` | `"grayscale"` | `"gray"` (default `"srgb"`).
- `path`: optional host filesystem destination path; when non-empty, writes the rendered PDF bytes to `path` (subject to the session's `ExternalPathPolicy`) in addition to returning `data_base64`, `sheet_id`, `sheet_ids`, `page_count`, and `bytes_len`.

`doc.export` accepts:
- `format`: required `"svg"` | `"pdf"` | `"emf"` | `"png"` | `"tiff"` | `"csv"` | `"tsv"` | `"json"`.
- `sheet_id` / `sheetId`: optional `SheetId` (defaults to the active or first compatible sheet).
- `path` / `output_path`: optional host filesystem destination path; when non-empty, writes the exported bytes atomically to disk (subject to the session's `ExternalPathPolicy`).
- Format-specific options: `pdf_color_space` / `color_space`, `pdf_version`, `ttf_font` (optional TrueType font path/embedding for PDF), `all_graphs` / `all`, `svg_precision`, `width_px`, `height_px`, `dpi`, `tiff_compression`, `transparent_bg` (optional `bool`, default `false`; supported for `svg`, `png`, and `tiff` — passing `transparent_bg: true` for other formats such as `pdf` or `emf` returns `bad_input`), `pretty`, `tidy`, `tab_id` / `tabId`, and `csv_delimiter`.

Each response (`ControlResponse`) is a single UTF-8 JSON object terminated by `\n`:

```json
{"id": 1, "ok": true, "result": {"commands": [...]}, "error": null}
```

Errors on well-framed requests—including unknown methods, invalid JSON, non-UTF-8 lines (`<= MAX_CONTROL_LINE_BYTES`), auth failures, validation/command errors, caught handler panics, and oversized responses (returned as a bounded overflow error)—return `ok: false` with `"result": null`, `"error"` set to a human-readable string, and optional structured `"error_detail"` (`ErrorCode`, `category`, `retryable`, `remediation`, `details`) without disconnecting the socket or crashing the session:

```json
{"id": 99, "ok": false, "result": null, "error": "unknown command `foo.bar`"}
```

Intentional socket termination occurs only on:
- **Oversized frames (`> MAX_CONTROL_LINE_BYTES = 16 MiB`)**: sends a best-effort `ok: false` JSON error frame and closes the connection (`InvalidData`) rather than buffering unbounded input.
- **HTTP request lines** (`GET `, `POST `, etc.): replies `403 Forbidden` with `Connection: close` and drops the connection.
- **TCP idle timeout (`2s`)**: drops stalled TCP connections that do not send a complete `\n`-terminated frame within 2 seconds (no reply written).
- **`"app.quit"`**: sends the `"ok": true` reply and shuts down the control loop cleanly.

## Interactive `ui.*` Actions (`R8.6`)

Implemented in `crates/ui-egui/src/{control_io.rs, gui_actions.rs, inspect.rs}` ([`gui-workflows.md`](gui-workflows.md)):

These 7 JSON-lines actions form the **semantic agent harness** for live desktop inspection and widget automation: widget IDs come from `ui.inspect` nodes synthesized from state. Passing them does not show that a person can operate the drawn GUI; human-operable chrome is gated by `EguiHumanPathDriver` ([`R10`](ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion)).

| Method | Purpose & Behavior (`crates/ui-egui/src/gui_actions.rs`) |
|---|---|
| `"ui.inspect"` | Returns a scoped [`UiTreeSnapshot`](../crates/ui-egui/src/inspect.rs) (`scope`: `all`/`workspace`, `menus`, `toolbar`, `navigator`, `tabs`, `canvas`, `grid`, `dialog`; `include_bounds`, `sheet_id`), including widget `bounds`, `enabled` / `disabled_reason` predicates, `grid_selection`, `brushed_points`, visible `grid_cells` (`grid.cell.*` and XY/Survival `grid.xcell.*`), and canvas hit targets. |
| `"ui.screenshot"` | Renders an in-memory PNG (`data_base64`, `bytes_len`, `width_px`, `height_px`, `dpi`) for `target` (`full_window`/`full`/`window`/`workspace`/`viewport`, `canvas`/`main`, `dialog`/`modal`; `64..=4096` px, default `1024x640`; `72..=600` dpi, default `144`), with an optional `path` checked against the session's `ExternalPathPolicy`. |
| `"ui.pointer_gesture"` | Dispatches `click`, `double_click`, `right_click`, `drag`, `hover`, `scroll`, or `wheel` on a `widget_id` (checking `node.enabled`) or viewport `(x, y)` coordinates, including `power.curve_hover`, Shift/Meta multi-row grid selection, graph point brushing, and layout panel drags. |
| `"ui.type_text"` | Focuses an optional `widget_id` and enters UTF-8 text or multi-cell TSV payloads into active `DataSheet` cells (`grid.cell.*` / `grid.xcell.*`), `InfoSheet` notes, or modal inputs. |
| `"ui.press_key"` | Dispatches key presses and shortcut combos (`Enter` dialog confirm/next, `Escape` cancel, `Cmd/Ctrl+T` on `ResultSheet` to open `EditAnalysisParameters`, `ShortcutRegistry` combos, or `UiInputEvent::KeyDown`). |
| `"ui.invoke_menu"` | Invokes a visible menu bar item by `command_id` or `path` (string or string array), returning `ErrorCode::Conflict` if disabled. |
| `"ui.switch_tab"` | Switches navigator sheets (`sheet_id` / `nav.sheet.*`), `EditAnalysisParameters` tabs (guarded by `evaluate_edit_params_tab`), `PowerWizardStage` tabs (`can_advance`), or `ResultSheet` sub-tabs. |

## Security Invariants

- Never binds to `0.0.0.0` or `::`; loopback (`127.0.0.1` / `::1` via `validate_loopback_addr`) only (`cargo xtask telemetry`).
- Disabled by default unless `--control <port>` (`1..=65535`) or `CURVEBOOK_CONTROL_PORT` is explicitly set.
- Requires bearer-token authentication (`ControlAuth`, via `--control-token` / `CURVEBOOK_CONTROL_TOKEN` or an auto-generated session token) supplied via `"auth"` handshake or per-request `"token"`.
- Rejects HTTP request lines (`403 Forbidden`) and bounds line framing (`MAX_PRE_AUTH_CONTROL_LINE_BYTES = 64 KiB` prior to authentication, promoted to `MAX_CONTROL_LINE_BYTES = 16 MiB` after either an `"auth"` handshake or a valid per-request `"token"` frame on the connection, with a 2s TCP idle timeout; clients sending a first frame larger than 64 KiB must authenticate first via `"auth"`).
