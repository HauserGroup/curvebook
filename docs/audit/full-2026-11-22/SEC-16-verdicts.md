# Verification Report: SEC-16 — Egui Shell Chrome, Canvas & Docks (`crates/ui-egui`)

- **Section:** `SEC-16` (`14` files in `crates/ui-egui/src/egui_shell/`: `mod.rs`, `brand.rs`, `canvas_view.rs`, `center_stage.rs`, `command_palette.rs`, `driver.rs`, `fonts.rs`, `inspect_bridge.rs`, `leading_dock.rs`, `rasterizer.rs`, `status_footer.rs`, `top_bar_fit.rs`, `top_chrome.rs`, `widgets.rs`; `21,426` total LOC)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-16.md`](../../../docs/audit/full-2026-11-22/SEC-16.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend-2`
- **Date:** `2026-11-22`

## Summary Table

> Note: `FA-SEC16-01` is settled as `DUPLICATE` of [`FA-SEC19-02`](../../../docs/audit/full-2026-11-22/SEC-19-verdicts.md#L47-L54) (`S1`, already recorded in `SEC-19` alongside `FA-SEC14-06` in `SEC-14`) and excluded from unique settled counts.

| Verdict | Count | Settled Findings |
|---|---:|---|
| `CONFIRMED` | 11 | `FA-SEC16-02`, `FA-SEC16-03`, `FA-SEC16-04`, `FA-SEC16-05`, `FA-SEC16-06`, `FA-SEC16-07`, `FA-SEC16-08`, `FA-SEC16-09`, `FA-SEC16-10`, `FA-SEC16-11`, `FA-SEC16-12` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 1 | `FA-SEC16-01` (duplicate of `FA-SEC19-02`) |

| Settled Severity (Unique) | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 3 | `FA-SEC16-02`, `FA-SEC16-03`, `FA-SEC16-04` |
| `S3` | 6 | `FA-SEC16-05`, `FA-SEC16-06`, `FA-SEC16-07`, `FA-SEC16-08`, `FA-SEC16-09`, `FA-SEC16-10` |
| `S4` | 2 | `FA-SEC16-11`, `FA-SEC16-12` |

## File Coverage Completeness Check

Verified all `14 / 14` files (`21,426` total LOC, matching `wc -l` counts exactly at `7c85a8e516165ddadb101236a88eec937bf1f095`) appear in [`SEC-16.md`](../../../docs/audit/full-2026-11-22/SEC-16.md#L18-L35) with `1..=N` inspection ranges (`PASS`):
- `crates/ui-egui/src/egui_shell/`: `mod.rs` (`3,264`), `brand.rs` (`90`), `canvas_view.rs` (`2,842`), `center_stage.rs` (`2,190`), `command_palette.rs` (`860`), `driver.rs` (`1,281`), `fonts.rs` (`898`), `inspect_bridge.rs` (`1,424`), `leading_dock.rs` (`2,495`), `rasterizer.rs` (`641`), `status_footer.rs` (`1,745`), `top_bar_fit.rs` (`140`), `top_chrome.rs` (`2,195`), `widgets.rs` (`1,361`).

---

## Per-Finding Verdicts

### Verdict `FA-SEC16-01` · `DUPLICATE` of `FA-SEC19-02` (`S1`)

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Rationale:** Reports the same root defect already `CONFIRMED` at `S1` in `FA-SEC19-02` (`SEC-19`, and previously deduplicated in `FA-SEC14-06`): [`DESKTOP_HOST_ROUTED_COMMANDS`](../../../crates/ui-egui/src/menus/mod.rs#L23-L48) omits file-backed export, template, and import actions (`export.tiff`, `graph.exportSvg`/`Pdf`/`Png`, `table.exportCsv`, `analysis.exportJson`/`Csv`, `result.generateReport`, `project.saveTemplate`, `project.instantiateTemplate`, `table.importCsv`, and legacy XML/binary archive importers), causing [`push_menu_command_intent`](../../../crates/ui-egui/src/egui_shell/top_chrome.rs#L844-L860) and [`command_palette.rs:717-737, 787`](../../../crates/ui-egui/src/egui_shell/command_palette.rs#L717-L787) to execute exports/templates without `"path"` and discard rendered bytes.
- **Verification note:** Confirmed the additional Command Palette route detail cited by `FA-SEC16-01`: [`command_palette.rs:673-678`](../../../crates/ui-egui/src/egui_shell/command_palette.rs#L673-L678) intercepts `"table.importCsv"` and the legacy XML/archive importers into `UiIntent::OpenImportWizard { origin: ExternalDelimitedFile, .. }` (delimited-text only) and `"project.instantiateTemplate"` into `UiIntent::SetLauncherOpen { open: true }` instead of native OS file pickers.

---

### Verdict `FA-SEC16-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Silent overwrite of an existing `"Constant"` parameter to `1.0` from the contextual Tier-2 toolbar (regression of `R15-GUI-27`).
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/top_chrome.rs:1749-1763`](../../../crates/ui-egui/src/egui_shell/top_chrome.rs#L1749-L1763): Under `Tier2SheetContext::InfoSheet`, `"toolbar.info_add_constant"` emits `info.setConstant` with hardcoded `{ "sheet_id": info_id, "name": "Constant", "value": 1.0 }` (and `info_id` falls back to `""` via `unwrap_or_default()` while `enabled` only checks `modal_enabled`), whereas [`info_view.rs:368-377, 442-448`](../../../crates/ui-egui/src/egui_shell/info_view.rs#L368-L448) generates `next_unused_constant_name(info)` with `Value::Null`.
  - [`crates/engine/src/cmd/info.rs:136-138`](../../../crates/engine/src/cmd/info.rs#L136-L138): `run_info_set_constant` executes `info.upsert_constant`, overwriting any existing `"Constant"` row to `1.0` and recalculating downstream links.

---

### Verdict `FA-SEC16-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Global and dock keyboard shortcuts hardcode raw `egui::Key` chords and ignore customized `app.shortcuts` (`ShortcutRegistry`).
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:1205-1305`](../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L1205-L1305): `handle_shell_global_shortcuts` matches raw chords (`Cmd/Ctrl+T`, `Cmd/Ctrl+K`, `Cmd/Ctrl+Shift+Z`/`Y`, `Cmd/Ctrl+Z`, `Cmd/Ctrl+C`, `Cmd/Ctrl+V`, `Cmd/Ctrl+\`, `F6`/`Shift+F6`) without checking `app.shortcuts`.
  - [`crates/ui-egui/src/egui_shell/leading_dock.rs:391-428`](../../../crates/ui-egui/src/egui_shell/leading_dock.rs#L391-L428): `handle_leading_dock_shortcuts` similarly hardcodes `Cmd/Ctrl+Shift+N`, `Cmd/Ctrl+Shift+F`, and `F2` while [`top_chrome.rs:876`](../../../crates/ui-egui/src/egui_shell/top_chrome.rs#L876) and [`command_palette.rs:162`](../../../crates/ui-egui/src/egui_shell/command_palette.rs#L162) display customized `app.shortcuts.active_combo(...)` hints.

---

### Verdict `FA-SEC16-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Active workspace input and shortcuts remain enabled behind the destructive Navigator cascade-delete modal, and `ui.inspect` reports wrong/missing dialog snapshots.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/leading_dock.rs:2398-2494`](../../../crates/ui-egui/src/egui_shell/leading_dock.rs#L2398-L2494) vs [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:215-231`](../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L215-L231): `HybridCoexistenceArbiter::evaluate_with_views` omits `views.leading_dock.delete_confirm.is_some()`, leaving `modal_backdrop_active == false` behind `render_navigator_delete_confirm_modal` so workspace controls and global/dock shortcuts continue firing.
  - [`inspect_bridge.rs:458-475`](../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L458-L475): `render_navigator_delete_confirm_modal` never records a `modal.dialog`/`dialog.*` frame-inspect entry, `build_dialog_snapshot_from_meta` omits `views.leading_dock.delete_confirm` and `"palette.dialog"` ([`command_palette.rs:231`](../../../crates/ui-egui/src/egui_shell/command_palette.rs#L231)), checks `app.canvas_dialogs.active_dialog != ActiveDialog::None` ahead of `"modal.dialog"`, and strips `"dialog."` rather than `"modal."` from `"modal.dialog"`.

---

### Verdict `FA-SEC16-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — `UiMetrics::solve_shell_layout` hardcodes `rtl_mirrored: false` while downstream code reads it (`status_footer.rs`).
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/mod.rs:456-600`](../../../crates/ui-egui/src/egui_shell/mod.rs#L456-L600): All three return paths in `solve_shell_layout` (`:495`, `:521`, `:599`) hardcode `rtl_mirrored: false`, while [`status_footer.rs:704`](../../../crates/ui-egui/src/egui_shell/status_footer.rs#L704) branches on `layout.rtl_mirrored`.

---

### Verdict `FA-SEC16-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Missing Arabic/Hebrew curated OS font queries/candidates, lexicographic saturation of `MAX_OS_FONT_FILES = 28`, and potential `font_data` key collisions under `"os_extra"`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/fonts.rs:41, 243-282, 534-575, 614-668`](../../../crates/ui-egui/src/egui_shell/fonts.rs#L41-L668): `assemble_font_definitions` queries only `OS_SYMBOL_FAMILIES` and `OS_CJK_FAMILIES`, `curated_os_font_candidates` omits Arabic/Hebrew paths, and `discover_host_font_blobs` sorts `dir_candidates` lexicographically under `MAX_OS_FONT_FILES = 28`.
  - [`fonts.rs:367-379`](../../../crates/ui-egui/src/egui_shell/fonts.rs#L367-L379): `register_face_for_family` keys `defs.font_data` as `format!("{prefix}_{}_{}", id_to_slug(db, id), face_index)` using only `families.first()`, colliding under `"os_extra"` when distinct faces share the same primary family name and `face_index`.

---

### Verdict `FA-SEC16-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Command Palette consumes `Key::Home`/`Key::End` before `TextEdit` and mismatches `"ui.export.figure.open"`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/command_palette.rs:139, 164, 281-287`](../../../crates/ui-egui/src/egui_shell/command_palette.rs#L139-L287): `consume_palette_keys` runs before rendering the search `TextEdit` and unconditionally consumes `Key::Home` and `Key::End` with `Modifiers::NONE`.
  - [`command_palette.rs:593-596`](../../../crates/ui-egui/src/egui_shell/command_palette.rs#L593-L596) vs [`crates/ui-egui/src/menus/ui_commands.rs:370`](../../../crates/ui-egui/src/menus/ui_commands.rs#L370): `resolve_palette_ui_route` matches `"ui.export.open"` instead of `"ui.export.figure.open"`, dropping `sheet_id` parameter resolution into the `{}` fallback.

---

### Verdict `FA-SEC16-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Find & Replace `"Find Next"` stays pinned on `first_match` and omits `x_or_time_cell`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/mod.rs:2758-2843`](../../../crates/ui-egui/src/egui_shell/mod.rs#L2758-L2843): `apply_find_replace_on_active_sheet` scans `(0..rows, 0..cols, 0..subcols)` checking only `ds.archetype.data_cell(...)` (`:2787`, skipping `x_or_time_cell`) and unconditionally selects `first_match` (`:2834-2837`) without advancing past `app.grid.tool.active` when `replace_all == false`.

---

### Verdict `FA-SEC16-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Status footer popovers stay open across modal disablement, `Escape`, and outside clicks, and cursor coordinate readout ignores `paper_rect` offset and `fit_scale`.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/status_footer.rs:893-914, 1425-1603, 1689-1701`](../../../crates/ui-egui/src/egui_shell/status_footer.rs#L893-L1701): `render_status_footer` calls `render_footer_popovers` unconditionally without checking `ctx_in.enabled`, and neither `show_footer_popup` nor `render_footer_popovers` closes `filter_popup_open`, `sheet_switcher_open`, `link_popover_open`, or `stats_popover_open` on `!ctx_in.enabled`, `Key::Escape`, or outside clicks (`stats_popover_open` at `:1570-1602` has no dismiss path).
  - [`status_footer.rs:702-717`](../../../crates/ui-egui/src/egui_shell/status_footer.rs#L702-L717): `center_stage_cursor_coords` computes `(pos - stage_rect.min - pan) / zoom` rather than transforming relative to `paper_rect.min` and `fit_scale * zoom`.

---

### Verdict `FA-SEC16-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Canvas `Insert Image…` omits `sheet_id`, zero-delta drags emit mutating `layout.setSpec` undo entries, and `CanvasViewEguiState` keeps unkeyed sheet-local selection/pan fields.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/canvas_view.rs:2189, 2295`](../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L2181-L2297): `canvas.graph_ctx.insert_image` and `canvas.layout_ctx.insert_image` emit `json!({})` without `"sheet_id"`, and [`apps/curvebook/src/desktop_host.rs:1876-1879`](../../../apps/curvebook/src/desktop_host.rs#L1876-L1879) resolves `active_sheet_id` instead of the clicked split-pane sheet.
  - [`canvas_view.rs:1571-1580, 2547-2558`](../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L1571-L2558): `drag_stopped()` emits `layout.setSpec` even when `drag.current_bounds_pt == drag.initial_bounds_pt` ([`crates/engine/src/cmd/layout.rs:554`](../../../crates/engine/src/cmd/layout.rs#L554) always marks mutated).
  - [`canvas_view.rs:91-112`](../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L91-L112): `pan_pt`, `selected_panel_index`, and `selected_graph_annotation` are single unkeyed fields shared across sheet switches.

---

### Verdict `FA-SEC16-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Unvirtualized split preview widget loop, missing inspect entry on `"Focus in Grid"`, and unused `note_pinned` toggle state.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/center_stage.rs:958-960, 975-1024`](../../../crates/ui-egui/src/egui_shell/center_stage.rs#L958-L1024): `render_split_column_sheet_preview` loops unconditionally up to `500 × 64` `egui::Button` widgets per frame instead of virtualizing with `show_rows`, and omits `meta.record_response` on `"Focus in Grid"`.
  - [`center_stage.rs:125, 1945-1947`](../../../crates/ui-egui/src/egui_shell/center_stage.rs#L125-L1947): Confirmed via crate-wide search that `note_pinned` is only inserted/read to paint `"P"`/`"P*"` and never alters note z-order, persistence, or sheet switching behavior.

---

### Verdict `FA-SEC16-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Hardcoded Navigator labels/IDs, frozen-sheet convert-menu enablement, unpruned `RasterWorkerResults`, and stale `#[allow(clippy::too_many_arguments)]` attributes.
- **Evidence:**
  - [`crates/ui-egui/src/egui_shell/leading_dock.rs:1231-1255, 1688, 2106-2119`](../../../crates/ui-egui/src/egui_shell/leading_dock.rs#L1231-L2119): `render_section_new_row` hardcodes English `"+ New ..."` strings and disabled reasons (`Data`/`Info`/`Graphs`/`Layouts`), `finalize_drag_state` displays `drag_id.as_str()`, and `nav.ctx.convert_archetype` keeps `enabled: true` even when `menu.is_frozen` (`table.convertArchetype` still rejects frozen sheets cleanly on commit).
  - [`crates/ui-egui/src/egui_shell/canvas_view.rs:246-269, 313-318, 358`](../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L246-L358) and [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:977`](../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L977): `RasterWorkerResults` (`completed`/`last_submitted`) never prunes deleted `SheetId` entries, and `render_canvas_view_for_sheet` (7 args) / `render_egui_shell_frame_with_host` (6 args) retain stale `#[allow(clippy::too_many_arguments)]`.
