# Security & Engineering Audit — SEC-21 (CLI & MCP Server)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-21` — `apps/curvebook-cli/` (`1` manifest + `9` `.rs` files) and `crates/mcp/` (`1` manifest + `10` `.rs` files) (`21` files, `13,011` LOC)
- **Date:** `2026-11-22`
- **Summary:** `S1=0`, `S2=4`, `S3=3`, `S4=1` (Total: `8` findings)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Checked | Findings |
| --- | ---: | --- | --- | --- |
| `apps/curvebook-cli/Cargo.toml` | 33 | `1..=33` | `[lints] workspace = true`, zero HTTP/network/telemetry crates, no `chrono`, minimal workspace dependencies | Clean — inherits workspace lints cleanly with offline-only dependencies |
| `crates/mcp/Cargo.toml` | 31 | `1..=31` | `[lints] workspace = true`, zero HTTP/network/telemetry crates, no `chrono`, minimal workspace dependencies | Clean — inherits workspace lints cleanly with offline-only dependencies |
| `apps/curvebook-cli/src/cli_catalog.rs` | 390 | `1..=390` | Subcommand/flag catalog parity vs `main.rs`/`cmd_*`, help text generation, JSON catalog formatting | Clean — matches CLI subcommands, flags, and exit codes |
| `apps/curvebook-cli/src/cmd_bench_perf.rs` | 509 | `1..=509` | 96-well & 1M-cell benchmark setup, `--require-release` enforcement, `clamped_px` (`max(1.0)` clears NaN), no unchecked indexing/panics | Clean — finite dimension clamping and guarded benchmark timing |
| `apps/curvebook-cli/src/cmd_commands.rs` | 878 | `1..=878` | `WorkdirGuard`/`PathResolver` sanitization before execution, JSON stdin parsing/errors, `persist_session_output`, `ensure_param_output_parents` ordering | [`FA-SEC21-06`](#fa-sec21-06) |
| `apps/curvebook-cli/src/cmd_convert.rs` | 1,029 | `1..=1029` | Regular-file & `MAX_ARCHIVE_FILE_BYTES` read guards, `.curvebook.d` unpacked detection, SVG/PDF/EMF/PNG/TIFF/CSV/TSV/JSON exports, `--ttf-font` validation, `run_guarded_cli` panic recovery | Clean — guarded exports, font path validation, and parent-dir creation |
| `apps/curvebook-cli/src/cmd_info.rs` | 362 | `1..=362` | Read-only project metadata inspection, terminal control-char sanitization (`sanitize_terminal_text`), JSON/human flag parsing | Clean — terminal output escaped and path reading delegated cleanly |
| `apps/curvebook-cli/src/cmd_run_batch.rs` | 833 | `1..=833` | `curvebook.batch.v1` strict manifest validation, `WorkdirGuard` param/input/export sanitization, `64 MiB` stdin cap, `FailedResultSheet` propagation, `ensure_parent_dir_exists` coverage | [`FA-SEC21-06`](#fa-sec21-06) |
| `apps/curvebook-cli/src/error.rs` | 282 | `1..=282` | Stable `CliError` exit codes (`2`/`3`/`4`/`5`/`6`/`130`), `--json` error schema formatting, `catch_unwind` support | Clean — deterministic exit-code mapping and non-panicking error formatting |
| `apps/curvebook-cli/src/main.rs` | 496 | `1..=496` | `#![forbid(unsafe_code)]`, signal/interrupt handling, `parse_mcp_args_with_env` (`--workdir` + `--control-port` rejection, `ControlAuth` validation), `InProcessUiGuiHost` normalization | Clean — forbids `unsafe`, enforces mutual exclusion, propagates `--gui-only` and `--workdir` |
| `apps/curvebook-cli/src/sandbox.rs` | 126 | `1..=126` | `WorkdirGuard` canonicalization and traversal/symlink escape rejection, recursive `PathResolver::sanitize_params` | Clean — delegates path confinement cleanly to `curvebook_platform` |
| `crates/mcp/src/errors.rs` | 153 | `1..=153` | `StructuredErrorPayload` conversion, error-code classification heuristics, retryable/remediation metadata preservation | Clean — structured error propagation without raw panics |
| `crates/mcp/src/formats.rs` | 1,392 | `1..=1392` | `resolve_mcp_path` `WorkdirGuard` confinement, `FormatRegistry` open/save/export budgets, `MAX_CAPPED_RASTER_DIM_PX` (`8192`) checks, live `--control-port` `.prism`/`.pzfx` forwarding | [`FA-SEC21-04`](#fa-sec21-04) |
| `crates/mcp/src/gui_tools.rs` | 798 | `1..=798` | 8 GUI tool schemas, `--gui-only` pre-checks, coordinate/pointer checks, `ui_inspect` & `ui_pointer_gesture` schema parity, `reject_unknown_args` coverage | [`FA-SEC21-01`](#fa-sec21-01), [`FA-SEC21-02`](#fa-sec21-02), [`FA-SEC21-05`](#fa-sec21-05) |
| `crates/mcp/src/lib.rs` | 56 | `1..=56` | `#![forbid(unsafe_code)]`, crate exports, `SERVER_NAME` / `SERVER_VERSION` constants | Clean — forbids `unsafe` at crate root with clean re-exports |
| `crates/mcp/src/prompts.rs` | 248 | `1..=248` | Built-in workflow prompt specs, standard vs `--gui-only` guidance switching, argument checks | Clean — bounded string formatting and consistent `--gui-only` prompt variants |
| `crates/mcp/src/protocol.rs` | 1,501 | `1..=1501` | `Content-Length` + JSONL dual framing, `MAX_REQUEST_BYTES` (`16 MiB`), `MAX_DRAIN_BYTES` (`64 MiB`), `MAX_BATCH_LEN` (`256`), cancellation & header limit bounds | [`FA-SEC21-08`](#fa-sec21-08) |
| `crates/mcp/src/resources.rs` | 336 | `1..=336` | `project://summary`, `sheet://<id>`, `result://<id>` URI routing, bounded CSV preview (`100` rows / `65,536` bytes), UTF-8-safe byte truncation | Clean — bounds data sheet previews safely at char boundaries |
| `crates/mcp/src/tools.rs` | 2,765 | `1..=2765` | Loopback `127.0.0.1` `live_control`, fail-closed `sync_remote_control_policy`, redacted `Debug`, `run_guarded_tool` panic barrier, `command_batch` rollback, `doc_inspect`, timeouts, `resolve_config_dir_with_env` | [`FA-SEC21-03`](#fa-sec21-03), [`FA-SEC21-04`](#fa-sec21-04), [`FA-SEC21-05`](#fa-sec21-05), [`FA-SEC21-07`](#fa-sec21-07) |
| `crates/mcp/src/toolsets.rs` | 351 | `1..=351` | `CommandGroup` progressive discovery, snake_case tool names, `list_toolsets`/`enable_toolset`, `--gui-only` filtering | Clean — deterministic toolset grouping and normalized lookup |
| `crates/mcp/src/ui_catalog.rs` | 442 | `1..=442` | Parity against `crates/ui-egui/src/menus/ui_commands.rs` (`diff -u` shows only header doc comments differ) | Clean — exact parity with `ui-egui` command metadata table |

---

## Findings

### FA-SEC21-01
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/gui_tools.rs:194`, `crates/mcp/src/gui_tools.rs:464-480`
- **Category:** `contract-drift`
- **Severity:** `S2`
- **Defect:** `def_ui_pointer_gesture` documents and advertises `metrics` as `{width_px, height_px, scale_factor}`, and `handle_ui_pointer_gesture` forwards `args.get("metrics")` raw as a JSON object. However, the UI backend (`crates/ui-egui/src/gui_actions.rs:206-211` and `crates/ui-egui/src/interaction.rs:24-29`) deserializes `ViewportMetrics` directly via Serde with three mandatory fields (`logical_width`, `logical_height`, `scale_factor`) and no field aliases or defaults.
- **Failure scenario:** Any MCP client following the `ui_pointer_gesture` schema and supplying `metrics: { "width_px": 1280, "height_px": 800, "scale_factor": 2.0 }` fails every invocation with `BadInput` (`"invalid viewport metrics: missing field \`logical_width\`"`). Passing `{}` or partial viewport fields similarly fails schema expectations at runtime.
- **Evidence:**
  ```rust
  // crates/mcp/src/gui_tools.rs:194
  "metrics": { "type": "object", "description": "Optional ViewportMetrics ({width_px, height_px, scale_factor}) for pointer dispatch." }

  // crates/mcp/src/gui_tools.rs:464-480
  let metrics = optional_object(args, "metrics")?;
  ...
  if let Some(m) = metrics {
      params.insert(String::from("metrics"), m);
  }
  ```
  Cross-check in UI backend:
  ```rust
  // crates/ui-egui/src/interaction.rs:24-29
  #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
  pub struct ViewportMetrics {
      pub logical_width: f64,
      pub logical_height: f64,
      pub scale_factor: f64,
  }
  ```
- **Suggested fix:** Update `def_ui_pointer_gesture` to specify explicit nested `properties` and `required` fields for `metrics`, and normalize `{ width_px, height_px, scale_factor }` into `{ logical_width, logical_height, scale_factor }` (or accept both forms) inside `handle_ui_pointer_gesture` with positive finite checks before forwarding `ui.pointer_gesture`.
- **Related:** `FA-SEC21-02`, `FA-SEC21-05`

---

### FA-SEC21-02
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/gui_tools.rs:86-100`, `crates/mcp/src/gui_tools.rs:338-354`
- **Category:** `contract-drift`
- **Severity:** `S2`
- **Defect:** `docs/mcp.md:195` documents `sheet_id` as an accepted parameter on `ui_inspect`, and `crates/ui-egui/src/gui_actions.rs:90-97` reads `metrics` (`ViewportMetrics`) in `handle_ui_inspect`. However, `def_ui_inspect` declares only `scope` and `include_bounds` with `"additionalProperties": false`, and `handle_ui_inspect` strictly rejects any other field via `reject_unknown_args("ui_inspect", args, &["scope", "include_bounds"])?;`.
- **Failure scenario:** An MCP caller following `docs/mcp.md` that passes `{ "scope": "grid", "sheet_id": "data-1" }` to `ui_inspect` is immediately rejected with `InvalidParams` (`"ui_inspect received unknown parameter(s): sheet_id"`), and callers needing HiDPI bounds computed at a non-default `metrics` viewport size cannot pass `metrics` either.
- **Evidence:**
  ```rust
  // crates/mcp/src/gui_tools.rs:86-100
  input_schema: json!({
      "type": "object",
      "properties": {
          "scope": {
              "type": "string",
              "enum": VALID_INSPECT_SCOPES,
              "description": "Optional subtree scope filter (defaults to 'all')."
          },
          "include_bounds": {
              "type": "boolean",
              "description": "Include widget/hit-target bounding boxes in the snapshot."
          }
      },
      "additionalProperties": false
  }),

  // crates/mcp/src/gui_tools.rs:338-341
  fn handle_ui_inspect(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
      reject_unknown_args("ui_inspect", args, &["scope", "include_bounds"])?;
  ```
- **Suggested fix:** Either remove `sheet_id` from `docs/mcp.md:195` if `ui_inspect` strictly reflects the active sheet (pointing users to `ui_switch_tab`), or implement `sheet_id` filtering and normalized `metrics` forwarding across `def_ui_inspect`, `handle_ui_inspect`, and `ui-egui`'s `handle_ui_inspect`.
- **Related:** `FA-SEC21-01`

---

### FA-SEC21-03
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/tools.rs:1297-1321`
- **Category:** `resources`
- **Severity:** `S2`
- **Defect:** Unlike `sheet://<id>` MCP resources (`crates/mcp/src/resources.rs:11-12, 217-232`), which slice `DataSheet` CSV previews at `MAX_CSV_PREVIEW_ROWS = 100` and `MAX_CSV_PREVIEW_BYTES = 65_536`, `inspect_single_sheet` in `crates/mcp/src/tools.rs:1297-1300` calls `export_datasheet_to_csv(ds, &CsvOptions::csv())` across all rows/columns and embeds the entire CSV string in `doc_inspect(include_cells: true)`. For `ResultSheet` (`tools.rs:1316-1320`), full `rs.tabs` and `rs.checklist` vectors are likewise serialized unbounded. In `--control-port` mode (`tools.rs:1252-1262`), `doc.inspect` delegates to `crates/ui-egui/src/control_io.rs:1955-1956, 1994-1998`, which has the same unbounded CSV/tab serialization and collides with the `16 MiB` `MAX_CONTROL_RESPONSE_BYTES` (`tools.rs:2015`) response cap.
- **Failure scenario:** Invoking `doc_inspect` with `{ "sheet_id": "bench-1m", "include_cells": true }` on a 1,000,000-cell benchmark sheet allocates and serializes tens of MiB into a single tool reply in headless mode (triggering memory spikes or MCP client buffer overflows), or fails with response-size exhaustion over the `16 MiB` live-control line frame.
- **Evidence:**
  ```rust
  // crates/mcp/src/tools.rs:1297-1300
  if include_cells && let Some(obj) = val.as_object_mut() {
      let csv = export_datasheet_to_csv(ds, &CsvOptions::csv()).unwrap_or_default();
      obj.insert(String::from("csv_preview"), Value::String(csv));
  }

  // crates/mcp/src/tools.rs:1316-1319
  if include_cells && let Some(obj) = val.as_object_mut() {
      obj.insert(String::from("tabs"), serde_json::to_value(&rs.tabs).unwrap_or(Value::Null));
      obj.insert(String::from("checklist"), serde_json::to_value(&rs.checklist).unwrap_or(Value::Null));
  }
  ```
- **Suggested fix:** Share the bounded preview slicer (`MAX_CSV_PREVIEW_ROWS`, `MAX_CSV_PREVIEW_BYTES`, plus explicit row-slice/pagination options if deeper paging is desired) between `resources.rs` and `tools.rs::inspect_single_sheet`, include `csv_preview_truncated: bool`, and apply an explicit row/cell cap when serializing `ResultSheet` tab data inline.
- **Related:** `FA-SEC21-04`

---

### FA-SEC21-04
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/formats.rs:375-403`, `crates/mcp/src/tools.rs:2013-2015`, `crates/mcp/src/tools.rs:2218-2219`
- **Category:** `resources`
- **Severity:** `S2`
- **Defect:** Two live `--control-port` transport bottlenecks break valid large workflows:
  1. In `apply_imported_payload_live` (`crates/mcp/src/formats.rs:375-400`), `open_file` reads `.prism` and `.pzfx` archives locally up to `MAX_ARCHIVE_FILE_BYTES` (`128 MiB`), then base64-encodes the entire raw byte buffer inline into a single `command.batch` / `engine.execute` control frame (`json!({ "bytes_base64": encode_base64(&bytes), ... })`) instead of forwarding `"path"` as `.curvebook`/`.cbtemplate` do (`formats.rs:331-358`). Because control frames are bounded by `MAX_CONTROL_LINE_BYTES` (`16 MiB`, `docs/control-protocol.md:27`), any `.prism`/`.pzfx` archive exceeding ~`12 MiB` is guaranteed to fail transmission after local parsing already succeeded.
  2. In `live_control` (`crates/mcp/src/tools.rs:2013-2014, 2218-2219`), `IO_TIMEOUT` is hardcoded to `Duration::from_secs(5)` on the blocking `TcpStream` for every synchronous request, including heavy recalculations, multi-page PDF exports (`all_graphs: true`), and high-resolution rasterizations.
- **Failure scenario:** Attaching MCP via `--control-port` and opening a `20 MiB` `.prism` project via `open_file` fails mid-transfer after reading/parsing the archive locally, leaving the desktop state unchanged (or failing after validating input). Triggering a multi-page `export` or heavy nonlinear `command_run` taking >5 s aborts with a socket read timeout `IoError` while the desktop server finishes mutating/writing state out-of-band.
- **Evidence:**
  ```rust
  // crates/mcp/src/formats.rs:385-399
  let cmd_id = if format_id == "prism" { "table.importPrism" } else { "table.importPzfx" };
  let import_params = json!({ "bytes_base64": encode_base64(&bytes), "merge": append });
  let send_res = if append {
      state.send_live_control("engine.execute", json!({ "command_id": cmd_id, "params": import_params }))
  } else {
      state.send_live_control(
          "command.batch",
          json!({
              "atomic": true,
              "commands": [
                  { "command": "file.new", "params": {} },
                  { "command": cmd_id, "params": import_params },
              ]
          }),
      )
  };

  // crates/mcp/src/tools.rs:2013-2015, 2218-2219
  const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
  const IO_TIMEOUT: Duration = Duration::from_secs(5);
  pub(crate) const MAX_CONTROL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
  ...
  let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
  let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
  ```
- **Suggested fix:** Pass `"path"` to `table.importPrism` / `table.importPzfx` in live-attach mode (matching `project.open`), or validate the ~12 MiB control-frame ceiling upfront before base64 allocation; raise or configure `IO_TIMEOUT` appropriately for synchronous heavy operations (`doc.export`, `command.batch`, `project.open`).
- **Related:** `FA-SEC21-03`

---

### FA-SEC21-05
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/gui_tools.rs:142, 197, 218, 235, 258, 288, 310, 356-560` and `crates/mcp/src/tools.rs:873-1099, 1171-1190`
- **Category:** `contract-drift`
- **Severity:** `S3`
- **Defect:** Every built-in MCP tool schema (all 10 core tools in `tools.rs:873-1099` and all 8 GUI tools in `gui_tools.rs:99-310`) publishes `"additionalProperties": false`. At runtime, however, only `handle_ui_inspect` (`gui_tools.rs:339`) and `handle_ui_answer_dialog` (`gui_tools.rs:561`) invoke `reject_unknown_args`; the remaining 16 tool handlers ignore unrecognized keys in `args`.
- **Failure scenario:** When a caller or LLM passes a typo or near-miss parameter (such as `output: "fig.png"` instead of `output_path` on `export`, or `source_sheet` on `doc_inspect`, or `widget` instead of `widget_id` on `ui_pointer_gesture`), the handler neither warns nor fails schema validation and instead executes without the parameter (e.g., returning inline text instead of writing the export file, or ignoring the intended filter).
- **Evidence:**
  ```rust
  // crates/mcp/src/gui_tools.rs:338-339 vs 356-358
  fn handle_ui_inspect(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
      reject_unknown_args("ui_inspect", args, &["scope", "include_bounds"])?;
  ...
  fn handle_ui_screenshot(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
      let mut params = Map::new(); // never calls reject_unknown_args
  ```
  Core dispatch (`tools.rs:1171-1189`) similarly forwards `args` directly into `handle_doc_inspect`, `handle_open_file`, `handle_save_file`, `handle_command_list`, `handle_command_run`, `handle_command_batch`, `handle_render_preview`, `handle_export`, `handle_list_toolsets`, and `handle_enable_toolset` without unknown-key checks.
- **Suggested fix:** Enforce `reject_unknown_args` consistently across all 18 tool handlers against the exact keys declared in their `input_schema` `"properties"`.
- **Related:** `FA-SEC21-01`, `FA-SEC21-02`

---

### FA-SEC21-06
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-cli/src/cmd_run_batch.rs:594-598, 632-669` and `apps/curvebook-cli/src/cmd_commands.rs:150-153, 749-764`
- **Category:** `contract-drift`
- **Severity:** `S3`
- **Defect:** In `cmd_run_batch.rs`, `is_file_write_command` (`632-656`) hardcodes 20 command names and omits `"result.generateReport"` (which has `Effects::WRITES_FILESYSTEM` in `crates/engine/src/cmd/analysis.rs:192`), and `ensure_parent_dir_exists` (`658-669`) inspects only `"path"`, ignoring `"file_path"` / `"filePath"` aliases accepted by `project.save`, `project.saveTemplate`, and `table.exportCsv`. Furthermore, both `cmd_run_batch.rs:594-598` and `cmd_commands.rs:151-153` call `fs::create_dir_all(parent)` **before** executing/validating the command (even though the engine's own `write_atomic_output_file` and `write_resolved_atomic` already create parent directories atomically on write).
- **Failure scenario:** Running `curvebook-cli run` or `curvebook-cli command` under `--workdir` with invalid command parameters or unmet preconditions (e.g., missing `sheet_id`, invalid export option, or disabled command) fails during `session.execute_with_caller(...)` while leaving empty created directories behind on the workspace filesystem; meanwhile `cmd_run_batch` treats `result.generateReport` and `file_path` aliases inconsistently with `cmd_commands`.
- **Evidence:**
  ```rust
  // apps/curvebook-cli/src/cmd_run_batch.rs:594-598, 632-666
  if ensure_output_parents && is_file_write_command(&entry.cmd) {
      ensure_parent_dir_exists(&params)?;
  }
  let cancelled = AtomicBool::new(false);
  let outcome = session.execute_with_caller(&entry.cmd, params, Caller::Cli, &cancelled)?;
  ...
  fn ensure_parent_dir_exists(params: &Value) -> Result<(), CliError> {
      if let Some(raw_path) = params.get("path").and_then(Value::as_str) {
          ...
          fs::create_dir_all(parent)...
      }
      Ok(())
  }

  // apps/curvebook-cli/src/cmd_commands.rs:150-153
  resolver.sanitize_params(&mut parsed.params)?;
  if parsed.workdir.is_some() && spec.effects.contains(Effects::WRITES_FILESYSTEM) {
      ensure_param_output_parents(&parsed.params)?;
  }
  ```
- **Suggested fix:** Remove redundant pre-execution `fs::create_dir_all` calls where the engine already creates parent directories after validation (or trigger parent directory creation only after command argument validation succeeds, deriving write commands uniformly from `Effects::WRITES_FILESYSTEM` rather than a partial hardcoded list).

---

### FA-SEC21-07
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/tools.rs:2115-2152`, `crates/mcp/src/tools.rs:2180-2206`
- **Category:** `security`
- **Severity:** `S3`
- **Defect:** `live_control::resolve_config_dir_with_env` enforces `verify_private_dir` (rejecting symlinks, non-directories, foreign UID ownership, and non-`0700` permissions) only on the Unix fallback path (`tools.rs:2146-2150`). Whenever `CURVEBOOK_CONFIG_DIR`, `HOME` (macOS/Linux), or `XDG_CONFIG_HOME` is set, the returned config directory and `control-{port}.token` file (`tools.rs:2180-2205`) skip directory ownership, symlink, and permission checks (only regular-file & 1 KiB size checks run on the token file itself).
- **Failure scenario:** On shared multi-user Unix hosts where `CURVEBOOK_CONFIG_DIR` or `XDG_CONFIG_HOME` points to a symlinked or group/world-writable directory (or contains a group/world-writable `control-{port}.token` file), the MCP client silently reads an unverified token file instead of failing closed as it does for the `/tmp` fallback root.
- **Evidence:**
  ```rust
  // crates/mcp/src/tools.rs:2115-2127
  pub(crate) fn resolve_config_dir_with_env(env_get: &impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, StructuredErrorPayload> {
      let nonempty_path = |key: &str| env_get(key).filter(|v| !v.is_empty()).map(PathBuf::from);
      if let Some(raw_cfg) = env_get("CURVEBOOK_CONFIG_DIR").filter(|v| !v.to_string_lossy().trim().is_empty()) {
          return Ok(PathBuf::from(raw_cfg));
      }

      #[cfg(target_os = "macos")]
      {
          if let Some(home) = nonempty_path("HOME") {
              return Ok(home.join("Library").join("Application Support").join("CurveBook"));
          }
          resolve_unix_private_fallback_root(env_get)
      }
  ```
- **Suggested fix:** Validate the final resolved config directory (`!symlink`, directory, UID match, and private mode on shared locations or at least symlink/ownership verification on `CURVEBOOK_CONFIG_DIR`) and verify token file UID ownership and restrictive mode (`(mode & 0o077) == 0`) on Unix before accepting a discovered `control-{port}.token`.

---

### FA-SEC21-08
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/src/protocol.rs:722-749`, `crates/mcp/src/protocol.rs:798-803`
- **Category:** `contract-drift`
- **Severity:** `S4`
- **Defect:** Two minor framing boundary inconsistencies exist in `protocol.rs`:
  1. In `read_content_length_frame` (`798-803`), `header_lines` is initialized with `first_line` (length 1) and the loop guard checks `while header_lines.len() <= MAX_HEADER_COUNT` (`32`), permitting `MAX_HEADER_COUNT + 1` (`33`) header lines rather than `32`.
  2. In `read_next_frame` (`722-749`), a malformed non-header line that does not start with `{` or `[` is read with `MAX_HEADER_LINE_BYTES` (`8,192`) instead of `effective_max`, producing `"line exceeds maximum request size of 8192 bytes"` if a client sends >8 KiB of invalid non-JSON text even though `effective_max` is up to `16 MiB`.
- **Failure scenario:** A strict protocol test or client sending exactly `33` MIME header lines in a `Content-Length` frame is accepted instead of rejected at `MAX_HEADER_COUNT = 32`; invalid non-JSON input lines between `8 KiB` and `16 MiB` report the 8 KiB header ceiling rather than the JSON-RPC request limit.
- **Evidence:**
  ```rust
  // crates/mcp/src/protocol.rs:798-803
  fn read_content_length_frame<R: BufRead>(reader: &mut R, first_line: Vec<u8>, max_bytes: usize) -> io::Result<NextFrame> {
      let mut header_lines = vec![first_line];
      let mut terminated_cleanly = false;

      while header_lines.len() <= MAX_HEADER_COUNT {
          match read_line_bytes(reader, MAX_HEADER_LINE_BYTES)? {
  ```
- **Suggested fix:** Change the header loop condition to `while header_lines.len() < MAX_HEADER_COUNT` so total headers never exceed `MAX_HEADER_COUNT = 32`.
