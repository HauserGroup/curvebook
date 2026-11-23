# Verification Verdicts: SEC-19 — Interactive Tools & Desktop Host

- **Section:** `SEC-19` (Interactive Tools & Desktop Host)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-19.md`](SEC-19.md)
- **Baseline SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-frontend`
- **Date:** `2026-11-22`

## Summary

| Verdict | Count | Finding IDs |
|---|---:|---|
| `CONFIRMED` | 11 | `FA-SEC19-01`..`FA-SEC19-11` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Finding IDs |
|---|---:|---|
| `S1` | 2 | `FA-SEC19-02`, `FA-SEC19-03` |
| `S2` | 6 | `FA-SEC19-01` *(adjusted from `S1`)*, `FA-SEC19-04`, `FA-SEC19-05`, `FA-SEC19-06`, `FA-SEC19-07`, `FA-SEC19-08` |
| `S3` | 2 | `FA-SEC19-09`, `FA-SEC19-10` |
| `S4` | 1 | `FA-SEC19-11` |

## File Coverage Completeness Check

- **Result:** **PASS (10 / 10 scoped source/config files + 2 `Cargo.toml` manifests = 12 files, 13,932 LOC — 100% covered)**
- Independently verified all files in `SEC-19`'s brief scope via directory enumeration and `wc -l`:
  - `crates/tools/`: `Cargo.toml` (`23`), `src/lib.rs` (`70`), `src/cursors.toml` (`27`), `src/canvas.rs` (`1,853`), `src/grid.rs` (`1,692`) — `3,665` LOC.
  - `apps/curvebook/`: `Cargo.toml` (`43`), `src/lib.rs` (`16`), `src/desktop_host.rs` (`4,343`), `src/gui_window.rs` (`2,146`), `src/live_control.rs` (`792`), `src/mac_menu.rs` (`625`), `src/main.rs` (`2,302`) — `10,267` LOC.
- Informational note: `apps/curvebook/build.rs` (`34` LOC) exists alongside `Cargo.toml` and was outside the `src/` list in `SEC-19`'s assignment; independently inspected (`1..=34`) and confirmed clean (`#![forbid(unsafe_code)]`, rerun directives and Windows icon/VERSIONINFO only).

---

## Per-Finding Verdicts

### Verdict `FA-SEC19-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S1` -> `S2`
- **Rationale for adjustment:** Per `docs/audit/README.md`, `S1` applies to crashes/aborts/hangs, data loss, silent wrong statistical/fitted outputs, or sandbox/loopback bypasses, while `S2` covers schema/contract errors or wrong outputs with an explicit error signal. Because `schema_graph_add_comparison_brackets` sets `additionalProperties: false` and requires `["sheet_id", "result_sheet_id"]`, passing the malformed `{"sheet_id", "brackets"}` payload fails explicitly with `ErrorCode::BadInput` (and `graph.id` fallback fails explicitly with `NotFound`/`BadInput`) rather than corrupting data silently, while the Pairwise Comparisons modal (`crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:230-234`) and Format Inspector (`crates/ui-egui/src/views/inspector.rs:1254`) emit valid flat payloads. Consistent with `FA-SEC19-06`'s `S2` schema-mismatch rating.
- **Evidence:**
  - `crates/tools/src/canvas.rs:522-525`: `build_add_comparison_brackets_intent` emits `json!({ "sheet_id": sheet_id.as_str(), "brackets": brackets_val })`.
  - `crates/engine/src/cmd/graph.rs:723-759`: `GraphAddComparisonBracketsParams` / `schema_graph_add_comparison_brackets` requires top-level `"sheet_id"` and `"result_sheet_id"` with `additionalProperties: false` (`:758`) and has no `"brackets"` property. Notably, `crates/ui-egui/tests/audit_r1_s4b_ui.rs:263-270` has to manually unpack `intent.params["brackets"][0]` into flat keys before calling `app.execute_command`.
  - `crates/tools/src/canvas.rs:335-340, 396`: `click_bracket` calls `self.pending_bracket_start.take()` before `resolve_bracket_group_pair(&first, &anchor)?` (`:829-833`), clearing the first click if the same group is clicked twice; both `click_bracket` (`:340`) and `end_drag_on_graph` (`:396`) substitute `sheet_id` / `graph.id` into `result_sheet_id` when no matching pairwise `ResultSheet` exists.

### Verdict `FA-SEC19-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent data loss via bypassed save pickers + dropped export options; complements `FA-SEC18-01` at the `apps/curvebook` host seam)
- **Evidence:**
  - `apps/curvebook/src/gui_window.rs:550-564`: `commit_pending_intents` only routes `UiIntent::UiCommand | UiIntent::EngineCommand` matching `is_desktop_host_routed_command` (`crates/ui-egui/src/menus/mod.rs:23-48`) plus pathless `graph.insertImage`/`layout.insertImage`. `DESKTOP_HOST_ROUTED_COMMANDS` omits `"export.tiff"`, `"graph.exportSvg"`, `"graph.exportPdf"`, `"graph.exportPng"`, `"project.saveTemplate"`, `"project.instantiateTemplate"`, `"table.importCsv"`, `"table.importPrism"`, `"table.importPzfx"`, `"table.exportCsv"`, `"analysis.exportJson"`, `"analysis.exportCsv"`, and `"result.generateReport"`, whereas `desktop_host.rs:1447-1568` (`menu_action_triggers_native_file_dialog`) includes all of them. Triggers from in-window menus or the Command Palette fall through to `remaining_intents.commit_all_if_settled` (`gui_window.rs:600`), executing without `"path"` and discarding output bytes.
  - `crates/ui-egui/src/gui_actions.rs:2542-2561` & `crates/ui-egui/src/dialogs_canvas/layout_export.rs:229-253`: `ActiveDialog::ExportFigure` executes inside `DialogButtonAction("ok")` in `remaining_intents`, bypassing `host_commands` / OS save pickers (`RfdNativeFileDialogAdapter`).
  - `apps/curvebook/src/desktop_host.rs:1916-1927`: even when routed through `dispatch_native_file_command_with_params`, `export_params` constructs a fresh `json!({ "sheet_id": ..., "path": path_str })` (or `{ "result_sheet_id", "format", "path" }`), dropping every user-supplied option (`width_px`, `height_px`, `dpi`, `color_space`, `compression`, etc.) from `params`.

### Verdict `FA-SEC19-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (unreachable unsaved/CLI sessions in single-viewport shell + `app.quit` cancel bug)
- **Evidence:**
  - `apps/curvebook/src/gui_window.rs:776-778`: `CurveBookNativeApp` runs a single `eframe` viewport (`ViewportId::ROOT`) rendering only `self.active_window_id()` (`self.runtime.focused_window_id().unwrap_or(self.primary_window_id)`), with no `show_viewport_deferred` or multi-window switcher UI. Meanwhile `install_window_session` (`apps/curvebook/src/desktop_host.rs:2253-2256`) unconditionally moves `self.focused_window` to each new session created by `window.new` (`:2902-2913`), `handle_file_dropped` with unsaved changes / `--new-instance` (`:2798-2812`), and `recover_stale_journals` (`apps/curvebook/src/main.rs:472-474`, `desktop_host.rs:2137-2158`), hiding prior unsaved edits or CLI-opened files.
  - `apps/curvebook/src/desktop_host.rs:3505`: `resolve_host_or_ui_route` maps `"file.quit"` and `"app.quit"` identically to `HostAction { action: "window.close" }`. When `runtime.window_count() > 0` after closing one window, `apply_close_outcome` (`gui_window.rs:809-814`) emits `ViewportCommand::CancelClose` instead of quitting across remaining windows.
  - `apps/curvebook/src/main.rs:504-509`: shutdown cleanup persists preferences only for `rt.focused_window_id()`.

### Verdict `FA-SEC19-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (broken keyboard shortcuts and hardcoded/unrebindable bindings)
- **Evidence:**
  - `apps/curvebook/src/desktop_host.rs:4141-4161`: `shortcut_uses_desktop_routing` omits `"ui.analyze.open"` (`Cmd+Shift+A`, `crates/ui-egui/src/menus/ui_commands.rs:50-56`), `"ui.print.open"` (`Cmd+P`, `:250-256`), `"ui.changeGraphType.open"` (`Cmd+Shift+G`, `:66-72`), `"ui.zoom.fitSheet"` (`Cmd+0`, `:82-88`), and `"ui.editParams.open"` (`Cmd+T`, `:58-64`). Unlike `Cmd+T`, which is separately handled in `crates/ui-egui/src/egui_shell/inspect_bridge.rs:1213-1219` (while `gui_window.rs:1082`'s `"ui.editParams.open"` branch is unreachable), `Cmd+Shift+A`, `Cmd+P`, `Cmd+Shift+G`, and `Cmd+0` have no handler in `inspect_bridge.rs:1205-1305` and silently fail (and custom rebindings of `ui.editParams.open` fail too).
  - `apps/curvebook/src/gui_window.rs:1078-1086`: even when `match_routed_desktop_shortcut` returns `"app.preferences"`, `"app.quit"`, `"file.quit"`, or `"edit.cut"`/`"edit.copy"`/`"edit.paste"`, `gui_window.rs:1079-1084` filters them out, leaving `Cmd+,` and `Cmd+Q` non-functional via desktop shortcut routing (`--in-window-menus` / non-macOS).
  - `apps/curvebook/src/gui_window.rs:1054-1070`: hardcodes `Cmd+0` to `UiScaleAdjust::Reset` (shadowing `ui.zoom.fitSheet`), hardcodes `Cmd+O`/`Cmd+S` before querying `app.shortcuts`, and gates `match_routed_desktop_shortcut` on `primary_cmd`, blocking `F1..F12`/`Alt+...` bindings.
  - `apps/curvebook/src/desktop_host.rs:2723-2730`: fallback clipboard handling checks `"c"` and `"v"` on `KeyDown`, omitting `"x"` (`cut_selection`).

### Verdict `FA-SEC19-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent wrong pointer-gesture targeting, batch misordering, and ignored `out.quit` on deferred flush)
- **Evidence:**
  - `apps/curvebook/src/gui_window.rs:412-439`: when `widget_id` / `target_id` is supplied without explicit `x`/`y` and is missing from `self.inspect_meta`, `target_pos` is `None` and `:437` falls back to canvas center (`last_regions.canvas_rect_pt` or `ctx.content_rect().center()`), clicking the wrong target and reporting `Ok(...)` instead of returning `ErrorCode::NotFound`. Also `:418` only blocks disabled controls for `"click" | "double_click"` (allowing `"right_click"` and `"drag"`), and `build_synthetic_egui_gesture_batches` (`:1194-1197`) maps unknown `action` names to a primary click.
  - `apps/curvebook/src/gui_window.rs:261-271`: `has_deferred_in_flight` is evaluated once before iterating `pending`. If `[ui.pointer_gesture, project.save]` (or `ui.inspect`) arrives in the same `try_recv` drain while `has_deferred_in_flight == false`, `ui.pointer_gesture` is pushed to `deferred_live_requests` (`should_defer_to_settled_frame` at `:1243`) while the following command stays in `immediate` and runs ahead of the gesture.
  - `apps/curvebook/src/gui_window.rs:378-381`: `flush_deferred_live_requests` never checks `res.as_ref().is_ok_and(|out| out.quit)` before sending on `reply_tx` (contrast `dispatch_pending_live_requests:296-298`).

### Verdict `FA-SEC19-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (unquoted special chars in excluded TSV tokens + `emit_paste_intent` schema drift dropping `parse_asterisk_exclusions = false`)
- **Evidence:**
  - `crates/tools/src/grid.rs:1017-1027, 1057-1077`: `format_cell_tsv_token` checks `if cell.excluded && !base.is_empty()` (`:1019`) *before* `else if base.contains(['\t', '\n', '\r', '"'])` (`:1021`), returning an unquoted `{base}*` string. `format_selection_tsv` (`:1072`) writes `format_cell_tsv_token` directly without `format_grid_tsv`'s secondary quoting check (`:1042-1047`).
  - `crates/tools/src/grid.rs:960-971` vs `crates/engine/src/cmd/table.rs:2470-2518, 2572-2580`: `emit_paste_intent` sends `"cells": serialized_rows` (whereas `PasteParams` expects `"rows"`/`"grid"`) and top-level `"parse_asterisk_exclusions": parse_asterisk_exclusions` (whereas `PasteParams` expects `"filter": { "parse_asterisk_exclusions": ... }` via `p.filter`, defaulting `ImportFilterSpec::default().parse_asterisk_exclusions` to `true`). Because `schema_paste` sets `additionalProperties: true` (`table.rs:2517`), `"parse_asterisk_exclusions": false` is silently ignored and `"tsv"` is parsed with `*` exclusions enabled.

### Verdict `FA-SEC19-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (premature WAL dirty-command reset on pathless `project.save` + client reply race on headless Egui exit)
- **Evidence:**
  - `apps/curvebook/src/desktop_host.rs:761-765, 3597-3602`: `resolve_project_or_sheet_route` executes `project.save` with `{ "return_bytes": true }` when `session_file_path` is `None`; inside `apply_control_journal_steps`, the `!step.mutated` branch `else if step.command_id == "project.save"` clears `dirty_commands.clear()` unconditionally even when nothing was written to disk and `app.session().is_dirty()` stays `true`.
  - `apps/curvebook/src/main.rs:678-696` & `apps/curvebook/src/live_control.rs:46-51, 221-229, 271-278`: on `out.quit`, the headless `--control` Egui loop sets `shutdown = true`, breaks, and exits `main()`, dropping `LiveControlServerGuard` whose `stop()` joins only `accept_thread` and not active detached `curvebook-control-client` threads still writing/flushing `out.response_bytes`.

### Verdict `FA-SEC19-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (`f64::clamp` panic on `NaN` segment share + missing `.is_finite()` / zero-drag / zoom-cap drift)
- **Evidence:**
  - `crates/tools/src/canvas.rs:846-872`: `adjust_axis_segment_share` checks `new_share.is_finite()`, computes `let pair_total = curr_share + neighbor_share;`, and guards `if pair_total <= 0.1 { return false; }`, passing `pair_total - 0.05` into `new_share.clamp(0.05, pair_total - 0.05)`. If `curr_share` or `neighbor_share` is `NaN` (or opposing `±Inf`), `NaN <= 0.1` is `false`, panicking in `f64::clamp`. *(Note: `crates/doc/src/graph.rs:462` rejects non-finite `length_share` on validated document loads, so triggering this requires a non-validated or programmatically mutated `AxisSpec`.)*
  - `crates/tools/src/canvas.rs:296-306`: `fit_scene` checks `<= 0.0` on `scene_size_pt` and `viewport_size_px` without `.is_finite()`, letting `NaN`/`Inf` propagate into `self.zoom`/`self.pan_pt`.
  - `crates/tools/src/canvas.rs:426`: `end_drag` for `CanvasToolMode::ArrowCallout` emits an intent unconditionally on zero-length clicks without checking non-finite coordinates or minimum drag distance.
  - `crates/tools/src/canvas.rs:28, 33` vs `crates/ui-egui/src/egui_shell/canvas_view.rs:455`: `MAX_ZOOM` (`16.0`), `INERTIA_MAX_ZOOM` (`32.0`), and render clamp (`8.0`) disagree.

### Verdict `FA-SEC19-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (clean `is_dirty()` on pathless startup imports/templates + stale `coasting_active` on dropped project load)
- **Evidence:**
  - `apps/curvebook/src/desktop_host.rs:501-535` & `crates/engine/src/lib.rs:1091-1102`: `Session::from_document(doc)` initializes `dirty: false`. In `open_document_from_files`, `session` only updates dirty status when `primary_archive_path` is `Some(...)`; launching with a pathless template, CSV/TSV tables, or foreign project importer returns a clean session (`is_dirty() == false`) so closing skips unsaved confirmation.
  - `apps/curvebook/src/desktop_host.rs:2843-2846`: `handle_file_dropped` replaces `win.app = next_app`, clears `dirty_commands`, and resets `ime_preedit = None` while leaving `win.coasting_active` intact across document replacement.

### Verdict `FA-SEC19-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (hardcoded English `muda` App submenu labels + unconditional per-frame `sync_macos_menu_for_window`)
- **Evidence:**
  - `apps/curvebook/src/mac_menu.rs:396-414`: `rebuild_live_muda_menu` hardcodes `"About CurveBook"`, `"Settings…"`, and `"Quit CurveBook"` instead of reading localized strings for the active locale. *(Confirmed `mac_menu.rs:528`'s use of `BETA_ACTIVE_LOCALES` matches the intentional beta RTL exclusion asserted in `apps/curvebook/src/gui_window.rs:1872-1877`.)*
  - `apps/curvebook/src/gui_window.rs:178-185, 684-692`: `step_ui` invokes `self.sync_macos_menu_for_window(bar_window_id)` unconditionally on every frame (building `win.chrome_state()` and synchronizing descriptors) before checking `self.mac_menu_host.is_installed_for_nsapp()`.

### Verdict `FA-SEC19-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (CLI flag pre-scan skips `--ui-backend` value + `#[allow(dead_code)]` lint-suppression shim)
- **Evidence:**
  - `apps/curvebook/src/main.rs:160-171, 241-245`: `args_contain_cli_control_token` advances `i += 2` for `"--control" | "--portable-dir" | "--ttf-font" | "--event-script"`, omitting `"--ui-backend"` which consumes two arguments at `:241-245`.
  - `crates/tools/src/grid.rs:1497-1500`: keeps `#[allow(dead_code)] const fn ensure_bounds_constants_used() -> usize { MAX_DATA_SETS + MAX_ROWS + Ordering::Equal as usize }` instead of scoping those imports under `#[cfg(test)]`.
