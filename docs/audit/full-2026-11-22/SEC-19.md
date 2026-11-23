# Audit Report: SEC-19 — Interactive Tools & Desktop Host

- **Section:** `SEC-19` (Interactive Tools & Desktop Host — `10` source/config files + `2` manifests = `12` files, `13,932` LOC)
- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Date:** `2026-11-22`
- **Scope:**
  - `crates/tools/`: `Cargo.toml`, `src/lib.rs`, `src/cursors.toml`, `src/canvas.rs`, `src/grid.rs`
  - `apps/curvebook/`: `Cargo.toml`, `src/lib.rs`, `src/desktop_host.rs`, `src/gui_window.rs`, `src/live_control.rs`, `src/mac_menu.rs`, `src/main.rs`
- **Mode:** Report-only static audit (`100%` file coverage `1..=N`, cross-checked against downstream handlers in `crates/ui-egui/src/{menus/mod.rs, menus/ui_commands.rs, menus/shortcuts.rs, egui_shell/mod.rs, egui_shell/inspect_bridge.rs, egui_shell/canvas_view.rs, gui_actions.rs, interaction.rs}` and `crates/engine/src/{lib.rs, cmd/graph.rs, cmd/table.rs, cmd/table/import.rs, cmd/project.rs}`).
- **Summary Counts:** `S1 = 3`, `S2 = 5`, `S3 = 2`, `S4 = 1` (**11 findings**; `FA-SEC19-01` .. `FA-SEC19-11`).

---

## File Coverage Table

| File | LOC | Lines Inspected | Invariants Checked | Findings / Status |
|---|---:|---|---|---|
| `crates/tools/Cargo.toml` | 23 | `1..=23` | `[lints] workspace = true` inheritance, layer-L4 crate dependencies | Clean — verified workspace lint inheritance and dependency boundaries. |
| `crates/tools/src/lib.rs` | 70 | `1..=70` | `#![forbid(unsafe_code)]`, `Cursor` variant set vs `cursors.toml`, smoke tests | Clean — verified `#![forbid(unsafe_code)]`, all 5 `Cursor` variants, and smoke coverage. |
| `crates/tools/src/cursors.toml` | 27 | `1..=27` | 1:1 cursor parity with `tools::Cursor`, clean-room phrasing, hotspot bounds | Clean — verified 5 cursor entries match `tools::Cursor` verbatim with clean-room copy. |
| `crates/tools/src/canvas.rs` | 1,853 | `1..=1853` | Engine command contract parity (`graph.*`, `layout.*`), NaN/Inf clamping, pan/zoom inertia bounds, bracket arming/drag resolution, ruler math | `FA-SEC19-01`, `FA-SEC19-08` |
| `crates/tools/src/grid.rs` | 1,692 | `1..=1692` | O(1) virtualized window, two-tier header mapping, quick stats finite filtering, TSV quoting/round-trip, `table.paste`/`pasteLink` schemas, `#[allow(dead_code)]` shims | `FA-SEC19-06`, `FA-SEC19-11` |
| `apps/curvebook/Cargo.toml` | 43 | `1..=43` | `[lints] workspace = true`, `publish = false`, target-gated native deps (`muda`, `eframe`, `rfd`) | Clean — verified workspace lint inheritance and platform dependency gating. |
| `apps/curvebook/src/lib.rs` | 16 | `1..=16` | `#![forbid(unsafe_code)]`, `cfg(not(target_arch = "wasm32"))` module exports | Clean — verified crate-level `#![forbid(unsafe_code)]` and wasm target gating. |
| `apps/curvebook/src/desktop_host.rs` | 4,343 | `1..=4343` | Native OS file dialog routing, export param preservation, multi-window focus/quit, WAL journal recovery & dirty tracking, shortcut routing, startup merge dirty flag, drag-and-drop reset | `FA-SEC19-02`, `FA-SEC19-03`, `FA-SEC19-04`, `FA-SEC19-07`, `FA-SEC19-09` |
| `apps/curvebook/src/gui_window.rs` | 2,146 | `1..=2146` | `#![forbid(unsafe_code)]`, `PendingUiIntents` native file interception, single-viewport window management & close flow, `ui.pointer_gesture` targeting & queue ordering, keyboard shortcuts, macOS menu sync frequency | `FA-SEC19-02`, `FA-SEC19-03`, `FA-SEC19-04`, `FA-SEC19-05`, `FA-SEC19-10` |
| `apps/curvebook/src/live_control.rs` | 792 | `1..=792` | `#![forbid(unsafe_code)]`, bounded pre-auth/post-auth line limits, slowloris deadline, `MAX_CONCURRENT_CONTROL_CLIENTS = 32`, shutdown thread-join semantics | `FA-SEC19-07` |
| `apps/curvebook/src/mac_menu.rs` | 625 | `1..=625` | `muda` accelerator parsing, incremental menu enablement diffing, App submenu i18n, intentional `BETA_ACTIVE_LOCALES` RTL exclusion parity | `FA-SEC19-10` |
| `apps/curvebook/src/main.rs` | 2,302 | `1..=2302` | `#![forbid(unsafe_code)]`, CLI flag/env parsing, private config/temp UID checks, loopback-only control binding, crash recovery window launch, headless `--control` exit synchronization | `FA-SEC19-03`, `FA-SEC19-07`, `FA-SEC19-11` |

---

## Findings

### `FA-SEC19-01` · `S1` · `canvas::build_add_comparison_brackets_intent` emits `{sheet_id, brackets: [...]}` array rejected by `graph.addComparisonBrackets` (`additionalProperties: false`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/tools/src/canvas.rs:333-354` (`click_bracket`), `:378-411` (`end_drag_on_graph`), `:522-525` (`build_add_comparison_brackets_intent`) (cross-checked with `crates/engine/src/cmd/graph.rs:723-760`)
- **Category:** `contract-drift` / `wrong-result`
- **Defect:**
  1. `build_add_comparison_brackets_intent` wraps specs in `"brackets": brackets_val`, while `schema_graph_add_comparison_brackets` (`crates/engine/src/cmd/graph.rs:740-760`) specifies `additionalProperties: false`, requires top-level `"sheet_id"` and `"result_sheet_id"`, and accepts top-level single-bracket fields (`"group_a"`, `"group_b"`, `"style"`, `"label_mode"`, `"hide_if_non_significant"`, `"stroke_width_pt"`, `"font_size_pt"`, `"bracket_step_pt"`), with no `"brackets"` property.
  2. `click_bracket` (`canvas.rs:335-339`) calls `self.pending_bracket_start.take()` *before* `resolve_bracket_group_pair(&first, &anchor)?` validates that the second click hit a distinct group, so clicking the same group twice silently discards the arming click instead of retaining it or toggling cleanly.
  3. Both `click_bracket` (`line 340`) and `end_drag_on_graph` (`line 396`) fall back to setting `result_sheet_id = sheet_id.clone()` / `graph.id.clone()` when no matching `ResultSheet` with pairwise comparisons exists, emitting a malformed command carrying a graph ID in `"result_sheet_id"`.
- **Failure scenario:** Placing a pairwise comparison bracket on a column or grouped graph via click-click or drag emits a `ToolCommandIntent` rejected by the engine schema (`BadInput`: unknown `"brackets"` property and missing top-level `"result_sheet_id"`). Clicking the same category bar twice silently cancels click 1.
- **Evidence:**
  ```rust
  // crates/tools/src/canvas.rs:333-340, 353
  pub fn click_bracket(&mut self, sheet_id: &SheetId, anchor: BracketAnchor, default_label: BracketLabelMode) -> Option<ToolCommandIntent> {
      let _group = anchor.group_index?;
      let Some(first) = self.pending_bracket_start.take() else {
          self.pending_bracket_start = Some(anchor);
          return None;
      };
      let (group_a, group_b) = resolve_bracket_group_pair(&first, &anchor)?;
      let result_sheet_id = anchor.result_sheet_id.or(first.result_sheet_id).unwrap_or_else(|| sheet_id.clone());
      ...
      Some(build_add_comparison_brackets_intent(sheet_id, &[spec]))
  }

  // crates/tools/src/canvas.rs:522-525
  pub fn build_add_comparison_brackets_intent(sheet_id: &SheetId, brackets: &[PairwiseBracketSpec]) -> ToolCommandIntent {
      let brackets_val = serialize_value(&brackets);
      ToolCommandIntent::new("graph.addComparisonBrackets", json!({ "sheet_id": sheet_id.as_str(), "brackets": brackets_val }))
  }
  ```
  Versus the engine schema in `crates/engine/src/cmd/graph.rs:740-759`:
  ```rust
  fn schema_graph_add_comparison_brackets() -> Value {
      obj_schema_with_aliases(
          vec![
              ("sheet_id", non_empty_str_schema("Graph sheet ID")),
              ("result_sheet_id", non_empty_str_schema("Result sheet ID with pairwise comparison records")),
              ("group_a", nullable_schema(any_json_schema("Optional first group index or label"))),
              ("group_b", nullable_schema(any_json_schema("Optional second group index or label"))),
              ...
          ],
          &["sheet_id", "result_sheet_id"],
          vec![("sheet_id", &["id"]), ("result_sheet_id", &["result_sheet"])],
          false,
      )
  }
  ```
- **Suggested fix:** Emit flat per-bracket `ToolCommandIntent` payloads matching `GraphAddComparisonBracketsParams` (no `"brackets"` wrapper), avoid `.take()`ing `pending_bracket_start` until `first.group_index != anchor.group_index` (or explicitly cancel on repeat click of the same anchor), and return `None` rather than substituting `graph.id` when no pairwise `ResultSheet` backs the graph.
- **Related:** None.

---

### `FA-SEC19-02` · `S1` · Native host bypasses Save File dialogs for in-window Export / Save Template commands and `ExportFigure` modal commits, and drops user export options (`R15-GUI-07` / `R15-GUI-23` incomplete fix)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/gui_window.rs:550-564` (`commit_pending_intents`), `apps/curvebook/src/desktop_host.rs:1447-1568` (`native_export_spec_for_command`, `menu_action_triggers_native_file_dialog`), `:1898-1932` (`dispatch_native_file_command_with_params`) (cross-checked with `crates/ui-egui/src/menus/mod.rs:23-54` and `crates/ui-egui/src/gui_actions.rs:2542-2561`)
- **Category:** `data-loss` / `contract-drift` (incomplete `R15-GUI-07` / `R15-GUI-23` fix)
- **Defect:**
  1. `DESKTOP_HOST_ROUTED_COMMANDS` (`crates/ui-egui/src/menus/mod.rs:23-48`) omits `"export.tiff"`, `"project.saveTemplate"`, `"project.instantiateTemplate"`, `"table.importCsv"`, legacy foreign-format sheet/project importers (`desktop_host.rs:1563-1564`), `"table.exportCsv"`, `"analysis.exportJson"`, `"analysis.exportCsv"`, `"result.generateReport"`, `"graph.insertImage"`, and `"layout.insertImage"`. In `gui_window.rs:551-564`, only `graph.insertImage` and `layout.insertImage` have a pathless-command fallback; all other commands triggered as `UiIntent::EngineCommand` without `"path"` (from the in-window menu bar on Windows/Linux/macOS `--in-window-menus` or the Command Palette) fall into `remaining_intents` and execute in memory at `line 600` (`commit_all_if_settled`) without opening an OS Save/Open dialog, discarding returned bytes.
  2. When the user commits `ExportFigure` (`crates/ui-egui/src/gui_actions.rs:2542-2561`) via OK (`UiIntent::DialogButtonAction { action: "ok" }`), `gui_window.rs:600` executes `graph.exportSvg` / `graph.exportPdf` / `graph.exportPng` / `export.tiff` directly without `"path"`, bypassing `RfdNativeFileDialogAdapter` (`FA-SEC18-01` host seam).
  3. Even when a file command reaches `dispatch_native_file_command_with_params` (`desktop_host.rs:1915-1927`), the host builds a fresh JSON object containing only `{ sheet_id, path }` (or `{ result_sheet_id, format, path }` for `result.generateReport`), dropping every non-path option from `params` (`width_px`, `height_px`, `dpi`, `color_space`, `compression`, etc.).
- **Failure scenario:** Triggering Export TIFF, Export CSV/JSON, Save Template, or `ExportFigure` (PDF/SVG/PNG/TIFF) in the in-window menu bar, Command Palette, or Export Figure dialog never prompts for a save destination and writes no file to disk; custom DPI, dimensions, or color space options passed to host export commands are silently reset to engine defaults.
- **Evidence:**
  ```rust
  // apps/curvebook/src/gui_window.rs:551-564
  match &intent {
      UiIntent::UiCommand { command_id, params } | UiIntent::EngineCommand { command_id, params }
          if is_desktop_host_routed_command(command_id) =>
      {
          host_commands.push((command_id.clone(), params.clone()));
      }
      UiIntent::EngineCommand { command_id, params }
          if (command_id == "graph.insertImage" || command_id == "layout.insertImage")
              && params.get("path").is_none()
              && params.get("bytes_base64").is_none()
              && params.get("asset_id").is_none() =>
      {
          host_commands.push((command_id.clone(), params.clone()));
      }
  ```
  ```rust
  // apps/curvebook/src/desktop_host.rs:1916-1927
  let export_params = if spec.command == "result.generateReport" {
      let report_fmt = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
          Some("tex") => "latex",
          Some("txt") => "plain",
          _ => "markdown",
      };
      json!({ "result_sheet_id": target_sheet_id.as_str(), "format": report_fmt, "path": path_str })
  } else {
      json!({ "sheet_id": target_sheet_id.as_str(), "path": path_str })
  };
  let win = self.window_mut(window_id)?;
  win.execute_and_journal(spec.command, export_params)?;
  ```
- **Suggested fix:** Route any `UiIntent::EngineCommand` where `menu_action_triggers_native_file_dialog(command_id)` is true and `params` lacks `"path"` / inline payload bytes to `host_commands` instead of relying on the incomplete `DESKTOP_HOST_ROUTED_COMMANDS` list; merge caller-supplied `params` into `export_params` before setting `"path"` and resolved `"sheet_id"`; and route `ExportFigure` modal confirmation through the host Save File picker before executing the export command.
- **Related:** `FA-SEC18-01` (`SEC-18`), `R15-GUI-07`, `R15-GUI-23`.

---

### `FA-SEC19-03` · `S1` · Single-viewport `CurveBookNativeApp` hides earlier unsaved/CLI windows unreachably when a second window opens and aliases `app.quit` to single-window close

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/gui_window.rs:776-838` (`active_window_id`, `apply_close_outcome`), `apps/curvebook/src/desktop_host.rs:2253-2256` (`install_window_session`), `:2796-2813` (`handle_file_dropped`), `:2902-2917` (`window.new`, `window.close`), `:3504-3505` (`resolve_host_or_ui_route`), `apps/curvebook/src/main.rs:472-474`, `:504-510`
- **Category:** `data-loss` / `robustness`
- **Defect:**
  1. `DesktopHostRuntime` allows multiple concurrent sessions in `self.windows`, and `install_window_session` (`desktop_host.rs:2253-2256`) switches `self.focused_window = Some(id)` whenever a second window is spawned — including `File > New Window`, dropping/opening a project file while the active window has unsaved changes (`desktop_host.rs:2798-2812`), and opening recovered crash journals after opening a CLI file (`main.rs:464-474`). Because `CurveBookNativeApp` (`gui_window.rs:776-778`) hosts only one `eframe` OS viewport rendering `self.active_window_id()` and provides no window-switcher UI, the prior window (along with any unsaved edits or user-opened CLI document) becomes completely invisible and unreachable until the newly opened window is closed.
  2. `resolve_host_or_ui_route` (`desktop_host.rs:3505`) maps `"file.quit"` and `"app.quit"` identically to `HostAction { action: "window.close" }`. When >=2 windows exist in `runtime.windows`, closing the top window triggers `apply_close_outcome` (`gui_window.rs:809-814`), which sees `self.runtime.window_count() > 0` and sends `ViewportCommand::CancelClose` instead of continuing the quit sequence across remaining windows.
  3. On exit, `main.rs:505-508` persists preferences only for `rt.focused_window_id()`.
- **Failure scenario:** Editing an unsaved Project A and dropping/opening Project B opens Project B in a second session that replaces the sole viewport; Project A cannot be inspected or saved while Project B is open. Launching `curvebook my_study.curvebook` with stale crash journals hides `my_study.curvebook` behind the recovered session. Pressing `Cmd+Q` / **Quit CurveBook** with two open windows closes only one window and cancels application exit.
- **Evidence:**
  ```rust
  // apps/curvebook/src/gui_window.rs:776-778, 807-820
  fn active_window_id(&self) -> DesktopWindowId {
      self.runtime.focused_window_id().unwrap_or(self.primary_window_id)
  }
  ...
  if matches!(outcome.close_disposition, Some(CloseDisposition::Closed)) {
      self.close_dialog_open = false;
      if self.runtime.window_count() > 0 {
          if let Some(ctx) = ctx {
              ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
          }
          self.close_confirmed = false;
          self.on_active_window_switched();
      } else {
          self.close_confirmed = true;
          ...
  ```
  ```rust
  // apps/curvebook/src/desktop_host.rs:3504-3505
  "window.new" | "file.newWindow" => Ok(Some(DesktopCommandRoute::HostAction { action: "window.new" })),
  "window.close" | "file.close" | "file.quit" | "app.quit" => Ok(Some(DesktopCommandRoute::HostAction { action: "window.close" })),
  ```
- **Suggested fix:** Either spawn real OS viewports per `DesktopWindowId` (e.g., `show_viewport_deferred` mapped by window ID) or expose an in-window switcher/tab strip for all open `DesktopWindowSession`s; separate `"app.quit"` / `"file.quit"` from `"window.close"` so quitting walks unsaved-changes confirmation across every open window and exits once all approve.
- **Related:** None.

---

### `FA-SEC19-04` · `S2` · Native keyboard dispatch breaks `ui.analyze.open` (`Cmd+Shift+A`), `ui.print.open` (`Cmd+P`), `app.preferences`, `app.quit`, and non-Cmd/custom shortcuts

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/gui_window.rs:1045-1088` (`dispatch_key_event`), `apps/curvebook/src/desktop_host.rs:2723-2730` (`handle_keyboard_input`), `:4141-4178` (`shortcut_uses_desktop_routing`, `match_routed_desktop_shortcut`), `crates/ui-egui/src/egui_shell/inspect_bridge.rs:1205-1265` (`handle_shell_global_shortcuts`)
- **Category:** `contract-drift` / `robustness`
- **Defect:**
  1. `shortcut_uses_desktop_routing` (`desktop_host.rs:4141-4161`) returns `false` for built-in UI command shortcuts `"ui.analyze.open"` (`Cmd+Shift+A`), `"ui.editParams.open"` (`Cmd+T`), `"ui.print.open"` (`Cmd+P`), `"ui.changeGraphType.open"` (`Cmd+Shift+G`), and `"ui.zoom.fitSheet"` (`Cmd+0`) because they are not in `is_desktop_host_routed_command`, not listed in the `matches!` block (`lines 4143-4154`), and not in `engine_cmd::registry()`. Consequently `match_routed_desktop_shortcut` (`line 4173`) never returns them — even though `gui_window.rs:1082` explicitly matches `"ui.editParams.open"` and `"ui.analyze.open"`, and `handle_shell_global_shortcuts` (`inspect_bridge.rs:1205-1305`) does not handle `Cmd+Shift+A`, `Cmd+Shift+G`, `Cmd+P`, or `Cmd+0`.
  2. `gui_window.rs:1079-1084` filters out `"app.preferences"`, `"app.quit"`, `"file.quit"`, and `"edit.cut"` / `"edit.copy"` / `"edit.paste"` even when `match_routed_desktop_shortcut` matches them (`desktop_host.rs:4142, 4146`).
  3. `gui_window.rs:1059-1069` gates `match_routed_desktop_shortcut` on `primary_cmd` (preventing custom `F1`-`F12` or `Alt+...` bindings from firing) and hardcodes `Cmd+O` / `Cmd+S` / `Cmd+Shift+S` before querying `app.shortcuts`, ignoring custom user rebindings for those keys; `gui_window.rs:1054` similarly intercepts `Cmd+0` (`Num0 && !shift`) for `UiScaleAdjust::Reset` instead of `ui.zoom.fitSheet` (`ui_commands.rs:84`).
  4. `desktop_host.rs:2723-2730` provides fallback handling for `Cmd+C` and `Cmd+V` on `DesktopHostEvent::KeyboardInput` but omits `Cmd+X` (`edit.cut`) when `KeyDown` is unhandled.
- **Failure scenario:** Pressing `Cmd+Shift+A` (Analyze), `Cmd+P` (Print on Linux/Windows/`--in-window-menus`), `Cmd+,` / `Cmd+Q` (non-macOS / `--in-window-menus`), custom function-key bindings, or rebound Open/Save shortcuts silently fails or triggers the wrong action.
- **Evidence:**
  ```rust
  // apps/curvebook/src/desktop_host.rs:4141-4161
  fn shortcut_uses_desktop_routing(command_id: &str) -> bool {
      if is_desktop_host_routed_command(command_id)
          || matches!(
              command_id,
              "sheet.createData"
                  | "app.preferences"
                  | "ui.theme.set"
                  | "ui.locale.set"
                  | "ui.previewFilter.set"
                  | "ui.canvas.applyPreset"
                  | "ui.shortcut.set"
                  | "ui.result.applyRemediation"
                  | "ui.event.dispatch"
          )
      {
          return true;
      }
      engine_cmd::registry()
          .get(command_id)
          .is_some_and(|spec| (spec.input_schema)().get("required").and_then(Value::as_array).is_some_and(|arr| !arr.is_empty()))
  }
  ```
  ```rust
  // apps/curvebook/src/gui_window.rs:1059-1086
  if pressed && dialog_inactive && primary_cmd && !ctx.text_edit_focused() {
      if !modifiers.alt {
          if key == egui::Key::O && !modifiers.shift { ... return; }
          if key == egui::Key::S { ... return; }
      }
      let routed_cmd = self.runtime.window(window_id).ok().and_then(|w| match_routed_desktop_shortcut(w.app(), &key_name, to_key_modifiers(modifiers)));
      if let Some(cmd) = routed_cmd {
          ...
          if !already_queued_edit_params
              && (menu_action_triggers_native_file_dialog(&cmd)
                  || matches!(
                      cmd.as_str(),
                      "window.close" | "file.close" | "window.new" | "file.newWindow" | "ui.editParams.open" | "ui.analyze.open"
                  ))
          {
              self.execute_menu_command(Some(&ctx), window_id, &cmd);
          }
      }
  }
  ```
- **Suggested fix:** Consult `app.shortcuts` before hardcoded fallbacks, remove the `primary_cmd` gate for non-text-editing keys so F-key/Alt bindings work, include `"ui.analyze.open"`, `"ui.editParams.open"`, `"ui.print.open"`, `"ui.changeGraphType.open"`, `"ui.zoom.fitSheet"`, `"app.preferences"`, `"app.quit"`, and `"file.quit"` in both `shortcut_uses_desktop_routing` and `gui_window.rs`'s dispatch path (or dispatch all `ShortcutRegistry` matches uniformly), and resolve the `Cmd+0` collision.
- **Related:** None.

---

### `FA-SEC19-05` · `S2` · `ui.pointer_gesture` silently clicks canvas center when `widget_id` is missing, misorders batched commands, and ignores `out.quit` on deferred flush

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/gui_window.rs:261-271` (`poll_live_control`), `:351-385` (`flush_deferred_live_requests`), `:412-439` (`try_enqueue_live_pointer_gesture`), `:1194-1197` (`build_synthetic_egui_gesture_batches`)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:**
  1. In `try_enqueue_live_pointer_gesture` (`gui_window.rs:412-437`), if a caller specifies `widget_id` / `target_id` and it is absent from `self.inspect_meta` (and no explicit `x`/`y` was supplied), `target_pos` evaluates to `None` and `pos` silently defaults to the canvas center (`self.last_regions.canvas_rect_pt.map_or_else(|| ctx.content_rect().center(), |r| r.center())`), clicking the canvas and replying `ok: true` instead of returning `ErrorCode::NotFound`. Additionally, `line 418` restricts disabled-widget rejection to `"click" | "double_click"` (allowing `"right_click"` and `"drag"` on disabled controls), and `build_synthetic_egui_gesture_batches` (`line 1194`) maps unknown `action` values to a primary click rather than rejecting invalid input.
  2. In `poll_live_control` (`gui_window.rs:261-271`), `has_deferred_in_flight` is computed once *before* looping over `pending`. If a single frame poll drains `[ui.pointer_gesture, project.save]` (or `[ui.pointer_gesture, ui.inspect]`), `ui.pointer_gesture` goes to `self.deferred_live_requests`, while `has_deferred_in_flight` remains `false`, causing the subsequent `ControlFrameKind::Command` in `pending` to run immediately *before* the synthetic gesture settles.
  3. In `flush_deferred_live_requests` (`gui_window.rs:378-381`), `res.as_ref().is_ok_and(|out| out.quit)` is never checked, so deferred `app.quit` frames do not set `close_confirmed = true` or send `ViewportCommand::Close` (unlike `dispatch_pending_live_requests:296`).
- **Failure scenario:** Automated live-control tests targeting a mistyped, scrolled-off, or hidden `widget_id` click the center of the graph canvas and pass falsely; sending a gesture followed immediately by a state query or command in one tick evaluates the follow-up command against pre-gesture state.
- **Evidence:**
  ```rust
  // apps/curvebook/src/gui_window.rs:261-271
  let has_deferred_in_flight =
      !self.deferred_live_requests.is_empty() || !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_gesture.is_some();
  let mut immediate = Vec::new();
  for req in pending {
      if should_defer_to_settled_frame(&req) || (has_deferred_in_flight && classify_control_frame(&req.raw_bytes) == ControlFrameKind::Command)
      {
          self.deferred_live_requests.push(req);
      } else {
          immediate.push(req);
      }
  }
  ```
  ```rust
  // apps/curvebook/src/gui_window.rs:415-437
  let meta_entry = widget_id.and_then(|wid| self.inspect_meta.find_by_widget_id(wid).cloned());
  if let Some(ref entry) = meta_entry
      && !entry.enabled
      && matches!(action.as_str(), "click" | "double_click")
  { ... }
  let explicit_xy = params.get("x").and_then(serde_json::Value::as_f64).zip(params.get("y").and_then(serde_json::Value::as_f64));
  let target_pos = meta_entry.as_ref().map(|e| e.clipped_rect.center()).or_else(|| { ... });
  let pos = target_pos.unwrap_or_else(|| self.last_regions.canvas_rect_pt.map_or_else(|| ctx.content_rect().center(), |r| r.center()));
  ```
- **Suggested fix:** Return `ErrorCode::NotFound` if `widget_id` is explicitly provided and missing from `inspect_meta` (and `BadInput` for unknown `action`), track `deferred_seen |= should_defer_to_settled_frame(&req)` inside the `pending` loop so subsequent commands preserve FIFO ordering behind newly deferred requests, and handle `out.quit` in `flush_deferred_live_requests`.
- **Related:** None.

---

### `FA-SEC19-06` · `S2` · `grid::format_cell_tsv_token` / `format_selection_tsv` leave excluded special-char cells unquoted, and `emit_paste_intent` sends ignored `cells` / top-level `parse_asterisk_exclusions`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/tools/src/grid.rs:947-974` (`emit_paste_intent`), `:1017-1027` (`format_cell_tsv_token`), `:1057-1077` (`format_selection_tsv`) (cross-checked with `crates/engine/src/cmd/table.rs:2470-2519`, `2569-2595`, and `crates/doc/src/links.rs:106-123`)
- **Category:** `wrong-result` / `contract-drift` / `data-loss`
- **Defect:**
  1. `format_cell_tsv_token` (`grid.rs:1017-1027`) checks `if cell.excluded && !base.is_empty()` *before* `else if base.contains(['\t', '\n', '\r', '"'])`. When an excluded cell's formatted text contains a tab, newline, carriage return, or quote, `format_cell_tsv_token` returns an unquoted `{base}*` string. `format_selection_tsv` (`grid.rs:1072`) writes `format_cell_tsv_token` directly without applying `format_grid_tsv`'s secondary quoting pass (`lines 1041-1049`).
  2. `GridToolState::emit_paste_intent` (`grid.rs:960-971`) puts `"cells": serialized_rows` and top-level `"parse_asterisk_exclusions": parse_asterisk_exclusions` on its `"table.paste"` payload. However, `PasteParams` / `schema_paste` (`crates/engine/src/cmd/table.rs:2470-2518`) only accepts `"rows"` (not `"cells"`) and has no top-level `"parse_asterisk_exclusions"` field; `"parse_asterisk_exclusions"` belongs inside `"filter": { "parse_asterisk_exclusions": ... }` (`table.rs:865`), so `run_paste` (`line 2580`) uses `ImportPipelineOptions::default()` and `ImportFilterSpec::default().parse_asterisk_exclusions == true`, ignoring `parse_asterisk_exclusions = false`.
- **Failure scenario:** Copying an excluded text cell containing `\t`, `\n`, or `"` from a data table corrupts clipboard TSV boundaries and splits into extra rows/columns when pasted; pasting raw `*`-suffixed literal text with `parse_asterisk_exclusions = false` via `emit_paste_intent` unexpectedly converts those strings into excluded values.
- **Evidence:**
  ```rust
  // crates/tools/src/grid.rs:960-971
  let intent = ToolCommandIntent::new(
      "table.paste",
      json!({
          "sheet_id": sheet_id.as_str(),
          "anchor": { "row": anchor.row, "col": anchor.col, "subcol": anchor.subcol },
          "tsv": raw_tsv,
          "cells": serialized_rows,
          "decimal_separator": self.decimal_separator.to_string(),
          "parse_asterisk_exclusions": parse_asterisk_exclusions,
          "transpose": transpose,
      }),
  );
  ```
  ```rust
  // crates/tools/src/grid.rs:1017-1026, 1071-1073
  pub fn format_cell_tsv_token(cell: &Cell, decimal_separator: char) -> String {
      let base = format_cell_display(cell, decimal_separator);
      if cell.excluded && !base.is_empty() {
          format!("{base}*")
      } else if base.contains(['\t', '\n', '\r', '"']) {
          let escaped = base.replace('"', "\"\"");
          format!("\"{escaped}\"")
      } else {
          base
      }
  }
  ...
  if let Some(cell) = sheet.cell(CellCoord::new(row, col, subcol)) {
      out.push_str(&format_cell_tsv_token(cell, decimal_separator));
  }
  ```
- **Suggested fix:** Compute `let token = if cell.excluded && !base.is_empty() { format!("{base}*") } else { base };` first in `format_cell_tsv_token` and quote/escape whenever `token.contains(['\t', '\n', '\r', '"'])`; remove redundant quoting from `format_grid_tsv`; and emit `"filter": { "parse_asterisk_exclusions": parse_asterisk_exclusions }` (or pass engine-typed `"rows"` cleanly without `"cells"`) in `emit_paste_intent`.
- **Related:** None.

---

### `FA-SEC19-07` · `S2` · `apply_control_journal_steps` clears `dirty_commands` on pathless `project.save`, and headless `--control` Egui loop exits before client reply flush

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/desktop_host.rs:761-765` (`apply_control_journal_steps`), `:3597-3602` (`resolve_project_or_sheet_route`), `apps/curvebook/src/main.rs:678-696` (`serve_control_with_options`), `apps/curvebook/src/live_control.rs:33-64, 271-278` (`LiveControlServerGuard`, `serve_live_control_io`)
- **Category:** `data-loss` / `robustness`
- **Defect:**
  1. In `apply_control_journal_steps` (`desktop_host.rs:761-765`), any non-mutating `"project.save"` command clears `dirty_commands.clear()` unconditionally — including `project.save` with `{ "return_bytes": true }` and no `"path"` (emitted by `resolve_project_or_sheet_route:3600` whenever `session_file_path` is `None`). This wipes the recovery journal's unsaved-command tracking even though nothing was saved to disk and `session.is_dirty()` remains true.
  2. In `main.rs:678-696` (headless `--control <port>` on `UiBackend::Egui`), `_guards` holds `LiveControlServerGuard` (`live_control.rs:33-64`), whose `stop()` joins only `accept_thread` and not active client handler threads (`serve_live_control_io`). Upon `app.quit`, the loop sends `res` onto `reply_tx` (`main.rs:690`), stores `shutdown = true`, breaks, and returns immediately from `main()`, exiting the process while the client thread may still be executing `writer.write_all(&out.response_bytes)` + `writer.flush()` (`live_control.rs:271-272`).
- **Failure scenario:** Triggering Save (`project.save` with `return_bytes: true`) on an untitled document drops recorded dirty commands from recovery diagnostics despite no file being saved; sending `app.quit` to `--control <port>` in headless Egui mode intermittently closes the socket before the client receives the quit JSON response.
- **Evidence:**
  ```rust
  // apps/curvebook/src/desktop_host.rs:761-765
  } else if step.command_id == "project.save" {
      recorded_any_step = true;
      dirty_commands.clear();
      requires_final_checkpoint = true;
  }
  ```
  ```rust
  // apps/curvebook/src/main.rs:689-696
  let quit = res.as_ref().is_ok_and(|out| out.quit);
  let _ = req.reply_tx.send(res);
  if quit {
      channel.shutdown.store(true, std::sync::atomic::Ordering::Release);
      break;
  }
  ```
- **Suggested fix:** Only clear `dirty_commands` on `"project.save"` when `!app.session().is_dirty()` (i.e., when a filesystem save updated clean state); track active client threads or wait for active client count to reach zero (with bounded timeout) in `LiveControlServerGuard::stop()`.
- **Related:** None.

---

### `FA-SEC19-08` · `S2` · `adjust_axis_segment_share` panics in `f64::clamp` on `NaN`/`Inf` `length_share`, and `fit_scene` / `ArrowCallout` / zoom caps diverge on bounds

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/tools/src/canvas.rs:28, 33`, `:296-306` (`fit_scene`), `:426` (`end_drag` `ArrowCallout`), `:846-872` (`adjust_axis_segment_share`) (cross-checked with `crates/ui-egui/src/egui_shell/canvas_view.rs:455`)
- **Category:** `no-panic` / `wrong-result`
- **Defect:**
  1. `adjust_axis_segment_share` (`canvas.rs:846-872`) checks `new_share.is_finite()`, sums `let pair_total = curr_share + neighbor_share;`, and guards `if pair_total <= 0.1 { return false; }` before calling `new_share.clamp(0.05, pair_total - 0.05)`. If either `curr_share` or `neighbor_share` is `NaN` (or opposing `±Inf` whose sum is `NaN`), `NaN <= 0.1` evaluates to `false`, passing `max = NaN` into `f64::clamp` and panicking (`min <= max` assertion failure inside `f64::clamp`).
  2. `fit_scene` (`canvas.rs:297`) checks `<= 0.0` without `.is_finite()` on `scene_size_pt` and `viewport_size_px`, letting `NaN`/`Inf` dimensions propagate `NaN`/`Inf` into `self.zoom` and `self.pan_pt`.
  3. `end_drag` (`canvas.rs:426`) for `CanvasToolMode::ArrowCallout` emits `build_add_arrow_callout_intent(sheet_id, start, end_pt, true)` unconditionally without rejecting zero-length clicks or non-finite endpoints (unlike `TextCallout` using `normalized_rect_from_drag`).
  4. Zoom upper limits drift across layers: `INERTIA_MAX_ZOOM = 32.0` (`canvas.rs:33`), `MAX_ZOOM = 16.0` (`canvas.rs:28`), while `canvas_view.rs:455` clamps rendered zoom to `8.0`.
- **Failure scenario:** Dragging an axis break on a graph with corrupted/`NaN` segment share panics and terminates the desktop app; clicking instead of dragging in Arrow Callout mode inserts zero-length degenerate arrows; zooming above `8x` snaps on render clamping while inertia accumulates up to `32x`.
- **Evidence:**
  ```rust
  // crates/tools/src/canvas.rs:857-861
  let pair_total = curr_share + neighbor_share;
  if pair_total <= 0.1 {
      return false;
  }
  let clamped = new_share.clamp(0.05, pair_total - 0.05);
  ```
  ```rust
  // crates/tools/src/canvas.rs:296-306, 426
  pub fn fit_scene(&mut self, scene_size_pt: PointPt, viewport_size_px: PointPt, padding_px: f64) {
      if scene_size_pt.x <= 0.0 || scene_size_pt.y <= 0.0 || viewport_size_px.x <= 0.0 || viewport_size_px.y <= 0.0 || !padding_px.is_finite() {
          return;
      }
      ...
  }
  ...
  CanvasToolMode::ArrowCallout => Some(build_add_arrow_callout_intent(sheet_id, start, end_pt, true)),
  ```
- **Suggested fix:** Check `curr_share.is_finite() && neighbor_share.is_finite() && pair_total > 0.1` in `adjust_axis_segment_share`; require `.is_finite()` on all `fit_scene` vector components and `ArrowCallout` drag coordinates (enforcing a non-zero drag threshold or default diagonal offset); and share one `MAX_ZOOM` constant between `tools::canvas` and `ui-egui`.
- **Related:** None.

---

### `FA-SEC19-09` · `S3` · `open_document_from_files` leaves merged pathless/template documents clean (`is_dirty() == false`) and `handle_file_dropped` does not clear `coasting_active`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/desktop_host.rs:501-535` (`open_document_from_files`), `:2820-2846` (`handle_file_dropped`) (cross-checked with `crates/engine/src/lib.rs:1091-1102`)
- **Category:** `data-loss` / `robustness`
- **Defect:**
  1. `open_document_from_files` (`desktop_host.rs:526-535`) builds `Session::from_document(doc)` (whose `dirty` flag starts `false` at `crates/engine/src/lib.rs:1096`) and only updates clean/dirty state inside `if let Some(path) = primary_archive_path`. When `primary_archive_path` is `None` (launching with a `.cbtemplate`, CSV import, unpacked dir, or foreign importer), even merging additional CLI CSV files (`merged_delimited == true`) leaves `session.is_dirty() == false`.
  2. `handle_file_dropped` (`desktop_host.rs:2843-2845`) replaces `win.app = next_app`, clears `dirty_commands`, and clears `ime_preedit = None`, while leaving `win.coasting_active` unaltered if an inertia coast animation was active when the user dropped a new document onto the window.
- **Failure scenario:** Launching CurveBook with a template + CSV tables and closing skips the unsaved-changes confirmation prompt, silently discarding the merged tables; dropping a file during pan inertia leaves `coasting_active` enabled across document replacement.
- **Evidence:**
  ```rust
  // apps/curvebook/src/desktop_host.rs:526-535
  let mut session = Session::from_document(doc);
  if merged_delimited && primary_had_project {
      session
          .execute("project.recalculate", json!({ "include_external_links": false }))
          .map_err(|err| format!("failed to recalculate merged startup project: {err}"))?;
  }
  if let Some(path) = primary_archive_path {
      session.set_file_path(Some(path), clean_archive && !merged_delimited);
  }
  Ok(session)
  ```
  ```rust
  // apps/curvebook/src/desktop_host.rs:2843-2846
  win.app = next_app;
  win.dirty_commands.clear();
  win.ime_preedit = None;
  win.checkpoint_recovery_journal()?;
  ```
- **Suggested fix:** Mark `session` dirty when `primary_archive_path.is_none()` and sheets were merged/imported into the new workspace (e.g., `merged_delimited` or template/importer load), and clear `win.coasting_active = false` in `handle_file_dropped` alongside `ime_preedit = None`.
- **Related:** None.

---

### `FA-SEC19-10` · `S3` · macOS `muda` App submenu labels are hardcoded English and `sync_macos_menu_for_window` runs every frame even under `--in-window-menus`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/mac_menu.rs:396-414` (`rebuild_live_muda_menu`), `apps/curvebook/src/gui_window.rs:684-692` (`step_ui`)
- **Category:** `i18n` / `resources`
- **Defect:**
  1. `rebuild_live_muda_menu` (`mac_menu.rs:398-412`) hardcodes English strings `"About CurveBook"`, `"Settings…"`, and `"Quit CurveBook"` instead of translating via active locale strings (unlike the rest of the macOS menu sections built from `DesktopMenuChromeState`). *(Note: `mac_menu.rs:528` restricting the locale switcher to `BETA_ACTIVE_LOCALES` matches the explicit beta RTL-exclusion contract enforced in `gui_window.rs:1872-1877`.)*
  2. `step_ui` (`gui_window.rs:686`) calls `self.sync_macos_menu_for_window(bar_window_id)` unconditionally on every rendered frame (~60Hz), calling `win.chrome_state()` and rebuilding `MudaMenuSectionSpec` trees and enablement signatures even when `self.mac_menu_host.is_installed_for_nsapp()` is `false` (e.g., `--in-window-menus`).
- **Failure scenario:** macOS users in non-English locales see mixed English/localized items in the application menu; every frame performs redundant menu descriptor allocations regardless of whether the native macOS menu bar is active.
- **Evidence:**
  ```rust
  // apps/curvebook/src/mac_menu.rs:401-411
  &[
      &muda::MenuItem::with_id(muda::MenuId::new("app.about"), "About CurveBook", true, None),
      &muda::PredefinedMenuItem::separator(),
      &muda::MenuItem::with_id(muda::MenuId::new("app.preferences"), "Settings…", true, parse_muda_accelerator("Cmd+,")),
      &muda::PredefinedMenuItem::separator(),
      &muda::PredefinedMenuItem::hide(None),
      &muda::PredefinedMenuItem::hide_others(None),
      &muda::PredefinedMenuItem::show_all(None),
      &muda::PredefinedMenuItem::separator(),
      &muda::MenuItem::with_id(muda::MenuId::new("app.quit"), "Quit CurveBook", true, parse_muda_accelerator("Cmd+Q")),
  ],
  ```
  ```rust
  // apps/curvebook/src/gui_window.rs:684-692
  #[cfg(target_os = "macos")]
  {
      self.sync_macos_menu_for_window(bar_window_id);
      if self.mac_menu_host.is_installed_for_nsapp() {
          while let Some(cmd_id) = self.mac_menu_host.poll_triggered_command_id() {
              self.execute_menu_command(Some(&ctx), bar_window_id, &cmd_id);
          }
      }
  }
  ```
- **Suggested fix:** Source `"app.about"`, `"app.preferences"`, and `"app.quit"` menu titles from the active locale table and rebuild on locale changes; gate `sync_macos_menu_for_window` behind `self.mac_menu_host.is_installed_for_nsapp()` and skip recomputing `chrome_state()` when relevant window/selection/locale state hasn't changed.
- **Related:** None.

---

### `FA-SEC19-11` · `S4` · `args_contain_cli_control_token` omits `--ui-backend` value skip and `tools/grid.rs` keeps a `#[allow(dead_code)]` import-silencing shim

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook/src/main.rs:160-171` (`args_contain_cli_control_token`), `crates/tools/src/grid.rs:1497-1500` (`ensure_bounds_constants_used`)
- **Category:** `cleanup`
- **Defect:**
  1. `args_contain_cli_control_token` (`main.rs:166`) skips 2-arg option values for `--control`, `--portable-dir`, `--ttf-font`, and `--event-script`, but omits `"--ui-backend"` (which also consumes `args[i + 1]` at `line 241-244`).
  2. `crates/tools/src/grid.rs:1497-1500` keeps a `#[allow(dead_code)] const fn ensure_bounds_constants_used()` dummy function solely to suppress unused-import warnings for `MAX_DATA_SETS`, `MAX_ROWS`, and `Ordering::Equal` in non-test compilation instead of placing those imports under `#[cfg(test)]`.
- **Failure scenario:** Passing `"--control-token"` as the value argument to `--ui-backend` causes `args_contain_cli_control_token` to misidentify the presence of a CLI `--control-token` flag before `parse_args_with_env_and_backend` runs; dead helper code remains in `crates/tools/src/grid.rs`.
- **Evidence:**
  ```rust
  // apps/curvebook/src/main.rs:160-171
  fn args_contain_cli_control_token(args: &[String]) -> bool {
      let mut i = 0usize;
      while let Some(arg) = args.get(i) {
          match arg.as_str() {
              "--" => return false,
              "--control-token" => return true,
              "--control" | "--portable-dir" | "--ttf-font" | "--event-script" => i += 2,
              _ => i += 1,
          }
      }
      false
  }
  ```
  ```rust
  // crates/tools/src/grid.rs:1497-1500
  #[allow(dead_code)]
  const fn ensure_bounds_constants_used() -> usize {
      MAX_DATA_SETS + MAX_ROWS + Ordering::Equal as usize
  }
  ```
- **Suggested fix:** Add `"--ui-backend"` to `args_contain_cli_control_token`'s `i += 2` arm in `apps/curvebook/src/main.rs`, and scope `MAX_DATA_SETS`, `MAX_ROWS`, and `Ordering` in `crates/tools/src/grid.rs` under `#[cfg(test)]` while deleting `ensure_bounds_constants_used`.
- **Related:** None.
