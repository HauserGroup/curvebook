# Audit Report: SEC-20 — Web / WASM Application

- **Section:** `SEC-20` (Web / WASM Application — `7` web source/asset files + `1` manifest = `8` files, `11,275` LOC)
- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Date:** `2026-11-22`
- **Scope:**
  - `apps/curvebook-web/Cargo.toml`, `apps/curvebook-web/Trunk.toml`, `apps/curvebook-web/index.html`
  - `apps/curvebook-web/src/`: `chrome_bridge.rs`, `host_adapters.rs`, `lib.rs`, `main.rs`, `text_bridge.rs`
- **Mode:** Report-only static audit (`100%` file coverage `1..=N`, cross-checked against downstream handlers in `crates/ui-egui/src/{control_io.rs, interaction.rs, gui_actions.rs, os_integration.rs, views/grid_core.rs, menus/shortcuts.rs}`, `crates/svg/src/writer.rs`, `crates/engine/src/cmd/export.rs`, and baseline tracking in `docs/audit/local-code-audit-2026-11-18.md:1286-1365`).
- **Summary Counts:** `S1 = 3`, `S2 = 5`, `S3 = 2`, `S4 = 0` (**10 findings**; `FA-SEC20-01` .. `FA-SEC20-10`).

---

## File Coverage Table

| File | LOC | Lines Inspected | Invariants Checked | Findings / Status |
|---|---:|---|---|---|
| `apps/curvebook-web/Cargo.toml` | 27 | `1..=27` | `[lints] workspace = true`, `publish = false`, zero `chrono` or HTTP client deps, `wasm32`-only `js-sys`/`wasm-bindgen`/`web-sys` gating | Clean — verified workspace lint inheritance, target-gated JS/WASM deps, and zero network/date crates. |
| `apps/curvebook-web/Trunk.toml` | 18 | `1..=18` | Loopback-only `[serve]` host (`127.0.0.1:8767`), relative `public_url = "./"`, `cargo xtask web-fonts` hook without shell expansion | Clean — verified loopback bind address and non-shell `xtask` build hook. |
| `apps/curvebook-web/index.html` | 1,807 | `1..=1807` | Strict CSP (`default-src 'self'`, `connect-src 'self'`, `object-src 'none'`), zero `fetch`/`XHR`/`WebSocket`/`sendBeacon`, print `DOMParser` SVG insertion & XML escaping, File System Access / ZIP confirmation, `localStorage` rollback, `navigator.locks`, keyboard/IME/drop/a11y bridges | `FA-SEC20-01`, `FA-SEC20-05`, `FA-SEC20-06`, `FA-SEC20-07`, `FA-SEC20-08` |
| `apps/curvebook-web/src/chrome_bridge.rs` | 2,254 | `1..=2254` | `#![forbid(unsafe_code)]`, `WebCommandRoute::is_read_only_hint`, `commit_active_modal_dialog` vs `ui-egui`, `handle_modal_key_down`, `WEB_SHORTCUT_MAPPINGS`, `a11y_tree` schema, popup/dropdown bounds, context menu items | `FA-SEC20-02`, `FA-SEC20-04`, `FA-SEC20-06`, `FA-SEC20-07`, `FA-SEC20-09` |
| `apps/curvebook-web/src/host_adapters.rs` | 4,623 | `1..=4623` | `#![forbid(unsafe_code)]`, read-only contention enforcement across all dispatchers, `doc_id` lifecycle, `#123` storage quota/rollback, FS diff confirmation & dropped folder normalization, explicit `sheet_id` targeting, pinch-vs-press cancellation | `FA-SEC20-01`, `FA-SEC20-02`, `FA-SEC20-03`, `FA-SEC20-04`, `FA-SEC20-05`, `FA-SEC20-08`, `FA-SEC20-09` |
| `apps/curvebook-web/src/lib.rs` | 760 | `1..=760` | `#![forbid(unsafe_code)]`, `WebPlatformUrlOpener` allowlist check, `install_session` workspace cleanup, `confirm_virtual_directory_persisted` dirty-flag check, `snapshot_virtual_entries_with_root` non-clearing preparation | `FA-SEC20-01` |
| `apps/curvebook-web/src/main.rs` | 141 | `1..=141` | `#![forbid(unsafe_code)]`, WASM callback `try_borrow_mut` re-entrancy guard & error notice fallback, initial session `doc_id` wiring | `FA-SEC20-01` |
| `apps/curvebook-web/src/text_bridge.rs` | 1,645 | `1..=1645` | `#![forbid(unsafe_code)]`, `RefCell` cache (no `Mutex`), pixel buffer OOM bounds before validation, checked integer casts in `LayoutRectPx`, BiDi/per-codepoint text shaping & fallback behavior | `FA-SEC20-03`, `FA-SEC20-09`, `FA-SEC20-10` |

---

## Baseline `#123` (`issue #21`) Verification Status

All six browser host adapter fixes recorded under `#123` (`docs/audit/local-code-audit-2026-11-18.md:1286-1295`) were re-inspected across Rust and JS:

| `#123` Item | Status at `7c85a8e5` | Verified Evidence & Residual Gaps |
|---|---|---|
| **1. Waiting tab stays read-only after lock grant** | **Core fix intact; adjacent gap** | `WebLockCoordinator::grant_browser_lock` (`host_adapters.rs:839-847`) and `AcquireLock` (`:2531-2537`) overwrite cached observations inside the 15s lease TTL; `index.html:1615-1630, 1690-1723` tracks queued `navigator.locks.request` with `AbortController` and `lockGeneration`. **Residual:** `doc_id` stays `"default"` across project loads (`FA-SEC20-01`), and `tryAcquireFallbackStorageLock` has interval/quota gaps (`FA-SEC20-08`). |
| **2. Folder save advances baseline & clears dirty before I/O** | **Fixed for sync/ZIP; cross-project gap** | `sync_directory_from_host` (`host_adapters.rs:1092-1114`) stages `PendingDirectorySync` without mutating `last_synced_tree` or clearing dirty; `confirm_directory_sync` (`:1123-1172`) commits only confirmed paths and calls `confirm_virtual_directory_persisted` (`lib.rs:439-453`), which verifies `document.json` equality before clearing dirty; ZIP export uses `snapshot_virtual_entries_with_root` (`lib.rs:428-435`). **Residual:** stale `virtual_workspace` and JS `activeDirectoryHandle` survive single-file opens (`FA-SEC20-01`). |
| **3. Preferences near quota drop eviction list** | **Fixed for `localStorage`; missing-storage gap** | `save_preferences_tracked` (`host_adapters.rs:358-376`), `write_with_safe_eviction_tracked` (`:558-606`), `reconcile_confirmed_storage` (`:533-553`), and `maybe_append_preferences_persist` (`:3122-3141`) propagate backups and reconcile on `ConfirmLocalStorage`; `index.html:685-771` writes first, retries evictions on quota failure, rolls back, and confirms. **Residual:** silent no-op return when `!window.localStorage` leaves `pending_storage_ops` un-rolled-back (`FA-SEC20-08`). |
| **4. Export and copy return wrong sheet** | **Verified Fixed (Clean)** | `export_csv_artifact` (`host_adapters.rs:1239-1252`), `export_figure_artifact` (`:1269-1294`), `resolve_clipboard_target` (`:3385-3442`), and `build_data_clipboard_payload` (`:3444-3469`) validate explicit `sheet_id`s with errors instead of silent fallback and emit tabular-only clipboard items for explicit data sheets. |
| **5. Dropped folders accept duplicate paths** | **Partially Fixed / Residual Bug** | `import_dropped_files` (`host_adapters.rs:1025-1033`) and `collect_unique_directory_entries` (`:3865-3882`) reject duplicate normalized paths before building the map. **Residual:** `import_dropped_files` passes `tree` to `import_directory_tree`, which calls `normalize_directory_entries` and strips `detected_root` a second time (`FA-SEC20-05`). |
| **6. Pinch after 1-finger press leaves pointer down** | **Verified Fixed (Clean)** | `TouchGestureRecognizer::touch_start` / `touch_move` (`host_adapters.rs:1424-1493`) emit synthetic `PointerUp { x: -1.0, y: -1.0 }` upon entering two-finger mode, and `TouchStart` / `TouchMove` / `TouchCancel` (`:2460-2484`) clear `app.interaction` drag/press fields via `cancel_armed_pointer_and_drags` (`:3590-3602`). |

---

## Findings

### `FA-SEC20-01` · `S1` · Stale `virtual_workspace` / `activeDirectoryHandle` and hardcoded `"default"` `doc_id` overwrite wrong folders on disk and collide locks/autosaves across projects

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/lib.rs:268-274`, `:531-539` (`install_session`), `apps/curvebook-web/src/host_adapters.rs:977-984` (`import_file_bytes`), `:1092-1105` (`sync_directory_from_host`), `:2263-2265`, `:2502-2525` (`OpenFileBytes`, `SyncDirectory`), `apps/curvebook-web/src/main.rs:76`, `apps/curvebook-web/index.html:259-260`, `:886-933`, `:1440-1445`, `:1784`
- **Category:** `data-loss` / `security`
- **Defect:**
  1. Opening a single archive or delimited file (`install_session` at `lib.rs:531-539`, `open_delimited_bytes` at `:268-274`) replaces `*self.app.session_mut()` without resetting `self.app.virtual_workspace`, and `host_adapters.rs:2502-2507` emits no command to clear JS `activeDirectoryHandle` (`index.html:259`, assigned at `:1441`). Because `sync_directory_from_host` (`host_adapters.rs:1093-1105`) falls back to `host.virtual_workspace().root_name` whenever `self.last_synced_root` is `None` (`import_file_bytes` at `:979`), and `applyDirectorySyncCommand` (`index.html:886-915`) writes/deletes directly through `activeDirectoryHandle`, triggering `SyncDirectory` after opening a different project writes the new project's files into (and removes previous sheet CSVs from) the previously opened `.curvebook.d` directory on disk.
  2. `connect_wasm_bridge` (`main.rs:76`) starts `WebBridgeSession::new()` with `doc_id = "default"` (`host_adapters.rs:2263-2265`) and JS invokes `coordinateDocumentLock(activeLockDocId)` only at startup with `"default"` (`index.html:260, 1784`). `OpenFileBytes`, `ImportDroppedFiles`, `ImportDirectoryEntries`, and `project.new` never update `self.doc_id`, JS `activeLockDocId`, or clear/rotate `curvebook.v1.autosave.default`, so unrelated documents across tabs collide on `curvebook.doc.default` (blocking edits with false read-only contention) and overwrite one another's `localStorage` autosave recovery entry.
- **Failure scenario:** A user opens directory `study_a.curvebook.d` (setting `activeDirectoryHandle` and `app.virtual_workspace`), then opens `study_b.curvebook` via the file picker (`OpenFileBytes`) or resets with `project.new`, and triggers folder sync: `study_b`'s `document.json` and `sheets/*.csv` overwrite `study_a.curvebook.d/` on disk while removing `study_a`'s unmatched CSV files. Opening two distinct files in separate browser tabs locks the second tab read-only under `curvebook.doc.default` and clobbers `curvebook.v1.autosave.default`.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/lib.rs:531-539
  fn install_session(&mut self, mut session: Session, tag: &str, warnings: Vec<String>, mark_clean: bool) {
      if mark_clean {
          session.set_file_path(None, true);
      }
      *self.app.session_mut() = session;
      self.app.sync_views();
      self.format_tag = String::from(tag);
      self.warnings = warnings;
  }
  ```
  ```rust
  // apps/curvebook-web/src/host_adapters.rs:1092-1101
  pub fn sync_directory_from_host(&mut self, host: &mut WebWorkspaceHost, root_name: Option<&str>) -> Result<DirectorySyncPlan, WebAdapterError> {
      let resolved_root = if let Some(explicit) = root_name.map(str::trim).filter(|s| !s.is_empty()) {
          explicit.to_owned()
      } else if let Some(prev) = &self.last_synced_root {
          prev.clone()
      } else if let Some(attempted) = &self.last_attempted_root {
          attempted.clone()
      } else {
          host.virtual_workspace().root_name.clone()
      };
  ```
  ```js
  // apps/curvebook-web/index.html:259-260, 886-888, 1784
  let activeDirectoryHandle = null;
  let activeLockDocId = "default";
  ...
  async function applyDirectorySyncCommand(cmd) {
    const rootName = (cmd && cmd.root_name) ? String(cmd.root_name) : "workspace.curvebook.d";
    if (!activeDirectoryHandle) { ... }
    ...
    coordinateDocumentLock(activeLockDocId);
  ```
- **Suggested fix:** Reset `self.app.virtual_workspace` inside `install_session` / `open_delimited_bytes` / `project.new`, emit an explicit bridge command to clear `activeDirectoryHandle` whenever a non-directory project is loaded or reset (and require matching folder name before `applyDirectorySyncCommand` writes/removes files), and derive and broadcast a per-document lock/autosave ID whenever a document is opened or created.
- **Related:** `#123.1`, `#123.2`.

---

### `FA-SEC20-02` · `S1` · Contended read-only lock bypassed by `WebCommandRoute::UiCommand` document mutations and origin-wide preference writes

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/chrome_bridge.rs:1456-1463` (`WebCommandRoute::is_read_only_hint`), `:1536`, `apps/curvebook-web/src/host_adapters.rs:2990-2996`, `:3017-3031` (`execute_chrome_command_or_dispatch`), `:3268-3315` (`handle_execute_command`, `handle_control_line`), `:3646-3689` (`is_command_read_only`) (cross-checked with `crates/ui-egui/src/control_io.rs:1409-1410`, `:2765-2767`)
- **Category:** `security` / `contract-drift`
- **Defect:**
  1. `execute_chrome_command_or_dispatch` (`host_adapters.rs:2993`) enforces the read-only lock only when **both** `!route.is_read_only_hint()` and `!is_command_read_only(...)` hold. Because `WebCommandRoute::is_read_only_hint` (`chrome_bridge.rs:1460`) returns `true` for every `UiCommand` other than `"ui.grid.excludeWithReason"` and `"ui.result.appendClusterColumn"`, commands mapped to `UiCommand` that mutate the document—specifically `"graph.applyCanvasPreset"` and `"layout.applyCanvasPreset"` (routed to `"ui.canvas.applyPreset"` at `chrome_bridge.rs:1536`, which executes `app.execute_command(&intent.command_id, intent.params)?` in `crates/ui-egui/src/control_io.rs:2765-2767`)—bypass `self.access_mode.is_read_only()` completely.
  2. `is_command_read_only` (`host_adapters.rs:3669-3688`) treats `"ui.theme.set"`, `"ui.locale.set"`, `"ui.shortcut.set"`, and `"ui.shortcut.reset"` as read-only, while all four dispatch paths (`host_adapters.rs:3030, 3277, 3289, 3314`) unconditionally call `maybe_append_preferences_persist`, persisting preference changes from a contended read-only tab into the shared `curvebook.v1.preferences` `localStorage` entry.
  3. Additionally, `is_command_read_only` (`host_adapters.rs:3646-3656`) inspects `"batch.run"` instead of the control dispatcher's `"command.batch"` (`crates/ui-egui/src/control_io.rs:1410`) and does not unwrap `"command.run"` (`:1409`), rejecting read-only `"command.batch"` requests while relying on default rejection for `"command.run"`.
- **Failure scenario:** In a browser tab held read-only due to active lock contention from another tab, invoking `graph.applyCanvasPreset`, `layout.applyCanvasPreset`, or `ui.canvas.applyPreset:<preset>` via chrome or Command Palette succeeds without `ReadOnlyContended` and mutates the graph/layout document; changing theme/locale/shortcuts persists writes into origin-shared `localStorage` despite read-only lock mode.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/chrome_bridge.rs:1456-1463, 1536
  pub fn is_read_only_hint(&self) -> bool {
      match self {
          Self::OpenFilePicker { .. } | Self::OpenEditAnalysisParameters(_) => false,
          Self::ExportFormat { .. } | Self::PrintCurrentSheet | Self::NoticeOnly { .. } => true,
          Self::UiCommand { command_id, .. } => !matches!(command_id.as_str(), "ui.grid.excludeWithReason" | "ui.result.appendClusterColumn"),
          Self::EngineCommand { command_id, .. } => cmd::registry().get(command_id).is_some_and(|spec| spec.effects.read_only_hint()),
      }
  }
  ...
      "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => Some("ui.canvas.applyPreset"),
  ```
  ```rust
  // apps/curvebook-web/src/host_adapters.rs:2993-2995
  if self.access_mode.is_read_only() && !route.is_read_only_hint() && !is_command_read_only(command_id, Some(&params)) {
      return Err(self.read_only_err(command_id));
  }
  ```
  ```rust
  // crates/ui-egui/src/control_io.rs:2765-2767
  let is_layout = app.document().layout_sheet(&sheet_id).is_some();
  let intent = app.canvas_dialogs.apply_canvas_preset_intent(&sheet_id, is_layout, preset);
  app.execute_command(&intent.command_id, intent.params)?;
  ```
- **Suggested fix:** Replace the negative blacklist in `WebCommandRoute::is_read_only_hint` with an explicit read-only allowlist (excluding `"ui.canvas.applyPreset"` and other mutating UI commands), check read-only authorization with `||` (or require explicit read-only confirmation on both), gate `maybe_append_preferences_persist` on `!self.access_mode.is_read_only()`, and align `is_command_read_only` with `"command.batch"` and `"command.run"`.
- **Related:** `#123.1`.

---

### `FA-SEC20-03` · `S1` · Uncapped pixel allocations in `WebTextBridge` abort `wasm32` on OOM before validation; unchecked integer truncation in `LayoutRectPx::from_json_value`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/text_bridge.rs:791-802` (`LayoutRectPx::from_json_value`), `:1169-1180` (`compose_placeholder_frame_with_popup`), `:1309-1316` (`solid_raster_image`), `:1318-1339` (`rasterize_band_layer`), `apps/curvebook-web/src/host_adapters.rs:2138-2140` (`WebBridgeCommand::from_json_value`), `:3337-3355` (`render_frame_command`) (cross-checked with `crates/ui-egui/src/interaction.rs:40-75`)
- **Category:** `no-panic` / `resources`
- **Defect:**
  1. `ViewportMetrics::physical_width_px` and `physical_height_px` (`crates/ui-egui/src/interaction.rs:66-75`) permit dimensions up to `16_384 × 16_384` per `ResizeViewport` event (`host_adapters.rs:2455-2459`), and public constructor `WebTextBridge::compose_placeholder_frame_with_popup` (`text_bridge.rs:1169-1180`) passes raw `width_px` and `height_px` with only `.max(1)` clamping into `solid_raster_image` (`text_bridge.rs:1309-1316`) and `rasterize_band_layer` (`:1332-1333`). Both functions eagerly allocate `Vec::with_capacity(pixel_count.saturating_mul(4))` and `vec![0; byte_len]` before any maximum-pixel budget check or `validate_layered_frame` (`:1263`). At `16_384 × 16_384`, a modal frame allocates `1 GiB` for the base frame, `1 GiB` for the modal backdrop band, plus `~1.33 GiB` base64 JSON encoding (`encode_base64_slice`), exhausting 32-bit `wasm32` address space (`OOM` / abort).
  2. `LayoutRectPx::from_json_value` (`text_bridge.rs:793-800`) truncates arbitrary JSON `i64`/`u64` numbers directly via `as i32` and `as u32`, wrapping out-of-range dimensions modulo `2^32`; similarly `WebBridgeCommand::from_json_value` (`host_adapters.rs:2139-2140`) coerces overflowing `u64` `width_px`/`height_px` to `0` instead of returning `WebAdapterError::Serialization`.
- **Failure scenario:** A high-DPI browser/WebGPU canvas at `16_384 × 16_384` (or a bridge `ResizeViewport` event / direct call to `compose_placeholder_frame_with_popup`) attempts multi-gigabyte RGBA + base64 allocations per rendered frame and aborts the WASM instance on allocation failure. Deserializing a JSON rect with `width = 4_294_967_297` silently wraps to `1 px`.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/text_bridge.rs:793-800, 1178-1180, 1309-1315
  #[allow(clippy::cast_possible_truncation)]
  let x = obj.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
  ...
  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  let width = obj.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
  ...
  let w = width_px.max(1);
  let h = height_px.max(1);
  let base_img = solid_raster_image(w, h, dpi.max(72), [surface_bg[0], surface_bg[1], surface_bg[2], 255]);
  ...
  fn solid_raster_image(width_px: u32, height_px: u32, dpi: u16, rgba: [u8; 4]) -> RasterImage {
      let pixel_count = (width_px as usize).saturating_mul(height_px as usize);
      let mut bytes = Vec::with_capacity(pixel_count.saturating_mul(4));
  ```
- **Suggested fix:** Enforce an explicit total-pixel/byte cap (e.g., `<= 64 MiB` total frame buffer) and checked `try_reserve` / `Result` error return before allocating RGBA buffers in `WebTextBridge` and `render_frame_command`, and replace `as i32`/`as u32` casts in `LayoutRectPx::from_json_value` and `WebBridgeCommand::from_json_value` with `i32::try_from` / `u32::try_from` validation.
- **Related:** None.

---

### `FA-SEC20-04` · `S2` · `commit_active_modal_dialog` silently discards 6 stateful dialogs, double-exports `ExportFigure` without options, and `handle_modal_key_down` blocks 29 dialogs

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/chrome_bridge.rs:1287-1360` (`handle_modal_key_down`), `:1696-1963` (`commit_active_modal_dialog`), `:1831-1841` (`ExportFigure`), `:1957-1961`, `apps/curvebook-web/src/host_adapters.rs:2915-2960` (`DialogConfirm`), `:2986` (`_ => Ok(Vec::new())`), `:3023-3026` (`ui.palette.open`) (cross-checked with `crates/ui-egui/src/gui_actions.rs:2542-2561`, `:2579-2641`)
- **Category:** `data-loss` / `contract-drift` / `wrong-result`
- **Defect:**
  1. `commit_active_modal_dialog` (`chrome_bridge.rs:1696-1963`) drops six active `ActiveDialog` variants handled in `crates/ui-egui/src/gui_actions.rs:2579-2641` into its `_ => { close_dialog(); ... "dialog.closed" }` catch-all (`:1957-1961`): `ImportReshape`, `RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`, `ConstantHookPicker`, and `Checkpoints` — closing the modal cleanly while silently discarding the pending commit.
  2. For `ActiveDialog::ExportFigure`, `commit_active_modal_dialog` (`chrome_bridge.rs:1831-1841`) executes `export_figure.commit_intent(&sheet_id)` and discards the exported bytes, after which `host_adapters.rs:2933-2959` re-exports a second time with only `{ sheet_id, dpi }` (`:1296-1302`), omitting `export_color_space` (`"cmyk"` CMYK-Proof) applied in `crates/ui-egui/src/gui_actions.rs:2544-2550`.
  3. `handle_modal_key_down` (`chrome_bridge.rs:1287-1360`) only accepts key presses inside `CommandPalette`, `AnalyzeSheet`, and `ExcludeWithReason`, returning `WebChromeDispatch::BlockedByModalBackdrop` at `:1359` for all remaining modal dialogs (including `FormatAxes`, `EditAnalysisParameters`, `Checkpoints`, `ImportReshape`, `Print`), blocking text editing inside those dialogs on web.
  4. In `dispatch_chrome_action` (`host_adapters.rs:2915-2987`), `WebChromeAction::SelectDialogItem`, `DialogStepBack`, `DialogStepNext`, and `FocusModalInput` hit `_ => Ok(Vec::new())` (`:2986`), and confirming `CommandPalette` at `:2922-2925` closes the dialog without clearing `app.focus_widget_id = Some("palette.query")` set at `:3025`.
- **Failure scenario:** Clicking OK on Checkpoints, Simulate Distributions, Randomize Groups, Random Sample Rows, Import Reshape, or Constant Hook Picker closes the dialog (`dialog.closed`) with zero changes applied; confirming `ExportFigure` in CMYK-Proof PDF mode silently downloads an sRGB PDF; typing into `FormatAxes` or `EditAnalysisParameters` is blocked; clicking a palette result row (`SelectDialogItem`) or wizard next/back step (`DialogStepNext`/`DialogStepBack`) does nothing.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/chrome_bridge.rs:1831-1841, 1957-1961
  ActiveDialog::ExportFigure { sheet_id } => {
      let intent = app.canvas_dialogs.export_figure.commit_intent(&sheet_id);
      let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
      app.canvas_dialogs.close_dialog();
      ...
  }
  ...
  _ => {
      app.canvas_dialogs.close_dialog();
      curvebook_ui_egui::interaction::clear_modal_text_buffers(app);
      Ok(UiEventOutcome { handled: true, action: String::from("dialog.closed"), ..Default::default() })
  }
  ```
  ```rust
  // apps/curvebook-web/src/host_adapters.rs:2951-2959, 2986
  if let Some((export_sid, web_fmt, export_dpi)) = export_info {
      let art = self.fs.export_bytes(&mut self.host, web_fmt, Some(export_sid.as_str()), export_dpi)?;
      ...
  }
  ...
  _ => Ok(Vec::new()),
  ```
- **Suggested fix:** Port the missing 6 dialog commit arms from `crates/ui-egui/src/gui_actions.rs:2579-2641` into `commit_active_modal_dialog`, return the single `ExportFigure` export payload directly (passing `color_space` and `transparent_bg`), clear `focus_widget_id` on palette confirm, handle `SelectDialogItem`/`DialogStepBack`/`DialogStepNext`/`FocusModalInput` in `dispatch_chrome_action`, and forward non-Confirm modal keys to `ui-egui` textbox/modal key handlers.
- **Related:** `SEC-19` (`FA-SEC19-02`).

---

### `FA-SEC20-05` · `S2` · `import_dropped_files` strips detected folder root prefixes twice and `collectDroppedEntry` hangs permanently if `file.arrayBuffer()` rejects

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/host_adapters.rs:991-1041` (`import_directory_tree`, `import_dropped_files`), `:1352-1377` (`normalize_directory_entries`, `normalize_relative_entry_path`), `apps/curvebook-web/index.html:1457-1479` (`collectDroppedEntry`)
- **Category:** `data-loss` / `wrong-result` / `no-panic`
- **Defect:**
  1. `import_dropped_files` (`host_adapters.rs:1025-1040`) strips `detected_root` from every entry via `normalize_relative_entry_path(&raw_path, detected_root.as_deref())`, checks for duplicates in `tree`, and then forwards `tree` with `&root_name = detected_root` into `import_directory_tree(host, &root_name, tree)` (`:1040`), which runs `normalize_directory_entries(root, entries)` (`:1001`) -> `normalize_relative_entry_path(&path, Some(root_name))` (`:1355`) a second time.
  2. In `index.html:1459-1466`, `collectDroppedEntry` passes an `async` callback `(async (file) => { await file.arrayBuffer(); ... resolve(); })` to `entry.file(successCb, reject)`. Because `FileSystemFileEntry.file()` does not track Promises returned by `successCb`, any rejection from `file.arrayBuffer()` triggers an unhandled promise rejection instead of calling `reject()`, causing `await collectDroppedEntry(...)` to hang permanently.
- **Failure scenario:** Dropping a directory `study.curvebook.d` that contains both `study.curvebook.d/document.json` and a nested subfolder with the same name (`study.curvebook.d/study.curvebook.d/document.json` or `study.curvebook.d/study.curvebook.d/sheets/s1.csv`) strips the inner `study.curvebook.d/` prefix during the second pass—either raising a spurious `duplicate normalized directory entry path` error or silently flattening nested files into the root. Dropping a directory containing an unreadable/revoked file hangs the browser drop handler Promise forever.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/host_adapters.rs:1025-1040
  let detected_root = detect_dropped_directory_root(&files);
  let mut tree = BTreeMap::new();
  for (raw_path, bytes) in files {
      let rel = normalize_relative_entry_path(&raw_path, detected_root.as_deref())?;
      if tree.contains_key(&rel) {
          return Err(WebAdapterError::Validation(format!("duplicate normalized directory entry path `{rel}`")));
      }
      tree.insert(rel, bytes);
  }
  ...
  let root_name = detected_root.unwrap_or_else(|| String::from("dropped.curvebook.d"));
  self.import_directory_tree(host, &root_name, tree)
  ```
  ```js
  // apps/curvebook-web/index.html:1459-1466
  await new Promise((resolve, reject) => {
    entry.file(async (file) => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const rel = prefix ? (prefix + "/" + file.name) : file.name;
      out.push({ path: rel, bytes_base64: bytesToBase64(bytes) });
      resolve();
    }, reject);
  });
  ```
- **Suggested fix:** Avoid re-stripping `root_name` when `import_dropped_files` has already removed `detected_root` (pass `None` as root prefix on already-relative paths or insert into a dedicated already-normalized internal helper), and wrap the `async` body inside `collectDroppedEntry` with `try { ... resolve(); } catch (err) { reject(err); }`.
- **Related:** `#123.5`.

---

### `FA-SEC20-06` · `S2` · `WebChromeSnapshot::a11y_tree` schema mismatches `index.html` DOM projector, leaving accessibility tree empty and `aria-activedescendant` unset

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/chrome_bridge.rs:1056-1058` (`build_layout`), `apps/curvebook-web/index.html:550-619` (`mirrorChromeAccessibility`)
- **Category:** `contract-drift` / `wrong-result`
- **Defect:**
  `WebChromeBridge::build_layout` (`chrome_bridge.rs:1056-1058`) constructs `a11y_tree` with `nodes` containing `{ "id", "label", "enabled" }`, omitting `dom_id`, `tag_name`, `attributes`, `parent_dom_id`, `value_text`, `dir`, `locale`, `live_announcement`, and top-level `focused_dom_id`. In `index.html:564-567, 613-617`, `mirrorChromeAccessibility` skips any node where `!node.dom_id` (`continue`) and clears `canvas`'s `aria-activedescendant` whenever `tree.focused_dom_id` is absent.
- **Failure scenario:** On every rendered frame in the browser, `#curvebook_a11y_chrome` contains zero projected menu, toolbar, modal, or status DOM elements (only `#cb-a11y-live`), and `canvas.removeAttribute("aria-activedescendant")` runs unconditionally, breaking screen-reader navigation of Web chrome controls.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/chrome_bridge.rs:1056-1058
  a11y_tree: Some(Box::new(json!({
      "nodes": nodes.iter().map(|n| json!({ "id": n.id, "label": n.label, "enabled": n.enabled })).collect::<Vec<_>>(),
  }))),
  ```
  ```js
  // apps/curvebook-web/index.html:564-567, 613-617
  for (const node of tree.nodes) {
    if (!node || !node.dom_id) {
      continue;
    }
  ...
  if (tree && typeof tree.focused_dom_id === "string" && tree.focused_dom_id.length > 0) {
    canvas.setAttribute("aria-activedescendant", tree.focused_dom_id);
  } else {
    canvas.removeAttribute("aria-activedescendant");
  }
  ```
- **Suggested fix:** Emit the full schema expected by `mirrorChromeAccessibility` (`dom_id`, `tag_name`, ARIA `attributes`, `focused_dom_id`, `dir`, `locale`, `live_announcement`) from `WebChromeBridge::build_layout`, derived directly from `nodes`, `focused_id`, `app.locale`, and `active_dialog`.
- **Related:** None.

---

### `FA-SEC20-07` · `S2` · `index.html` never emits `UiInputEvent::TextInput` for non-IME keys and omits 5 `WEB_SHORTCUT_MAPPINGS` keys from `shouldPreventBrowserShortcut`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/index.html:1281-1323` (`shouldPreventBrowserShortcut`, `keydown` / `compositionend` listeners), `apps/curvebook-web/src/chrome_bridge.rs:279-331` (`WEB_SHORTCUT_MAPPINGS`), `:1262-1285` (`handle_key_down`) (cross-checked with `crates/ui-egui/src/interaction.rs:1544-1585`, `:1644-1657`, `crates/ui-egui/src/views/grid_core.rs:191-249`)
- **Category:** `contract-drift` / `wrong-result`
- **Defect:**
  1. `shouldPreventBrowserShortcut` (`index.html:1281-1293`) allows default browser behavior for `F6` (and `Shift+F6`), `Cmd/Ctrl+Shift+A`, `Cmd/Ctrl+Shift+F`, `Cmd/Ctrl+Shift+G`, and `Cmd/Ctrl+Enter`, even though `WEB_SHORTCUT_MAPPINGS` (`chrome_bridge.rs:279-331`) explicitly marks all five `prevent_default_on_canvas: true` for `ui.pane.cycle`, `ui.analyze.open`, `ui.family.toggle`, `ui.changeGraphType.open`, and `analyze.run`.
  2. `index.html:1295-1323` dispatches `UiInputEvent::KeyDown` on every non-IME `keydown` and `ime_commit` only on `compositionend`, never emitting `UiInputEvent::TextInput` for regular Latin/ASCII printable keystrokes. Meanwhile `crates/ui-egui/src/interaction.rs:1544-1585` only handles navigation and shortcut combos in `handle_key_down`, leaving `UiInputEvent::TextInput` paths such as `handle_grid_text_input` (`crates/ui-egui/src/views/grid_core.rs:233`, e.g., frozen/linked-cell guard prompts and non-inline-edit direct cell entry paths) unreachable from non-IME typing in the browser.
- **Failure scenario:** Pressing `F6` on the canvas jumps focus to the browser omnibox/tab bar instead of staying strictly in-canvas; pressing `Ctrl+Shift+G`/`Ctrl+Shift+F` triggers browser find-next/fullscreen behaviors alongside app handling; typing ordinary ASCII characters on a frozen or linked data table cell never invokes `handle_grid_text_input` (`UiInputEvent::TextInput`) until an IME composition commits.
- **Evidence:**
  ```js
  // apps/curvebook-web/index.html:1281-1293
  function shouldPreventBrowserShortcut(ev) {
    const lower = (ev.key || "").toLowerCase();
    if (ev.key === "Tab") {
      return true;
    }
    if (ev.altKey && !ev.metaKey && !ev.ctrlKey) {
      return ["w", "t", "n", "b", "i", "p", "l", "k", "0", "e"].includes(lower);
    }
    if ((ev.metaKey || ev.ctrlKey) && !ev.altKey) {
      return ["c", "p", "o", "s", "k", "t", "z", "y", "e", "0", "\\"].includes(lower);
    }
    return false;
  }
  ```
  ```rust
  // apps/curvebook-web/src/chrome_bridge.rs:279-331
  command_id: "ui.analyze.open",        ... prevent_default_on_canvas: true,
  command_id: "ui.changeGraphType.open",... prevent_default_on_canvas: true,
  command_id: "ui.family.toggle",       ... prevent_default_on_canvas: true,
  command_id: "ui.pane.cycle",          ... web_combo: "F6", prevent_default_on_canvas: true,
  command_id: "analyze.run",            ... web_combo: "Cmd+Enter", prevent_default_on_canvas: true,
  ```
- **Suggested fix:** Add `F6`, `a`, `f`, `g`, and `enter` (with the matching shift/primary-modifier guards) to `shouldPreventBrowserShortcut` in `index.html`, and either emit `UiInputEvent::TextInput` for unhandled non-modifier single-grapheme `keydown` events (or route unhandled printable `KeyDown` in `WebChromeBridge`/`WebBridgeSession` when `handle_key_down` falls through without matching a shortcut).
- **Related:** None.

---

### `FA-SEC20-08` · `S2` · Missing `confirm_local_storage` failure callback leaks un-reconciled `pending_storage_ops`; `hydrate_local_storage` fails swapped shortcuts and resets autosave recency to `0`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/index.html:680-683` (`applyLocalStorageCommand`), `:1594-1607`, `:1754-1773`, `apps/curvebook-web/src/host_adapters.rs:434-441` (`load_autosave_snapshot`), `:533-553` (`reconcile_confirmed_storage`), `:3109-3114`, `:3165-3215` (`handle_confirm_local_storage`, `handle_hydrate_local_storage`) (cross-checked with `crates/ui-egui/src/menus/shortcuts.rs:212-310`)
- **Category:** `data-loss` / `wrong-result`
- **Defect:**
  1. In `index.html:680-683`, `applyLocalStorageCommand` returns without emitting `{ kind: "confirm_local_storage", ..., succeeded: false }` when `!window.localStorage` (and its top-level check `if (!window.localStorage)` is outside `try/catch`, which can throw `SecurityError` in storage-blocked contexts). Any speculative eviction recorded in `pending_storage_ops` (`host_adapters.rs:3109-3114`) is therefore never rolled back via `reconcile_confirmed_storage` (`:533-553`).
  2. In `handle_hydrate_local_storage` (`host_adapters.rs:3198-3200`), restored shortcut overrides are applied sequentially via `self.host.app_mut().shortcuts.set_shortcut(cmd_id, combo)` instead of atomic `import_overrides_json` (`crates/ui-egui/src/menus/shortcuts.rs:269-310`), so swapped bindings between two commands fail `find_conflict` (`:217`) during hydration and silently drop the second override (`let _ =`).
  3. Line `3215` calls `self.storage.load_autosave_snapshot(&self.doc_id, 0)` with hardcoded `timestamp_ms = 0`, touching the restored autosave entry's recency timestamp to `(0, seq)` (`:438`) instead of current wall time, biasing future LRU quota evictions against the actively resumed document. Additionally `tryAcquireFallbackStorageLock` (`index.html:1594-1603`) installs `fallbackLockTimerId` without clearing any previous interval and `:1605-1607` grants exclusive ownership if `localStorage` throws on lock write.
- **Failure scenario:** Users who swap shortcuts between two commands lose one override on page reload; if `window.localStorage` is disabled/restricted mid-session, `pending_storage_ops` retains evicted backups un-restored in the Rust mirror; restored autosave entries are marked with `timestamp_ms = 0` and evicted ahead of newer entries when quota tightens.
- **Evidence:**
  ```js
  // apps/curvebook-web/index.html:680-683
  function applyLocalStorageCommand(cmd) {
    if (!window.localStorage) {
      return;
    }
  ```
  ```rust
  // apps/curvebook-web/src/host_adapters.rs:3198-3200, 3215
  for (cmd_id, combo) in &prefs.shortcuts {
      let _ = self.host.app_mut().shortcuts.set_shortcut(cmd_id, combo);
  }
  ...
  match self.storage.load_autosave_snapshot(&self.doc_id, 0) {
  ```
- **Suggested fix:** Emit `confirm_local_storage` with `succeeded: false` inside a `try/catch` around `window.localStorage` access in `applyLocalStorageCommand`, hydrate `prefs.shortcuts` via `shortcuts.import_overrides_json(...)`, and pass an explicit hydration `now_ms` (from JS `connectBridge` / `HydrateLocalStorage`) when loading the initial autosave snapshot.
- **Related:** `#123.1`, `#123.3`.

---

### `FA-SEC20-09` · `S3` · Chrome layout & interaction gaps: hardcoded-enabled `toolbar.axes`, dead `context_menu_items_for_target`, and unclipped dropdown overflow with blocked popup wheel

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/chrome_bridge.rs:820-868` (`build_layout` dropdowns), `:874-875` (`toolbar.axes`), `:1007-1023` (single `popup.context_menu` node), `:1173-1179` (`Secondary` `PointerUp` pass-through), `:1218-1220` (`route_wheel`), `:2011-2035` (`context_menu_items_for_target`), `apps/curvebook-web/src/host_adapters.rs:3345-3353`, `apps/curvebook-web/src/text_bridge.rs:1235-1255`
- **Category:** `contract-drift` / `cleanup` / `wrong-result`
- **Defect:**
  While `docs/evidence/user-interface.md:10, 32` and `chrome_bridge.rs:1-2` note that the web UI uses an intentional strict-CSP placeholder until post-Beta `R11.11`, three concrete logic/layout defects exist inside the placeholder chrome itself:
  1. Dropdown popup height (`chrome_bridge.rs:823-842`) grows linearly (`items.len() * 14 + 8`) without clamping to viewport height `vh` (extending past the bottom of small/640×420 viewports while `text_bridge.rs:1236` clips the overlay rectangle to `min(240)`), and `route_wheel` (`chrome_bridge.rs:1218-1220`) drops all scroll events (`popup.wheel_ignored`) while a popup or dropdown is open, making overflowed menu items unreachable.
  2. `build_layout` (`chrome_bridge.rs:875`) hardcodes `let enabled = btn.tooltip.enabled || btn.button.id == "toolbar.axes";`, leaving `toolbar.axes` clickable even when no `GraphSheet` exists, while `context_menu_items_for_target` (`:2011-2035`) is unused dead code and right-clicking spawns only a single `200×120` node bound to `WebChromeAction::DismissContextMenu` (`:1007-1023`) while passing `PointerUp { button: Secondary }` through to the workspace (`:1173-1179`).
  3. `render_frame_command` (`host_adapters.rs:3345-3353`) forwards only `status_summary` and `popup_open` to `compose_placeholder_frame_with_popup`, painting `"Menu"` (`text_bridge.rs:1246`) instead of any `WebChromeLayout` menu titles, items, toolbar labels, or modal titles.
- **Failure scenario:** Opening a long menu (such as File/Analyze) in a compact viewport places bottom menu items off-screen where wheel scrolling cannot reach them (`popup.wheel_ignored`); right-clicking produces a dismiss-only box instead of `context_menu_items_for_target`; clicking `toolbar.axes` on an empty workspace fires `ui.graph.axes.open` despite `tooltip.enabled == false`.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/chrome_bridge.rs:823-824, 875, 1007-1023, 1218-1220
  let pop_h = u32::try_from(items.len()).unwrap_or(1).saturating_mul(14).saturating_add(8).max(28);
  let pop_rect = LayoutRectPx::new(mx, 28, 220, pop_h);
  ...
  let enabled = btn.tooltip.enabled || btn.button.id == "toolbar.axes";
  ...
  if let Some(ctx) = &self.context_menu {
      let pop_rect = LayoutRectPx::new(ctx.anchor_px[0], ctx.anchor_px[1], 200, 120);
      nodes.push(WebChromeNode {
          id: String::from("popup.context_menu"),
          ...
          action: WebChromeAction::DismissContextMenu,
      });
  }
  ...
  if self.context_menu.is_some() || self.open_menu_key.is_some() || app.interaction.open_menu.is_some() {
      return WebChromeDispatch::HandledRedraw { action: String::from("popup.wheel_ignored"), command_result: None };
  }
  ```
- **Suggested fix:** Remove the `toolbar.axes` enablement override, clamp dropdown height to `vh` with popup scroll offset support in `route_wheel`, wire `context_menu_items_for_target` into `build_layout` (consuming `Secondary` `PointerUp` when context menu is open), and project `WebChromeLayout` labels into `text_ops`.
- **Related:** None.

---

### `FA-SEC20-10` · `S3` · `BrowserCanvasTextMeasurer::shape_line` executes an empty `shape_bidi_runs` loop and falls back per-codepoint with linear advance rescaling

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/src/text_bridge.rs:42-48` (`css_family_stack`), `:593-639` (`BrowserCanvasTextMeasurer::shape_line`), `:603-605`
- **Category:** `wrong-result` / `cleanup`
- **Defect:**
  `BrowserCanvasTextMeasurer::shape_line` (`text_bridge.rs:593-639`) iterates `for _run in shape_bidi_runs(&visual_text, direction) {}` with an empty body (`:603-605`), then recomputes individual character widths one Unicode codepoint at a time (`:606-616`)—substituting fixed `eff_px * 0.60` / `eff_px * 0.56` constants for `UiMono` / tabular digits—before applying uniform multiplicative scaling (`scale = metrics.width_px / pen_x`, `:628-636`) across all glyphs. Because ligatures, kerning pairs, contextual shaping (Arabic/Hebrew/complex scripts), and browser system font fallback metrics (`css_family_stack` at `:42-48`) are non-additive across isolated codepoints and differ from bundled desktop fonts, linear rescaling drifts intermediate caret and hit-test glyph x-offsets (`ShapedGlyphRun::x_px`) relative to actual canvas rendering.
- **Failure scenario:** Measuring kerning pairs (e.g., `"AV"`/`"To"`), ligatures, or RTL contextual runs on `<canvas>` misplaces intermediate `glyphs[i].x_px` positions when compared with browser `measureText` prefix substrings or desktop `curvebook-text` shaping.
- **Evidence:**
  ```rust
  // apps/curvebook-web/src/text_bridge.rs:603-616, 628-636
  for _run in shape_bidi_runs(&visual_text, direction) {
      // `visual_text` is already visually ordered; iterate its chars with measured advances.
  }
  for ch in visual_text.chars() {
      let adv_px = if matches!(style.family, FontFamilyKind::UiMono) {
          eff_px * 0.60
      } else if style.tabular_nums && ch.is_ascii_digit() {
          eff_px * 0.56
      } else {
          let mut buf = [0u8; 4];
          let s = ch.encode_utf8(&mut buf);
          let measured_ch = self.measure(style, s, dpi).width_px;
          if measured_ch > 0.0 { measured_ch } else { measure_text_pt(&RichText::plain(s), size_pt).width_pt * pt_to_px }
      };
  ```
- **Suggested fix:** Remove the dead `shape_bidi_runs` loop, measure cumulative string prefixes via `self.measure` (deriving each cluster advance from prefix differences in visual run order so kerning and ligatures are preserved), and reuse `shape_bidi_runs` spans when computing RTL vs LTR run segments.
- **Related:** None.
