# Live TCP Control Protocol (`--control`)

When launched with `lumencraft --control <port>`, the desktop application binds strictly to loopback (`127.0.0.1:<port>`) and serves a line-delimited JSON (`JSONL`) automation channel (`crates/ui-egui/src/control_io.rs`) backed by the same typed command registries as `lumencraft-cli` and `lumencraft-mcp` ([`commands.md`](commands.md), [`mcp.md`](mcp.md), [`STATUS.md`](STATUS.md)).

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

Supported built-in methods today are `"auth"`, `"ping"` (includes `"gui_only"`), `"command.list"` (all registered engine commands in [`commands.md`](commands.md) plus all synced `UI_COMMANDS` in `crates/mcp/src/ui_catalog.rs`), `"command.run"`, `"command.batch"`, `"engine.execute"`, `"doc.inspect"`, `"doc.export"`, `"render.svg"`, `"render.pdf"`, `"render.preview"`, `"ui.state"` (alias `"state.snapshot"`, includes `"gui_only"`), `"app.quit"`, the 8 interactive GUI actions (`"ui.inspect"`, `"ui.screenshot"`, `"ui.pointer_gesture"`, `"ui.type_text"`, `"ui.press_key"`, `"ui.invoke_menu"`, `"ui.switch_tab"`, `"ui.answer_dialog"`), or any registered engine / `ui.*` command ID (`"ui.canvas.preflight"`, `"ui.event.dispatch"`, etc.) passed directly as `"method"`/`"op"` (`MAX_CONTROL_LINE_BYTES = 16 MiB` in `crates/ui-egui/src/control_io.rs`).

`render.pdf` accepts:
- `sheet_id` / `sheetId`: optional `SheetId` (defaults to the active or first graph/layout sheet).
- `all`: optional `bool` (default `false`); when `true`, renders all graph and layout sheets into a multi-page PDF.
- `color_space`: optional `"srgb"` | `"rgb"` | `"cmyk"` | `"grayscale"` | `"gray"` (default `"srgb"`).
- `path`: optional host filesystem destination path; when non-empty outside `--gui-only`, writes the rendered PDF bytes to `path` in addition to returning `data_base64`, `sheet_id`, `sheet_ids`, `page_count`, and `bytes_len`. Under `--gui-only`, non-empty `path` is rejected with `ErrorCode::GuiOnlyViolation`.

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

## Interactive `ui.*` Actions & `--gui-only` Lock (`R8.6`)

Implemented in `crates/ui-egui/src/{control_io.rs, gui_actions.rs, inspect.rs}` and `crates/engine/src/lib.rs` ([`gui-workflows.md`](gui-workflows.md)):

### 1. Strict `--gui-only` Enforcement (`Caller::Control` vs. `Caller::Ui`)
- Passing `--gui-only` on launch (`lumencraft --control <port> --gui-only` or via `LUMENCRAFT_CONTROL_PORT`) configures every `DesktopHostRuntime` window with `GuiOnlyPolicy::Strict` (`app.set_gui_only(true)`).
- **Direct domain commands (`Caller::Control`):** `command.run`, `command.batch`, `engine.execute`, and raw domain command IDs run through `app.execute_control_command` (`Caller::Control`). Under `GuiOnlyPolicy::Strict`, `Session::execute_with_caller` blocks any command called by `Caller::Control` (as well as `Caller::Mcp` or `Caller::Cli`) unless it is read-only without `Effects::READS_FILESYSTEM` (or `table.convertArchetype` with `preview_only` / `previewOnly` / `preview` / `dry_run` / `dryRun: true`), returning `ErrorCode::GuiOnlyViolation` (`"gui_only_violation"`, `ErrorCategory::Policy`).
- **Direct `ui.*` / menu bypasses:** Under `--gui-only`, `dispatch_command_by_id` in `control_io.rs` rejects direct `ui.*` commands, `"app.about"`, and `"file.print"` with `ErrorCode::GuiOnlyViolation`, **except** the 8 interactive GUI JSON-lines actions below and the read-only `"ui.canvas.preflight"` check.
- **Internal UI execution (`Caller::Ui`):** Visible menus, shortcuts, palette actions, and modal commit handlers inside `UiApp` execute via `Caller::Ui` (`app.execute_command` / `dispatch_ui_command_as_ui`), remaining permitted so human-equivalent UI actions drive the live `Session`.
- **Allowed read-only RPCs under `--gui-only`:** `"ping"`, `"doc.inspect"`, `"ui.state"` (alias `"state.snapshot"`), `"command.list"`, `"ui.inspect"`, `"ui.canvas.preflight"`, and in-memory `"ui.screenshot"` (without `path`).
- **Disk-write restriction (`ui.screenshot` & `render.pdf`):** Under `--gui-only`, `ui.screenshot` returns `data_base64` PNG bytes in memory (`path` is `null`); supplying a non-empty `path` to `"ui.screenshot"` or `"render.pdf"` fails with `ErrorCode::GuiOnlyViolation`.

### 2. The 8 `ui.*` Interactive GUI JSON-Lines Actions
| Method | Purpose & Behavior (`crates/ui-egui/src/gui_actions.rs`) |
|---|---|
| `"ui.inspect"` | Returns a scoped [`UiTreeSnapshot`](../crates/ui-egui/src/inspect.rs) (`scope`: `all`/`workspace`, `menus`, `toolbar`, `navigator`, `tabs`, `canvas`, `grid`, `dialog`; `include_bounds`, `sheet_id`), including widget `bounds`, `enabled` / `disabled_reason` predicates, `grid_selection`, `brushed_points`, visible `grid_cells` (`grid.cell.*` and XY/Survival `grid.xcell.*`), and canvas hit targets. |
| `"ui.screenshot"` | Renders an in-memory PNG (`data_base64`, `bytes_len`, `width_px`, `height_px`, `dpi`) for `target` (`full_window`/`full`/`window`/`workspace`/`viewport`, `canvas`/`main`, `dialog`/`modal`; `64..=4096` px, default `1024x640`; `72..=600` dpi, default `144`). Rejects non-empty `path` under `--gui-only` with `ErrorCode::GuiOnlyViolation`. |
| `"ui.pointer_gesture"` | Dispatches `click`, `double_click`, `right_click`, `drag`, `hover`, `scroll`, or `wheel` on a `widget_id` (checking `node.enabled`) or viewport `(x, y)` coordinates, including `power.curve_hover`, Shift/Meta multi-row grid selection, graph point brushing, and layout panel drags. |
| `"ui.type_text"` | Focuses an optional `widget_id` and enters UTF-8 text or multi-cell TSV payloads into active `DataSheet` cells (`grid.cell.*` / `grid.xcell.*`), `InfoSheet` notes, or modal inputs. |
| `"ui.press_key"` | Dispatches key presses and shortcut combos (`Enter` dialog confirm/next, `Escape` cancel, `Cmd/Ctrl+T` on `ResultSheet` to open `EditAnalysisParameters`, `ShortcutRegistry` combos, or `UiInputEvent::KeyDown`). |
| `"ui.invoke_menu"` | Invokes a visible menu bar item by `command_id` or `path` (string or string array), returning `ErrorCode::Conflict` if disabled. |
| `"ui.switch_tab"` | Switches navigator sheets (`sheet_id` / `nav.sheet.*`), `EditAnalysisParameters` tabs (guarded by `evaluate_edit_params_tab`), `PowerWizardStage` tabs (`can_advance`), or `ResultSheet` sub-tabs. |
| `"ui.answer_dialog"` | Patches active modal/wizard `fields` / `values` / `patches` and/or triggers footer buttons (`ok`, `cancel`, `apply`, `back`, `next`, `create`, `preview` via `evaluate_dialog_button`), extracting tabular `ResultSheet` tabs via `sheet.extractResultTab` when invoking `AnalyzeSheet` from a `ResultSheet`. |

## Security Invariants

- Never binds to `0.0.0.0` or `::`; loopback (`127.0.0.1` / `::1` via `validate_loopback_addr`) only (`cargo xtask telemetry`).
- Disabled by default unless `--control <port>` (`1..=65535`) or `LUMENCRAFT_CONTROL_PORT` is explicitly set.
- Requires bearer-token authentication (`ControlAuth`, via `--control-token` / `LUMENCRAFT_CONTROL_TOKEN` or an auto-generated session token) supplied via `"auth"` handshake or per-request `"token"`.
- Rejects HTTP request lines (`403 Forbidden`) and bounds line framing (`MAX_CONTROL_LINE_BYTES = 16 MiB`, 2s TCP idle timeout).
