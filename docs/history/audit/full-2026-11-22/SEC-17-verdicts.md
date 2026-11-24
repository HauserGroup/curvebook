# Verification Report: SEC-17 — Egui Shell Grid, Studio & Views (`crates/ui-egui`)

- **Section:** `SEC-17` (`23` files in `crates/ui-egui/src/egui_shell/`: `info_view.rs`, `results_view.rs`, `unit_spinners.rs`, `welcome_hub.rs`, `data_grid/**`, `trailing_studio/**`; `16,194` total LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-17.md`](SEC-17.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend-2`
- **Date:** `2026-11-22`

## Summary Table

| Verdict | Count | Settled Findings |
|---|---:|---|
| `CONFIRMED` | 11 | `FA-SEC17-01`, `FA-SEC17-02`, `FA-SEC17-03`, `FA-SEC17-04`, `FA-SEC17-05`, `FA-SEC17-06`, `FA-SEC17-07`, `FA-SEC17-08`, `FA-SEC17-09`, `FA-SEC17-10`, `FA-SEC17-11` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 4 | `FA-SEC17-01`, `FA-SEC17-02`, `FA-SEC17-03`, `FA-SEC17-04` |
| `S3` | 6 | `FA-SEC17-05`, `FA-SEC17-06`, `FA-SEC17-07`, `FA-SEC17-08`, `FA-SEC17-09`, `FA-SEC17-10` |
| `S4` | 1 | `FA-SEC17-11` |

## File Coverage Completeness Check

Verified all `23 / 23` scoped files (`16,194` total LOC, matching line counts at `7c85a8e516165ddadb101236a88eec937bf1f095` exactly) appear in [`SEC-17.md`](SEC-17.md#L24-L48) with `1..=N` inspection ranges (`PASS`):
- `crates/ui-egui/src/egui_shell/`: `info_view.rs` (`1,139`), `results_view.rs` (`1,234`), `unit_spinners.rs` (`657`), `welcome_hub.rs` (`2,899`).
- `crates/ui-egui/src/egui_shell/data_grid/`: `column.rs` (`754`), `headers.rs` (`1,336`), `inline_edit.rs` (`442`), `linked_regions.rs` (`440`), `mod.rs` (`152`), `multi_archetype.rs` (`1,191`), `mv_roles.rs` (`180`), `shared.rs` (`943`), `shortcuts.rs` (`540`), `tests.rs` (`134`).
- `crates/ui-egui/src/egui_shell/trailing_studio/`: `drill_back.rs` (`193`), `graph_inspector.rs` (`951`), `helpers.rs` (`480`), `metadata_hooks.rs` (`430`), `mod.rs` (`529`), `mv_formula.rs` (`486`), `page_layout.rs` (`529`), `results_interpret.rs` (`305`), `table_selection.rs` (`250`).

---

## Per-Finding Verdicts

### Verdict `FA-SEC17-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Trailing Studio Z-order controls and canvas media controls always read `None` on live desktop runs because `win.app().egui_views` is moved out during frame rendering.
- **Evidence:**
  - [`apps/curvebook/src/gui_window.rs:711-720`](../../../../apps/curvebook/src/gui_window.rs#L711-L720): `let mut views = std::mem::take(&mut win.app_mut().egui_views);` leaves `win.app().egui_views` as `Default::default()` across `render_egui_shell_frame_with_host`.
  - [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:1062-1069`](../../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L1062-L1069): `TrailingStudioContext` passes `{ tokens, layout, enabled, focused }` without `views.canvas_view.selected_panel_index` or `views.canvas_view.selected_graph_annotation`.
  - [`crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs:358, 514`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs#L358) and [`crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs:936`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/graph_inspector.rs#L936): read `app.egui_views.canvas_view.selected_panel_index` / `.selected_graph_annotation` directly, collapsing to panel `0` (`unwrap_or(0)`) or the first matching annotation.

---

### Verdict `FA-SEC17-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Switching away from an Info Sheet or clicking a `🔗` hook badge discards dirty unsaved notes without emitting `info.setNote`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/info_view.rs:290, 836-848`](../../../../crates/ui-egui/src/egui_shell/info_view.rs#L836-L848): `sync_notes_draft_with_sheet` overwrites `state.notes_draft` and clears `state.notes_dirty = false` whenever `!same_sheet` without flushing any prior dirty draft.
  - [`info_view.rs:571-574, 982-1014`](../../../../crates/ui-egui/src/egui_shell/info_view.rs#L571-L1014): clicking a `🔗` hook badge pushes `sheet.select`, unmounting `info_view` on the next frame so `resp.lost_focus()` (`:983`) never runs to commit `info.setNote`.

---

### Verdict `FA-SEC17-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Page Layout Margin, Snap-to-Grid, and Table Display Decimals controls mutate UI-only fields that are never read by rendering or command emission.
- **Evidence:**
  - Repo search across `crates/ui-egui/src` confirmed [`TrailingStudioEguiState`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mod.rs#L163-L181) fields `table_decimals`, `layout_margin_mm`, and `snap_enabled` are referenced exclusively in `trailing_studio/mod.rs`, [`page_layout.rs:192-223, 252-270`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/page_layout.rs#L192-L270), and [`table_selection.rs:64-91`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/table_selection.rs#L64-L91), with zero readers in `canvas_view`, `data_grid`, or `PendingUiIntents` (`views/inspector.rs:1440` is an unrelated function on `InspectorViewState`).

---

### Verdict `FA-SEC17-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Multi-archetype data grid headers, row titles, `Cmd/Ctrl+E`, `F2`, and MV Formula Studio omit `!ds.frozen` and cell-level read-only guards.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/data_grid/headers.rs:699-735, 950-983, 1118-1154, 1198-1235`](../../../../crates/ui-egui/src/egui_shell/data_grid/headers.rs#L699-L1235) and [`multi_archetype.rs:256-289`](../../../../crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs#L256-L289): `SheetTitle`, `XTitle`, `ColumnTitle`, Nested `SubcolumnTitle`, and `RowTitle` click handlers/inspect metadata check only `!blocked` without `!ds.frozen` (unlike factor titles at `headers.rs:769, 1059` and Column headers at `:522, 528`).
  - [`data_grid/shortcuts.rs:392-413, 478-489`](../../../../crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs#L392-L489): `Cmd/Ctrl+E` omits `!ds.frozen` (contrast Column `:40` and Delete `:419`), and `Key::F2` omits `is_x_column_readonly(ds)`, `VariableSource::Calculated`, and `linked_region_for_cell` checks (contrast typing right below at `:504-514`).
  - [`trailing_studio/mv_formula.rs:253, 388, 423, 452`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mv_formula.rs#L253-L452): `Apply Formula`, `Role`, `Order`, and `Recode Level` omit `!ds.frozen`.

---

### Verdict `FA-SEC17-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Studio `Insert Image` buttons emit `json!({})` omitting required `sheet_id` (triggering schema rejection in direct `PendingUiIntents` hosts, though masked on desktop).
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/trailing_studio/helpers.rs:416`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/helpers.rs#L416) pushes `spec.insert_cmd` (`graph.insertImage` / `layout.insertImage`) with `json!({})`, whereas [`cmd/graph.rs:3795`](../../../../crates/engine/src/cmd/graph.rs#L3795) and [`cmd/layout.rs:1297`](../../../../crates/engine/src/cmd/layout.rs#L1297) require `"sheet_id"`, and [`egui_shell/mod.rs:1876-1883`](../../../../crates/ui-egui/src/egui_shell/mod.rs#L1876-L1883) only auto-injects `"sheet_id"` for `sheet.*` commands.
  - **Verification nuance:** Unlike the overstated `"always triggers an engine schema error"` wording in the report, [`apps/curvebook/src/gui_window.rs:557-564`](../../../../apps/curvebook/src/gui_window.rs#L557-L564) and [`desktop_host.rs:1874-1896`](../../../../apps/curvebook/src/desktop_host.rs#L1874-L1896) intercept media-less `graph.insertImage`/`layout.insertImage` intents on desktop, open the native open-file dialog, and resolve `doc.active_sheet_id` (matching [`resolve_studio_surface`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mod.rs#L247-L270)). Direct `PendingUiIntents::commit_all_if_settled` callers (`driver.rs` / headless `ui-egui` integrations) still hit the `missing sheet_id` error, confirming the `S3` contract-drift hazard.

---

### Verdict `FA-SEC17-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Welcome Hub `sheet.createData` builder drops user-configured `survival_event_code`, `survival_censored_code`, and `parts_check_total`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/welcome_hub.rs:981-1033`](../../../../crates/ui-egui/src/egui_shell/welcome_hub.rs#L981-L1033) mutates `survival_event_code`, `survival_censored_code`, and `parts_check_total` on `WelcomeSchemaDraft`, while [`build_empty_create_params`](../../../../crates/ui-egui/src/egui_shell/welcome_hub.rs#L1967-L1973) omits all three fields and [`cmd/sheet.rs:386-398`](../../../../crates/engine/src/cmd/sheet.rs#L386-L398) does not parse them.

---

### Verdict `FA-SEC17-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — `results_view.rs` `Copy` controls misroute via `push_workspace_copy` when a Result sheet lacks primary focus in Split View, and `render_result_tab_table` iterates all rows unvirtualized.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/results_view.rs:294-296, 1112-1114`](../../../../crates/ui-egui/src/egui_shell/results_view.rs#L294-L1114) emit `intents.push_workspace_copy(144)` without updating `app.focus_widget_id` (unlike `Copy Table` at `:1135`, which passes `results` `copy_table:<id>`). When a non-Result sheet is active in Split View, both [`gui_window.rs:546-570`](../../../../apps/curvebook/src/gui_window.rs#L546-L570) (`!active_is_result => host_copy = true`) and [`interaction.rs:1200-1228`](../../../../crates/ui-egui/src/interaction.rs#L1200-L1228) (`resolve_result_sheet_for_shortcut(app, false)`) bypass the secondary Result sheet.
  - [`results_view.rs:1032-1044`](../../../../crates/ui-egui/src/egui_shell/results_view.rs#L1032-L1044): `render_result_tab_table` iterates `for (r_idx, row) in tab.rows.iter().enumerate()` without viewport row-range slicing.

---

### Verdict `FA-SEC17-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — `drill_back.rs` omits `brushed.subcol` from `ui.formatPoints.open` and matches cached points without checking `sheet_id`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs:79`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs#L79) matches `(row, col, subcol)` against `ensure_drill_points_cache(app, gs, state)` without verifying `brushed.sheet_id`.
  - [`drill_back.rs:165-171`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/drill_back.rs#L165-L171) sends `"points": [[brushed.col, brushed.row]]` (dropping `brushed.subcol`) even though [`control_io.rs:2469-2476`](../../../../crates/ui-egui/src/control_io.rs#L2469-L2476) accepts `[col, row, subcol]` triplets and `drill_back.rs:144, 191` pass `brushed.subcol`.

---

### Verdict `FA-SEC17-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Multi-archetype grid popovers lack scrim/`Escape`/outside-click dismissal, inline editors lack `will_discard` retry & key consumption, and popups ignore `record_inspect_meta`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs:348-440`](../../../../crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs#L348-L440): `render_multi_link_guard_popover` has no `Order::Middle` scrim, no outside-click blocker, and no `Key::Escape` handler (contrast `:124-144`).
  - [`data_grid/inline_edit.rs:218-332`](../../../../crates/ui-egui/src/egui_shell/data_grid/inline_edit.rs#L218-L332): omits `will_discard()` (`pending_commit_key`) retry and `consume_key` calls present in [`render_inline_text_edit`](../../../../crates/ui-egui/src/egui_shell/data_grid/inline_edit.rs#L97-L116), and omits `!first_frame` on `enter || tab.is_some()` at `:256`.
  - [`data_grid/mv_roles.rs:105-180`](../../../../crates/ui-egui/src/egui_shell/data_grid/mv_roles.rs#L105-L180): `render_mv_role_menu_popup` omits `Key::Escape` and outside-click dismissal (contrast [`multi_archetype.rs:1180-1184`](../../../../crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs#L1180-L1184)).
  - [`multi_archetype.rs:1003`](../../../../crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs#L1003), [`mv_roles.rs:133, 159`](../../../../crates/ui-egui/src/egui_shell/data_grid/mv_roles.rs#L133), and [`linked_regions.rs:371, 396, 419`](../../../../crates/ui-egui/src/egui_shell/data_grid/linked_regions.rs#L371) pass literal `true` instead of `state.record_inspect_meta`.

---

### Verdict `FA-SEC17-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Trailing Studio hardcodes `Cmd/Ctrl+Shift+I` instead of reading `app.shortcuts`, and mounts `Panel::right` without mirroring on `ctx_in.layout.rtl_mirrored`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/trailing_studio/mod.rs:290-300, 315, 463`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/mod.rs#L290-L463): matches raw `Cmd/Ctrl+Shift+I` without checking `app.shortcuts`, and unconditionally mounts both `egui_shell.trailing_studio` and `egui_shell.trailing_dock_splitter` (`:463`) via `egui::Panel::right(...)` regardless of `ctx_in.layout.rtl_mirrored`.

---

### Verdict `FA-SEC17-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Minor UI label, validation, and capacity inconsistencies in `headers.rs`, `metadata_hooks.rs`, and `welcome_hub.rs`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/data_grid/headers.rs:397, 613-641, 1244-1250, 1281`](../../../../crates/ui-egui/src/egui_shell/data_grid/headers.rs#L397-L1281): emits `ellipsized: elided && false` (`:397`), falls back to `Y`/`Y2`/`Y3` for `MeanLowHigh`/`UpperLowerLimits`/`MedianIqr` (`:634-640`), labels only `subcol == 1` as `"±X Error"` (`:1245`), and wraps `col % 26` (`:1281`) instead of `default_column_label(col)` (`:447`).
  - [`trailing_studio/metadata_hooks.rs:155, 288`](../../../../crates/ui-egui/src/egui_shell/trailing_studio/metadata_hooks.rs#L155-L288): compares selected constant chip name case-sensitively against case-insensitive `InfoSheet::get_constant` ([`crates/doc/src/info.rs:113-116`](../../../../crates/doc/src/info.rs#L113-L116)), and skips ISO-date validation when `info_kind_draft == 2` (`Date`, contrast [`info_view.rs:748`](../../../../crates/ui-egui/src/egui_shell/info_view.rs#L748)).
  - [`welcome_hub.rs:175, 1037-1048, 1657-1670`](../../../../crates/ui-egui/src/egui_shell/welcome_hub.rs#L175-L1670): `mv_role_kinds: [usize; 6]` exposes role chips for at most 6 of up to 32 MV columns (`:1978` defaults `idx >= 6` to Continuous), and starter template instantiation silently ignores `build_starter_template_bytes` errors.
