# Verification Verdicts: SEC-13 — UI Core Control, Actions & Input (`crates/ui-egui`)

- **Section:** `SEC-13` (UI Core Control, Actions & Input)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-13.md`](SEC-13.md)
- **Baseline SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-frontend`
- **Date:** `2026-11-22`

## Summary

| Verdict | Count | Finding IDs |
|---|---:|---|
| `CONFIRMED` | 14 | `FA-SEC13-01`..`FA-SEC13-14` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Finding IDs |
|---|---:|---|
| `S1` | 1 | `FA-SEC13-01` |
| `S2` | 8 | `FA-SEC13-02`, `FA-SEC13-03`, `FA-SEC13-06`, `FA-SEC13-07`, `FA-SEC13-08`, `FA-SEC13-09`, `FA-SEC13-13`, `FA-SEC13-14` |
| `S3` | 5 | `FA-SEC13-04`, `FA-SEC13-05`, `FA-SEC13-10`, `FA-SEC13-11`, `FA-SEC13-12` |
| `S4` | 0 | — |

## File Coverage Completeness Check

- **Result:** **PASS (10 / 10 scoped source files + `crates/ui-egui/Cargo.toml` = 11 files, 20,368 LOC — 100% covered)**
- Independently verified all files in `SEC-13`'s brief scope (`#38`) via `wc -l` and top-level directory listing:
  - `crates/ui-egui/Cargo.toml` (`47`)
  - `crates/ui-egui/src/`: `control_io.rs` (`4,799`), `feedback.rs` (`632`), `gui_actions.rs` (`3,197`), `inspect.rs` (`2,506`), `interaction.rs` (`1,830`), `lib.rs` (`1,840`), `modal_predicates.rs` (`1,131`), `os_integration.rs` (`2,130`), `theme.rs` (`664`), `workspace_nav.rs` (`1,592`).
- Every file and exact LOC matches the File Coverage Table in [`docs/audit/full-2026-11-22/SEC-13.md`](SEC-13.md). *(Note: the remaining five top-level files under `crates/ui-egui/src/` — `dialogs_r9.rs`, `grid_view.rs`, `i18n.rs`, `power_wizard.rs`, `text_bridge.rs` — are assigned to `SEC-14`.)*

---

## Per-Finding Verdicts

### Verdict `FA-SEC13-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S1` (filesystem sandbox / `ExternalPathPolicy` bypass)
- **Evidence:**
  - `crates/ui-egui/src/control_io.rs:3681-3684`: `write_host_file(path, bytes)` passes `std::path::Path::new(path)` directly to `curvebook_platform::fs::write_atomic` without consulting `app.session().path_policy()`.
  - `crates/ui-egui/src/gui_actions.rs:118-135` (`handle_ui_screenshot`), `crates/ui-egui/src/lib.rs:1434-1452` (`dispatch_egui_screenshot_action`), and `crates/ui-egui/src/control_io.rs:2091-2101` (`handle_render_pdf`) only reject writes when `app.is_gui_only()` is `true`; whenever `gui_only == false` (even when a host configures `ExternalPathPolicy::Workdir` or `RejectExternal` via `session.set_path_policy`, as in `apps/curvebook-cli/src/main.rs:175` and `crates/mcp/src/tools.rs:372-400`), `ui.screenshot` and `render.pdf` with `"path"` bypass `ExternalPathPolicy::write_atomic_bytes` (`crates/engine/src/path_policy.rs:181-197`, `crates/engine/src/lib.rs:1298-1300`).

### Verdict `FA-SEC13-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (silent incomplete rollback on large atomic `command.batch` after undo-stack eviction)
- **Evidence:**
  - `crates/ui-egui/src/lib.rs:301-317, 509-528`: `BatchUndoTracker::capture_fallback_baseline_before` only clones a full pre-batch `fallback_session` when `(t.baseline_dirty && command_id == "project.save") || (t.net_batch_depth < 0 && !is_undo_or_redo)`.
  - `crates/engine/src/lib.rs:713-721, 1197-1209`: `push_undo_snapshot` evicts `undo_stack.pop_front()` whenever `undo_stack.len() > MAX_UNDO_ENTRIES` (`64`), `total_cells > MAX_UNDO_CELL_BUDGET` (`25_000_000`), or `total_cp_bytes > MAX_UNDO_CHECKPOINT_BYTES_BUDGET` (`16 MiB`). Meanwhile `control_io.rs:40, 1720` permits up to `MAX_CONTROL_BATCH_STEPS = 1_000` steps per batch *(note: `SEC-13.md` states `256`; the actual ceiling in code is `1_000`)*.
  - `crates/ui-egui/src/control_io.rs:1671-1686`: on rollback, when `tracker.fallback_session` is `None`, `rollback_or_commit_partial_batch` loops `0..tracker.net_batch_depth` executing `let _ = app.session.undo();`, discarding `Ok(false)` / `Err(...)` once evicted entries run out while leaving prior mutations committed and returning `rolled_back: true`.

### Verdict `FA-SEC13-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (panic-guard and per-request token check bypass + `ui.inspect` backend mismatch in `command.batch` / `ui.action`)
- **Evidence:**
  - `crates/ui-egui/src/lib.rs:922-934, 1052-1064`: `dispatch_control_line_with_action` calls `try_dispatch_egui_backend` *before* `handle_control_line_with_action` (`control_io.rs:1039-1057`), bypassing `std::panic::catch_unwind`. Similarly, `apps/curvebook/src/desktop_host.rs:896-928` runs `try_dispatch_egui_backend` on authenticated connections before `serve_control_stream_with_quit_and_hook` (`control_io.rs:883-985`), skipping both `catch_unwind` (`:894`) and `authorize_request_envelope`'s per-request `"token"` verification (`:974-985`).
  - `crates/ui-egui/src/gui_actions.rs:90-97`: `handle_ui_inspect` (reached when `"ui.inspect"` runs inside `command.batch` or `ui.action`) unconditionally calls `inspect_ui_tree_for_metrics` instead of `app.inspect_egui_shell_snapshot`, diverging from `try_dispatch_egui_backend` (`lib.rs:1060`).
  - `crates/ui-egui/src/lib.rs:1399-1470` (`dispatch_egui_screenshot_action`) duplicates `gui_actions.rs:99-153` (`handle_ui_screenshot`).

### Verdict `FA-SEC13-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` (clipboard adapter / `human_path_only` bypass in `ui.result.copyMethodsReport`; returned JSON still carries `"report"`, workaround via `skip_os_clipboard`)
- **Evidence:**
  - `crates/ui-egui/src/control_io.rs:2611-2650`: `dispatch_ui_result_copy_methods_report` only checks `app.is_gui_only() || skip_os_clipboard`, calls `write_os_pasteboard(&report)` directly under `#[cfg(not(target_arch = "wasm32"))]` (`:2632-2637`), ignores `app.is_human_path_only()` and `app.clipboard_adapter` (contrast `UiApp::write_clipboard_text` at `lib.rs:776-781`), and swallows `Err` returns into `Value::Null`.

### Verdict `FA-SEC13-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` (headless-first architecture bypass on public `WorkspaceNavState` / draft methods; currently exercised only in `tests/grid_perf_s1.rs`)
- **Evidence:**
  - `crates/ui-egui/src/workspace_nav.rs:953-983`: `click_navigator_entry` and `click_family_entry` take `&mut ProjectDocument`, mutate it directly via `doc.select_in_navigator(sheet_id)?` / `doc.select_in_family_pane(sheet_id)?`, *and* enqueue a `sheet.select` `ToolCommandIntent`.
  - `workspace_nav.rs:293-300, 477-489`: `TableCreationDraft::create_into_document` and `ClonePipelineDraft::execute_in_document` similarly call `doc.add_data_sheet(...)` / `doc.select_in_navigator(...)` directly outside `Session::execute`, bypassing undo, dirty tracking, and recovery journaling. Repo-wide grep confirms callers are currently limited to `crates/ui-egui/tests/grid_perf_s1.rs`.

### Verdict `FA-SEC13-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (plain `Enter` in multiline modal textboxes prematurely submits and closes open modal)
- **Evidence:**
  - `crates/ui-egui/src/interaction.rs:1624-1637, 1660-1742`: inside `handle_modal_key_down`, when a focused textbox has `is_text_widget_id(&fid) == true`, `handle_textbox_key` only handles `"Enter" | "Return"` for `"formula_bar.input" | "mv_formula.name.input"` (`:1667-1681`) and returns `Ok(None)` for all other IDs (including `import_paste.input.csv`, `import_paste.input.notes`, `import_paste.input.plate_values`, `page_layout.callout_input`, `exclude.note_input`). `handle_modal_key_down` then falls through to `:1631-1636` and executes `submit_active_dialog_ok(app)`.

### Verdict `FA-SEC13-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (textbox `Cmd/Ctrl+C` ignored by `Cmd/Ctrl+V` when OS/adapter clipboard is non-empty, plus missing `Cmd/Ctrl+X`)
- **Evidence:**
  - `crates/ui-egui/src/interaction.rs:1694-1708`: `handle_textbox_key` handles `(ctrl || meta) && c` by assigning only `app.interaction.last_clipboard_text = Some(sel)` without calling `app.write_clipboard_text(&sel)`. Meanwhile `(ctrl || meta) && v` calls `read_clipboard_text_for_paste(app)` (`:1317-1326`), which returns `app.read_clipboard_text()` first whenever available and non-empty, silently pasting stale OS/adapter clipboard content instead of the copied textbox selection. `Cmd/Ctrl+X` has no handler in `handle_textbox_key`.

### Verdict `FA-SEC13-08` · `CONFIRMED` `[PARTIAL REGRESSION vs R15-GUI-12]`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (silent wrong-column Y paste on `Cmd/Ctrl+V` when an X/Time cell is active)
- **Evidence:**
  - `crates/ui-egui/src/grid_view.rs:1365-1371`: `GridViewState::select_x_cell_sync` sets `self.active_x_cell = Some((row, subcol))` and clears `self.tool.active = None`.
  - `crates/ui-egui/src/interaction.rs:1328-1358`: `try_handle_workspace_paste_shortcut` ignores `app.grid.active_x_cell` and always reads `app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0))` to invoke `table.paste` into Y columns — unlike `toggle_exclude_active_grid_cells` (`:1389-1403`) and `apply_type_text_to_grid` (`gui_actions.rs:1415-1433`, `apply_type_text_to_x_cell` at `:1488-1563`).

### Verdict `FA-SEC13-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (unlisted active textbox IDs bypass text editing + `info.notes` edits dropped by `sync_focused_text_field`)
- **Evidence:**
  - `crates/ui-egui/src/interaction.rs:1182-1198`: `is_text_widget_id` matches 11 explicit IDs plus `*search*`/`*input*`, omitting 26 `textbox` IDs emitted in `crates/ui-egui/src/inspect.rs` (`power.standardized_effect:839`, `power.alpha:848`, `power.target_power:849`, `format_points.color:900`, `format_points.size_pt:902`, `edit.contingency.expected_proportions:1566`, `edit.glm.predictor_columns:1602`, `edit.cox.predictor_columns:1609`, `edit.fit.model_id:1633`, `edit.fit.custom_dsl:1636`, `import.title:1705`, `import.x_title:1707`, `import.tidy_csv:1710`, `axes.thresholds:1866` [whereas `is_text_widget_id` checks `"axes.y_thresholds"`/`"axes.x_thresholds"`], `create_graph.y_axis_title:2073`, `create_graph.title:2074`, `randomize.groups:2180`, `randomize.block_sizes:2183`, `randomize.title:2185`, `sample.title:2228`, `simulate.title:2255`, `simulate.column_name:2264`, `change_data.selected_data_sets:2358`, `exclude.note:2411` [whereas `sync_focused_text_field:1120` matches `"exclude.note_input"`], `checkpoints.name:2436`, `checkpoints.note:2437`). *(Precision note: `"edit.custom_title"` cited in `SEC-13.md` appears as a patch/read key in `gui_actions.rs:1227, 1301` and `edit_params.rs:500`, not as a node in `inspect.rs`.)*
  - `crates/ui-egui/src/interaction.rs:1095-1141`: `sync_focused_text_field` has no arm for `"info.notes"` or `"info.secondary.notes"`, falling into `_ => {}` without updating the Info sheet.

### Verdict `FA-SEC13-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` (redundant unscoped `Tab` intercept when `formula_bar.active` is true; workaround: close formula bar or click target)
- **Evidence:**
  - `crates/ui-egui/src/interaction.rs:1441-1457` already handles `Tab` completion when `app.focus_widget_id.as_deref() == Some("formula_bar.input")`. Lines `:1467-1479` then check `if app.grid.formula_bar.active` unconditionally without checking `focus_widget_id`, intercepting `Tab` even when keyboard focus is elsewhere.

### Verdict `FA-SEC13-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` (stale initial enablement for `edit.contingency.ci.newcombe_wilson`; in-dialog workaround: toggle `edit.contingency.attributable_risk`, matching `FA-SEC18-19`'s `S3` classification)
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/edit_params.rs:285-288`: `sync_rule_flags_from_kind` sets `contingency_odds_ratio_checked` and `contingency_relative_risk_checked` from `spec.compute_odds_and_risk_ratios`, leaving `contingency_attributable_risk_checked` unset (`false`).
  - `crates/ui-egui/src/modal_predicates.rs:885-898` and `crates/ui-egui/src/inspect.rs:1376-1399`: Baptista-Pike and Koopman check `edit.contingency_*_checked || cont.compute_odds_and_risk_ratios`, whereas Newcombe-Wilson (`modal_predicates.rs:897`, `inspect.rs:1397`) checks only `contingency_attributable_risk_checked`.

### Verdict `FA-SEC13-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` (predicate drift between `modal_predicates.rs`, `gui_actions.rs`, and `inspect.rs`)
- **Evidence:**
  - `crates/ui-egui/src/modal_predicates.rs:743-745` vs `crates/ui-egui/src/gui_actions.rs:2140, 2196, 2678-2685`: `evaluate_format_points_button` disables `ok`/`apply` whenever `dialogs.format_points.selected_points.is_empty()`. Because `submit_active_dialog_ok` (`:2140`) and `handle_answer_dialog` (`:2196`) both gate on `evaluate_dialog_button`, the brushed/grid fallback inside `commit_format_points_action` (`:2679-2685`) cannot execute if `selected_points` starts empty *(normally pre-populated by `dispatch_ui_format_points_open` at `control_io.rs:2468-2490`)*.
  - `crates/ui-egui/src/modal_predicates.rs:932-961` vs `crates/ui-egui/src/inspect.rs:1117-1123`: `evaluate_analyze_widget` has no match arm for `"analyze.factorial.residual_covariance"` (`ControlEnablement::enabled()`), whereas `inspect.rs:1122` gates it with `r1_sphericity_or_reml_control(st.factorial_repeated_measures != RepeatedMeasuresDesign::None)`.

### Verdict `FA-SEC13-13` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (wrong `ui.inspect` value for `duplicate.choice` with no signal that underlying choice differs)
- **Evidence:**
  - `crates/ui-egui/src/inspect.rs:2343-2346`: `build_generic_dialog_nodes` hardcodes `json!("create_new")` on `"duplicate.choice"` instead of inspecting `app.view_shards.analyze_picker.duplicate_choice` (contrast `crates/ui-egui/src/gui_actions.rs:2753-2779`).

### Verdict `FA-SEC13-14` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` (non-atomic two-command X+Y paste splits undo and risks partial table write)
- **Evidence:**
  - `crates/ui-egui/src/gui_actions.rs:1517-1551`: in `apply_type_text_to_x_cell`, when tabbed multi-column text spills past `avail_x_cols`, Y columns are written first via `app.execute_command("table.paste", ...)` (`:1519-1530`) and X cells are written second via `app.execute_command("table.setXCells", ...)` (`:1533-1551`). A single `edit.undo` only reverts `table.setXCells` while leaving Y columns pasted, and a failure in `table.setXCells` leaves the `table.paste` mutation applied.
