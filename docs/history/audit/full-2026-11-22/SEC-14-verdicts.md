# Verification Report: SEC-14 — UI Grid, Menus & Canvas Dialogs (`crates/ui-egui`)

- **Section:** `SEC-14` (`26` files in `crates/ui-egui/src/{grid_view.rs, dialogs_r9.rs, power_wizard.rs, text_bridge.rs, i18n.rs}`, `menus/*`, `dialogs_canvas/*`; `19,322` LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-14.md`](SEC-14.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend`

## Summary Table

> Note: `SEC-14.md` reports aggregate severity counts (`S1=0, S2=7, S3=6, S4=0`) without per-finding severity labels, so each confirmed finding below carries a settled severity assigned under [`docs/audit/README.md`](../../../../docs/audit/README.md). `FA-SEC14-06` is settled as `DUPLICATE` of `FA-SEC19-02` (`S1`, already recorded in `SEC-19`) and excluded from unique settled counts.

| Verdict | Count | Settled Findings |
|---|---:|---|
| `CONFIRMED` | 12 | `FA-SEC14-01`, `FA-SEC14-02`, `FA-SEC14-03`, `FA-SEC14-04`, `FA-SEC14-05`, `FA-SEC14-07`, `FA-SEC14-08`, `FA-SEC14-09`, `FA-SEC14-10`, `FA-SEC14-11`, `FA-SEC14-12`, `FA-SEC14-13` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 1 | `FA-SEC14-06` (duplicate of `FA-SEC19-02`) |

| Settled Severity (Unique) | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 9 | `FA-SEC14-01`, `FA-SEC14-02`, `FA-SEC14-03`, `FA-SEC14-04`, `FA-SEC14-05`, `FA-SEC14-07`, `FA-SEC14-09`, `FA-SEC14-10`, `FA-SEC14-13` |
| `S3` | 3 | `FA-SEC14-08`, `FA-SEC14-11`, `FA-SEC14-12` |
| `S4` | 0 | — |

## File Coverage Completeness Check

Verified all `26 / 26` files (`19,322` total LOC, matching `wc -l` counts exactly) appear in the File Coverage Table with complete `1..=N` inspection ranges:
- Root `crates/ui-egui/src/` (`5` files): `grid_view.rs` (`3,305`), `dialogs_r9.rs` (`1,504`), `power_wizard.rs` (`1,254`), `text_bridge.rs` (`802`), `i18n.rs` (`566`).
- `crates/ui-egui/src/menus/` (`5` files): `menu_bar.rs` (`369`), `mod.rs` (`54`), `shortcuts.rs` (`653`), `tooltips.rs` (`316`), `ui_commands.rs` (`441`).
- `crates/ui-egui/src/dialogs_canvas/` (`16` files): `analysis_catalog.rs` (`373`), `analyze_dialog.rs` (`1,533`), `command_palette.rs` (`325`), `edit_params.rs` (`1,564`), `format_axes.rs` (`618`), `format_graph.rs` (`857`), `graph_overlays.rs` (`780`), `graph_type.rs` (`485`), `layout_export.rs` (`277`), `mod.rs` (`702`), `parse_helpers.rs` (`252`), `print_dialog.rs` (`157`), `r13_dialogs.rs` (`583`), `results_hooks.rs` (`445`), `tests.rs` (`453`), `workspace_dialogs.rs` (`654`).

---

## Per-Finding Verdicts

### `FA-SEC14-01` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Visible wrong/resurrected pairwise comparison brackets on graph renders when saving edited `>= 2` or empty bracket sets.
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:206-235`: `PairwiseComparisonsState::brackets_commit_intent` returns `None` if `self.brackets.is_empty()` (`:209-211`), and otherwise inserts `group_a`/`group_b` only inside `if let ([first], Some(map)) = (self.brackets.as_slice(), ...)` (`:230-233`). When `self.brackets.len() >= 2`, `clear_existing: true` is sent without `group_a`/`group_b`, hitting the `(None, None)` loop in `crates/engine/src/cmd/graph.rs:2010-2036`, which re-adds every comparison from the `ResultSheet`.
  - `graph_overlays.rs:283-295`: `commit_intents()` additionally guards `&& !self.brackets.is_empty()`, so clearing all brackets never clears existing `GraphSpec::comparisons`.

### `FA-SEC14-02` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Committing `BubblePlotInspectorState` silently drops custom `x_level_order_override` from existing `MultipleVariables` graph specs (visibly changing categorical X axis order).
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:448-461`: `sync_from_graph` copies `plot_mode`, `x_column`, `y_column`, `color`, `size`, `label_column`, `show_convex_hulls`, and `show_confidence_ellipses`, omitting `mv.x_level_order_override`.
  - `graph_overlays.rs:482-494, 504-514`: `to_encoding()` hardcodes `x_level_order_override: None` (`:490`) and `commit_intent()` sends `graph.setSpec` with `multivariate: self.to_encoding()`.
  - `crates/engine/src/cmd/graph.rs:3011-3014`: `parse_multivariate_encoding` succeeds with `serde_json::from_value::<MultivariateGraphEncoding>` and replaces the full struct, wiping `x_level_order_override`.

### `FA-SEC14-03` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Running or editing `"stats.pcr"` drops `pcr_outcome_column` and produces a visibly labeled `"Principal component analysis"` result sheet rather than PCR regression.
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/analysis_catalog.rs:351`: `"stats.pca" | "stats.pcr"` maps identically to `MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(7) }`, and `crates/doc/src/analysis.rs:897-902` has no `pcr_outcome_column` field (titled `"Principal component analysis"` at `:1926`).
  - `crates/ui-egui/src/dialogs_canvas/analyze_dialog.rs:1437-1475`: `try_build_run_intent`, `resolve_duplicate_choice`, and `build_create_new_intent` send only `source_sheet_id`, `selected_data_sets`, and `kind` without `pcr_outcome_column` (`gui_actions.rs:2702-2745` does not inject it either). `toggle_variable` (`:1135-1140`) mutates `selected_data_sets` without updating `pca_predictors`, and `apply_field_patch_localized` (`:733-743`) silently skips non-`u64` `pcr_outcome_column` values.
  - `crates/ui-egui/src/dialogs_canvas/edit_params.rs:189` resets `pcr_outcome_column = None` in `open_for_result`, and although `commit_intent` (`:1526-1528`) sends `"pcr_outcome_column"` and `crates/engine/src/cmd/analysis.rs:2218` whitelists the key, `apply_multivariate_overrides` (`analysis.rs:3192-3199`) ignores `"pcr_outcome_column"` completely.

### `FA-SEC14-04` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Interactive `EditAnalysisParametersState` checkboxes/selectors accept user changes and commit cleanly while silently dropping them from `AnalysisSpec`. Distinct from `FA-SEC13-11` (which covered stale `Newcombe-Wilson` button enablement via `sync_rule_flags_from_kind`), though both note the omission in `edit_params.rs:285-288`.
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/edit_params.rs:669-673` sets `self.contingency_attributable_risk_checked = b` without calling `update_contingency_spec(|c| c.compute_odds_and_risk_ratios = b)` (compare `:662-667`), and `sync_rule_flags_from_kind` (`:285-288`) omits `contingency_attributable_risk_checked`.
  - `edit_params.rs:646-661` mutates `contingency_or_ci_method`, `contingency_rr_ci_method`, `contingency_arr_ci_method`, and `contingency_diag_ci_method`, and `:421-453` (alongside `gui_actions.rs:792-803` and `egui_shell/stats_modals/two_group.rs:103-110`) mutates `graph_differences`, `graph_ranks`, and `hodges_lehmann_ci`, none of which are serialized by `commit_intent()` (`:1510-1531`) or present in `crates/doc` / `crates/engine`.

### `FA-SEC14-05` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Switching between `X`, `Y`, and `Y2` tabs in `FormatAxesState` discards `Y2` thresholds, discards non-threshold axis edits on prior tabs, and drops `enable_y2` toggles made before switching back to `X`/`Y`.
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/format_axes.rs:102-109, 149-156`: `stage_active_thresholds` and `set_active_thresholds` match `AxisTarget::Y2 => {}`, and `commit_intent` (`:366`) passes `None` for `Y2` thresholds.
  - `format_axes.rs:197-231`: `switch_target_axis_preserving_thresholds` saves only thresholds and additional ticks before calling `self.sync_from_graph(gs, next)` (`:211`), overwriting `axis_draft` bounds, scale, title, segments, tick formatting, and break ornaments.
  - `format_axes.rs:345-369`: `"frame_preset"` (`DualYSecondaryRight`) and `"y2_axis": Value::Null` are only emitted inside the `AxisTarget::Y2` arm (`:345-352`), while line `:365` skips `y2_axis` whenever `!self.enable_y2` or `pending_y2_additional_ticks.is_none()`.

### `FA-SEC14-06` — `DUPLICATE` of `FA-SEC19-02` (`S1`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Evidence:**
  - `FA-SEC19-02` (`SEC-19`, already `CONFIRMED` at `S1`) reported the exact same omission in `DESKTOP_HOST_ROUTED_COMMANDS` (`crates/ui-egui/src/menus/mod.rs:23-48`) omitting menu/palette file actions (`"export.tiff"`, `"table.exportCsv"`, `"analysis.exportCsv"`, `"analysis.exportJson"`, `"table.importCsv"`, `"table.importPzfx"`, `"table.importPrism"`, `"project.saveTemplate"`, `"project.instantiateTemplate"`), causing `commit_pending_intents` (`apps/curvebook/src/gui_window.rs:552-600`) to skip native OS file dialogs.
  - Verification notes on remaining commands named in `FA-SEC14-06`: `gui_window.rs:557-564` already intercepts pathless `"graph.insertImage"` and `"layout.insertImage"`; `"ui.print.open"` opens the in-app Print modal; and `"edit.pasteImage"` (`menu_bar.rs:244`) is filtered out by `is_palette_hidden_engine_command` (`crates/ui-egui/src/egui_shell/command_palette.rs:763, 774`) when `resolve_shell_command_route(app, "edit.pasteImage", false)` runs from `top_chrome.rs:849-859`, silently returning `None` from the in-window menu bar.

### `FA-SEC14-07` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Deselecting a rectangular multi-cell selection down to 1 cell reverts selection predicates and clipboard/exclusion targets to `tool.selection_bounds()`, operating on explicitly deselected cells.
- **Evidence:**
  - `crates/ui-egui/src/grid_view.rs:1383-1405, 1442-1468`: When `toggle_cmd_selection` or `toggle_command_row_selection` removes from a prior range selection down to `selected_coords.len() == 1` while `tool.anchor != tool.active`, `has_discontiguous_selection` (`:1500-1502`) and `visible_frame_for_sheet` (`:1825-1830`) evaluate `selected_coords.len() > 1 || (!selected_coords.is_empty() && tool.anchor == tool.active)` as `false`. Consequently `is_cell_selected` (`:1490-1497`), `clipboard_target` (`:1515-1535`), and rendering revert to `tool.selection_bounds()`, whereas `all_selected_coords` (`:1611`) returns only the single item in `selected_coords`.
  - `grid_view.rs:1885-1913`: `GridViewState::navigate` updates `self.tool.anchor`/`active` without clearing `selected_coords`, `selected_rows`, or `active_x_cell` or refreshing selection stats (`views/grid_core.rs:325-331` compensates manually only in its arrow handler).

### `FA-SEC14-08` — `CONFIRMED` · Severity assigned: `S3`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** UI command enablement / metadata drift and unused fallback method (`activate_selected`), with shell callers largely mitigated by `resolve_palette_route` except contextual UI enablement on empty projects.
- **Evidence:**
  - `crates/ui-egui/src/dialogs_canvas/command_palette.rs:199-235`: `collect_matches` sets `params_doc: spec.summary` (`:207`) for Engine commands and hardcodes `menu_path: "UI"` (`:221`), `enabled: true` (`:226`), and search category `"View"` (`:229`) for every entry in `UI_COMMANDS`. Downstream `filter_palette_commands` (`crates/ui-egui/src/egui_shell/command_palette.rs:76`) only filters `"ui.result.applyRemediation"`.
  - `dialogs_canvas/command_palette.rs:274-278`: `activate_selected(&self)` calls `self.matches()` (with `session = None`) and always emits `ToolCommandIntent::new(entry.id, json!({}))`.

### `FA-SEC14-09` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Passing `"decimal_sep"` alongside `"data"` or `"plate_values"` silently parses comma-decimal numbers as delimiter-separated values (especially since `"data"` precedes `"decimal_sep"` alphabetically in `serde_json::Map`).
- **Evidence:**
  - `crates/ui-egui/src/grid_view.rs:805-822`: `apply_field_patch_localized` hoists `"decimal_separator"` and `"import.decimal_separator"` (`:806, :816`) before parsing `"data"` / `"plate_values"` (`:854-864`), omitting `"decimal_sep"`, which is accepted later inside the loop (`:879-886`).

### `FA-SEC14-10` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Valid small empty-row or multi-cell selections on tables with `> 4_096` rows collapse to empty, trailing empty rows in small-table `Shift+Click` row selections are truncated, and sparse selections past `window_r1` are dropped.
- **Evidence:**
  - `crates/ui-egui/src/grid_view.rs:1743-1777`: `materialized_selection_flat_rect` gates fallback unpopulated selection on `&& total_rows <= MAX_MATERIALIZED_SELECTION_COORDS` (`4_096` at `:24`, checked at `:1773`), causing multi-cell unpopulated selections on `> 4_096`-row sheets to return `None`, while searching only up to `window_r1` (`:1768-1769`) drops sparse populated rows beyond `window_r1`.
  - `grid_view.rs:2114-2130`: `bounded_row_range_vec` truncates small-sheet row ranges at `last_populated_row_in_range(sheet, r0, clamped_r1)` (`:2121`), and on `> 4_096`-row sheets returns `Vec::new()` when `r0 != clamped_r1` and no row in `[r0, window_r1]` is populated (`:2125-2129`).

### `FA-SEC14-11` — `CONFIRMED` · Severity assigned: `S3`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Partial-creation robustness hazard under sheet-count exhaustion or command failure, recoverable via undo/delete without corrupting prior sheets.
- **Evidence:**
  - `crates/ui-egui/src/power_wizard.rs:915-993` and `crates/ui-egui/src/lib.rs:538-546`: `PowerWizardState::execute_create_with` and `UiApp::execute_power_wizard_create` issue sequential individual commands (`sheet.createData`, `table.setTitles`, `table.setXCells`, `table.setCells`, `info.*`, `graph.*`, `analysis.run`, `sheet.select`) via `record_session_cmd_on` without an enclosing `Session` snapshot/rollback transaction, leaving created Data/Info/Graph sheets and partial undo entries if `analysis.run` (`:964`) fails (e.g., `TooManySheets`).

### `FA-SEC14-12` — `CONFIRMED` · Severity assigned: `S3`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** Tooltip bucket, menu path, and session enablement metadata drift with no data loss.
- **Evidence:**
  - Cross-checking `UI_COMMANDS` (`crates/ui-egui/src/menus/ui_commands.rs`, `53` commands) and `curated_menu_bar` (`crates/ui-egui/src/menus/menu_bar.rs:254-305`) against `crates/ui-egui/src/menus/tooltips.rs:119-207`:
    - `ui_command_menu_info` (`:119-160`) handles `37` IDs explicitly and falls back on the remaining `16` IDs (including `11` View-menu entries at `menu_bar.rs:256-271`) to `_ => ("menu.view", "UI")`.
    - Lines `:146-148` and `:150` return bucket `"menu.insert"` for `"Data/..."` items placed under `build_data_menu` (`menu_bar.rs:290-305`).
    - `ui_command_enabled` (`:162-207`) omits `"ui.brush.point"`, `"ui.zoom.fitSheet"`, and `"ui.view.rowTitles.toggle"`, letting them fall through to `_ => true`.

### `FA-SEC14-13` — `CONFIRMED` · Severity assigned: `S2`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity rationale:** On a multi-curve graph with two or more `ModelEquation` annotations (`update_existing` defaults to `true` at `dialogs_r9.rs:1245`), updating the second equation overwrites and destroys the first equation annotation and produces duplicates on the graph.
- **Evidence:**
  - `crates/ui-egui/src/dialogs_r9.rs:1311-1342`: When `self.update_existing` is `true`, `commit_intent` iterates `&mut next_annotations` checking `if let GraphAnnotation::ModelEquation { result_sheet_id, ... } = ann && (result_sheet_id == res_id || !updated)`. With `[eq_A, eq_B]` and `res_id == res_B`, `eq_A` matches `!updated` first and is overwritten with `res_B` (`updated = true`), and then `eq_B` matches `result_sheet_id == res_id` and is also overwritten, leaving two copies of `eq_B` and deleting `eq_A`.
