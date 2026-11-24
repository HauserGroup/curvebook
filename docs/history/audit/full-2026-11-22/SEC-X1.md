# SEC-X1 Cross-Cutting Audit Report — Untrusted Input & Trust Boundaries (End-to-End)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `SEC-X1` — End-to-End Untrusted Input & Trust Boundaries across `apps/curvebook-cli`, `crates/mcp`, `crates/ui-egui`, `apps/curvebook`, `apps/curvebook-web`, `crates/engine`, `crates/platform`, `crates/format`, `crates/doc`, `crates/fit`, `crates/transform`, `crates/chart`, `crates/svg`, `crates/pdf`, `crates/render`, and `crates/text`
- **Date:** `2026-11-22`
- **Reviewer:** `rev-x1` (report-only cross-crate seam trace; baseline SHA verified on `HEAD`)
- **Summary Counts:** `S1 = 1`, `S2 = 4`, `S3 = 1`, `S4 = 0` (Total cross-cutting seam findings: `6`; `26` end-to-end seams traced)

> [!NOTE]
> Single-crate defects already recorded in `FA-SEC01`–`FA-SEC22` are cross-referenced in the **Seam Coverage Matrix** and `Related` fields rather than duplicated as new `FA-X1-*` IDs. `FA-X1-01` through `FA-X1-06` capture genuine cross-crate seam gaps and compounding trust-boundary interactions discovered during this trace.

---

## Seam Coverage Matrix (`26` End-to-End Seams Traced)

### Family 1: CLI Entrypoints (`--workdir`, `--batch`, `run`, `convert`, `export`, `info`, `commands`, `<command-id>`) -> `platform` / `engine` / `format` / `render`

| # | Entry Point | Downstream Targets | Bounding Guards Verified | Findings / Status |
|---:|---|---|---|---|
| 1 | `curvebook-cli --workdir <dir>` (`apps/curvebook-cli/src/sandbox.rs:1-126`, `main.rs:158-248`) | `crates/platform/src/path_guard.rs:23-51, 89-428` (`PATH_PARAM_KEYS`), `crates/engine/src/path_policy.rs:18-19, 70-197, 354-394` (`ENGINE_PATH_PARAM_KEYS`, `ExternalPathPolicy`) | `WorkdirGuard::new` canonicalizes root; `sanitize_param_value` recurses into nested objects (`link_spec.source.ExternalDelimitedFile.path`, `27` CLI keys vs `11` Engine keys) and rejects NUL/`..`/symlink escapes with `PathEscape`. | **Verified Safe** (core confinement); cross-ref `FA-SEC01-23` (`resolve_atomic_target` symlink/special-file check) and `FA-SEC21-06` (premature `create_dir_all`). |
| 2 | `curvebook-cli command` / `<command-id>` (`apps/curvebook-cli/src/cmd_commands.rs:20-33, 150-240`) | `crates/engine/src/lib.rs:1286-1300`, `crates/engine/src/cmd/{project,export,image,edit,analysis,table,table/import}.rs` | `PathResolver::sanitize_params` runs before `ensure_param_output_parents` and before `--in` load; engine re-enforces `path_policy.enforce_command_path_params` on `READS_FILESYSTEM \| WRITES_FILESYSTEM` commands. | **Cross-Ref Only**: `FA-SEC21-06` (pre-exec `create_dir_all` leaving empty dirs on invalid params) and `FA-SEC11-09` (`export.rs:191-194` -> `helpers::write_output_file:301` raw `fs::write` on `wasm32`). |
| 3 | `curvebook-cli run` / `batch` (`apps/curvebook-cli/src/cmd_run_batch.rs:128-168, 252-270, 458-533`) | `crates/format/src/schemas.rs`, `cmd_convert::load_project_from_path`, `engine/src/lib.rs`, `cmd_convert::rasterize_scene` | `64 MiB` stdin cap (`MAX_SCRIPT_FILE_BYTES`), `WorkdirGuard` sanitization on manifest `inputs[]`, `exports[]` (`out`, `ttf_font`), and `commands[]`; `recalculate_dirty_with_links` passes `include_external_links = false`. | **Impacted by `FA-X1-04` & `FA-X1-05`** via `load_project_from_path` (`:533`) and `export_from_document` (`:160`); cross-ref `FA-SEC21-06` (`result.generateReport` & `file_path` parent prep). |
| 4 | `curvebook-cli convert` / `export` / `info` / `bench-perf` unpacked `.curvebook.d` (`apps/curvebook-cli/src/cmd_convert.rs:253-269, 326-344, 421`, `cmd_info.rs:20`, `cmd_bench_perf.rs:44`) | `crates/format/src/archive.rs:473-496, 974-1051` (`open_project_unpacked_dir`), `crates/format/src/registry.rs:126-140` | `archive.rs` restricts project opens to tracked `manifest.json`, `document.json`, `sheets/*.csv`, and `assets/*` via `resolve_contained_path`. | **`FA-X1-04` (`S2`)**: post-open/export `cmd_convert::dir_size_bytes` recursively walks all subdirectories following symlinks without cycle/depth/entry caps. Cross-ref `FA-SEC04-04` (`cmd_convert.rs:683` failed result CSV bypass). |
| 5 | `curvebook-cli convert` / `export` / `batch` PNG & TIFF raster export (`apps/curvebook-cli/src/cmd_convert.rs:29-30, 721-731, 825-831`, `cmd_run_batch.rs:156-168`) | `crates/render/src/rasterizer.rs:17-20, 62-117, 148-155` | Contrast `crates/engine/src/cmd/export.rs:637-702` (`resolve_raster_target`) and `RenderTargetSpec::from_scene_dpi` (`rasterizer.rs:87-117`). | **`FA-X1-05` (`S2`)**: `rasterize_scene` clamps width/height independently to `16_384` (`MAX_RASTER_EDGE_PX`) and permits `--dpi` up to `65_535`, distorting aspect ratios and bypassing `from_scene_dpi`. |
| 6 | `curvebook-cli` `--ttf-font` (`cmd_convert.rs:33, 146, 223-244`, `cmd_run_batch.rs:154, 458, 515`) | `crates/text/src/metrics.rs:1160-1241`, `crates/pdf/src/writer.rs:189-216, 484-512` | `read_regular_file_bytes(..., 64 MiB)` + SFNT validation (`parse_truetype_font_info`); `sanitize_pdf_font_name` strips whitespace and PDF delimiters (`()<>[]{}/%#`). | **Verified Safe**. |

### Family 2: MCP Stdio <-> `--control-port` <-> `ui-egui/control_io.rs` <-> `apps/curvebook/live_control.rs` & `--gui-only`

| # | Entry Point | Downstream Targets | Bounding Guards Verified | Findings / Status |
|---:|---|---|---|---|
| 7 | MCP stdio transport & JSON-RPC framing (`crates/mcp/src/protocol.rs:53-73, 712-840`, `tools.rs:1150-1154`) | `crates/mcp/src/tools.rs`, `crates/engine/src/guard.rs` | `MAX_REQUEST_BYTES = 16 MiB`, `MAX_DRAIN_BYTES = 64 MiB`, `MAX_HEADER_LINE_BYTES = 8 KiB` (non-JSON preambles & framing lines), `MAX_HEADER_COUNT = 32`, `MAX_BATCH_LEN = 256`, `run_guarded` panic barrier. | **Verified Safe** (ref `FA-SEC21-08` refuted in `SEC-21-verdicts.md`). |
| 8 | MCP `initialize`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get` under `--gui-only` (`crates/mcp/src/tools.rs:549-581, 1156-1204`, `protocol.rs:390-445, 675-710`, `prompts.rs:50-114`) | `crates/mcp/src/toolsets.rs`, `crates/ui-egui/src/control_io.rs:1402-1407` (`ping`) | `sync_remote_control_policy` verifies remote `ping.gui_only`, fails closed if remote host lacks `--gui-only`, shrinks `tools/list` to `8` GUI tools, blocks non-GUI tools (`GuiOnlyViolation`), and emits GUI-only prompts. | **Verified Safe** (`control_policy_synced` assumes immutable startup `gui_only` flag on desktop host, enforced by `apps/curvebook/src/desktop_host.rs:988, 2841`). |
| 9 | MCP / `--control` `ui_invoke_menu` -> `ui-egui` -> `engine` (`crates/mcp/src/gui_tools.rs:514-541, 771-792`, `crates/ui-egui/src/gui_actions.rs:1764-1957`, `lib.rs:896-901`) | `crates/engine/src/lib.rs:789-797, 1286-1300`, `cmd/project.rs:149-183, 503-522`, `cmd/table/import.rs:60-63` | `is_gui_only_blocked_menu_command` blocks non-menu commands (`menu_path.is_empty()`), `ui.screenshot`, `file.print`, `ui.result.copyMethodsReport`, `app.quit`; `sanitize_command_params` confines `--workdir` when set. | **`FA-X1-01` (`S1`)**: `handle_invoke_menu` dispatches via `Caller::Ui` (exempt from `GuiOnlyPolicy::Strict`); when `--workdir` is absent (`--gui-only` standalone or `--control`), `project.open { bytes_base64 }` + `project.recalculate { include_external_links: true }` reads arbitrary host files via `ExternalDelimitedFile`. |
| 10 | MCP `ui_screenshot` / control `render.pdf` -> `write_host_file` (`crates/mcp/src/gui_tools.rs:376-391`, `ui-egui/src/gui_actions.rs:118-135`, `lib.rs:1434-1452`, `control_io.rs:2091-2101, 3681-3689`) | `crates/platform/src/fs.rs` (`write_atomic`) | `--gui-only` rejects `path` in MCP `gui_tools`, `gui_actions`, `dispatch_egui_screenshot_action`, and `handle_render_pdf`; MCP `handle_ui_screenshot` enforces `resolve_mcp_path` (`WorkdirGuard`). | **Cross-Ref Only**: `FA-SEC13-01` (`write_host_file` omits `session.path_policy()` when `gui_only == false` on `UiApp` with `Workdir`/`RejectExternal`, e.g. `InProcessUiGuiHost` or direct control calls). |
| 11 | MCP `resources/list`, `resources/read`, & `doc_inspect` (`crates/mcp/src/resources.rs:11-12, 99-336`, `protocol.rs:479-536, 622-664`, `tools.rs:1232-1347`, `control_io.rs:1411-1414, 1843-2049`) | Live desktop control socket or in-memory `Session` | `resources/read` and `doc.inspect` reject `--gui-only` with `GuiOnlyViolation`; `resources/list` returns `[]` under `--gui-only`; `resources.rs` caps CSV preview at `100` rows / `64 KiB`. | **Cross-Ref Only**: `FA-SEC21-03` (`doc_inspect(include_cells: true)` and live `sheet://` serialize unbounded CSV/tabs, overflowing `16 MiB` `MAX_CONTROL_LINE_BYTES`). |
| 12 | MCP `open_file`, `save_file`, `export`, `render_preview`, `command_run`, `command_batch` (`crates/mcp/src/formats.rs:80-460, 565-900`, `apps/curvebook-cli/src/main.rs:158-188, 244-248`) | `FormatRegistry`, `crates/ui-egui/src/control_io.rs` (`--control-port`), `crates/engine` | Combining `--workdir` + `--control-port` is explicitly rejected (`tools.rs:354`, `main.rs:244`); `--workdir` enforces `WorkdirGuard` + `read_regular_file_bytes(..., max_input_bytes())`. | **Cross-Ref Only**: `FA-SEC21-04` (`apply_imported_payload_live` inline `bytes_base64` >12 MiB & `5s` `IO_TIMEOUT`) and `FA-SEC10-03` (`from_scene_capped` aspect distortion). |
| 13 | Live TCP `--control` auth, framing, HTTP rejection, & token file (`apps/curvebook/src/main.rs:573-600, 664-740`, `live_control.rs:17-27, 260-406`, `ui-egui/src/control_io.rs:70-185, 940-1003`, `mcp/src/tools.rs:2069-2207`) | `127.0.0.1:<port>` loopback socket, OS `$XDG_CONFIG_HOME` / `Library/Application Support` / `APPDATA` | Loopback-only (`127.0.0.1`), `MAX_CONCURRENT_CONTROL_CLIENTS = 32`, HTTP method rejection (`is_http_request_line`), `64 KiB` pre-auth / `16 MiB` post-auth line cap, `2s` idle/line deadline, constant-time compare (`constant_time_eq`), `0o600` atomic token file write. | **Cross-Ref Only**: `FA-SEC21-07` (`resolve_config_dir_with_env` & token file permissions/symlink checks on client read) and `FA-SEC13-03` (`try_dispatch_egui_backend` fast path). *(Note: non-Unix `control_io.rs:143-161` uses `RandomState` rather than OS CSPRNG.)* |
| 14 | Control `command.batch` / `command.run` / `engine.execute` -> `ui-egui` -> `engine` (`crates/ui-egui/src/control_io.rs:1409-1421, 1471-1505, 1661-1802`) | `UiApp::execute_control_command` (`Caller::Control`), `crates/engine/src/lib.rs:1286-1300` | `MAX_CONTROL_BATCH_STEPS = 1_000`; `16 MiB` accumulated response cap; `Caller::Control` enforces `GuiOnlyPolicy::Strict` and `ExternalPathPolicy`; direct `ui.*` blocked under `--gui-only` (`:1480-1490`). | **Cross-Ref Only**: `FA-SEC13-02` (incomplete rollback on `command.batch` after undo eviction) and `FA-SEC13-03` (`try_dispatch_egui_backend` skips `catch_unwind` & token check). |

### Family 3: Desktop GUI (`apps/curvebook`) & Web (`apps/curvebook-web`) -> `ui-egui` -> `engine` -> `platform`

| # | Entry Point | Downstream Targets | Bounding Guards Verified | Findings / Status |
|---:|---|---|---|---|
| 15 | Desktop CLI files, `--ttf-font`, `--event-script`, & WAL recovery (`apps/curvebook/src/main.rs:430-493, 816-865, 1084-1089`, `desktop_host.rs:409-535, 2346-2359`) | `crates/platform/src/fs.rs`, `crates/format/src/recovery.rs`, `crates/text/src/metrics.rs` | `read_regular_file_bytes(..., MAX_FONT_FILE_BYTES = 64 MiB)` + SFNT parse; `read_regular_file_utf8(..., MAX_EVENT_SCRIPT_BYTES)` before `run_event_script_jsonl`; `open_document_from_files` caps single project + bounded CSVs. | **Cross-Ref Only**: `FA-SEC04-02`, `FA-SEC04-03` (`DirectoryJournalBackend` / `curvebook.lock.evict` unbounded reads/FIFO hang) and `FA-SEC19-09` (dirty flag on merged startup). |
| 16 | Desktop drag-and-drop (`FileDropped`) & window reload (`apps/curvebook/src/gui_window.rs:927-957`, `desktop_host.rs:409-455, 2088-2233, 2765-2855`) | `crates/format/src/registry.rs`, `crates/ui-egui/src/lib.rs`, `crates/engine/src/lib.rs` | `classify_and_load_path` routes CSV/TSV through `table.importCsv` (`MAX_CSV_FILE_BYTES`) and archives/templates through `FormatRegistry` (`MAX_ARCHIVE_FILE_BYTES` / `MAX_XML_BYTES`). | **`FA-X1-06` (`S3`)**: `handle_file_dropped` replaces `win.app` via `UiApp::from_session`, preserving `gui_only`/`url_opener` while dropping `human_path_only`, `path_policy`, `clipboard_adapter`, and `egui_dispatch_hook`; cross-ref `FA-SEC19-09`. |
| 17 | Web CSP, print stage (`#curvebook_print_stage`), & external URL opener (`apps/curvebook-web/index.html:5, 832-857`, `lib.rs:80-114`, `host_adapters.rs:2390-2444, 3732-3749`, `crates/svg/src/writer.rs:188-237, 559-572, 766-782`) | Browser DOM, `WebPlatformUrlOpener` | Zero `fetch`/XHR/WebSocket (`connect-src 'self'`, `object-src 'none'`, `form-action 'none'`); `applyPrintCommand` parses `svg_pages` via `DOMParser("image/svg+xml")` + `importNode` (bypassing `build_print_html_document`); `svg/writer.rs` XML-escapes all user fields; `WebPlatformUrlOpener` restricts `window.open` to allowlisted GitHub issue URLs with `noopener,noreferrer`. | **Verified Safe** against untrusted document XSS/open-redirect; cross-ref `FA-SEC10-07` (`BUNDLED_FONTS` `<![CDATA[...]]>` lacks `]]>` escaping, static asset only). |
| 18 | Web JS -> WASM File Picker / FS Access / Drag-Drop -> `host_adapters` -> `VirtualWorkspaceFolder` (`apps/curvebook-web/index.html:441-449, 1335-1538`, `host_adapters.rs:977-1041, 2502-2519, 2567, 3853-4075`, `lib.rs:203-377, 531-539`, `ui-egui/src/interaction.rs:342-487`) | `crates/format`, `crates/engine` | `VirtualWorkspaceFolder::normalize_rel_path` (`interaction.rs:342-355`) rejects `/`, `\`, NUL, `.`, and `..` segments in virtual folder paths. | **`FA-X1-02` (`S2`)**: Zero byte-size/entry-count limits across JS `file.arrayBuffer()`/`bytesToBase64`, WASM `handle_event_json`/`decode_base64_field`, `open_project_json_bytes`, `open_delimited_bytes`, and `VirtualWorkspaceFolder`; `open_unpacked_snapshot` skips `manifest.json` & ZIP caps. Cross-ref `FA-SEC20-01`, `FA-SEC20-02`, `FA-SEC20-03`, `FA-SEC20-05`, `FA-SEC20-08`. |
| 19 | `wasm32` `ExternalPathPolicy` & OS FS guards (`crates/engine/src/path_policy.rs:126-194`, `cmd/image.rs:121-129`, `cmd/export.rs:191-194`, `ui-egui/src/control_io.rs:3686-3689`) | `crates/engine/src/cmd/helpers.rs:301-318` | `path_policy::{read_utf8_file, read_binary_file, write_atomic_bytes}`, `read_image_file_bytes`, and `write_host_file` return `UnsupportedFormat`/`IoError` on `wasm32`. | **Cross-Ref Only**: `FA-SEC11-09` (`export.rs:191-194` calls `helpers::write_output_file` directly on `wasm32`, calling raw `std::fs::create_dir_all`/`write` instead of `path_policy.write_atomic_bytes`). |

### Family 4: Untrusted Payload Ingestion -> Memory/Recursion/Timeout Budgets -> Publication Output (`SVG` / `PDF` / `PNG` / `TIFF` / `EMF` / `Text`)

| # | Entry Point | Downstream Targets | Bounding Guards Verified | Findings / Status |
|---:|---|---|---|---|
| 20 | `.curvebook` / `.cbtemplate` / `.prism` ZIP & `.pzfx` XML ingestion (`crates/format/src/zip.rs:13-20, 247-264`, `xml.rs:17-35, 111-161`, `archive.rs:42, 473-496, 2115-2185`, `prism.rs:21, 57-62, 156, 282-290`, `pzfx.rs:22, 139-146`) | `crates/doc::ProjectDocument`, `crates/engine/src/cmd/{project,table/import}.rs` | `MAX_ARCHIVE_FILE_BYTES = 128 MiB`, `ZipLimits::default()` (`4,096` entries, `64 MiB`/entry, `128 MiB` total), `XmlLimits::default()` (`64 MiB`, depth `128`, `2M` nodes), `MAX_PROJECT_IMAGE_ASSETS = 256`, `MAX_SHEETS_PER_PROJECT = 500`, `MAX_PROJECT_CELLS = 50M` (on `.prism`) / `MAX_IMPORT_CELLS = 10M` (on `.pzfx`), cycle detection. | **Cross-Ref Only**: `FA-SEC04-01`, `FA-SEC04-05`, `FA-SEC02-04`, `FA-SEC11-02` (`decode_base64` uncapped), `FA-SEC12-03` (`table.importPrism { bytes }` skips `MAX_ARCHIVE_FILE_BYTES`). |
| 21 | Delimited CSV/TSV & Table cell budgets (`crates/format/src/csv.rs:17-20, 151-208, 655-862`, `crates/engine/src/cmd/table/import.rs:608-615, 744-746, 1370-1377`) | `crates/doc::DataSheet` | `MAX_CSV_FILE_BYTES = 64 MiB`, `MAX_ROWS` + `MAX_DATA_SETS * MAX_SUBCOLUMNS` field cap, `MAX_IMPORT_CELLS = 10M` pre-allocation checks (`check_csv_cell_limit`). | **Cross-Ref Only**: `FA-SEC03-08`, `FA-SEC03-10`, `FA-SEC03-11`, `FA-SEC11-03`, `FA-SEC12-02` (`table.importCsv`/generation bypasses `50M` `MAX_PROJECT_CELLS`), `FA-SEC12-03` (uncapped inline `csv`). |
| 22 | Embedded PNG/JPEG/TIFF images -> `chart` -> `svg`/`pdf`/`render` (`crates/doc/src/image.rs:16-25`, `format/src/image_asset.rs:23-125`, `engine/src/cmd/image.rs:83-222`, `chart/src/image_decode.rs:24-133`, `chart/src/{scales,xy,survival_cat,multivariate_layout}.rs`, `pdf/src/writer.rs:262-315`) | `ChartScene`, `PdfExportOptions`, `SvgExportOptions`, `RasterImage`, `Emf` | URL schemes (`http`/`https`/`file`/`ftp`/`data`) rejected; `MAX_IMAGE_ASSET_BYTES = 32 MiB`, `MAX_IMAGE_DIM_PX = 16_384`, `MAX_IMAGE_PIXELS = 67_108_864`, `image::Limits` (`max_alloc = 16 * MAX_IMAGE_PIXELS`), `decode_base64_capped`; `compile_layout` tracks `layout_image_pixels`. | **`FA-X1-03` (`S2`)**: All three graph compilers (`scales.rs`, `xy.rs`, `survival_cat.rs`) pass `None` as `pixel_budget` while `GraphSpec.annotations` has no length cap, causing multi-GiB RGBA decode OOM across `chart` -> `pdf`/`svg`/`render` (`PdfImageRegistry::register` also allocates raw RGB/CMYK+alpha before `MAX_PDF_OUTPUT_BYTES` check). |
| 23 | Custom fit DSL & calculated-variable formulas (`crates/fit/src/dsl/{limits.rs:3-18, parser.rs, vm.rs:57-160}`, `crates/transform/src/formula/{parser.rs:5-6, 606, 876, eval.rs:521-536}`) | `crates/engine/src/dag.rs`, `crates/chart/src/equation_box.rs` | `MAX_PARSE_DEPTH = 128`, `MAX_AST_NODES = 4_096`, `MAX_DSL_SOURCE_BYTES = 1 MiB`, `MAX_TOTAL_OPCODES = 100_000`, `MAX_VM_STEPS = 100_000`, `MAX_ODE_STEPS = 50_000`, `MAX_TOKENS = 16_384`, iterative formula evaluator. | **Verified Safe** (DSL/formula parsers & VMs); cross-ref `FA-SEC09-01` (TeX `equation_box.rs` parser infinite loop/OOM reachable from untrusted graph/equation labels). |
| 24 | Publication SVG output (`crates/svg/src/writer.rs:142-237, 487-578, 766-797, 842-887`) | Disk (`write_atomic_output_file`), MCP inline, Web DOM print (`#curvebook_print_stage`) | `escape_xml` escapes `&<>"'` and strips non-XML-1.0 control chars (`is_valid_xml_char`) across `scene.title`, `font_family`, and `span.text`; `validate_svg_structure` verifies balanced tags/namespaces before emission. | **Verified Safe** (user/document data); cross-ref `FA-SEC10-07` (compile-time `BUNDLED_FONTS` `<![CDATA[...]]>` lacks `]]>` escaping). |
| 25 | Publication PDF output (`crates/pdf/src/writer.rs:61-67, 120-163, 189-216, 460-535, 1594-1624`) | Disk (`write_atomic_output_file`), MCP/Control (`base64`), Print spool | `MAX_PDF_PAGES = 64`, `MAX_PDF_TOTAL_PRIMITIVES = 500_000`, `MAX_PDF_OUTPUT_BYTES = 32 MiB`; `write_pdf_bytes_literal` & `format_pdf_info_string` escape literals/UTF-16BE metadata; `sanitize_pdf_font_name` strips PDF Name delimiters/whitespace. | **Impacted by `FA-X1-03`** (pre-compression `PdfImageRegistry` RGBA->CMYK/SMask allocations); cross-ref `FA-SEC10-01`. |
| 26 | Publication PNG/TIFF/EMF raster & host font discovery (`crates/render/src/rasterizer.rs:17-49, 62-117`, `emf.rs:680-750, 975`, `engine/src/cmd/export.rs:637-702`, `text/src/shaping.rs:24-28, 1034-1096`, `ui-egui/src/theme.rs:355, 407-412`) | Disk, MCP/Control PNG/TIFF/EMF, Desktop UI fonts | `MAX_RASTER_DIM_PX = 16_384`, `MAX_RASTER_PIXELS = 67_108_864`, `MAX_BEZIER_DEPTH = 12`, `MAX_DASH_SEGMENTS = 65_536`, EMF 32-bit `nBytes` check & DIB pixel cap, `MAX_HOST_FONT_BYTES = 64 MiB` (`read_regular_bytes_bounded`), `MAX_DIR_FONTS = 64` (depth `<= 3`). | **Impacted by `FA-X1-05`** (`cmd_convert::rasterize_scene`); cross-ref `FA-SEC10-03` (`from_scene_capped`), `FA-SEC10-06` (EMF primitive/output caps), `FA-SEC10-10` (font `manifest.txt` relative path check). |

---

## Cross-Cutting Findings (`S1` -> `S4`)

### `FA-X1-01` · `S1` · `--gui-only` unconstrained host-filesystem barrier bypassed via `ui_invoke_menu` `Caller::Ui` escalation and embedded `ExternalDelimitedFile` recalculation

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/mcp/src/gui_tools.rs:514-541` (`handle_ui_invoke_menu`)
  - `crates/ui-egui/src/gui_actions.rs:1764-1856, 1858-1957` (`is_gui_only_blocked_menu_command`, `find_non_empty_path_key`, `enforce_gui_only_menu_command_policy`, `handle_invoke_menu`)
  - `crates/ui-egui/src/lib.rs:668-674, 896-901` (`is_human_path_only`, `UiApp::execute_command`)
  - `crates/engine/src/lib.rs:789-797, 1286-1300` (`is_strict_gui_only_allowed`)
  - `crates/engine/src/cmd/project.rs:149-183, 503-522` (`run_project_open`, `run_project_recalculate`)
  - `crates/engine/src/cmd/table/import.rs:60-63` (`LinkSource::ExternalDelimitedFile`)
  - `apps/curvebook/src/main.rs:447-456` & `apps/curvebook-cli/src/main.rs:170-186`
- **Category:** `security` / `policy-bypass`
- **Defect:** Across the `mcp`/`control` <-> `ui-egui` <-> `engine` trust boundary, `GuiOnlyPolicy::Strict` (`--gui-only`) is bypassed for unconstrained host filesystem reads when `--workdir` is not configured:
  1. `Engine::is_strict_gui_only_allowed` (`crates/engine/src/lib.rs:789-797`) trusts `Caller::Ui` unconditionally on the assumption that `Caller::Ui` originates only from interactive GUI actions.
  2. In `crates/ui-egui/src/gui_actions.rs:1858-1957`, `handle_invoke_menu` rejects caller-supplied `params` only when `app.is_human_path_only()` is `true` (`:1860`), and `set_human_path_only(true)` is never invoked in production (`apps/curvebook` or `apps/curvebook-cli`). Consequently, `ui_invoke_menu` (`crates/mcp/src/gui_tools.rs:514-541`) forwards arbitrary `params` straight to `app.execute_command(&cmd_id, extra_params)` as `Caller::Ui`.
  3. When `--workdir` is absent (`apps/curvebook --control <port> --gui-only` or `curvebook-cli mcp --gui-only`), `enforce_gui_only_menu_command_policy` (`gui_actions.rs:1834-1856`) relies on two checks: blocking commands with `menu_path.is_empty()` (`:1779`) and scanning JSON keys in `extra_params` against `find_non_empty_path_key` (`:1797-1832`, 12 path keys).
  4. Both `project.open` (`menu_path: "File/Open..."`) and `project.recalculate` (`menu_path: "Analyze/Recalculate All"`) have non-empty `menu_path`s, accept `"bytes_base64"` and `"include_external_links": true` with zero path-named keys in `extra_params`, and execute under `ExternalPathPolicy::Unconstrained` (`crates/engine/src/cmd/project.rs:506`).
- **Failure scenario:** An untrusted MCP client attached to `curvebook --control <port> --gui-only` (or `curvebook-cli mcp --gui-only` without `--workdir`) calls:
  1. `ui_invoke_menu { "command_id": "project.open", "params": { "bytes_base64": "<valid .curvebook containing a DataSheet with LinkedDataRegion::ExternalDelimitedFile { path: \"/etc/passwd\", ... }>" } }`
  2. `ui_invoke_menu { "command_id": "project.recalculate", "params": { "include_external_links": true } }`
  `project.recalculate` invokes `resolve_refresh_source` (`crates/engine/src/cmd/table/import.rs:60-63`), reads `/etc/passwd` (or any host delimited text file up to `MAX_CSV_FILE_BYTES = 64 MiB`) via `ExternalPathPolicy::Unconstrained` into `DataSheet` cells, and exposes the contents to the `--gui-only` agent via `ui_inspect` or `ui_screenshot`.
- **Evidence:**
  ```rust
  // crates/engine/src/lib.rs:789-797
  fn is_strict_gui_only_allowed(spec: &cmd::CommandSpec, caller: Caller, params: &serde_json::Value) -> bool {
      if !matches!(caller, Caller::Mcp | Caller::Control | Caller::Cli) {
          return true;
      }
      ...
  ```
  ```rust
  // crates/ui-egui/src/gui_actions.rs:1846-1854, 1950
  if !app.session().path_policy().is_constrained()
      && let Some(path_key) = find_non_empty_path_key(extra_params)
  {
      return Err(EngineError::with_code(
          ErrorCode::GuiOnlyViolation,
          format!("unconstrained host filesystem `{path_key}` parameter via `ui.invoke_menu` is blocked under --gui-only without --workdir"),
      ) ...);
  }
  ...
  let outcome = app.execute_command(&cmd_id, extra_params)?;
  ```
- **Suggested fix:**
  1. Under `--gui-only`, do not run external `ui.invoke_menu` `params` through `Caller::Ui` without policy enforcement: reject or sanitize embedded `LinkSource::ExternalDelimitedFile` links during `project.open { bytes_base64 }` / `project.instantiateTemplate { bytes_base64 }`, and default `UiApp` sessions under `--gui-only` without `--workdir` to `ExternalPathPolicy::RejectExternal` (or block `include_external_links: true` in `recalculate` when `!path_policy.is_constrained()` under `gui_only`).
  2. Strip `extra_params` on `ui.invoke_menu` for `GuiOnlyPolicy::Strict` filesystem/open/recalculate commands unless `WorkdirGuard` (`--workdir`) is active, so `--gui-only` agents drive visible UI dialogs instead of bypassing engine `Caller::Mcp`/`Caller::Control` restrictions.
- **Related:** `FA-SEC13-01` (`write_host_file` bypassing `ExternalPathPolicy` on `ui.screenshot`/`render.pdf`).

---

### `FA-X1-02` · `S2` · Unbounded browser-to-WASM ingestion (`index.html` -> `host_adapters.rs` -> `lib.rs` -> `VirtualWorkspaceFolder`) bypasses native archive/entry/manifest caps

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `apps/curvebook-web/index.html:441-449, 1335-1538` (`bytesToBase64`, `fileInputEl`, `dirInputEl`, `readDirectoryHandleEntries`, `openWithFilePicker`, `collectDroppedEntry`, `window.drop`)
  - `apps/curvebook-web/src/host_adapters.rs:977-1041, 2502-2519, 2567-2571, 3853-3863, 3938-3954, 4030-4075`
  - `apps/curvebook-web/src/lib.rs:247-262, 268-305, 345-377, 531-539` (`open_project_json_bytes`, `open_delimited_bytes`, `load_virtual_folder_entries`, `open_virtual_unpacked_snapshot`, `install_session`)
  - `crates/ui-egui/src/interaction.rs:361-487` (`VirtualWorkspaceFolder::write_file`, `open_unpacked_snapshot`)
- **Category:** `resources` / `DoS` / `validation-bypass`
- **Defect:** Native file ingestion enforces strict byte, entry-count, and manifest invariants (`MAX_ARCHIVE_FILE_BYTES = 128 MiB`, `MAX_XML_BYTES = 64 MiB`, `MAX_CSV_FILE_BYTES = 64 MiB`, `ZipLimits::default()`, and `open_project_unpacked_dir` manifest/CRC/schema checks). Across the browser JS <-> WASM <-> `ui-egui` <-> `engine`/`format` boundary, all of those ingress caps and unpacked-manifest validations are absent:
  1. `apps/curvebook-web/index.html:1335-1538` eagerly calls `await file.arrayBuffer()`, base64-expands every file via `bytesToBase64` (`441-449`), and `JSON.stringify`s the entire payload into `dispatchFn(...)` with zero per-file byte limit, directory-entry count limit, or cumulative batch byte cap.
  2. `WebBridgeSession::handle_event_json` (`host_adapters.rs:2567`) parses arbitrary-sized JSON strings via `serde_json::from_str`, and `decode_base64_field` / `parse_named_bytes_vec` (`:3853-3863, 3938-3954, 4030-4075`) allocate unbounded `Vec<u8>` buffers (from either `bytes_base64` or arbitrary-length `"bytes": [u8...]` JSON arrays) before format parsing begins.
  3. `WebWorkspaceHost::open_project_json_bytes` (`lib.rs:247-262`) passes unbounded JSON bytes directly to `serde_json::from_slice` prior to `migrate_document_json`'s DOM cap and never invokes `validate_project_cells`.
  4. `WebWorkspaceHost::load_virtual_folder_entries` / `open_virtual_unpacked_snapshot` (`lib.rs:345-377`) -> `VirtualWorkspaceFolder::write_file` / `open_unpacked_snapshot` (`crates/ui-egui/src/interaction.rs:361-487`) enforces zero entry-count (`MAX_ZIP_ENTRIES = 4_096`), entry-byte (`MAX_ZIP_ENTRY_BYTES = 64 MiB`), or cumulative byte (`MAX_ZIP_UNCOMPRESSED_BYTES = 128 MiB`) caps, skips `manifest.json` verification entirely (`open_unpacked_snapshot` reads only `document.json`), and decodes unbounded raw `document.json` via `serde_json::from_slice`.
  5. `WebWorkspaceHost::open_delimited_bytes` (`lib.rs:268-305`) passes unbounded UTF-8 text via inline `"text"` to `table.importCsv` (compounding `FA-SEC12-03`'s uncapped inline `csv` path) while resetting `*self.app.session_mut() = Session::new()` instead of going through `install_session`.
- **Failure scenario:** Dropping or picking a large file (e.g., `> 128 MiB` archive/JSON, a `100 MiB` CSV, or a directory with `10_000` files) in `apps/curvebook-web` triggers duplicate multi-hundred-MiB allocations across JS `ArrayBuffer` -> base64 -> JSON -> WASM `String` -> `serde_json::Value` -> `Vec<u8>`, exhausting 32-bit `wasm32` address space and aborting the browser tab before format limits or `open_project_unpacked_dir` checks ever run; forged `.curvebook.d` folders without valid `manifest.json`/CRCs also load silently via `open_unpacked_snapshot`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/interaction.rs:361-365, 474-478
  pub fn write_file(&mut self, rel_path: &str, bytes: Vec<u8>) -> Result<(), String> {
      let clean = Self::normalize_rel_path(rel_path)?;
      self.files.insert(clean, bytes);
      Ok(())
  }
  ...
  pub fn open_unpacked_snapshot(&self) -> Result<ProjectDocument, String> {
      let bytes = self.read_file("document.json").ok_or_else(|| String::from("virtual snapshot missing `document.json`"))?;
      let raw: Value = serde_json::from_slice(bytes).map_err(|err| format!("failed decoding virtual `document.json`: {err}"))?;
  ```
- **Suggested fix:**
  1. Enforce pre-read `file.size` and batch-entry/total-byte caps (`<= MAX_ARCHIVE_FILE_BYTES`, `<= MAX_ZIP_ENTRIES`) in `apps/curvebook-web/index.html` before `arrayBuffer()` / `bytesToBase64()`, and reject oversized `raw_event_json` / `bytes` payloads at `WebBridgeSession::handle_event_json` / `decode_base64_field`.
  2. Enforce `ZipLimits::default()` (`max_entries`, `max_entry_uncompressed_bytes`, `max_total_uncompressed_bytes`) and `manifest.json` + CRC/schema validation inside `VirtualWorkspaceFolder` (reusing `crates/format`'s snapshot/manifest validator), cap `open_project_json_bytes` + run `validate_project_cells`, and pass `ExternalPathPolicy::RejectExternal` + `gui_only` across `install_session` / `open_delimited_bytes`.
- **Related:** `FA-SEC20-03` (WASM RGBA pixel OOM), `FA-SEC20-05` (`import_dropped_files` double strip & hang), `FA-SEC11-02` (`decode_base64`), `FA-SEC12-03` (inline `csv`/`bytes`), `FA-SEC04-05` (migration DOM budget).

---

### `FA-X1-03` · `S2` · Graph compilers (`scales.rs`, `xy.rs`, `survival_cat.rs`) drop `emit_embedded_image_primitive`'s `pixel_budget`, enabling multi-gigabyte RGBA amplification into PDF/SVG/Raster/EMF

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/chart/src/image_decode.rs:80-144` (`emit_embedded_image_primitive`)
  - `crates/chart/src/multivariate_layout.rs:3828-3841` (passes `Some(&mut layout_image_pixels)`)
  - `crates/chart/src/scales.rs:3556-3569`, `crates/chart/src/xy.rs:1167-1180`, `crates/chart/src/survival_cat.rs:955-968` (pass `None`)
  - `crates/doc/src/graph.rs:1569-2005` (`GraphSpec::validate` — no `annotations.len()` bound)
  - `crates/pdf/src/writer.rs:262-315` (`PdfImageRegistry::register`)
- **Category:** `resources` / `DoS`
- **Defect:** `crates/format/src/image_asset.rs` and `crates/doc/src/image.rs` cap each image asset at `MAX_IMAGE_ASSET_BYTES = 32 MiB`, `MAX_IMAGE_DIM_PX = 16_384`, `MAX_IMAGE_PIXELS = 67_108_864` (~`256 MiB` decoded RGBA8), and `MAX_PROJECT_IMAGE_ASSETS = 256` per project, while `GraphSpec::validate` places no bound on `GraphSpec.annotations`. To prevent cumulative decoded RGBA explosion during scene compilation, `emit_embedded_image_primitive` (`crates/chart/src/image_decode.rs:84-133`) accepts `pixel_budget: Option<&mut u64>` bounded by `MAX_IMAGE_PIXELS`. However, only `compile_layout` (`crates/chart/src/multivariate_layout.rs:3840`) supplies `Some(&mut layout_image_pixels)`; all three graph compilers (`scales.rs:3568`, `xy.rs:1179`, `survival_cat.rs:967`) pass **`None`**. Furthermore, `compile_graph` decodes every `GraphAnnotation::EmbeddedImage` to a fresh owned `SceneRasterImage.rgba` buffer before downstream renderers (`SvgRenderCtx`, `PdfImageRegistry`) ever attempt asset deduplication, and `PdfImageRegistry::register` (`crates/pdf/src/writer.rs:278-315`) allocates full uncompressed RGB/CMYK + SMask buffers per image before checking `MAX_PDF_OUTPUT_BYTES` (`32 MiB`).
- **Failure scenario:** An attacker embeds a single legal `4096 x 4096` blank PNG (`< 50 KiB` compressed, `16,777,216` pixels = `64 MiB` RGBA8) in a `.curvebook` archive and references it `100` times in `GraphSpec.annotations` (or embeds `256` distinct `16M`-pixel assets across graph annotations). The archive passes `open_project_bytes`, `validate_project_document`, and `compile_layout` limits, but rendering or exporting the graph (`curvebook-cli convert`, MCP `export`/`render_preview`, desktop canvas, web WASM, or engine `export.*`) forces `compile_graph` to materialize `100 x 64 MiB = 6.4 GiB` (up to `16+ GiB` across `256` assets) of simultaneous RGBA8 buffers in `ChartScene.primitives`, triggering OOM aborts.
- **Evidence:**
  ```rust
  // crates/chart/src/xy.rs:1167-1180 (same pattern at scales.rs:3556-3569, survival_cat.rs:955-968)
  crate::image_decode::emit_embedded_image_primitive(
      scene,
      doc,
      &crate::image_decode::EmbeddedImageDrawSpec { ... },
      None,
  );
  ```
  Contrast `crates/chart/src/multivariate_layout.rs:3828-3841`:
  ```rust
  crate::image_decode::emit_embedded_image_primitive(
      &mut scene,
      doc,
      &crate::image_decode::EmbeddedImageDrawSpec { ... },
      Some(&mut layout_image_pixels),
  );
  ```
- **Suggested fix:**
  1. Thread a shared `graph_image_pixels: u64` (`Some(&mut graph_image_pixels)`) capped at `MAX_IMAGE_PIXELS` through `scales.rs`, `xy.rs`, and `survival_cat.rs`, and deduplicate decoded RGBA buffers per `asset_id` within a scene (or store `Arc<[u8]>` on `SceneRasterImage` / `decode_asset_rgba8` cache) before populating `scene.primitives`.
  2. Cap `GraphSpec.annotations.len()` in `crates/doc/src/graph.rs` (analogous to `MAX_LAYOUT_ITEMS`), and have `PdfImageRegistry::register` estimate worst-case uncompressed byte growth before allocating `color_raw` / `alpha_raw` when `compress_streams` is disabled.
- **Related:** `FA-SEC04-01` (`.prism` embedded `document.json` image asset cloning), `FA-SEC02-04` (`ProjectImageAsset::Deserialize` pre-allocation).

---

### `FA-X1-04` · `S2` · Symlink-following `dir_size_bytes` in CLI `.curvebook.d` loader/exporter traverses untracked symlinks & loops after `archive.rs` & `WorkdirGuard` validation

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/format/src/archive.rs:473-496, 974-1015, 1035-1051` (`open_project_unpacked_dir`, `resolve_contained_path`)
  - `apps/curvebook-cli/src/cmd_convert.rs:253-269, 326-336, 421` (`dir_size_bytes`, `load_project_from_path`, `export_from_document`)
  - Downstream CLI entrypoints: `apps/curvebook-cli/src/cmd_info.rs:20`, `apps/curvebook-cli/src/cmd_run_batch.rs:533`, `apps/curvebook-cli/src/cmd_commands.rs:157`, `apps/curvebook-cli/src/cmd_bench_perf.rs:44`
- **Category:** `resources` / `DoS` / `path-traversal`
- **Defect:** When opening an unpacked `.curvebook.d` directory, `crates/format/src/archive.rs::open_project_unpacked_dir` validates only `manifest.json`, `document.json`, `sheets/`, and `assets/` via `resolve_contained_path`, rejecting escaping symlinks on tracked entries while leaving any extra untracked files/subdirectories (such as `.git` or arbitrary directories) untouched. Immediately after `importer.import_path(...)` returns, `cmd_convert::load_project_from_path` (`cmd_convert.rs:334`) — and `export_from_document` (`:421`) on `.curvebook.d` writes — invokes `dir_size_bytes(path)` (`253-269`). `dir_size_bytes` walks the entire directory tree using `entry.metadata()` (which **follows symlinks**) with no `file_type().is_symlink()` check, no visited `(dev, ino)` cycle guard, no depth limit, and no entry-count limit.
- **Failure scenario:** Passing an untrusted `.curvebook.d` bundle containing an untracked directory symlink loop (e.g., `extra/loop -> ..`) or an external symlink (e.g., `extra/host -> /`) to `curvebook-cli convert`, `curvebook-cli export`, `curvebook-cli info`, `curvebook-cli run`, `curvebook-cli command --in`, or `curvebook-cli bench-perf` (even inside `--workdir`, because `WorkdirGuard` only checks the root `.curvebook.d` directory path) passes `open_project_unpacked_dir` and then enters an infinite symlink-loop traversal / memory-exhausting queue growth or traverses outside `.curvebook.d` and `--workdir` inside `dir_size_bytes`.
- **Evidence:**
  ```rust
  // apps/curvebook-cli/src/cmd_convert.rs:253-267
  pub fn dir_size_bytes(root: &Path) -> Result<usize, CliError> {
      let mut total = 0_u64;
      let mut stack = vec![root.to_path_buf()];
      while let Some(dir) = stack.pop() {
          let entries = fs::read_dir(&dir)...?;
          for entry in entries {
              let entry = entry...?;
              let meta = entry.metadata()...?;
              if meta.is_dir() {
                  stack.push(entry.path());
              } else {
                  total = total.saturating_add(meta.len());
              }
          }
      }
      Ok(usize::try_from(total).unwrap_or(usize::MAX))
  }
  ```
- **Suggested fix:** Replace `entry.metadata()` in `dir_size_bytes` with `fs::symlink_metadata(&entry.path())`, skip or reject symlinks (`if meta.file_type().is_symlink()` — ideally rejecting external symlinks consistently with `archive.rs` or never descending into symlinks), enforce strict traversal depth (`<= 16`) and entry-count (`<= 16_384`) limits, or compute size solely from the manifest-tracked unpacked entries validated by `archive.rs`.
- **Related:** `FA-SEC21-06`, `FA-SEC01-23`.

---

### `FA-X1-05` · `S2` · CLI `cmd_convert::rasterize_scene` clamps axes independently up to `16_384` and permits `--dpi` up to `65_535`, bypassing `RenderTargetSpec::from_scene_dpi`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `apps/curvebook-cli/src/cmd_convert.rs:29-30, 721-731, 825-831` (`MAX_RASTER_EDGE_PX`, `rasterize_scene`, `clamped_pixel_dim`, `parse_dpi_value`)
  - `apps/curvebook-cli/src/cmd_run_batch.rs:156-168` (`run_batch` -> `export_from_document`)
  - `crates/render/src/rasterizer.rs:17-20, 87-117` (`MAX_RASTER_DIM_PX = 16_384`, `MAX_RASTER_PIXELS = 67_108_864`, `RenderTargetSpec::from_scene_dpi`)
  - Contrast `crates/engine/src/cmd/export.rs:637-702` (`resolve_raster_target`)
- **Category:** `correctness` / `resources` / `contract-drift`
- **Defect:** Across the `cli` <-> `render` boundary, `cmd_convert::rasterize_scene` constructs `RenderTargetSpec { width_px, height_px, dpi }` manually by calling `clamped_pixel_dim(scene.width_pt.max(1.0) * scale)` and `clamped_pixel_dim(scene.height_pt.max(1.0) * scale)` independently (`clamp(1.0, 16_384.0)`) while leaving `dpi` unadjusted (`parse_dpi_value` allows `1..=65_535`). By contrast, `crates/engine/src/cmd/export.rs::resolve_raster_target` and `crates/render/src/rasterizer.rs::RenderTargetSpec::from_scene_dpi` reject oversized scenes cleanly instead of independently distorting axes, and MCP/Control cap `--dpi` at `18..=2400`.
- **Failure scenario:** Passing an untrusted document with extreme page dimensions (e.g., `1000 x 10 in` = `72_000 x 720 pt`) or high `--dpi` to `curvebook-cli convert`, `export`, or `run` (`300 DPI`) clamps only width (`300_000 px -> 16_384 px`) while leaving height at `3_000 px` (`49.15M <= MAX_RASTER_PIXELS = 67.1M`), silently writing a ~`18.3x` horizontally squashed PNG/TIFF stamped with a false `300 DPI` physical tag. When both clamped axes stay large (e.g., `600 x 600 pt` at `--dpi 2000`), `rasterize_scene` silently clamps to `16_384 x 16_384` before `render_scene` rejects `268M > MAX_RASTER_PIXELS` with a confusing clamped dimension error.
- **Evidence:**
  ```rust
  // apps/curvebook-cli/src/cmd_convert.rs:721-731
  fn rasterize_scene(scene: &ChartScene, dpi: u16) -> Result<curvebook_render::RasterImage, CliError> {
      let scale = f64::from(dpi) / 72.0;
      let width_px = clamped_pixel_dim(scene.width_pt.max(1.0) * scale);
      let height_px = clamped_pixel_dim(scene.height_pt.max(1.0) * scale);
      Ok(render_scene(scene, RenderTargetSpec { width_px, height_px, dpi })?)
  }
  fn clamped_pixel_dim(raw: f64) -> u32 {
      raw.round().clamp(1.0, MAX_RASTER_EDGE_PX) as u32
  }
  ```
- **Suggested fix:** Replace `clamped_pixel_dim` in `cmd_convert::rasterize_scene` with `RenderTargetSpec::from_scene_dpi(scene, dpi)?` so oversized dimensions fail cleanly without aspect distortion, and align `parse_dpi_value` bounds (`18..=2400` or explicitly documented range) across CLI, Engine, and MCP.
- **Related:** `FA-SEC10-03` (`RenderTargetSpec::from_scene_capped` independent axis clamping in `mcp/formats.rs` and `ui-egui/control_io.rs`).

---

### `FA-X1-06` · `S3` · Desktop drag-and-drop window reload (`desktop_host::handle_file_dropped`) drops `human_path_only`, `path_policy`, `clipboard_adapter`, and `egui_dispatch_hook`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `apps/curvebook/src/desktop_host.rs:2795-2854, 2088-2233, 979-990` (`handle_file_dropped`, `open_window_with_session`, `install_window_session`, `configure_app_overrides`)
  - Contrast `crates/ui-egui/src/lib.rs:1243-1257` (`UiApp::load_from_virtual_workspace`) and `crates/mcp/src/tools.rs:421-436` (`McpServerState::install_session`)
- **Category:** `security` / `policy-drift`
- **Defect:** Unlike `UiApp::load_from_virtual_workspace` and `McpServerState::install_session`, when a `.curvebook`, `.curvebook.d`, `.cbtemplate`, `.prism`, or `.pzfx` file is dropped onto a clean existing desktop window (`apps/curvebook/src/desktop_host.rs:2815-2843`), `handle_file_dropped` replaces `win.app` with `UiApp::from_session(session)` (`:2830`) and explicitly re-applies `url_opener`, `locale`, `theme`, `ui_scale`, `ui_backend`, `host_draws_menu_bar`, `shortcuts`, `rulers_visible`, and `gui_only`—**but omits `human_path_only`, `path_policy` (always defaulting to `ExternalPathPolicy::Unconstrained` on `Session::from_document`), `clipboard_adapter`, and `egui_dispatch_hook`**. Opening a second window on a dirty-window drop (`:2800-2805` -> `open_window_with_session` -> `configure_app_overrides`) exhibits the same omission for `path_policy` and `human_path_only`.
- **Failure scenario:** Any `DesktopHostRuntime` instance configured with `ExternalPathPolicy::Workdir` / `RejectExternal`, `human_path_only = true`, a sandboxed `clipboard_adapter`, or an `egui_dispatch_hook` silently reverts to `ExternalPathPolicy::Unconstrained`, `human_path_only = false`, and the live OS pasteboard/default hook as soon as a user or script drops a `.curvebook` / `.prism` / `.pzfx` file onto the window.
- **Evidence:**
  ```rust
  // apps/curvebook/src/desktop_host.rs:2828-2843
  let prev_gui_only = win.app.is_gui_only();

  let mut next_app = UiApp::from_session(session);
  if let Some(opener) = inherited_url_opener {
      next_app.set_url_opener(opener);
  }
  ...
  next_app.set_gui_only(prev_gui_only);
  next_app.sync_views();
  win.app = next_app;
  ```
- **Suggested fix:** Add a shared `UiApp::replace_session_preserving_policies(&mut self, session: Session)` helper in `crates/ui-egui` (preserving `gui_only`, `path_policy`, `human_path_only`, adapters, hooks, and UI preferences cleanly in-place without rebuilding `UiApp`), and store/propagate runtime `path_policy` and `human_path_only` inside `DesktopHostConfig` for new windows (`open_window_with_session`).
- **Related:** `FA-SEC19-09` (`open_document_from_files` clean flag & drop `coasting_active`), `FA-SEC19-03` (single-viewport second-window hiding), `FA-SEC20-01` (web `install_session` workspace reset).
