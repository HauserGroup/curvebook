# Verification Report: SEC-20 — Web / WASM Application

- **Section:** `SEC-20` (`apps/curvebook-web/`: `8` files, `11,275` LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-20.md`](SEC-20.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend`

## Summary Table

| Verdict | Count | Settled S1 | Settled S2 | Settled S3 | Settled S4 | Total |
|---|---:|---:|---:|---:|---:|---:|
| `CONFIRMED` | 10 | 3 | 5 | 2 | 0 | 10 |
| `PLAUSIBLE` | 0 | 0 | 0 | 0 | 0 | 0 |
| `REFUTED` | 0 | 0 | 0 | 0 | 0 | 0 |
| `DUPLICATE` | 0 | 0 | 0 | 0 | 0 | 0 |
| **Total** | **10** | **3** | **5** | **2** | **0** | **10** |

## File Coverage Completeness Check

Verified `8 / 8` non-test files under `apps/curvebook-web/` (`11,275` total LOC, matching `wc -l` counts) are present in the File Coverage Table with full line ranges:

| File | Report LOC | Actual LOC | Status |
|---|---:|---:|---|
| `apps/curvebook-web/Cargo.toml` | 27 | 27 | Complete (`1..=27`) |
| `apps/curvebook-web/Trunk.toml` | 18 | 18 | Complete (`1..=18`) |
| `apps/curvebook-web/index.html` | 1,807 | 1,807 | Complete (`1..=1807`) |
| `apps/curvebook-web/src/chrome_bridge.rs` | 2,254 | 2,254 | Complete (`1..=2254`) |
| `apps/curvebook-web/src/host_adapters.rs` | 4,623 | 4,623 | Complete (`1..=4623`) |
| `apps/curvebook-web/src/lib.rs` | 760 | 760 | Complete (`1..=760`) |
| `apps/curvebook-web/src/main.rs` | 141 | 141 | Complete (`1..=141`) |
| `apps/curvebook-web/src/text_bridge.rs` | 1,645 | 1,645 | Complete (`1..=1645`) |

Baseline `#123` cross-check: `#123.4` (`explicit sheet_id` export/clipboard at `host_adapters.rs:1239-1330, 3385-3469`) and `#123.6` (`TouchGestureRecognizer` pinch pointer-up & drag cancel at `host_adapters.rs:1424-1493, 2460-2484, 3590-3602`) are confirmed fixed; findings `FA-SEC20-01`, `FA-SEC20-05`, and `FA-SEC20-08` document distinct residual defects beyond `#123.1`, `#123.2`, `#123.3`, and `#123.5`.

---

## Per-Finding Verdicts

### `FA-SEC20-01` — `CONFIRMED` · Severity agreed (`S1`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `WebWorkspaceHost::install_session` (`apps/curvebook-web/src/lib.rs:531-539`) and `open_delimited_bytes` (`lib.rs:268-274`) replace `*self.app.session_mut()` without resetting `self.app.virtual_workspace`, and `project.new` never clears `virtual_workspace` anywhere in `crates/ui-egui` or `apps/curvebook-web`. `FileSystemAccessBridge::import_file_bytes` (`apps/curvebook-web/src/host_adapters.rs:977-984`) clears `last_synced_root` and `last_synced_tree`, while `OpenFileBytes` (`host_adapters.rs:2502-2507`) emits only `summary_render_command` — there is no `WebBridgeCommand` variant (`host_adapters.rs:2030-2053`) to clear JS `activeDirectoryHandle` (`apps/curvebook-web/index.html:259`, assigned at `:1441`). Calling `sync_directory_from_host` (`host_adapters.rs:1092-1114`) with `root_name = None` then falls back to `host.virtual_workspace().root_name.clone()` (`:1100`) and emits `ApplyDirectorySync`, causing `applyDirectorySyncCommand` (`index.html:886-933`) to write the newly opened/reset project's `document.json` and `sheets/*.csv` directly into the previous folder's `activeDirectoryHandle` on disk (and remove unmatched prior CSVs after `project.new`, where `last_synced_tree` remains populated).
  2. `connect_wasm_bridge` (`apps/curvebook-web/src/main.rs:76`) initializes `WebBridgeSession::new()` with `doc_id = "default"` (`host_adapters.rs:2263-2265`), and JS sets `activeLockDocId = "default"` (`index.html:260`) and calls `coordinateDocumentLock(activeLockDocId)` only at startup (`index.html:1784`). Neither Rust (`OpenFileBytes`, `ImportDroppedFiles`, `ImportDirectoryEntries`) nor JS (`openWithFilePicker`, `openDirectoryPickerTree`) ever updates `self.doc_id` / `activeLockDocId` across project opens/creations, so distinct projects collide on `curvebook.doc.default` locks and `curvebook.v1.autosave.default` entries (`host_adapters.rs:3321`).

### `FA-SEC20-02` — `CONFIRMED` · Severity agreed (`S1`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `execute_chrome_command_or_dispatch` (`apps/curvebook-web/src/host_adapters.rs:2993`) checks `self.access_mode.is_read_only() && !route.is_read_only_hint() && !is_command_read_only(...)`. Meanwhile `WebCommandRoute::is_read_only_hint` (`apps/curvebook-web/src/chrome_bridge.rs:1460`) returns `true` for every `UiCommand` except `"ui.grid.excludeWithReason"` and `"ui.result.appendClusterColumn"`. Mutating commands mapped to `UiCommand`—including `"graph.applyCanvasPreset"` and `"layout.applyCanvasPreset"` (`chrome_bridge.rs:1536`, executing `app.execute_command(&intent.command_id, intent.params)?` via `dispatch_ui_canvas_apply_preset` at `crates/ui-egui/src/control_io.rs:2757-2767`) as well as mutating `UI_COMMANDS` matched at `chrome_bridge.rs:1553-1556`—bypass `ReadOnlyContended` completely.
  2. `is_command_read_only` (`host_adapters.rs:3669-3688`) treats `"ui.theme.set"`, `"ui.locale.set"`, `"ui.shortcut.set"`, and `"ui.shortcut.reset"` as read-only, while all dispatch paths (`host_adapters.rs:3030, 3261, 3277, 3289, 3314`) unconditionally call `maybe_append_preferences_persist` (`:3122-3141`), emitting `PersistLocalStorage` writes to shared `curvebook.v1.preferences` even when `access_mode.is_read_only()` is true.
  3. `is_command_read_only` (`host_adapters.rs:3646-3656`) checks `"batch.run"`, whereas `crates/ui-egui/src/control_io.rs:1409-1410` dispatches `"command.run"` and `"command.batch"`.

### `FA-SEC20-03` — `CONFIRMED` · Severity agreed (`S1`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `ViewportMetrics::physical_width_px` / `physical_height_px` (`crates/ui-egui/src/interaction.rs:66-78`) clamp up to `16_384` per axis (with `MAX_SCALE_FACTOR = 8.0`). `render_frame_command` (`apps/curvebook-web/src/host_adapters.rs:3341-3354`) passes raw `width_px`/`height_px` into `WebTextBridge::compose_placeholder_frame_with_popup` (`apps/curvebook-web/src/text_bridge.rs:1169-1210`) BEFORE calling `validate_layered_frame` (`:1263`, which itself checks no max pixel cap). `solid_raster_image` (`text_bridge.rs:1309-1315`) eagerly allocates `pixel_count.saturating_mul(4)` bytes (`1 GiB` at `16_384 × 16_384`), and when `modal_active` is true (`text_bridge.rs:1201-1209`), `rasterize_band_layer` (`:1332-1333`) allocates a second `1 GiB` backdrop layer before `WebBridgeCommand::to_json_value` (`host_adapters.rs:2066-2067`) base64-encodes both buffers, exhausting `wasm32` address space (OOM abort). Direct calls with larger `width_px`/`height_px` also lack any budget check.
  2. `LayoutRectPx::from_json_value` (`text_bridge.rs:793-800`) truncates `i64`/`u64` fields with `as i32` / `as u32` (`#[allow(clippy::cast_possible_truncation)]`), and `WebBridgeCommand::from_json_value` (`host_adapters.rs:2139-2140`) silently coerces overflowing `u64` `width_px`/`height_px` via `.unwrap_or(0)` instead of returning `WebAdapterError::Serialization`.

### `FA-SEC20-04` — `CONFIRMED` · Severity agreed (`S2`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. Cross-checking `ActiveDialog` (`crates/ui-egui/src/dialogs_canvas/mod.rs:64-153`, `32` non-`None` variants) and `gui_actions.rs:2579-2641` against `commit_active_modal_dialog` (`apps/curvebook-web/src/chrome_bridge.rs:1696-1963`), six stateful commit variants—`ImportReshape`, `RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`, `ConstantHookPicker`, and `Checkpoints`—fall into `_ => { close_dialog(); ... "dialog.closed" }` (`:1957-1961`) without committing.
  2. For `ActiveDialog::ExportFigure`, `commit_active_modal_dialog` (`chrome_bridge.rs:1831-1841`) executes `app.execute_command(&intent.command_id, intent.params)` and places its result into `RenderRgbaFrame.command_result` rather than using it for file download, while `dispatch_chrome_action` (`host_adapters.rs:2933-2959`) re-runs `self.fs.export_bytes` with only `(sheet_id, web_fmt, dpi)` via `export_figure_artifact` (`host_adapters.rs:1296-1302`), omitting `export_color_space` (`"cmyk"` CMYK-Proof from `gui_actions.rs:2544-2550`) as well as PNG `width_px`/`height_px` and TIFF `compression`.
  3. `handle_modal_key_down` (`chrome_bridge.rs:1287-1360`) handles only `Enter` (`DialogConfirm` at `:1288`), `CommandPalette`, `AnalyzeSheet`, and `ExcludeWithReason`, returning `WebChromeDispatch::BlockedByModalBackdrop` (`:1359`) for the remaining `29` non-`None` `ActiveDialog` variants.
  4. `WebChromeAction::SelectDialogItem`, `DialogStepBack`, `DialogStepNext`, and `FocusModalInput` (`chrome_bridge.rs:586-591`) hit `_ => Ok(Vec::new())` in `dispatch_chrome_action` (`host_adapters.rs:2986`), and confirming `CommandPalette` (`host_adapters.rs:2922-2925`) closes the dialog without clearing `app.focus_widget_id = Some("palette.query")` set at `:3025`.

### `FA-SEC20-05` — `CONFIRMED` · Severity agreed (`S2`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `FileSystemAccessBridge::import_dropped_files` (`apps/curvebook-web/src/host_adapters.rs:1025-1040`) strips `detected_root` once via `normalize_relative_entry_path(&raw_path, detected_root.as_deref())?`, stores the stripped keys into `tree`, and then calls `self.import_directory_tree(host, &root_name, tree)` (`:1040`), which calls `normalize_directory_entries(root, entries)` -> `normalize_relative_entry_path(&path, Some(root_name))?` (`:1001, 1355, 1367-1372`) a second time. A nested folder matching `root_name` loses its inner prefix on the second pass, colliding or silently flattening nested paths.
  2. `collectDroppedEntry` (`apps/curvebook-web/index.html:1459-1466`) passes an unguarded `async (file) => { ... await file.arrayBuffer(); ... resolve(); }` callback to `entry.file(..., reject)`. Because `FileSystemFileEntry.file()` does not await Promises returned by its success callback, a rejection in `file.arrayBuffer()` never invokes `reject`, hanging `await collectDroppedEntry(...)` indefinitely.

### `FA-SEC20-06` — `CONFIRMED` · Severity agreed (`S2`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  - `WebChromeBridge::build_layout` (`apps/curvebook-web/src/chrome_bridge.rs:1056-1058`) emits `"a11y_tree": { "nodes": [{ "id", "label", "enabled" }] }` without `dom_id`, `tag_name`, `attributes`, `parent_dom_id`, `value_text`, `dir`, `locale`, `live_announcement`, or `focused_dom_id`.
  - `mirrorChromeAccessibility` (`apps/curvebook-web/index.html:550-619`) skips every node missing `node.dom_id` (`:565-567`: `if (!node || !node.dom_id) { continue; }`), projecting only `#cb-a11y-live`, and executes `canvas.removeAttribute("aria-activedescendant")` (`:615-617`) whenever `tree.focused_dom_id` is absent.

### `FA-SEC20-07` — `CONFIRMED` · Severity agreed (`S2`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `shouldPreventBrowserShortcut` (`apps/curvebook-web/index.html:1281-1293`) omits `F6`, `Cmd/Ctrl+Shift+A` (`ui.analyze.open`), `Cmd/Ctrl+Shift+G` (`ui.changeGraphType.open`), `Cmd/Ctrl+Shift+F` (`ui.family.toggle`), and `Cmd/Ctrl+Enter` (`analyze.run`), all of which are marked `prevent_default_on_canvas: true` in `WEB_SHORTCUT_MAPPINGS` (`apps/curvebook-web/src/chrome_bridge.rs:278-331`).
  2. `index.html:1295-1323` emits `UiInputEvent::KeyDown` on non-IME `keydown` and `ime_commit` (`UiInputEvent::TextInput` at `host_adapters.rs:2717 -> chrome_bridge.rs:1386`) only on `compositionend`. In `crates/ui-egui/src/interaction.rs:1421-1585`, printable 1-char `KeyDown` is translated to `handle_text_input` only inside active `inline_editor` (`:1437, 1655`) or a focused `is_text_widget_id` textbox (`:1515, 1737`), whereas direct typing on a non-inline-editing data grid cell falls through to `key_down_unhandled` (`:1584`) without reaching `handle_grid_text_input` (`crates/ui-egui/src/views/grid_core.rs:233`, reachable only via `UiInputEvent::TextInput`).

### `FA-SEC20-08` — `CONFIRMED` · Severity agreed (`S2`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `applyLocalStorageCommand` (`apps/curvebook-web/index.html:680-683`) checks `if (!window.localStorage)` outside `try/catch` (throwing `SecurityError` in storage-blocked contexts) and returns without emitting `{ kind: "confirm_local_storage", succeeded: false }`, stranding un-reconciled `StorageRollbackOp` records in `WebBridgeSession::pending_storage_ops` (`apps/curvebook-web/src/host_adapters.rs:3109-3141, 3322, 3165-3176`).
  2. `handle_hydrate_local_storage` (`host_adapters.rs:3198-3200`) replays `prefs.shortcuts` one binding at a time via `shortcuts.set_shortcut(cmd_id, combo)` (`let _ =`), which rejects swapped shortcuts against default bindings (`crates/ui-egui/src/menus/shortcuts.rs:217-219`) instead of using `import_overrides_json` (`shortcuts.rs:269-310`).
  3. Line `3215` calls `self.storage.load_autosave_snapshot(&self.doc_id, 0)`, resetting the restored autosave's LRU recency (`host_adapters.rs:438`) to `(0, seq)` so `oldest_autosave_key` (`:608-613`) evicts it ahead of entries with real wall-clock timestamps. Additionally, `tryAcquireFallbackStorageLock` (`index.html:1594-1608`) does not clear any pre-existing `fallbackLockTimerId` before calling `setInterval` and falls through the `catch` block to `holdsCurrentWebLock = true` if `localStorage` throws.

### `FA-SEC20-09` — `CONFIRMED` · Severity agreed (`S3`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  1. `WebChromeBridge::build_layout` (`apps/curvebook-web/src/chrome_bridge.rs:823-842`) computes dropdown popup height `pop_h = items.len() * 14 + 8` without clamping to viewport height `vh` (while `compose_placeholder_frame_with_popup` caps the visual box height at `h.saturating_sub(32).min(240)` at `apps/curvebook-web/src/text_bridge.rs:1236`), and `route_wheel` (`chrome_bridge.rs:1218-1220`) returns `popup.wheel_ignored` whenever a dropdown or context menu is open.
  2. `chrome_bridge.rs:874-875` hardcodes `let enabled = btn.tooltip.enabled || btn.button.id == "toolbar.axes";`; `context_menu_items_for_target` (`:2011-2035`) has no production callers (`rg` confirmed zero callers outside its definition), while right-click `build_layout` (`:1007-1023`) emits a single `200×120` `DismissContextMenu` node and `handle_pointer_up` (`:1173-1179`) passes `PointerButton::Secondary` releases straight to the workspace.
  3. `render_frame_command` (`apps/curvebook-web/src/host_adapters.rs:3345-3353`) forwards only `status_summary` and `popup_open` to `compose_placeholder_frame_with_popup`, painting `"Menu"` (`text_bridge.rs:1246`) rather than `WebChromeLayout` labels.

### `FA-SEC20-10` — `CONFIRMED` · Severity agreed (`S3`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  - `BrowserCanvasTextMeasurer::shape_line` (`apps/curvebook-web/src/text_bridge.rs:593-639`) runs an empty `for _run in shape_bidi_runs(&visual_text, direction) {}` loop (`:603-605`), sums per-codepoint advances using fixed `eff_px * 0.60` (`UiMono`) / `eff_px * 0.56` (`tabular_nums` digits) or isolated single-char `self.measure(style, s, dpi).width_px` calls (`:606-616`), and then applies uniform linear rescaling `scale = metrics.width_px / pen_x` (`:628-636`) against system CSS fallback stacks (`css_family_stack` at `:42-48`), introducing glyph `x_px` drift on kerning pairs, ligatures, and contextual scripts.
