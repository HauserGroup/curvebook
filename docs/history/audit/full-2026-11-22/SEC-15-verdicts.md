# Verification Report: SEC-15 — UI Views & Localization (`crates/ui-egui`)

- **Section:** `SEC-15` (`45` files: `15` in `crates/ui-egui/src/views/*` [`12,230` LOC] and `30` in `crates/ui-egui/src/i18n/*` [`17,190` LOC]; `29,420` total LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-15.md`](SEC-15.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend-2`
- **Date:** `2026-11-22`

## Summary Table

> Note: `SEC-15.md` reports aggregate severity counts (`S1 = 0`, `S2 = 7`, `S3 = 7`, `S4 = 1`) without per-finding severity labels. Each confirmed finding below carries a settled severity assigned under [`docs/audit/README.md`](../../../../docs/audit/README.md) with independent caller/reachability checks at `7c85a8e516165ddadb101236a88eec937bf1f095`.

| Verdict | Count | Settled Findings |
|---|---:|---|
| `CONFIRMED` | 15 | `FA-SEC15-01`, `FA-SEC15-02`, `FA-SEC15-03`, `FA-SEC15-04`, `FA-SEC15-05`, `FA-SEC15-06`, `FA-SEC15-07`, `FA-SEC15-08`, `FA-SEC15-09`, `FA-SEC15-10`, `FA-SEC15-11`, `FA-SEC15-12`, `FA-SEC15-13`, `FA-SEC15-14`, `FA-SEC15-15` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity (Unique) | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 7 | `FA-SEC15-01`, `FA-SEC15-02`, `FA-SEC15-03`, `FA-SEC15-04`, `FA-SEC15-05`, `FA-SEC15-10`, `FA-SEC15-13` |
| `S3` | 7 | `FA-SEC15-06`, `FA-SEC15-07`, `FA-SEC15-08`, `FA-SEC15-09`, `FA-SEC15-11`, `FA-SEC15-12`, `FA-SEC15-14` |
| `S4` | 1 | `FA-SEC15-15` |

## File Coverage Completeness Check

Verified `45 / 45` files (`29,420` total LOC, verified directly via `wc -l` at `7c85a8e516165ddadb101236a88eec937bf1f095`) appear in [`SEC-15.md`](SEC-15.md#L10-L58) with matching line counts and `1..=N` inspection ranges (`PASS`):
- `crates/ui-egui/src/views/` (`15` files, `12,230` LOC): `mod.rs` (`83`), `a11y.rs` (`243`), `analyze_picker.rs` (`190`), `dialogs_graph_misc.rs` (`818`), `fit_params.rs` (`1,311`), `grid_core.rs` (`951`), `import_paste.rs` (`1,701`), `inspector.rs` (`1,855`), `mv_formula.rs` (`790`), `nav_notes.rs` (`306`), `power_wizard.rs` (`736`), `results.rs` (`348`), `stat_params.rs` (`1,387`), `toolbar_menus.rs` (`645`), `welcome.rs` (`866`).
- `crates/ui-egui/src/i18n/` (`30` files, `17,190` LOC): `cjk_rtl.rs` (`633`), `core_analyses.rs` (`817`), `core_cmd_analysis_graph.rs` (`870`), `core_cmd_project_sheet.rs` (`564`), `core_cmd_table.rs` (`510`), `core_cmd_ui.rs` (`891`), `core_dialogs.rs` (`295`), `core_inspector.rs` (`763`), `core_menus.rs` (`1,159`), `core_nav_tables.rs` (`260`), `eu.rs` (`1,516`), `r13_8_ui.rs` (`1,071`), `shard_extra.rs` (`8`), `tests.rs` (`520`), `w5_analyze_picker.rs` (`297`), `w5_dialogs_graph_misc.rs` (`369`), `w5_fit_params.rs` (`474`), `w5_grid_core.rs` (`548`), `w5_import_paste.rs` (`472`), `w5_inspector.rs` (`837`), `w5_mv_formula.rs` (`585`), `w5_nav_notes.rs` (`334`), `w5_power_wizard.rs` (`322`), `w5_results.rs` (`346`), `w5_stat_params.rs` (`280`), `w5_toolbar_menus.rs` (`877`), `w5_welcome.rs` (`1,001`), `w7_mv_ui.rs` (`348`), `w7_rm_reml.rs` (`121`), `w8_rv_ui.rs` (`102`).

---

## Per-Finding Verdicts

### Verdict `FA-SEC15-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — UI hang / multi-million-element heap allocation on valid `Shift+Arrow` / `Shift+Cmd/Ctrl+Arrow` navigation on large datasheets, directly reachable from keyboard shortcuts and pointer/key event dispatch.
- **Evidence:**
  - [`crates/ui-egui/src/views/grid_core.rs:324-331, 399-420`](../../../../crates/ui-egui/src/views/grid_core.rs#L324-L420): When `GridNavEvent::Arrow { extend: true, .. }` runs (`:327-328`), `sync_extended_selection` loops `for row in min_c.row..=max_row` and `for flat in 0..total_flat`, pushing every coordinate into `grid.selected_coords` and every row index into `grid.selected_rows` without the `MAX_MATERIALIZED_SELECTION_COORDS` guard or `bounded_selected_rows` used by `GridViewState::extend_shift_range` ([`crates/ui-egui/src/grid_view.rs:1375-1380`](../../../../crates/ui-egui/src/grid_view.rs#L1375-L1380)).
  - Caller verification: Live-reachable from [`crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs:94-118`](../../../../crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs#L94-L118) (`push_custom_view_action("grid_core", "nav:shift_*")`) and [`crates/ui-egui/src/interaction.rs:1561`](../../../../crates/ui-egui/src/interaction.rs#L1561) (`handle_grid_key_down`). Note that `handle_grid_key_down` (`grid_core.rs:214-224`) and `handle_nav_custom_action` (`grid_core.rs:791-814`) bind Arrow/Tab/Enter/Delete/Backspace (including `nav:shift_jump_*`), not `PageUp`/`PageDown`/`Home`/`End`. Distinct from `FA-SEC14-07` and `FA-SEC14-10`.

---

### Verdict `FA-SEC15-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — Silent deletion and wrong-range copying of unselected cells on discontiguous selections or empty selections in the public `"toolbar_menus"` custom-action API (distinct from `FA-SEC14-07`).
- **Evidence:**
  - [`crates/ui-egui/src/views/toolbar_menus.rs:120-131`](../../../../crates/ui-egui/src/views/toolbar_menus.rs#L120-L131): `active_selection_coords_for` falls back to pushing `GridCellCoord::new(0, 0, 0)` whenever `coords` and `app.grid.tool.active` are empty on a non-empty sheet; called by `"clipboard.cut"` (`:340`), `"ctx.exclude_with_reason"` (`:505`), and `"ctx.clear_cells"` (`:522`).
  - [`toolbar_menus.rs:179-187, 341-343, 523-525`](../../../../crates/ui-egui/src/views/toolbar_menus.rs#L164-L530): `"clipboard.cut"` and `"ctx.clear_cells"` collapse `coords` via `selection_bounding_range` and send `"range"` to `table.clearCells` even though [`crates/engine/src/cmd/table.rs:2390-2399`](../../../../crates/engine/src/cmd/table.rs#L2390-L2399) accepts explicit `"cells"`, and `copy_active_sheet_to_clipboard` copies the bounding rectangle instead of calling [`GridViewState::prepare_clipboard_selection`](../../../../crates/ui-egui/src/grid_view.rs#L1550-L1557).
  - [`toolbar_menus.rs:165, 336, 503, 516, 532, 540, 632`](../../../../crates/ui-egui/src/views/toolbar_menus.rs#L164-L635): Repeated `app.document().clone()` on dispatch. (Note: desktop `egui_shell/data_grid/shortcuts.rs:72` uses `push_grid_cut_selection()` directly; `"toolbar_menus"` is reached via [`dispatch_custom_view_action`](../../../../crates/ui-egui/src/views/mod.rs#L70).)

---

### Verdict `FA-SEC15-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — Live-reachable wrong column titles and dropped row labels in the `Import & Paste Special` commit handler.
- **Evidence:**
  - Caller verification: [`crates/ui-egui/src/gui_actions.rs:2580`](../../../../crates/ui-egui/src/gui_actions.rs#L2580) invokes `commit_import_paste_dialog(app)` -> `execute_import_paste_commit`.
  - [`crates/ui-egui/src/views/import_paste.rs:1152-1168`](../../../../crates/ui-egui/src/views/import_paste.rs#L1152-L1168): `compute_import_preview` removes `headers[0]` when `state.has_row_titles` is true and stores data rows' first tokens in `row_labels`.
  - [`import_paste.rs:988-1013, 1068-1085`](../../../../crates/ui-egui/src/views/import_paste.rs#L988-L1085): `serialize_preview_as_csv_with_headers` writes `preview.headers` without a row-title placeholder header while prepending `preview.row_labels[r_idx]` to every data row, and passes `"has_header": true, "has_row_titles": state.has_row_titles && !preview.row_labels.is_empty()` to `table.importCsv`. In [`crates/format/src/csv.rs:693-734`](../../../../crates/format/src/csv.rs#L693-L734), `has_row_titles: true` sets `col_offset = 1`, treating `headers[0]` as the row-title column header and shifting dataset titles left by one column.
  - [`import_paste.rs:959-984, 1056-1066`](../../../../crates/ui-egui/src/views/import_paste.rs#L959-L1066): On `state.target_mode == "existing_sheet"`, `serialize_preview_as_tsv` omits `preview.row_labels`.

---

### Verdict `FA-SEC15-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — Live-reachable failure of non-live external CSV import and silent ignore of row-title/exclusion/header flags in `TidyLongPivot` fast-path.
- **Evidence:**
  - [`crates/ui-egui/src/views/import_paste.rs:228-231, 790-799, 916-957, 1111-1120`](../../../../crates/ui-egui/src/views/import_paste.rs#L790-L1120): `WID_PATH` writes `state.external_path`, which is only read inside `if state.paste_as_live_link && matches!(state.source_origin.as_str(), "internal_sheet" | "external_csv")` (`:921, :946`). When `source_origin == "external_csv"` and `paste_as_live_link == false`, execution falls through to `compute_import_preview` (`:916`) -> `effective_source_text` (`:790-799`), which reads only `tidy_csv`, `raw_source_text`, `raw_clipboard_text`, and `WID_CSV`, returning `"Enter or paste CSV/TSV data in Source tab."` (`:1118`) without reading `WID_PATH` or forwarding `"path"` to `table.importCsv` ([`crates/engine/src/cmd/table/import.rs:610-611`](../../../../crates/engine/src/cmd/table/import.rs#L610-L611)).
  - [`import_paste.rs:897-913`](../../../../crates/ui-egui/src/views/import_paste.rs#L897-L913): When `row_wrap_width`, slice bounds, decimation, filters, transpose, and `target_mode` are at defaults, `TidyLongPivot` forwards straight to `app.grid.import_reshape.build_import_intent()` (`table.importTidyLong`), which hardcodes `header_rows: 1`, `has_row_titles: false`, and `parse_asterisk_exclusions: false` in [`crates/engine/src/cmd/table/import.rs:1384-1393`](../../../../crates/engine/src/cmd/table/import.rs#L1384-L1393), ignoring `state.has_row_titles`, `state.parse_asterisk_exclusions`, `state.skip_contiguous_blanks`, `state.has_header_row = false`, and `state.propagate_title_header`.

---

### Verdict `FA-SEC15-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — Wrong statistical summary measure plotted on `"mv.metric:Count"` in the `"inspector"` view shard.
- **Evidence:**
  - [`crates/ui-egui/src/views/inspector.rs:1084-1091`](../../../../crates/ui-egui/src/views/inspector.rs#L1084-L1091): Inside `dispatch_mv_encoding_action`, `"Count"` matches directly to `SummaryMeasure::GeometricMean`, while [`crates/doc/src/graph.rs:508-517`](../../../../crates/doc/src/graph.rs#L508-L517) only provides `Mean`, `Median`, and `GeometricMean` (note: `"inspector"` custom actions are exposed via [`dispatch_custom_view_action`](../../../../crates/ui-egui/src/views/mod.rs#L80) rather than `egui_shell/graph_modals/change_graph_type.rs`).

---

### Verdict `FA-SEC15-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — State/command drift isolated to the `"dialogs_graph_misc"` shard `graph_type:apply` action; desktop modal (`egui_shell/graph_modals/change_graph_type.rs`) commits through `ChangeGraphTypeState` (`gui_actions.rs`) rather than `graph_type:apply`.
- **Evidence:**
  - [`crates/ui-egui/src/views/dialogs_graph_misc.rs:231-268`](../../../../crates/ui-egui/src/views/dialogs_graph_misc.rs#L231-L268): `graph_type:select:`, `graph_type:summary:`, and `graph_type:errorbar:` update `app.canvas_dialogs.change_graph_type`, but `"graph_type:apply"` (`:257-268`) executes `"graph.setKind"` with only `{ "sheet_id": gid.as_str(), "kind": graph_kind }`, omitting `"option_id"` (consumed by [`crates/engine/src/cmd/graph.rs:1532-1543`](../../../../crates/engine/src/cmd/graph.rs#L1532-L1543)) and never applying `summary_measure` or `error_bar_metric` (contrast `ChangeGraphTypeState::commit_intents` in [`crates/ui-egui/src/dialogs_canvas/graph_type.rs:380-431`](../../../../crates/ui-egui/src/dialogs_canvas/graph_type.rs#L380-L431); note `SEC-15.md` line `368` refers to it as `build_intents`).

---

### Verdict `FA-SEC15-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — UI commands selected in `"palette:run"` fail cleanly with an engine error (not silent corruption), and `egui_shell/command_palette.rs` handles desktop palette execution separately; distinct from `FA-SEC14-08`.
- **Evidence:**
  - [`crates/ui-egui/src/views/dialogs_graph_misc.rs:202-218`](../../../../crates/ui-egui/src/views/dialogs_graph_misc.rs#L202-L218): `"palette:run"` resolves `cmd_id` from `command_palette.matches_for_session(app.session())`—which populates both `CommandOrigin::Engine` and `CommandOrigin::Ui` (`UI_COMMANDS`) in [`crates/ui-egui/src/dialogs_canvas/command_palette.rs:199-234`](../../../../crates/ui-egui/src/dialogs_canvas/command_palette.rs#L199-L234)—and unconditionally calls `app.execute_command(&cmd_id, json!({}))` instead of dispatching `UI_COMMANDS` via `dispatch_ui_command`.

---

### Verdict `FA-SEC15-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — Viewport zoom/pan state poisoning requiring malformed non-finite payload strings (`"zoom:set:*:NaN"` or `"pan:nudge:*:NaN:*"`) and recoverable via `"zoom:set:*:fit"` or `"pan:reset:*"`.
- **Evidence:**
  - [`crates/ui-egui/src/views/dialogs_graph_misc.rs:102-117, 132-135, 151-159`](../../../../crates/ui-egui/src/views/dialogs_graph_misc.rs#L102-L159): `preset.parse::<f64>()` (`:132`) and `dx_s.parse::<f64>()` / `dy_s.parse::<f64>()` (`:155`) do not check `.is_finite()`, and `apply_zoom_and_pan` runs `z.clamp(0.25, 4.0)` and `(pan_pt + delta).clamp(-1200.0, 1200.0)`, which return `f64::NAN` on `NaN` inputs and store `NaN` into `canvas_dialogs.canvas` and `pan_zoom`.

---

### Verdict `FA-SEC15-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — Comma-decimal values dropped in `"fit_params"` custom actions (while `egui_shell/fit_modal.rs` manages its own parameter widgets, `sync_fit_parameter_inputs` already uses `parse_localized_number_input`, and dot-decimals provide a workaround).
- **Evidence:**
  - [`crates/ui-egui/src/views/fit_params.rs:781-810, 924-930, 989-996, 1178-1181`](../../../../crates/ui-egui/src/views/fit_params.rs#L781-L1181): Nonlinear constant, bound, initial override, CI level, and `parse_optional_bound` (used by `"apply_range"` at `:1099-1101`) parse via stdlib `.parse::<f64>()` instead of `parse_localized_number_input(..., &app.locale_preset)` (which `read_linear_range_inputs` at `:1191` and `read_r_squared_threshold` at `:1225` use), silently keeping previous values while returning `true` (`handled: true`).
  - [`fit_params.rs:961-964, 1007-1010`](../../../../crates/ui-egui/src/views/fit_params.rs#L961-L1010): `handle_confidence_action` silently returns `false` (`handled: false`, no diagnostic message) when `"ci_method:profile"` is rejected under `robust_or_rout(nl.method)` (`:962`) or when `"toggle_emit_cov"` is rejected under `FitMethod::RobustCauchy` (`:1008`; precision note: `:1008` blocks only `RobustCauchy`, not `ROUT`).

---

### Verdict `FA-SEC15-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — Unintended destruction of **all** `linked_regions` on the datasheet when unlinking a single region via `grid_core` or `toolbar_menus`.
- **Evidence:**
  - [`crates/ui-egui/src/views/grid_core.rs:762-777`](../../../../crates/ui-egui/src/views/grid_core.rs#L762-L777): `"link:unlink:{region_id}"` immediately drops `region_id` (`let _ = region_id;` at `:767`) and dispatches `table.clearCells` with `"cells": [], "unlink": true`.
  - [`crates/ui-egui/src/views/toolbar_menus.rs:631-635`](../../../../crates/ui-egui/src/views/toolbar_menus.rs#L631-L635): `"ctx.unlink_region"` similarly sends `"unlink": true, "cells": []`.
  - [`crates/engine/src/cmd/table.rs:2410-2412`](../../../../crates/engine/src/cmd/table.rs#L2410-L2412): On `unlink == true`, the engine executes `ds.linked_regions.clear()`, detaching every linked region on the sheet.

---

### Verdict `FA-SEC15-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — Non-atomic multi-command sample creation recoverable via undo/sheet deletion (same severity calibration as `FA-SEC14-11` for `PowerWizardState`).
- **Evidence:**
  - [`crates/ui-egui/src/views/welcome.rs:532-557, 606-766`](../../../../crates/ui-egui/src/views/welcome.rs#L532-L766): `instantiate_sample_via_commands` executes `sheet.createData` (`:544`) followed sequentially by `populate_sample_x_values`, `populate_sample_titles`, `populate_sample_cells`, and `populate_sample_mv_roles_and_formulas` (`:546-549`) without rollback on partial failure and emitting multiple separate undo entries per sample.

---

### Verdict `FA-SEC15-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — Five localized UI state/layout/stepper bugs, with item 5 live in the `egui_shell` Simulate Distributions modal and items 1–4 in `"inspector"` custom-action helpers.
- **Evidence:**
  1. [`crates/ui-egui/src/views/inspector.rs:747-809`](../../../../crates/ui-egui/src/views/inspector.rs#L747-L809): `build_segments_for_count` defaults `Auto` bounds to `(0.0, 100.0)` (`:749-751`) and clears `major_tick_interval: None` across multi-segment splits (`:769, :777, :788, :796, :804`).
  2. [`inspector.rs:942-946`](../../../../crates/ui-egui/src/views/inspector.rs#L942-L946): `hi <= lo` in `"range_apply"` silently returns `Ok(UiEventOutcome::default())` (`handled: false`) without error feedback.
  3. [`inspector.rs:1272-1292`](../../../../crates/ui-egui/src/views/inspector.rs#L1272-L1292): `"sig_only:toggle"` returns `Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")))` even when `find_pairwise_result_sheet_id` returns `None` or `gs.spec.comparisons.is_empty()` and no command executed.
  4. [`inspector.rs:1475`](../../../../crates/ui-egui/src/views/inspector.rs#L1462-L1482): `rearrange_layout_grid` wraps `(panel_idx / usize::from(cols)) % usize::from(rows)`, stacking panels beyond `rows * cols` onto row `0`.
  5. [`crates/ui-egui/src/views/dialogs_graph_misc.rs:753-766`](../../../../crates/ui-egui/src/views/dialogs_graph_misc.rs#L753-L766): Called live from [`crates/ui-egui/src/egui_shell/stats_modals/r9_modals.rs:427, 437`](../../../../crates/ui-egui/src/egui_shell/stats_modals/r9_modals.rs#L427-L437); `apply_sim_dist_param_step` updates `SimDistKind::Uniform => col.min += s` unclamped (`:754`), allowing `col.min >= col.max` in [`SimulateDistributionsDialogState`](../../../../crates/ui-egui/src/dialogs_r9.rs#L1030) while `p2_*` clamps `col.max = (col.max + s).max(col.min + 0.5)`.

---

### Verdict `FA-SEC15-13` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S2` — Live-reachable wrong-tab / lost-selection clipboard copy in `results::handle_copy_action`, plus invalid `"stage:0"` coalescing in `power_wizard`.
- **Evidence:**
  - Caller verification: `results::handle_custom_action(..., "copy_selection:...")` is invoked live from [`crates/ui-egui/src/egui_shell/mod.rs:2471`](../../../../crates/ui-egui/src/egui_shell/mod.rs#L2471), [`crates/ui-egui/src/interaction.rs:1300`](../../../../crates/ui-egui/src/interaction.rs#L1300), and [`crates/ui-egui/src/lib.rs:1321`](../../../../crates/ui-egui/src/lib.rs#L1321) (and `"copy_table:..."` from [`crates/ui-egui/src/egui_shell/results_view.rs:1135`](../../../../crates/ui-egui/src/egui_shell/results_view.rs#L1135)).
  - [`crates/ui-egui/src/views/results.rs:53-60, 238-247`](../../../../crates/ui-egui/src/views/results.rs#L53-L247): `handle_copy_action` ignores `state.bound_tab_idx`, looking up `let active_idx = app.canvas_dialogs.result_view.active_tab_index;` (`:238`), resolving `tab_ref = rs.tabs.get(active_idx).or_else(|| rs.tabs.first())` (`:241`), and passing `active_idx` to `state.selection_bounds(sheet_id, active_idx)` (`:245`). Verification precision note: contrary to `SEC-15.md:602` ("using tab N's selection coordinates"), `selection_bounds` (`:54`) checks `self.bound_tab_idx != Some(tab_idx)` and therefore returns `None` on mismatch (`bound_tab_idx != active_tab_index`, e.g. via `select_cell:`/`select_row:`/`select_col:` or when `active_tab_index >= rs.tabs.len()` while `egui_shell/mod.rs:2641` synced the clamped tab index from `results_view.rs:213`), causing `handle_copy_action` to discard the selection and copy the **entire** `tabs[active_idx]` (or `tabs[0]`) instead.
  - [`crates/ui-egui/src/views/power_wizard.rs:146-148`](../../../../crates/ui-egui/src/views/power_wizard.rs#L146-L148): `PowerWizardStage::from_index(idx.saturating_sub(1))` coalesces `"stage:0"` to stage `0` (`1. Experiment Design`) instead of rejecting `0`.

---

### Verdict `FA-SEC15-14` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S3` — Visible non-English localization quality gaps and leaked internal `"QOL-7"` milestone identifier without functional blockage.
- **Evidence:**
  - [`crates/ui-egui/src/i18n/core_menus.rs:1051-1068`](../../../../crates/ui-egui/src/i18n/core_menus.rs#L1051-L1068): `menu.file.import_reshape` retains `"Tidy Long"` in all `15` non-English locales, whereas [`core_cmd_table.rs:438-455`](../../../../crates/ui-egui/src/i18n/core_cmd_table.rs#L438-L455) (`cmd.table.importTidyLong`) localizes it across all `16` locales.
  - [`crates/ui-egui/src/i18n/core_cmd_ui.rs:351-368`](../../../../crates/ui-egui/src/i18n/core_cmd_ui.rs#L351-L368): `cmd.ui.result.applyRemediation` embeds internal milestone tag `"QOL-7"` across all `16` locales.
  - [`crates/ui-egui/src/i18n/w5_power_wizard.rs:160-213, 304-321`](../../../../crates/ui-egui/src/i18n/w5_power_wizard.rs#L160-L321): `pw.section.sensitivity`, `pw.param.alpha`, `pw.param.target_power`, and `pw.plot.title` retain English/Latin `"Alpha x Power"`, `"1 - beta"`, `"Power vs."`, and `"vs."` in multiple European locales (e.g., `nl` `"Gevoeligheidsmatrix (Alpha x Power)"`, `"Doelpower (1 - bèta)"`, `"Power vs. Steekproefgrootte"`; `fr` `"Puissance vs Taille d'échantillon"`).
  - [`crates/ui-egui/src/i18n/w5_import_paste.rs:367-387`](../../../../crates/ui-egui/src/i18n/w5_import_paste.rs#L367-L387): `import_paste.notes.record_info` retains `"Info-Blatt"` / `"Info"` in `de`, `es`, `fr`, `pt-BR`, `it`, and `pl`.

---

### Verdict `FA-SEC15-15` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity assigned:** `S4` — Unused Wave 6 localization staging code, already-applied proposal tables, and stale module documentation.
- **Evidence:**
  - [`crates/ui-egui/src/i18n/cjk_rtl.rs:1-55, 335-633`](../../../../crates/ui-egui/src/i18n/cjk_rtl.rs#L1-L633): Verified via repo-wide search that `try_tr_staged` (`:31-35`) and all `14` `w6.cjk_rtl.*` entries in `STAGED_ENTRIES` (`:38-333`) have zero references outside `cjk_rtl.rs`, while all `37` `CatalogReplacementProposal` items in `REPLACEMENT_PROPOSALS` (`:336-633`) already have their `proposed` text present in target i18n files (`37/37`) while still referencing `crates/ui-egui/src/i18n.rs`.
  - [`crates/ui-egui/src/i18n/eu.rs:1-35`](../../../../crates/ui-egui/src/i18n/eu.rs#L1-L35): Verified `eu.rs` defines `83` unique `w6.view.*` entries, of which `62` have zero references outside `crates/ui-egui/src/i18n/`.
  - `cjk_rtl.rs:3-16` and `eu.rs:3-13`: Doc comments state `ENTRIES` stays empty at the `653`-key baseline, contradicting `pub const ENTRIES: &[CatalogEntry] = STAGED_ENTRIES;` and [`crates/ui-egui/src/i18n/tests.rs:60`](../../../../crates/ui-egui/src/i18n/tests.rs#L60) (`catalog_len() == 867`).
  - [`crates/ui-egui/src/i18n/shard_extra.rs:1-8`](../../../../crates/ui-egui/src/i18n/shard_extra.rs#L1-L8): Exports only `pub const ENTRIES: &[CatalogEntry] = &[];`.
