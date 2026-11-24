# Audit Report — SEC-17: Egui Shell Grid, Studio & Views

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `crates/ui-egui/src/egui_shell/` (`info_view.rs`, `results_view.rs`, `unit_spinners.rs`, `welcome_hub.rs`, `data_grid/**`, `trailing_studio/**`) — 23 files, 16,194 LOC
- **Date:** `2026-11-22`
- **Summary counts:** `S1=0`, `S2=4`, `S3=6`, `S4=1` (Total: `11` findings)

---

## Verified Baseline & Resolved-Ticket Status

- **Architecture & Safety Invariants:** All 23 files preserve headless-first discipline (zero direct mutations to `ProjectDocument`; all document changes dispatch through `PendingUiIntents` -> `Session::execute_as_ui` / `dispatch_ui_command_as_ui`), zero `unsafe`, zero `chrono`, zero network calls, zero UTF-8 byte-index slicing panics, and zero `.unwrap()`/`.expect()`/`panic!()` outside `#[cfg(test)]`.
- **Resolved prior tickets verified intact:**
  - `#14` (`mv_formula.rs:74-75`): `sumif(var, cond)` and `meanif(var, cond)` signatures match `curvebook-fit`.
  - `R15-GUI-29` & `R15-GUI-32` (`data_grid/multi_archetype.rs:948-1177`): Row/cell context menu exposes `ui.formatPoints.open`, `graph.addAnnotation`, `graph.formatSelectedPoints`, `table.excludeCells`, `ui.grid.excludeWithReason`, `edit.cut`, `edit.copy`, `edit.paste`, and `Duplicate Column` (`table.paste` + `table.setTitles`).
  - `R15-GUI-36` (`trailing_studio/mod.rs:453-529`): Draggable splitter (`studio.splitter`) updates `TrailingWidth` via `SyncTrailingWidth` and supports double-click width reset.
  - `GWF-15` (`trailing_studio/page_layout.rs:362-392, 454-504`): Page Layout Studio includes `+ Graph Panel` (`layout.addPanel`) and `Front`/`Forward`/`Backward`/`Back` (`layout.reorderPanel`), noting the runtime `egui_views` selection gap captured in `FA-SEC17-01`.
  - Engine param contracts verified cleanly matching: `table.excludeCells` flat range & X-target forms (`table_selection.rs:126-169`, `multi_archetype.rs:1073-1087`), `table.setXCells` single-cell & batch forms (`inline_edit.rs:424`, `welcome_hub.rs:2090`), `table.clearCells` with `"cells": [], "unlink": true` (`linked_regions.rs:218, 412`), `table.paste` `"mode": "append_columns"` (`multi_archetype.rs:1154-1162`), `layout.setSpec` presets `"85mm"`/`"174mm"`/`"a4_portrait"`/`"letter_portrait"` (`page_layout.rs:54-131`), `ui.export.figure.open` `"dpi"` patch (`page_layout.rs:450`), `graph.setSeries` `"alpha"` and `graph.setAxes` `"frame_preset"` (`graph_inspector.rs:344, 634`), and `ui.table.openFormulaBar` `"col"` (`mv_formula.rs:484`).

---

## File Coverage Table

| File | LOC | Lines Inspected | Concrete Invariants Checked | Findings |
|---|---:|---|---|---|
| [`crates/ui-egui/src/egui_shell/info_view.rs`](../../../../crates/ui-egui/src/egui_shell/info_view.rs) | 1139 | `1..=1139` | Split-ratio bounds clamping (`MIN_SPLIT_RATIO`/`MAX_SPLIT_RATIO`), `next_unused_constant_name` collision loop, localized constant number/ISO-date draft validation, `info.setConstant`/`info.setNote` payloads, hook preview expansion, `notes_draft` dirty lifecycle across sheet switches | `FA-SEC17-02` |
| [`crates/ui-egui/src/egui_shell/results_view.rs`](../../../../crates/ui-egui/src/egui_shell/results_view.rs) | 1234 | `1..=1234` | `ResultTabCategory` badge formatting, ambiguous bound tooltip & cell formatting, `Ctrl+PageUp`/`PageDown` modulo tab cycling, `row.get(c_idx)` bounds checks, context menu Esc/outside-click/disabled closure, header Copy workspace routing & row-loop virtualization | `FA-SEC17-07` |
| [`crates/ui-egui/src/egui_shell/unit_spinners.rs`](../../../../crates/ui-egui/src/egui_shell/unit_spinners.rs) | 657 | `1..=657` | Single-commit `committed_value` buffering on Enter/Tab/focus-loss & Esc revert, bounded `clamp(min_val, max_val)`, ASCII-lowercased unit suffix stripping preserving UTF-8 char boundaries, `parse_localized_number_input`, AccessKit `Role::SpinButton` metadata | Clean — bounded clamps, checked UTF-8 suffix slicing, localized parsing, single-commit editing |
| [`crates/ui-egui/src/egui_shell/welcome_hub.rs`](../../../../crates/ui-egui/src/egui_shell/welcome_hub.rs) | 2899 | `1..=2899` | Empty-workspace Cancel hiding & Esc closing, all 8 archetypes' schema builders vs `sheet.createData`, sample fixture population (`sheet.createData` + `table.setXCells`/`setTitles`/`setCells`), `.cbtemplate` base64 instantiation, Clone Options submodal focus/scrim, vector preview bounds | `FA-SEC17-06`, `FA-SEC17-11` |
| [`crates/ui-egui/src/egui_shell/data_grid/mod.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/mod.rs) | 152 | `1..=152` | Archetype router (`Column` fast path vs unified multi-archetype grid), shared state/report defaults (`record_inspect_meta: true`), public re-exports without `Session` mutation | Clean — pure router & state definitions dispatching to `render_column_grid` / `render_multi_archetype_grid` |
| [`crates/ui-egui/src/egui_shell/data_grid/column.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/column.rs) | 754 | `1..=754` | Two-axis virtualized viewport (`row_range` × `flat_range`), discard-pass header sync (`request_discard`), frozen & linked-region double-click/right-click guards, checked `.get(col)/.get(row)/.get(subcol)` access, gutter row-number selection & context menu | Clean — two-axis virtualization, checked indexing, frozen/linked-region guards, scrollbar bounds |
| [`crates/ui-egui/src/egui_shell/data_grid/headers.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/headers.rs) | 1336 | `1..=1336` | 3-tier frozen headers (`Col`/`Group`/`Var`, editable titles, subcolumn role chips), corner archetype trigger, row-title collapse toggle, Grouped/Contingency factor title editing, frozen-sheet edit guards, metadata `ellipsized` flags | `FA-SEC17-04`, `FA-SEC17-11` |
| [`crates/ui-egui/src/egui_shell/data_grid/inline_edit.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/inline_edit.rs) | 442 | `1..=442` | `Column` `render_inline_text_edit` (`was_focused`, `will_discard` `pending_commit_key` retry, key consumption) vs `render_multi_inline_editors` (`first_frame`, discard-pass retention, Enter/Tab/Esc key consumption, `table.setXCells`/`setTitles`/`setRowTitles`/`setSubcolumnTitles` payloads) | `FA-SEC17-09` |
| [`crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs) | 440 | `1..=440` | Linked provenance chip strip width budgeting & broken-link glyphs, `Column` guard modal scrim/Esc/click-blocking vs multi-archetype guard popover, `table.clearCells` unlink payload & frozen guard | `FA-SEC17-09` |
| [`crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs) | 1191 | `1..=1191` | All 7 non-Column archetypes' virtualized rendering, `X` generated-sequence read-only display, MV calculated/categorical styling & click-to-Formula-Bar, Contingency validation, context menu (`R15-GUI-29/32`), UTF-8 safe ` (Copy)` title truncation, frozen & inspect-meta gating | `FA-SEC17-04`, `FA-SEC17-09` |
| [`crates/ui-egui/src/egui_shell/data_grid/mv_roles.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/mv_roles.rs) | 180 | `1..=180` | MV calculated formula operand highlighting, Tier-3 role chip `Fx=`/`NUM`/`CAT`/`LBL` rendering & `table.setVariableRole`/`ui.table.openFormulaBar` routing, role popup dismissal & inspect-meta flag | `FA-SEC17-09` |
| [`crates/ui-egui/src/egui_shell/data_grid/shared.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/shared.rs) | 943 | `1..=943` | `MultiGridGeom` variable-subcolumn binary search (`Nested`), `compute_visible_range` float-to-usize clamping, `clamp_scroll_offset` finiteness guard, localized number formatting/parsing, `SelectionFastLookup`, perimeter linked-edge painting, `Any` grid focus & drag-select keys | Clean — checked geometry math, finite scroll clamping, safe UTF-8 minus/decimal replacement, bounded visible ranges |
| [`crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs) | 540 | `1..=540` | `Cmd/Ctrl+E` / `Cmd/Ctrl+Shift+E` exclusion, Cut/Copy/Paste + `X`-cell TSV multi-line split paste (`table.setXCells` + `table.paste`), arrow/Tab/Enter/Delete/F2/typing routing vs `frozen`, `is_x_column_readonly`, `VariableSource::Calculated`, and `linked_region_for_cell` | `FA-SEC17-04` |
| [`crates/ui-egui/src/egui_shell/data_grid/tests.rs`](../../../../crates/ui-egui/src/egui_shell/data_grid/tests.rs) | 134 | `1..=134` | 1M-cell Column grid virtualization (`< 600` cells) & AccessKit parity, linked-region double-click guard test, Enter single-commit inline-edit test | Clean — `#[cfg(test)]` suite verifying virtualization, guard blocking, and single-commit semantics |
| [`crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs) | 193 | `1..=193` | Plot-to-table drill-back card, exclusion toggle (`table.excludeCells` with frozen guard), `ui.formatPoints.open` point payload, `sheet.select` + grid cell jump, cache lookup key | `FA-SEC17-08` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs) | 951 | `1..=951` | Graph series/axis/canvas/legend controls, bounded `alpha` (`0..=255`) & `Series` index clamping, `graph.setSpec`/`setAxes`/`setSeries`/`applyCanvasPreset` schema compatibility, `selected_graph_annotation` source sync | `FA-SEC17-01` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/helpers.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/helpers.rs) | 480 | `1..=480` | Formula parser validation (`parse_mv_formula` cycle/arg checks), `resolve_studio_surface` priority order, studio chip/button truncation & inspect metadata, shared canvas media buttons (`insertImage`, `pasteImage`, `formatImage`, `formatResultTable`) | `FA-SEC17-05` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/metadata_hooks.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/metadata_hooks.rs) | 430 | `1..=430` | `InfoSheet` constant selection & `info.setConstant` parameter dispatch, localized number formatting/parsing, live `{{info:...}}`/`{{result:...}}` hook scanning, `Date` draft validation parity | `FA-SEC17-11` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/mod.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mod.rs) | 529 | `1..=529` | 7-surface dispatcher, collapsed `36 pt` rail expand button, draggable dock width splitter (`R15-GUI-36`), toggle shortcut binding vs `app.shortcuts`, `Panel::right` RTL mirroring | `FA-SEC17-10` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/mv_formula.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mv_formula.rs) | 486 | `1..=486` | 30-function formula helper table including `sumif(var, cond)` / `meanif(var, cond)` (`#14`), variable/function chip insertion, `table.setCalculatedVariable`/`setVariableRole`/`setCategoricalOrder`/`recodeLevels` payloads, frozen MV table guards | `FA-SEC17-04` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs) | 529 | `1..=529` | Layout preset/orientation/badge `layout.setSpec` chips, `ui.export.figure.open` `"dpi"` patch, `+ Graph Panel` (`layout.addPanel`, `GWF-15`), `layout_margin_mm`/`snap_enabled` state wiring, `selected_panel_index` source sync | `FA-SEC17-01`, `FA-SEC17-03` |
| [`crates/ui-egui/src/egui_shell/trailing_studio/results_interpret.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/results_interpret.rs) | 305 | `1..=305` | Results Assumptions/Narrative/Methods sub-tabs, `ui.result.copyMethodsReport` + clipboard write, `ChecklistSeverityFilter` chip state, `ui.editParams.open` `result_sheet_id` routing | Clean — bounded scalar slice, complete `ui.result.copyMethodsReport` & `ui.editParams.open` routing |
| [`crates/ui-egui/src/egui_shell/trailing_studio/table_selection.rs`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/table_selection.rs) | 250 | `1..=250` | Selection summary, `table.excludeCells` flat range bounds & frozen guard, downstream graph jump & `ui.formatPoints.open`, linked provenance list, `table_decimals` state wiring | `FA-SEC17-03` |

---

## Findings

### FA-SEC17-01 — Trailing Studio layout Z-order and canvas media buttons read moved-out `app.egui_views` (`None` at runtime)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs:358,514`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs#L358), [`crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs:936`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs#L936) (callers [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:1062-1070`](../../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L1062-L1070), [`apps/curvebook/src/gui_window.rs:711-720`](../../../../apps/curvebook/src/gui_window.rs#L711-L720))
- **Category:** `wrong-result` / `contract-drift` (`GWF-15` partial gap)
- **Defect:** `gui_window.rs:711` moves `egui_views` out of `win.app_mut().egui_views` via `std::mem::take` into `views`, leaving `win.app().egui_views` as `Default::default()` during the entire `render_egui_shell_frame_with_host` call. `inspect_bridge.rs:1062-1070` passes `app` and `&mut views.trailing_studio` to `render_trailing_studio` without passing `views.canvas_view`, yet `page_layout.rs:358, 514` reads `app.egui_views.canvas_view.selected_panel_index` and `graph_inspector.rs:936` reads `app.egui_views.canvas_view.selected_graph_annotation`.
- **Failure scenario:** During live desktop execution, both `Option<usize>` fields are unconditionally `None`. Selecting Panel 2 (or Embedded Image/Result Table annotation 1) on the canvas and clicking `Front`/`Forward`/`Backward`/`Back` (`layout.reorderPanel`) or `Format Image…`/`Format Table…` in Trailing Studio always reorders panel `0` (`unwrap_or(0)`) or edits the first matching annotation instead of the user's canvas selection.
- **Evidence:**
  ```rust
  // apps/curvebook/src/gui_window.rs:711-715
  let mut views = std::mem::take(&mut win.app_mut().egui_views);
  let rep = inspect_bridge::render_egui_shell_frame_with_host(ui, win.app(), &mut views, ...);

  // crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs:358, 514
  let target_panel_idx = app.egui_views.canvas_view.selected_panel_index.unwrap_or(0).min(spec.items.len().saturating_sub(1));
  ...
  let sel = app.egui_views.canvas_view.selected_panel_index;

  // crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs:936
  let sel = app.egui_views.canvas_view.selected_graph_annotation;
  ```
- **Suggested fix:** Thread `selected_panel_index: Option<usize>` and `selected_graph_annotation: Option<usize>` (from `views.canvas_view`) through `TrailingStudioContext` in `inspect_bridge.rs:1062-1070`, and read those context fields in `page_layout.rs` and `graph_inspector.rs`.
- **Related:** `GWF-15`, `SEC-16` canvas panel z-order handling.

---

### FA-SEC17-02 — Info Sheet unsaved notes draft is silently discarded when switching sheets or clicking a hook badge

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/info_view.rs:290,571-574,836-848,976-1015`](../../../../crates/ui-egui/src/egui_shell/info_view.rs#L836-L848)
- **Category:** `data-loss`
- **Defect:** `render_info_view_for_sheet` calls `sync_notes_draft_with_sheet(state, info)` on line 290, which checks `state.notes_draft.as_ref().is_some_and(|(id, _)| id == &info.id)`. Whenever `info.id` differs, it immediately overwrites `state.notes_draft` with `info.notes_markdown` and resets `state.notes_dirty = false` without flushing the prior dirty draft via `info.setNote`. When another sheet takes over `CenterStage` (for instance via Navigator selection or clicking a `🔗` hook badge at `info_view.rs:571-574`, which queues `sheet.select`), `TextEdit::lost_focus()` (`:983`) does not fire because the editor is no longer rendered on the following frame.
- **Failure scenario:** A user edits experiment notes in Info Sheet 1 without pressing `Save` or `Cmd/Ctrl+Enter`, clicks a linked `🔗` hook badge (or selects another Info Sheet / Data Sheet), and later returns to Info Sheet 1; `sync_notes_draft_with_sheet` reloads the stale `info.notes_markdown`, silently erasing the unsaved notes.
- **Evidence:**
  ```rust
  // info_view.rs:571-574
  if hook_resp.clicked() && enabled {
      state.selected_row = Some(idx);
      intents.push_engine("sheet.select", json!({ "sheet_id": refs[0].target_sheet_id.as_str() }));
  }

  // info_view.rs:836-842
  fn sync_notes_draft_with_sheet(state: &mut InfoViewEguiState, info: &InfoSheet) {
      let same_sheet = state.notes_draft.as_ref().is_some_and(|(id, _)| id == &info.id);
      if !same_sheet {
          state.notes_draft = Some((info.id.clone(), info.notes_markdown.clone()));
          state.notes_dirty = false;
          state.notes_focus_seen = false;
  ```
- **Suggested fix:** Before replacing `state.notes_draft` (in `sync_notes_draft_with_sheet` when `state.notes_dirty` is true, and when clicking a sheet-switching `🔗` hook badge), flush `info.setNote` for the previous `(prev_id, prev_draft)`, or preserve dirty drafts per `SheetId` / flush cleanly whenever `notes_draft` changes outside an active edit.
- **Related:** None.

---

### FA-SEC17-03 — Dead Trailing Studio controls (`layout_margin_mm`, `snap_enabled`, `table_decimals`) mutate unread UI-only state

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs:192-223,252-270`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs#L192-L223), [`crates/ui-egui/src/egui_shell/trailing_studio/table_selection.rs:64-91`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/table_selection.rs#L64-L91)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Three interactive control groups inside Trailing Studio mutate fields on `TrailingStudioEguiState` (`layout_margin_mm`, `snap_enabled`, and `table_decimals`) that are never read anywhere else in `crates/ui-egui` (neither by `canvas_view.rs`, `data_grid`, `trailing_studio`, nor any `PendingUiIntents` builder) and emit no engine or UI command.
- **Failure scenario:** Users adjusting Page Margins (`-`/`+` `mm`), `Snap: On`/`Snap: Off`, or Table `Display Decimals` (`Auto`, `0..=4`) see active selection states update in the dock, while layout panel dragging, page margin geometry, and data-grid numeric formatting remain completely unchanged.
- **Evidence:**
  ```rust
  // trailing_studio/page_layout.rs:200, 222, 270
  state.layout_margin_mm = state.layout_margin_mm.saturating_sub(2).max(2);
  ...
  state.layout_margin_mm = (state.layout_margin_mm + 2).min(40);
  ...
  state.snap_enabled = !state.snap_enabled;

  // trailing_studio/table_selection.rs:80, 89
  selected: state.table_decimals == opt, ... {
      state.table_decimals = opt;
  }
  ```
- **Suggested fix:** Wire `snap_enabled` and `layout_margin_mm` into `canvas_view` layout panel snap/guide & arrangement intents (matching `views/inspector.rs` `layout_margin_mm`), and wire `table_decimals` into an explicit document/grid display-format setting (or `PendingUiIntents` preference) consumed by `data_grid` numeric formatting.
- **Related:** `SEC-16` canvas layout guide handling.

---

### FA-SEC17-04 — Frozen-sheet read-only contract and read-only guard bypassed across multi-archetype data grid headers, row titles, `F2`, `Cmd/Ctrl+E`, and MV Formula Studio

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/data_grid/headers.rs:699-735,950-983,1118-1154,1198-1235`](../../../../crates/ui-egui/src/egui_shell/data_grid/headers.rs#L1118-L1154), [`crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs:256-289`](../../../../crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs#L256-L289), [`crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs:392-413,478-489`](../../../../crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs#L478-L489), [`crates/ui-egui/src/egui_shell/trailing_studio/mv_formula.rs:253-283,388,423,452`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mv_formula.rs#L253-L283)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Unlike `Column` grid (`headers.rs:522-528`, `column.rs:672-679`, `shortcuts.rs:40`), the multi-archetype data grid and MV Formula studio omit read-only guards in multiple paths:
  1. `headers.rs:699-735` (`SheetTitle`), `:950-983` (`XTitle`), `:1118-1154` (`ColumnTitle`), `:1198-1235` (`Nested` `SubcolumnTitle`), and `multi_archetype.rs:256-289` (`RowTitle`) all allow clicking into inline edit and advertise `enabled: !blocked` even when `ds.frozen` is `true` (note factor titles at `:769` and `:1059` properly check `!ds.frozen`).
  2. `shortcuts.rs:392-413` omits `!ds.frozen` on `Cmd/Ctrl+E` exclusion in multi-archetype tables (`column` checks `!rctx.ds.frozen` at `:40`).
  3. `shortcuts.rs:478-489` (`F2` in `handle_multi_keyboard_shortcuts`) opens inline editing without checking `is_x_column_readonly(ds)` (`GeneratedSequence`), `VariableSource::Calculated`, or `linked_region_for_cell`, whereas typing on lines `505-513` checks all three.
  4. `mv_formula.rs:253-283, 388, 423, 452` keeps `Apply Formula`, `Role`, `Order`, and `Recode Level` buttons enabled on frozen MV tables (unlike `table_selection.rs:109`).
- **Failure scenario:** Users on frozen tables, generated-sequence X columns, calculated MV columns, or linked regions can enter inline edits or click enabled studio mutation buttons only for edits to be rejected by engine precondition errors, and `F2` on a linked-region cell fails to display the read-only guard popover.
- **Evidence:**
  ```rust
  // data_grid/headers.rs:1151-1154 (similarly :732, :980, :1232, and multi_archetype.rs:286)
  if resp.clicked() && !blocked {
      fstate.aux.aux_edit = None;
      begin_inline_edit(ui, fstate.state, GridInlineEditTarget::ColumnTitle { col }, col_title);
  }

  // data_grid/shortcuts.rs:478-488 (contrast typing at :505-513)
  if pressed_key_event_modifiers(ui, Key::F2).is_some() && !ds.frozen {
      if let Some((xr, xs)) = fstate.aux.selected_x_cell {
          let init = initial_edit_text_for_cell(ds.archetype.x_or_time_cell(xr, xs), &app.locale_preset);
          fstate.aux.aux_edit = Some((AuxInlineTarget::XCell { row: xr, subcol: xs }, init, true));
          return;
      }
      if let Some(c) = app.grid.tool.active {
          let init = initial_edit_text_for_cell(ds.archetype.data_cell(c.into()), &app.locale_preset);
          begin_inline_edit(ui, fstate.state, GridInlineEditTarget::Cell(c), init);
          return;
      }
  }
  ```
- **Suggested fix:** Gate `ds.frozen` consistently across all multi-archetype headers/titles, `Cmd/Ctrl+E`, and `mv_formula.rs` controls; make `F2` in `shortcuts.rs:478-489` mirror lines `505-513` (check `!is_x_column_readonly(ds)` for X cells, reject `VariableSource::Calculated` or open formula bar, and open the linked-region guard popover when `linked_region_for_cell` matches).
- **Related:** None.

---

### FA-SEC17-05 — Studio `Insert Image` buttons (`graph.insertImage` / `layout.insertImage`) dispatch `{}` missing required `sheet_id`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/trailing_studio/helpers.rs:416`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/helpers.rs#L416) (schemas [`crates/engine/src/cmd/graph.rs:3792-3796`](../../../../crates/engine/src/cmd/graph.rs#L3792-L3796), [`crates/engine/src/cmd/layout.rs:1290-1298`](../../../../crates/engine/src/cmd/layout.rs#L1290-L1298))
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** `render_canvas_media_buttons` emits `intents.push_engine(spec.insert_cmd, json!({}));` when `Insert Image` is clicked (`inspector.canvas.insert_image` or `inspector.layout.insert_image`), omitting both the schema-required `"sheet_id"` and any image source (`path`, `bytes_base64`, or `asset_id`).
- **Failure scenario:** Clicking `Insert Image` in the Graph or Page Layout Trailing Studio always triggers an engine schema error (`missing sheet_id`) in `status_banner` instead of prompting for a file and targeting `spec.sheet_id`.
- **Evidence:**
  ```rust
  // trailing_studio/helpers.rs:416
  intents.push_engine(spec.insert_cmd, json!({}));
  ```
- **Suggested fix:** Route `Insert Image` through a native/host file-picker intent (carrying `"sheet_id": spec.sheet_id.as_str()`) or include `"sheet_id"` and open the image-insert dialog flow matching `Paste Image` / `desktop_host` file dialogs.
- **Related:** `SEC-16` (`FA-SEC16-01` OS file dialog routing).

---

### FA-SEC17-06 — `welcome_hub.rs` empty table creation drops user-configured `survival_event_code`, `survival_censored_code`, and `parts_check_total`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/welcome_hub.rs:981-1033,1967-1973`](../../../../crates/ui-egui/src/egui_shell/welcome_hub.rs#L1967-L1973) (engine [`crates/engine/src/cmd/sheet.rs:386-398`](../../../../crates/engine/src/cmd/sheet.rs#L386-L398))
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `render_archetype_schema_controls` renders interactive steppers for `survival_event_code` / `survival_censored_code` (`Survival`, `:981-1009`) and the `Normalize / 100% total check` checkbox (`PartsOfWhole`, `:1019-1033`) updating `WelcomeSchemaDraft`. However, `build_empty_create_params` (`:1967-1973`) omits all three fields from the `sheet.createData` JSON payload (and `sheet.rs:386-398` `CreateDataParams` does not parse them).
- **Failure scenario:** Changing Survival event/censored codes (e.g., `2` / `0`) or unchecking `parts_check_total` in the Welcome Hub before clicking `Create Table` creates a table with hardcoded defaults (`1`/`0` and default check behavior), ignoring the user's configuration.
- **Evidence:**
  ```rust
  // welcome_hub.rs:1967-1973
  WelcomeArchetypeChoice::Survival => {
      let time_title = if schema.survival_time_kind == 1 { "Date" } else { schema.survival_time_title.as_str() };
      map.insert(String::from("time_title"), json!(time_title));
  }
  WelcomeArchetypeChoice::PartsOfWhole => {
      map.insert(String::from("cols"), json!(schema.cols.clamp(1, 16)));
  }
  ```
- **Suggested fix:** Add `event_code`/`censored_code` (or Survival table config) and `check_total`/fraction options to `sheet.createData` (`CreateDataParams` + `build_archetype_from_params`) and include them in `build_empty_create_params`, or remove/read-only-label the unbacked controls if engine support is deferred.
- **Related:** None.

---

### FA-SEC17-07 — `results_view.rs` header `Copy` button (`push_workspace_copy`) misroutes in split view without `result.*` focus, and result table uses an unvirtualized row loop

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/results_view.rs:294-296,1032-1044`](../../../../crates/ui-egui/src/egui_shell/results_view.rs#L294-L296) (dispatch [`crates/ui-egui/src/egui_shell/mod.rs:2468-2494`](../../../../crates/ui-egui/src/egui_shell/mod.rs#L2468-L2494), [`crates/ui-egui/src/interaction.rs:1200-1228`](../../../../crates/ui-egui/src/interaction.rs#L1200-L1228))
- **Category:** `wrong-result` / `resources`
- **Defect:**
  1. The Result View header `Copy` button (`result.copy_selection`, `:295`) queues `intents.push_workspace_copy(144)` instead of `intents.push_custom_view_action("results", format!("copy_selection:{}", rs.id.as_str()))` (compare the context menu's `Copy Table` at `:1135`). `execute_workspace_copy` calls `resolve_result_sheet_for_shortcut(app, false)`, which returns `None` when a `DataSheet`/`GraphSheet` is `active_sheet_id` in Split View unless `app.focus_widget_id` already starts with `"result."` or `"secondary."` — and `results_view.rs` never updates `app.focus_widget_id`.
  2. `render_result_tab_table` (`:1032-1044`) loops over every row in `tab.rows` without computing a visible `row_range` from viewport scroll bounds (`allocate_space` + O(rows) loop runs every frame even for multi-thousand-row result tabs, unlike `data_grid`).
- **Failure scenario:** (1) In Split View with an active Data/Graph sheet and a Result sheet visible in the secondary pane, clicking the Result sheet's own `Copy` button copies the active Data/Graph sheet instead of the Result sheet's selection (and context menu `result.ctx.copy_selection` at `:1113` shares the same `push_workspace_copy(144)` path). (2) Large analysis output tabs (e.g., Kaplan-Meier / residual tables) incur O(rows) per-frame layout work.
- **Evidence:**
  ```rust
  // results_view.rs:294-296, 1134-1136
  if copy_resp.clicked() && enabled && has_tabs {
      intents.push_workspace_copy(144);
  }
  ...
  if btn_all.clicked() {
      intents.push_custom_view_action("results", format!("copy_table:{}", rs.id.as_str()));
  ```
- **Suggested fix:** Emit `intents.push_custom_view_action("results", format!("copy_selection:{}", rs.id.as_str()))` from both `result.copy_selection` and `result.ctx.copy_selection`, and add viewport-range row slicing inside `render_result_tab_table` (skipping collapsed section blocks by precomputed offset or cumulative height) so offscreen rows do not allocate/iterate per frame.
- **Related:** None.

---

### FA-SEC17-08 — `drill_back.rs` omits `subcol` in `ui.formatPoints.open` and matches cached points without checking `sheet_id`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs:79,165-171`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs#L79)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Line `169` passes `"points": [[brushed.col, brushed.row]]` to `ui.formatPoints.open`, omitting `brushed.subcol` even though `parse_selected_points_with_subcols` (`control_io.rs:2469`) supports `[col, row, subcol]` triplets and `table.excludeCells` (`:144`) + `push_select_grid_cell` (`:191`) preserve `brushed.subcol`. On line `79`, `matched_pt` searches `ensure_drill_points_cache(app, gs, state)` by `(row, col, subcol)` without verifying `brushed.sheet_id` against the graph's source sheet.
- **Failure scenario:** Clicking `Format Selected Point…` from Drill-Back on a replicate/Grouped/Nested plot applies point styling to all subcolumns of `(col, row)` instead of only the inspected replicate; and when `app.brushed_cell` originates from a different sheet in split view, Drill-Back can display series/canvas coordinates from an unrelated cached point with matching `(row, col, subcol)`.
- **Evidence:**
  ```rust
  // trailing_studio/drill_back.rs:79, 165-171
  let matched_pt = points.iter().find(|p| p.row == brushed.row && p.col == brushed.col && p.subcol == brushed.subcol);
  ...
  intents.push_ui(
      "ui.formatPoints.open",
      json!({
          "sheet_id": gs.id.as_str(),
          "points": [[brushed.col, brushed.row]],
      }),
  );
  ```
- **Suggested fix:** Include `brushed.subcol` (`[[brushed.col, brushed.row, brushed.subcol]]` when source column has >1 subcolumn, matching `control_io.rs:2474-2476`) in `ui.formatPoints.open`, and guard `matched_pt` lookup with `brushed.sheet_id == gs.spec.primary_source_sheet_id` (or direct source/result ancestry).
- **Related:** None.

---

### FA-SEC17-09 — Multi-archetype data grid modal/popup dismissal, `will_discard` inline-edit retry, and `record_inspect_meta` parity gaps

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs:348-440`](../../../../crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs#L348-L440), [`crates/ui-egui/src/egui_shell/data_grid/inline_edit.rs:218-332`](../../../../crates/ui-egui/src/egui_shell/data_grid/inline_edit.rs#L250-L332), [`crates/ui-egui/src/egui_shell/data_grid/mv_roles.rs:105-180`](../../../../crates/ui-egui/src/egui_shell/data_grid/mv_roles.rs#L105-L180), [`crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs:1003`](../../../../crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs#L1003)
- **Category:** `wrong-result` / `ui-contract`
- **Defect:** Four localized parity gaps between the `Column` grid (`column.rs`) and the multi-archetype grid:
  1. `render_multi_link_guard_popover` (`linked_regions.rs:348-440`) renders an `Order::Foreground` frame with no backdrop scrim (`Order::Middle` click catcher), no outside-click blocking (`outside_click_blocked` stays false in `multi_archetype.rs:86`), and no `Escape` handler (contrast `render_linked_region_guard_popover` at `linked_regions.rs:124-144`).
  2. `render_multi_inline_editors` (`inline_edit.rs:218-332`) takes `inline_edit.take()` / `aux_edit.take()` and discards the edit buffer upon commit without a `will_discard()` (`pending_commit_key`) retry guard (which `render_inline_text_edit` uses at `:107-116` so callers using per-pass `PendingUiIntents::commit_if_settled` do not lose edits on discarded layout passes), does not guard `Enter`/`Tab` with `!first_frame` on standard targets, and does not consume `Escape`/`Enter`/`Tab` keys.
  3. `render_mv_role_menu_popup` (`mv_roles.rs:105-180`) neither closes on `Key::Escape` nor closes on pointer clicks outside the popup area (contrast `render_grid_context_menu_popup` at `multi_archetype.rs:1180-1184`).
  4. `multi_archetype.rs:1003` (`render_grid_context_menu_popup`), `mv_roles.rs:133, 159`, and `linked_regions.rs:371, 396, 419` pass literal `true` instead of `state.record_inspect_meta` to `record_meta_if_enabled`.
- **Failure scenario:** Pressing `Escape` leaves the multi-archetype linked-region guard or MV role menu stuck open; clicking outside the multi-archetype link guard can leak through; callers not retaining `PendingUiIntents` across `will_discard()` lose inline edits on resize/scroll passes; and disabling `record_inspect_meta` still records popup menu entries.
- **Evidence:**
  ```rust
  // data_grid/inline_edit.rs:256-271
  if enter || tab.is_some() || (resp.lost_focus() && !first_frame) {
      commit_standard_inline_edit(app, ds, edit.target, edit.draft, fstate.intents);
      ...
      return; // unconditionally drops edit without checking ui.ctx().will_discard() or consuming keys
  }

  // data_grid/multi_archetype.rs:1000-1003
  record_meta_if_enabled(list, meta, true, &r, ...);
  ```
- **Suggested fix:** Add backdrop scrim + `Escape` dismissal to `render_multi_link_guard_popover` and `Escape`/outside-click closure to `render_mv_role_menu_popup`; mirror `render_inline_text_edit`'s `pending_commit_key` / `consume_key` / `!first_frame` guards in `render_multi_inline_editors`; and thread `state.record_inspect_meta` into popup metadata recording.
- **Related:** None.

---

### FA-SEC17-10 — Trailing Studio `Panel::right` ignores `ctx_in.layout.rtl_mirrored` and hardcodes `Cmd/Ctrl+Shift+I` instead of reading `app.shortcuts`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/trailing_studio/mod.rs:290-300,315`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mod.rs#L290-L315)
- **Category:** `ui-contract` / `contract-drift`
- **Defect:** `render_trailing_studio` hardcodes `Cmd/Ctrl+Shift+I` (`:291-296`) to toggle the inspector rail instead of consulting `app.shortcuts` for user-customized bindings, and unconditionally mounts `egui::Panel::right("egui_shell.trailing_studio")` (`:315`) without flipping to `Panel::left` when `ctx_in.layout.rtl_mirrored` is enabled.
- **Failure scenario:** Rebinding the inspector toggle shortcut leaves `Cmd/Ctrl+Shift+I` active (or triggers unintended toggles), and RTL locales place Trailing Studio on the right side instead of mirroring it with the rest of `ShellLayoutInput` / `ResponsiveShellLayout`.
- **Evidence:**
  ```rust
  // trailing_studio/mod.rs:294-296, 315
  let toggle_shortcut = ui.ctx().input_mut(|i| {
      i.consume_key(shift_cmd, egui::Key::I) || i.consume_key(shift_ctrl, egui::Key::I) || i.consume_key(shift_mac, egui::Key::I)
  });
  ...
  let resp = egui::Panel::right("egui_shell.trailing_studio")
  ```
- **Suggested fix:** Read the inspector toggle shortcut from `app.shortcuts` (matching `top_chrome.rs` / `command_palette.rs`), and choose `egui::Panel::left` vs `egui::Panel::right` based on `ctx_in.layout.rtl_mirrored`.
- **Related:** `SEC-16` (`leading_dock.rs` RTL dock layout parity).

---

### FA-SEC17-11 — Minor UI/metadata inconsistencies across `headers.rs`, `metadata_hooks.rs`, and `welcome_hub.rs`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S4` (Low)
- **Location:** [`crates/ui-egui/src/egui_shell/data_grid/headers.rs:397,613-641,1244-1264,1281`](../../../../crates/ui-egui/src/egui_shell/data_grid/headers.rs#L613-L641), [`crates/ui-egui/src/egui_shell/trailing_studio/metadata_hooks.rs:155,288`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/metadata_hooks.rs#L288), [`crates/ui-egui/src/egui_shell/welcome_hub.rs:1037,1048,1657-1670`](../../../../crates/ui-egui/src/egui_shell/welcome_hub.rs#L1048)
- **Category:** `wrong-result` / `cleanup`
- **Defect:**
  1. `headers.rs:613-641` (`column_subcolumn_role_label`) falls back to generic `Y`/`Y2`/`Y3` for `MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr` instead of descriptive role names; `x_column_title` (`:1244-1250`) only labels `subcol == 1` as `"±X Error"` (leaving `subcol == 2` on 3-subcolumn `MeanSdN` X-error tables showing `xy.x_title` over `"X Error"`); `data_super_column_label` (`:1281`) wraps column letters mod 26 (`A..Z` back to `A`) instead of using `default_column_label(col)` like `:447`; and `:397` passes `ellipsized: elided && false` so truncation on `grid.row_titles.toggle` is never reported.
  2. `metadata_hooks.rs:155` highlights the selected constant chip with case-sensitive `==` while `info.get_constant` lookup is case-insensitive (`info_view.rs:1124-1128`), and `:288` skips `YYYY-MM-DD` validation when `state.info_kind_draft == 2` (`Date`, unlike `info_view.rs:748`).
  3. `welcome_hub.rs:1037, 1048` exposes MV variable-role chips for at most 6 columns (`mv_role_kinds: [usize; 6]`) while allowing up to 32 columns (columns 7..=32 silently default to Continuous at `:1978`), and `:1657-1670` silently ignores `Err` from `build_starter_template_bytes` without closing or surfacing status feedback.
- **Failure scenario:** Subtle visual label/metadata inaccuracies in wide tables (>26 columns), summary/X-error subcolumns, Date-typed studio constants, and >6-variable MV welcome setup.
- **Evidence:**
  ```rust
  // data_grid/headers.rs:397, 1281
  ellipsized: elided && false,
  ...
  let letter = (b'A' + u8::try_from(col % 26).unwrap_or(0)) as char;

  // trailing_studio/metadata_hooks.rs:155, 288
  let selected = state.info_selected_constant.as_deref() == Some(c.name.as_str());
  ...
  let value_ok = state.info_kind_draft != 0 || raw_val.is_empty() || parsed_num.is_some();

  // welcome_hub.rs:1048
  let shown_vars = cx.state.schema.cols.clamp(1, 6);
  ```
- **Suggested fix:** Add role labels for `MeanLowHigh`/`UpperLowerLimits`/`MedianIqr` and 3-subcolumn X error (`X`, `SD`, `N`), use `default_column_label(col)` in `data_super_column_label`, and remove `&& false` at `headers.rs:397`; use case-insensitive selection match and ISO-date validation in `metadata_hooks.rs`; and dynamically size `mv_role_kinds` (`Vec<usize>`) in `WelcomeSchemaDraft`.
- **Related:** None.
