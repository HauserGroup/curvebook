# Verification Verdicts: SEC-21 — CLI & MCP Server (`apps/curvebook-cli`, `crates/mcp`)

- **Section:** `SEC-21` (`apps/curvebook-cli/` and `crates/mcp/`: `19` source files + `2` `Cargo.toml` manifests = `21` files, `13,011` LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-21.md`](../../../docs/audit/full-2026-11-22/SEC-21.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-frontend-2`
- **Date:** `2026-11-22`

## Summary Table

| Verdict | Count | Settled S1 | Settled S2 | Settled S3 | Settled S4 | Finding IDs |
|---|---:|---:|---:|---:|---:|---|
| `CONFIRMED` | 7 | 0 | 3 | 4 | 0 | `FA-SEC21-01`..`FA-SEC21-07` |
| `PLAUSIBLE` | 0 | 0 | 0 | 0 | 0 | — |
| `REFUTED` | 1 | — | — | — | — | `FA-SEC21-08` |
| `DUPLICATE` | 0 | 0 | 0 | 0 | 0 | — |
| **Settled Total** | **7** | **0** | **3** | **4** | **0** | *(excludes 1 `REFUTED`)* |

Severity summary (`CONFIRMED + PLAUSIBLE`):
- `S1`: `0`
- `S2`: `3` (`FA-SEC21-01`, `FA-SEC21-03`, `FA-SEC21-04`)
- `S3`: `4` (`FA-SEC21-02` adjusted `S2 -> S3`, `FA-SEC21-05`, `FA-SEC21-06`, `FA-SEC21-07`)
- `S4`: `0` (`FA-SEC21-08` refuted)

## File Coverage Completeness Check

- **Result:** **PASS (`21 / 21` scoped files — `19` `.rs` source files + `2` `Cargo.toml` manifests, `13,011` LOC — 100% covered)**
- Independently verified every file under `apps/curvebook-cli/` and `crates/mcp/` (excluding `tests/`, which belong to `SEC-X3`) via filesystem listing and `wc -l`:
  - `apps/curvebook-cli/Cargo.toml` (`33`), `src/`: `cli_catalog.rs` (`390`), `cmd_bench_perf.rs` (`509`), `cmd_commands.rs` (`878`), `cmd_convert.rs` (`1,029`), `cmd_info.rs` (`362`), `cmd_run_batch.rs` (`833`), `error.rs` (`282`), `main.rs` (`496`), `sandbox.rs` (`126`).
  - `crates/mcp/Cargo.toml` (`31`), `src/`: `errors.rs` (`153`), `formats.rs` (`1,392`), `gui_tools.rs` (`798`), `lib.rs` (`56`), `prompts.rs` (`248`), `protocol.rs` (`1,501`), `resources.rs` (`336`), `tools.rs` (`2,765`), `toolsets.rs` (`351`), `ui_catalog.rs` (`442`).
- Every file and line count matches the File Coverage Table in [`SEC-21.md`](../../../docs/audit/full-2026-11-22/SEC-21.md) with `1..=N` coverage and zero skipped files.

---

## Per-Finding Verdicts

### Verdict `FA-SEC21-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (every `ui_pointer_gesture` call passing `metrics` per the tool's advertised MCP schema fails at runtime with `BadInput`).
- **Evidence:**
  - [`crates/mcp/src/gui_tools.rs:194`](../../../crates/mcp/src/gui_tools.rs#L194) documents `metrics` in `def_ui_pointer_gesture`'s `input_schema` as `"Optional ViewportMetrics ({width_px, height_px, scale_factor}) for pointer dispatch."`, and [`crates/mcp/src/gui_tools.rs:464, 478-480`](../../../crates/mcp/src/gui_tools.rs#L464-L480) forwards the object verbatim under `"metrics"`.
  - Neither [`normalize_in_process_gui_params`](../../../apps/curvebook-cli/src/main.rs#L109-L135) (which only rewrites `"ui.screenshot"` and `"ui.invoke_menu"`) nor [`control_io::dispatch_request_inner`](../../../crates/ui-egui/src/control_io.rs#L1398-L1400) translates `"metrics"`.
  - Downstream [`parse_metrics`](../../../crates/ui-egui/src/gui_actions.rs#L206-L211) calls `serde_json::from_value(m.clone())` directly into [`ViewportMetrics`](../../../crates/ui-egui/src/interaction.rs#L24-L29), whose mandatory Serde fields (with no `alias` or `default`) are `logical_width`, `logical_height`, and `scale_factor`, failing any `{width_px, height_px, scale_factor}` payload with `ErrorCode::BadInput` (`"invalid viewport metrics: missing field \`logical_width\`"`).

### Verdict `FA-SEC21-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S2` -> `S3`
- **Rationale for adjustment:** Per [`docs/audit/README.md`](../../../docs/audit/README.md), `S3` covers contract drift / robustness issues with a clean error signal and workaround. Unlike `FA-SEC21-01` (where the tool's own `tools/list` schema causes valid calls to fail), `def_ui_inspect`'s `input_schema` accurately exposes only `scope` and `include_bounds` (`additionalProperties: false`), `handle_ui_inspect` fails loudly with `InvalidParams`, `ui_switch_tab` + `ui_inspect` provides a workaround for targeting another `sheet_id`, and `ui_inspect` works at default `ViewportMetrics` (`1024x640@1.0`) when `metrics` is omitted.
- **Evidence:**
  - [`docs/mcp.md:195`](../../../docs/mcp.md#L195) (and [`docs/gui-workflows.md:65`](../../../docs/gui-workflows.md#L65)) documents `sheet_id` as an accepted input on `ui_inspect`.
  - [`crates/ui-egui/src/gui_actions.rs:90-97`](../../../crates/ui-egui/src/gui_actions.rs#L90-L97) calls `parse_metrics(params)?` and passes `metrics` to [`inspect_ui_tree_for_metrics`](../../../crates/ui-egui/src/inspect.rs#L280-L345) (which also never reads `sheet_id`, inspecting only `doc.active_sheet_id`).
  - [`crates/mcp/src/gui_tools.rs:86-100, 338-341, 732-739`](../../../crates/mcp/src/gui_tools.rs#L86-L100) declares only `scope` and `include_bounds` with `"additionalProperties": false` and calls `reject_unknown_args("ui_inspect", args, &["scope", "include_bounds"])?;`, rejecting both `sheet_id` and `metrics` with `ToolCallError::InvalidParams`. *(Precision note: [`gui_tools.rs:735`](../../../crates/mcp/src/gui_tools.rs#L735) formats `"unsupported parameter \"sheet_id\" for ui_inspect"`, rather than the paraphrased `"ui_inspect received unknown parameter(s): sheet_id"` in `SEC-21.md`.)*

### Verdict `FA-SEC21-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (unbounded CSV/result serialization causes large memory spikes in headless `doc_inspect(include_cells: true)` and breaks `--control-port` `doc_inspect` / live `sheet://` resource reads against the `16 MiB` response line limit on valid large sheets).
- **Evidence:**
  - Contrast [`crates/mcp/src/resources.rs:11-12, 217-232`](../../../crates/mcp/src/resources.rs#L217-L232) (`MAX_CSV_PREVIEW_ROWS = 100`, `MAX_CSV_PREVIEW_BYTES = 65_536`) with [`crates/mcp/src/tools.rs:1297-1300, 1316-1320`](../../../crates/mcp/src/tools.rs#L1297-L1320), where `inspect_single_sheet` embeds full unbounded `export_datasheet_to_csv(ds, &CsvOptions::csv()).unwrap_or_default()` and full `rs.tabs` / `rs.checklist` vectors whenever `include_cells` is `true`.
  - In `--control-port` mode, [`crates/mcp/src/tools.rs:1252-1262`](../../../crates/mcp/src/tools.rs#L1252-L1262) forwards `doc.inspect` (as does [`read_live_resource`](../../../crates/mcp/src/protocol.rs#L630-L632) for `sheet://` URIs) to [`control_io::inspect_single_sheet`](../../../crates/ui-egui/src/control_io.rs#L1955-L1956), which has the exact same unbounded `export_datasheet_to_csv` and `s.tabs` / `s.scalars` / `s.checklist` (`:1994-1998`) serialization, colliding with `MAX_CONTROL_LINE_BYTES` (`16 MiB` at [`control_io.rs:37, 1560-1582`](../../../crates/ui-egui/src/control_io.rs#L1560-L1582)) and `MAX_CONTROL_RESPONSE_BYTES` (`16 MiB` at [`tools.rs:2015`](../../../crates/mcp/src/tools.rs#L2015)).

### Verdict `FA-SEC21-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (valid >~12 MiB `.prism`/`.pzfx` opens fail in `--control-port` mode after local parsing, and valid synchronous operations exceeding `5s` drop with `IoError` while desktop execution continues).
- **Evidence:**
  - [`crates/mcp/src/formats.rs:142`](../../../crates/mcp/src/formats.rs#L142) reads `.prism` (`MAX_ARCHIVE_FILE_BYTES = 128 MiB`, [`crates/format/src/archive.rs:42`](../../../crates/format/src/archive.rs#L42)) and `.pzfx` (`MAX_XML_BYTES = 64 MiB`, [`crates/format/src/xml.rs:17`](../../../crates/format/src/xml.rs#L17)). Unlike `.curvebook`/`.cbtemplate` ([`formats.rs:330-358`](../../../crates/mcp/src/formats.rs#L330-L358)), [`apply_imported_payload_live`](../../../crates/mcp/src/formats.rs#L375-L403) base64-encodes the entire file inline into `"bytes_base64"` instead of forwarding `"path"` (even though [`table.importPrism` and `table.importPzfx`](../../../crates/engine/src/cmd/table/import.rs#L801-L862) natively accept `"path"`), exceeding `MAX_CONTROL_LINE_BYTES` (`16 MiB` at [`control_io.rs:37, 632, 841`](../../../crates/ui-egui/src/control_io.rs#L37) and [`apps/curvebook/src/live_control.rs:239, 251`](../../../apps/curvebook/src/live_control.rs#L239-L251); `docs/control-protocol.md:9, 21`) once raw file size exceeds ~`12 MiB`. *(Precision note: `SEC-21.md:141` cited `docs/control-protocol.md:27`, whereas `MAX_CONTROL_LINE_BYTES` is documented at `docs/control-protocol.md:9, 21`.)*
  - [`crates/mcp/src/tools.rs:2013-2014, 2218-2219`](../../../crates/mcp/src/tools.rs#L2013-L2019) hardcodes `IO_TIMEOUT: Duration = Duration::from_secs(5)` on every `TcpStream` read/write in `send_control_detailed_with_token`, timing out heavy multi-page `doc.export` or `command.batch` runs while the desktop listener completes them out-of-band.

### Verdict `FA-SEC21-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (contract drift between `"additionalProperties": false` in MCP tool schemas and runtime handlers on 16 of 18 built-in tools; valid schema-conformant requests are unaffected).
- **Evidence:**
  - All 10 core tools ([`crates/mcp/src/tools.rs:873, 899, 918, 939, 959, 996, 1024, 1063, 1079, 1099`](../../../crates/mcp/src/tools.rs#L873-L1100)) and all 8 GUI tools ([`crates/mcp/src/gui_tools.rs:99, 142, 197, 218, 235, 258, 288, 310`](../../../crates/mcp/src/gui_tools.rs#L99-L310)) publish `"additionalProperties": false`.
  - Only `handle_ui_inspect` ([`gui_tools.rs:339`](../../../crates/mcp/src/gui_tools.rs#L339)) and `handle_ui_answer_dialog` ([`gui_tools.rs:561`](../../../crates/mcp/src/gui_tools.rs#L561)) invoke `reject_unknown_args`. The other 6 GUI handlers ([`gui_tools.rs:356-558`](../../../crates/mcp/src/gui_tools.rs#L356-L558)) and all 10 core handlers across [`tools.rs:1171-1189`](../../../crates/mcp/src/tools.rs#L1171-L1189), [`formats.rs:117, 565, 681, 760`](../../../crates/mcp/src/formats.rs#L117), and [`toolsets.rs:142, 202`](../../../crates/mcp/src/toolsets.rs#L142-L235) silently ignore unrecognized keys (as does `optional_modifiers` at [`gui_tools.rs:703-709`](../../../crates/mcp/src/gui_tools.rs#L703-L709) despite `additionalProperties: false` at `:160`).

### Verdict `FA-SEC21-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (contract drift across CLI batch/direct runners and stray empty directories left inside `--workdir` when invalid commands fail).
- **Evidence:**
  - [`apps/curvebook-cli/src/cmd_run_batch.rs:632-656`](../../../apps/curvebook-cli/src/cmd_run_batch.rs#L632-L656): `is_file_write_command` hardcodes 20 command IDs and omits `"result.generateReport"`, which declares `EXPORT_EFFECTS` (`Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT)`) at [`crates/engine/src/cmd/analysis.rs:45, 192`](../../../crates/engine/src/cmd/analysis.rs#L184-L196).
  - [`apps/curvebook-cli/src/cmd_run_batch.rs:658-669`](../../../apps/curvebook-cli/src/cmd_run_batch.rs#L658-L669): `ensure_parent_dir_exists` checks only `params.get("path")`, ignoring `"file_path"` / `"filePath"` accepted by `project.save` ([`project.rs:187, 207`](../../../crates/engine/src/cmd/project.rs#L187-L207)), `project.saveTemplate` ([`project.rs:260, 276`](../../../crates/engine/src/cmd/project.rs#L260-L276)), and `table.exportCsv` ([`table.rs:2649`](../../../crates/engine/src/cmd/table.rs#L2649)).
  - Both [`cmd_run_batch.rs:94, 145, 594-598`](../../../apps/curvebook-cli/src/cmd_run_batch.rs#L594-L598) and [`cmd_commands.rs:150-153, 167, 749-764`](../../../apps/curvebook-cli/src/cmd_commands.rs#L150-L153) run `fs::create_dir_all(parent)` before `session.execute_with_caller(...)` validates/runs the command—even though [`WorkdirGuard::resolve`](../../../crates/platform/src/path_guard.rs#L89-L100) already supports nonexistent tails and engine write helpers ([`write_resolved_atomic`](../../../crates/engine/src/path_policy.rs#L333-L341) and [`write_atomic_output_file`](../../../crates/engine/src/cmd/export.rs#L178-L186)) already create parent directories after validation upon writing.

### Verdict `FA-SEC21-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (requires a multi-user host or misconfigured `CURVEBOOK_CONFIG_DIR` / `XDG_CONFIG_HOME` / symlinked or permissive `control-{port}.token` file; fallback `$TMPDIR` / `$XDG_RUNTIME_DIR` path is hardened).
- **Evidence:**
  - [`crates/mcp/src/tools.rs:2115-2152`](../../../crates/mcp/src/tools.rs#L2115-L2152): `live_control::resolve_config_dir_with_env` calls [`verify_private_dir`](../../../crates/mcp/src/tools.rs#L2069-L2097) (`!is_symlink() && is_dir()`, `meta.uid() == expected_uid`, `(meta.mode() & 0o077) == 0`) only inside `resolve_unix_private_fallback_root` (`:2100-2113`) and the `HOME`/`XDG_CONFIG_HOME`-unset Linux fallback (`:2146-2150`). Whenever `CURVEBOOK_CONFIG_DIR`, macOS `HOME`, or Linux/BSD `XDG_CONFIG_HOME`/`HOME` is set (`:2117-2125, 2140-2145`), the returned directory skips symlink, ownership, and permission checks.
  - [`crates/mcp/src/tools.rs:2180-2206`](../../../crates/mcp/src/tools.rs#L2180-L2206): `resolve_default_control_token_with_env` reads `control-{port}.token` via [`curvebook_platform::fs::read_regular_file_bytes_if_exists`](../../../crates/platform/src/fs.rs#L289-L344), which uses `std::fs::metadata` (following symlinks) and checks only `meta.file_type().is_file()` and `meta.len() <= 1024`, omitting symlink, UID ownership, and `(mode & 0o077) == 0` checks on the discovered token file. *(Precision note: the fallback root uses `$XDG_RUNTIME_DIR/curvebook` or `$TMPDIR/curvebook-uid-<uid>`, not bare `/tmp`.)*

### Verdict `FA-SEC21-08` · `REFUTED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** Not applicable (`REFUTED`)
- **Evidence:**
  1. **33-header claim disproved by loop/terminator invariant (`crates/mcp/src/protocol.rs:798-840`):** `header_lines` is initialized with `first_line` (`len == 1`). Inside `while header_lines.len() <= MAX_HEADER_COUNT` (`MAX_HEADER_COUNT = 32` at `:67`), [`read_content_length_frame`](../../../crates/mcp/src/protocol.rs#L798-L840) pushes a header line *only* when `!is_header_terminator(&line)` (`:824-828`). When `32` headers are sent followed by `\r\n`, the 32nd header brings `header_lines.len()` to `32`, and the loop MUST enter one final iteration (`32 <= 32`) to read the blank `\r\n` terminator (`terminated_cleanly = true; break;` at `:824-827`). If `33` MIME header lines are sent, iteration 32 pushes the 33rd header (`len == 33`), the next `33 <= 32` check exits without reading `\r\n`, `terminated_cleanly` remains `false`, and lines `:833-839` **reject** the frame with `ERR_INVALID_REQUEST` (`"framing header block missing blank separator or exceeded header cap"`). Applying `SEC-21.md`'s suggested fix (`< MAX_HEADER_COUNT`) would introduce an off-by-one regression rejecting valid 32-header frames.
  2. **8 KiB non-`{`/non-`[` line cap is intentional DoS protection (`crates/mcp/src/protocol.rs:718-750, 1423-1437`):** Valid JSON-RPC 2.0 requests (`{...}` or `[...]`) always enter [`read_json_newline_frame`](../../../crates/mcp/src/protocol.rs#L718-L720) with `effective_max` (up to `16 MiB`). Non-`{`/non-`[` preambles are intentionally capped at `MAX_HEADER_LINE_BYTES` (`8,192`) so malformed non-JSON/header lines cannot force a `16 MiB` line buffer allocation (explicitly asserted by `rv15_serve_with_max_bytes_clamps_both_framings_and_enforces_first_header_cap` at [`protocol.rs:1423-1437`](../../../crates/mcp/src/protocol.rs#L1423-L1437)), and line `:748` intentionally formats `effective_max.min(MAX_HEADER_LINE_BYTES)` while returning `ERR_INVALID_REQUEST` (`-32600`). At most the diagnostic phrase `"maximum request size of 8192 bytes"` is minor wording vs `"maximum non-JSON/header line size of 8192 bytes"` on already-invalid input.
