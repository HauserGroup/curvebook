# SEC-15 Audit Report: UI Views & Localization (`crates/ui-egui`)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-15` — `crates/ui-egui/src/views/*` (`15` files, `12,230` lines) and `crates/ui-egui/src/i18n/*` (`30` files, `17,190` lines) (`45` files total, `29,420` lines)
- **Date:** `2026-11-22`
- **Summary Counts:** `S1 = 0`, `S2 = 7`, `S3 = 7`, `S4 = 1` (`15` total findings)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Verified | Findings |
| :--- | ---: | :--- | :--- | :--- |
| `crates/ui-egui/src/views/mod.rs` | 83 | `1..=83` | 14-domain `ViewShardStates` ownership and `dispatch_custom_view_action` prefix routing across view shards without panics | Clean — shard state routing and error propagation verified |
| `crates/ui-egui/src/views/a11y.rs` | 243 | `1..=243` | ARIA live region announcements, focus-ring styling and high-contrast toggle state without direct document mutation | Clean — accessibility view state and focus announcement invariants verified |
| `crates/ui-egui/src/views/analyze_picker.rs` | 190 | `1..=190` | Category toggles, `execute_duplicate_confirm`, outlier routing, and `select_pcr_outcome` target-leakage guard | Clean — dialog picker actions and PCR outcome isolation verified |
| `crates/ui-egui/src/views/dialogs_graph_misc.rs` | 818 | `1..=818` | Zoom/pan clamping, command palette dispatch, change graph type, constant hooks, figure export, page layout, distribution simulation, and print actions | [`FA-SEC15-06`](#fa-sec15-06), [`FA-SEC15-07`](#fa-sec15-07), [`FA-SEC15-08`](#fa-sec15-08), [`FA-SEC15-12`](#fa-sec15-12) |
| `crates/ui-egui/src/views/fit_params.rs` | 1,311 | `1..=1311` | Nonlinear equation DSL validation, parameter constraints/initials, CI & covariance guards, model comparison toggles, linear X-range & R² readers | [`FA-SEC15-09`](#fa-sec15-09) |
| `crates/ui-egui/src/views/grid_core.rs` | 951 | `1..=951` | Arrow/Tab navigation, shift-selection materialization, clear selected cells, subcolumn formatting, and live-link popover go-to/unlink handlers | [`FA-SEC15-01`](#fa-sec15-01), [`FA-SEC15-10`](#fa-sec15-10) |
| `crates/ui-egui/src/views/import_paste.rs` | 1,701 | `1..=1701` | 5-tab Import & Paste Special preview pipeline, plate/tidy reshape paths, external CSV path/link modes, header & row-title TSV/CSV serialization | [`FA-SEC15-03`](#fa-sec15-03), [`FA-SEC15-04`](#fa-sec15-04) |
| `crates/ui-egui/src/views/inspector.rs` | 1,855 | `1..=1855` | Inspector tab routing, axis range/segment breaks, MV summary encoding, pairwise bracket/CLD updates, page layout grid arrangement, canvas presets | [`FA-SEC15-05`](#fa-sec15-05), [`FA-SEC15-12`](#fa-sec15-12) |
| `crates/ui-egui/src/views/mv_formula.rs` | 790 | `1..=790` | UTF-8 safe formula snippet insertion, role lock checks, formula function builder, level recoding, and explicit categorical level ordering | Clean — UTF-8 cursor slicing, role checks, and command dispatch verified |
| `crates/ui-egui/src/views/nav_notes.rs` | 306 | `1..=306` | `nav.create` target resolution, bounds-clamped `nav.reorder`, `family.select`, and `note.*` engine command dispatch | Clean — navigator CRUD/reorder bounds and error propagation verified |
| `crates/ui-egui/src/views/power_wizard.rs` | 736 | `1..=736` | Stage validation/switching, 8-family card selection, pilot inputs, RTL slider normalization, and sample-size project creation | [`FA-SEC15-13`](#fa-sec15-13) |
| `crates/ui-egui/src/views/results.rs` | 348 | `1..=348` | Bound tab resolution, cell/row/column selection tracking, TSV clipboard serialization, and preview fallback formatting | [`FA-SEC15-13`](#fa-sec15-13) |
| `crates/ui-egui/src/views/stat_params.rs` | 1,387 | `1..=1387` | ANOVA/RM/REML, Two-Group, Multiplicity, Outlier, Contingency, Survival, ROC, GLM, Multivariate, and PValueStack dispatches with `modal_predicates` gating | Clean — `modal_predicates` enforcement and parameter command parity verified |
| `crates/ui-egui/src/views/toolbar_menus.rs` | 645 | `1..=645` | Toolbar/context clipboard Cut/Copy/Clear/Exclude handlers, active selection resolution, full-document cloning, and region unlink actions | [`FA-SEC15-02`](#fa-sec15-02), [`FA-SEC15-10`](#fa-sec15-10) |
| `crates/ui-egui/src/views/welcome.rs` | 866 | `1..=866` | Welcome setup modes, X/subcolumn builders, sample fixture instantiation, template document builder, and clone pipeline | [`FA-SEC15-11`](#fa-sec15-11) |
| `crates/ui-egui/src/i18n/cjk_rtl.rs` | 633 | `1..=633` | 16-locale `STAGED_ENTRIES` parity, `try_tr_staged` helper usage, docstring accuracy, and `REPLACEMENT_PROPOSALS` currency | [`FA-SEC15-15`](#fa-sec15-15) |
| `crates/ui-egui/src/i18n/core_analyses.rs` | 817 | `1..=817` | 16-locale translation array completeness, placeholder tokens, and clean-room terminology across analysis catalog strings | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/core_cmd_analysis_graph.rs` | 870 | `1..=870` | 16-locale translation completeness across analysis/graph command labels and clean-room phrasing | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/core_cmd_project_sheet.rs` | 564 | `1..=564` | 16-locale translation completeness across project/sheet command labels and clean-room phrasing | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/core_cmd_table.rs` | 510 | `1..=510` | 16-locale translation completeness across table command labels; user-facing archive/XML import strings are clean-room (two internal legacy command IDs at `:276` and `:294` retain old format tokens) | Clean — 16-locale coverage and clean-room user labels verified |
| `crates/ui-egui/src/i18n/core_cmd_ui.rs` | 891 | `1..=891` | 16-locale UI command label parity and clean-room/terminology check across `cmd.ui.*` entries | [`FA-SEC15-14`](#fa-sec15-14) |
| `crates/ui-egui/src/i18n/core_dialogs.rs` | 295 | `1..=295` | 16-locale translation completeness across core dialog buttons/labels and placeholder consistency | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/core_inspector.rs` | 763 | `1..=763` | 16-locale translation completeness across inspector controls and statistical/graph terms | Clean — 16-locale coverage and clean-room terminology verified |
| `crates/ui-egui/src/i18n/core_menus.rs` | 1,159 | `1..=1159` | 16-locale translation completeness across desktop menus, tooltips, and file import labels | [`FA-SEC15-14`](#fa-sec15-14) |
| `crates/ui-egui/src/i18n/core_nav_tables.rs` | 260 | `1..=260` | 16-locale translation completeness across navigator folders and table archetypes | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/eu.rs` | 1,516 | `1..=1516` | 16-locale `STAGED_ENTRIES` (`83` `w6.view.*` entries) vs live callers and module header documentation accuracy | [`FA-SEC15-15`](#fa-sec15-15) |
| `crates/ui-egui/src/i18n/r13_8_ui.rs` | 1,071 | `1..=1071` | 16-locale translation completeness across R13.8 UI dialog/toolbar keys and placeholder parity | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/shard_extra.rs` | 8 | `1..=8` | Empty placeholder shard slice `ENTRIES: &[CatalogEntry] = &[]` mounted into `EXTRA_SHARD_CATALOGS` | [`FA-SEC15-15`](#fa-sec15-15) |
| `crates/ui-egui/src/i18n/tests.rs` | 520 | `1..=520` | Catalog uniqueness (`867` keys), 16-locale completeness, command label checks, non-Latin script guards, Beta RTL fallback, and source literal extraction tests | Clean — automated i18n integrity test suite verified |
| `crates/ui-egui/src/i18n/w5_analyze_picker.rs` | 297 | `1..=297` | 16-locale translation completeness across Wave 5 analyze-picker keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_dialogs_graph_misc.rs` | 369 | `1..=369` | 16-locale translation completeness across Wave 5 dialog/graph misc keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_fit_params.rs` | 474 | `1..=474` | 16-locale translation completeness across Wave 5 nonlinear/linear fit keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_grid_core.rs` | 548 | `1..=548` | 16-locale translation completeness across Wave 5 grid-core keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_import_paste.rs` | 472 | `1..=472` | 16-locale translation completeness across Wave 5 Import & Paste Special keys | [`FA-SEC15-14`](#fa-sec15-14) |
| `crates/ui-egui/src/i18n/w5_inspector.rs` | 837 | `1..=837` | 16-locale translation completeness across Wave 5 inspector keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_mv_formula.rs` | 585 | `1..=585` | 16-locale translation completeness across Wave 5 MV formula keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_nav_notes.rs` | 334 | `1..=334` | 16-locale translation completeness across Wave 5 navigator/notes keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_power_wizard.rs` | 322 | `1..=322` | 16-locale translation completeness across Wave 5 Power Wizard keys | [`FA-SEC15-14`](#fa-sec15-14) |
| `crates/ui-egui/src/i18n/w5_results.rs` | 346 | `1..=346` | 16-locale translation completeness across Wave 5 results view keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_stat_params.rs` | 280 | `1..=280` | 16-locale translation completeness across Wave 5 statistical parameter keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_toolbar_menus.rs` | 877 | `1..=877` | 16-locale translation completeness across Wave 5 toolbar/context menu keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w5_welcome.rs` | 1,001 | `1..=1001` | 16-locale translation completeness across Wave 5 welcome dialog keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w7_mv_ui.rs` | 348 | `1..=348` | 16-locale translation completeness across Wave 7 multivariable UI keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w7_rm_reml.rs` | 121 | `1..=121` | 16-locale translation completeness across Wave 7 RM/REML keys | Clean — 16-locale coverage and placeholder parity verified |
| `crates/ui-egui/src/i18n/w8_rv_ui.rs` | 102 | `1..=102` | 16-locale translation completeness across Wave 8 result viewer UI keys | Clean — 16-locale coverage and placeholder parity verified |

---

## Findings

### FA-SEC15-01

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/grid_core.rs:324-331, 399-420`
- **Category:** `resources` / `no-hang` (regression against bounded selection in `grid_view.rs`)
- **Defect:** `handle_grid_nav_action` handles extended keyboard navigation (`Shift+Arrow`, `Shift+PageUp/PageDown`, `Shift+Home/End`, `Shift+Cmd+Arrow`) by calling `sync_extended_selection(&mut app.grid, ds)` (`327-328`). Unlike `GridViewState::extend_shift_range` (`crates/ui-egui/src/grid_view.rs:1375-1380`), `sync_extended_selection` allocates an unbounded `coords` vector by iterating `for row in min_c.row..=max_row` and `for flat in 0..total_flat`, pushing every `GridCellCoord` into `grid.selected_coords` and every row index into `grid.selected_rows` without applying `MAX_MATERIALIZED_SELECTION_COORDS = 4096` or bounded row materialization.
- **Failure scenario:** Pressing `Shift+Cmd+Down` or `Shift+End` on a `1,000,000`-row datasheet causes `sync_extended_selection` to iterate `1,000,000 * total_flat` times and materialize millions of `GridCellCoord` and `usize` entries in memory, freezing the UI thread or triggering memory exhaustion.
- **Evidence:**
```rust
// crates/ui-egui/src/views/grid_core.rs:324-331
            GridNavEvent::Arrow { dir, extend, jump } => {
                app.grid.navigate(ds, dir, extend, jump);
                let cur = app.grid.tool.active.unwrap_or_default();
                if extend {
                    sync_extended_selection(&mut app.grid, ds);
                } else {
                    app.grid.select_single_sync(cur);
                }

// crates/ui-egui/src/views/grid_core.rs:399-420
fn sync_extended_selection(grid: &mut crate::grid_view::GridViewState, ds: &DataSheet) {
    let Some((min_c, max_c)) = grid.tool.selection_bounds() else {
        return;
    };
    let total_flat = total_flat_data_subcolumns(&ds.archetype);
    let max_row = max_c.row.min(ds.archetype.row_count().saturating_sub(1));
    let mut coords = Vec::new();
    let mut rows = Vec::new();
    for row in min_c.row..=max_row {
        rows.push(row);
        for flat in 0..total_flat {
            if let Some((col, subcol)) = resolve_flat_coord(&ds.archetype, flat)
                && (col, subcol) >= (min_c.col, min_c.subcol)
                && (col, subcol) <= (max_c.col, max_c.subcol)
            {
                coords.push(GridCellCoord::new(row, col, subcol));
            }
        }
    }
    grid.selected_coords = coords;
    grid.selected_rows = rows;
}
```
- **Suggested fix:** Remove manual full-grid coordinate materialization in `sync_extended_selection`; synchronize via `app.grid.tool` rectangular bounds and bounded `grid.bounded_selected_rows(ds)` (matching `GridViewState::extend_shift_range` in `crates/ui-egui/src/grid_view.rs:1375-1380`), clearing `selected_coords` whenever the rectangular cell count exceeds `MAX_MATERIALIZED_SELECTION_COORDS`.
- **Related:** `crates/ui-egui/src/grid_view.rs:1375-1380`; see also `SEC-14` `FA-SEC14-07`.

---

### FA-SEC15-02

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/toolbar_menus.rs:120-131, 179-188, 333-346, 515-527`
- **Category:** `data-loss` / `wrong-result` / `resources`
- **Defect:** Toolbar and context-menu clipboard/clear handlers introduce three coupled defects:
  1. **Discontiguous selection wiped to bounding rectangle:** `clipboard.cut` (`340-345`) and `ctx.clear_cells` (`522-527`) convert `coords` via `selection_bounding_range(&coords)` and send `"range"` to `table.clearCells`, even though `table.clearCells` supports explicit `"cells"` (`crates/engine/src/cmd/table.rs:2390`). Likewise, `copy_active_sheet_to_clipboard` (`180-185`) calls `ClipboardBundle::tsv_from_datasheet_range(ds, range, dec)` on the bounding rectangle instead of delegating to `app.grid.prepare_clipboard_selection(ds, dec)`.
  2. **Empty-selection fallback fabricates `(0, 0, 0)`:** `active_selection_coords_for` (`127-129`) pushes `GridCellCoord::new(0, 0, 0)` whenever `coords.is_empty()` and `tool.active` is `None`. Calling `clipboard.cut`, `ctx.clear_cells`, or `ctx.exclude_with_reason` on an unselected non-empty sheet silently mutates or deletes cell `(0, 0, 0)`.
  3. **Full-document clone on action dispatch:** `clipboard.cut` (`336`) and `ctx.clear_cells` (`516`, plus `:532`, `:632`) call `app.document().clone()` instead of reading sheet metadata (`ds.id`, `ds.frozen`) or borrowing the document before dropping the borrow and calling `execute_command`.
- **Failure scenario:** A user `Cmd+Click`-selects two discontiguous cells `(0, 0, 0)` and `(4, 2, 0)` and chooses `Cut` or `Clear Cells`; `selection_bounding_range` clears and copies every unselected cell in rows `0..=4`, columns `0..=2`. Conversely, triggering `Clear Cells` or `Cut` before selecting any cell silently deletes cell `(0, 0, 0)`.
- **Evidence:**
```rust
// crates/ui-egui/src/views/toolbar_menus.rs:120-131
fn active_selection_coords_for(app: &UiApp, ds: &DataSheet) -> Vec<GridCellCoord> {
    let mut coords = app.grid.all_selected_coords(ds);
    if coords.is_empty()
        && let Some(active) = app.grid.tool.active
    {
        coords.push(active);
    }
    if coords.is_empty() && ds.archetype.row_count() > 0 && ds.archetype.column_count() > 0 {
        coords.push(GridCellCoord::new(0, 0, 0));
    }
    coords
}

// crates/ui-egui/src/views/toolbar_menus.rs:179-188
        } else {
            let coords = app.grid.all_selected_coords(ds);
            if let Some(range) = selection_bounding_range(&coords)
                && !coords.is_empty()
            {
                ClipboardBundle::tsv_from_datasheet_range(ds, range, dec)
            } else {
                ClipboardBundle::tsv_from_datasheet(ds, dec)?
            }
        }

// crates/ui-egui/src/views/toolbar_menus.rs:333-346
    if payload == "clipboard.cut" {
        dismiss_context_overlays(app, state);
        let msg = copy_active_sheet_to_clipboard(app, None)?;
        let doc = app.document().clone();
        if let Some(ds) = active_data_sheet(&doc)
            && !ds.frozen
        {
            let coords = active_selection_coords_for(app, ds);
            if let Some(range) = selection_bounding_range(&coords) {
                app.execute_command("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "range": range_to_json(range) }))
                    .map_err(|e| e.to_string())?;
                app.grid.refresh_selection_stats(app.session.document());
            }
        }

// crates/ui-egui/src/views/toolbar_menus.rs:515-527
        "ctx.clear_cells" => {
            let doc = app.document().clone();
            let ds = active_data_sheet(&doc).ok_or_else(|| String::from("no active data sheet"))?;
            if ds.frozen {
                state.last_toolbar_notice = Some(String::from(crate::i18n::tr(&app.locale, "w7.mv.frozen_clear_blocked")));
                return Ok(UiEventOutcome { handled: true, action: String::from("grid_core.frozen_read_only"), ..Default::default() });
            }
            let coords = active_selection_coords_for(app, ds);
            if let Some(range) = selection_bounding_range(&coords) {
                app.execute_command("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "range": range_to_json(range) }))
                    .map_err(|e| e.to_string())?;
                app.grid.refresh_selection_stats(app.session.document());
            }
```
- **Suggested fix:** Remove the fabricated `(0, 0, 0)` fallback from `active_selection_coords_for` (require an explicit active or selected cell), send explicit `"cells"` rather than `"range"` to `table.clearCells` whenever the selection is discontiguous (preserving `"range"` only for contiguous rectangular tool selections), use `app.grid.prepare_clipboard_selection(ds, dec)` when copying selections, and extract `SheetId`/frozen state/coords from a short immutable borrow of `app.document()` without `.clone()`.
- **Related:** `crates/engine/src/cmd/table.rs:2390`, `crates/ui-egui/src/grid_view.rs:1550-1557`.

---

### FA-SEC15-03

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/import_paste.rs:959-1015, 1056-1085`
- **Category:** `wrong-result` / `contract-drift` / `data-loss`
- **Defect:** `execute_import_paste_commit` misaligns headers and drops row titles in its serialized preview payloads:
  1. **Left-shifted column headers on new sheet creation:** `serialize_preview_as_csv_with_headers` (`1068-1085`) writes `preview.headers` (data-column titles only, without a leading row-title column header) on line 1, while prepending `preview.row_labels[r_idx]` as column `0` of every data row (`1078-1080`). It then calls `table.importCsv` with `"has_header": true` and `"has_row_titles": true` (`1009-1010`). In `crates/format/src/csv.rs:694-734`, `has_row_titles: true` sets `col_offset = 1`, skipping `headers[0]` as the row-title column header, assigning `headers[1]` to data column `0`, and dropping the final column's header.
  2. **Row titles dropped on existing-sheet paste:** When `state.target_mode == "existing_sheet"` (`959-985`), `serialize_preview_as_tsv` (`1056-1066`) serializes only `preview.rows` and omits `preview.row_labels`, discarding parsed row labels during paste into an existing sheet.
- **Failure scenario:** Importing a CSV with `Has Header` + `Has Row Titles` (`RowLabel,Dose,Response`) via the Import & Paste Special modal creates a new sheet whose data columns are labeled `Response` and `C` (shifted left by one) instead of `Dose` and `Response`. Switching target mode to `existing_sheet` silently drops all row titles.
- **Evidence:**
```rust
// crates/ui-egui/src/views/import_paste.rs:959-1013
    if state.target_mode == "existing_sheet" {
        let target_id = resolve_existing_target_sheet(app, state)?;
        let tsv = serialize_preview_as_tsv(&preview, state.propagate_title_header);
...
    }

    // Create new sheet via table.importCsv from the reshaped preview matrix.
    let csv_text = serialize_preview_as_csv_with_headers(&preview);
...
    let out = app
        .execute_command(
            "table.importCsv",
            json!({
                "title": title,
                "archetype": table_kind_wire_name(target_kind),
                "subcolumn_mode": subcol_wire,
                "replicates": 1,
                "csv": csv_text,
                "delimiter": "\t",
                "decimal_separator": ".",
                "has_header": true,
                "has_row_titles": state.has_row_titles && !preview.row_labels.is_empty(),
            }),
        )

// crates/ui-egui/src/views/import_paste.rs:1056-1085
fn serialize_preview_as_tsv(preview: &ImportPreviewGrid, include_headers: bool) -> String {
    let mut lines = Vec::new();
    if include_headers && !preview.headers.is_empty() {
        lines.push(preview.headers.join("\t"));
    }
    for row in &preview.rows {
        let toks: Vec<String> = row.iter().map(format_linked_paste_tsv_cell).collect();
        lines.push(toks.join("\t"));
    }
    lines.join("\n")
}

fn serialize_preview_as_csv_with_headers(preview: &ImportPreviewGrid) -> String {
    let mut lines = Vec::new();
    if !preview.headers.is_empty() {
        lines.push(preview.headers.join("\t"));
    } else if let Some(first) = preview.rows.first() {
        let hdrs: Vec<String> = (0..first.len()).map(curvebook_doc::default_column_label).collect();
        lines.push(hdrs.join("\t"));
    }
    for (r_idx, row) in preview.rows.iter().enumerate() {
        let mut toks = Vec::new();
        if !preview.row_labels.is_empty() {
            toks.push(preview.row_labels.get(r_idx).cloned().unwrap_or_default());
        }
        toks.extend(row.iter().map(format_linked_paste_tsv_cell));
        lines.push(toks.join("\t"));
    }
    lines.join("\n")
}
```
- **Suggested fix:** Prepend a placeholder header column (e.g. `"Row"`) in `serialize_preview_as_csv_with_headers` whenever `!preview.row_labels.is_empty()` and `state.has_row_titles` is enabled (and preserve `decimal_separator`), and serialize/apply `preview.row_labels` in the `existing_sheet` `table.paste` path when row titles are active and compatible with the target anchor.
- **Related:** `crates/format/src/csv.rs:694-734`.

---

### FA-SEC15-04

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/import_paste.rs:889-957, 1098-1120`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Two input branches in `execute_import_paste_commit` ignore active UI state fields:
  1. **Non-live `external_csv` ignores `external_path` (`WID_PATH`):** When `state.source_origin == "external_csv"` and `state.paste_as_live_link == false`, `execute_import_paste_commit` bypasses lines `921-957` and runs `compute_import_preview` (`916`), which reads only `WID_CSV` (`effective_source_text` at `1111`), never reading `WID_PATH` or passing `"path"` to `table.importCsv` (which supports file-path import per `crates/engine/src/cmd/table/import.rs:610-611`).
  2. **`TidyLongPivot` fast-path drops view/filter options:** Lines `898-913` forward directly to `app.grid.import_reshape.build_import_intent()` (`table.importTidyLong`) whenever slicing/wrap/filter/transpose fields are at defaults, ignoring `state.has_row_titles`, `state.skip_contiguous_blanks`, `state.parse_asterisk_exclusions`, `state.has_header_row = false`, and `state.propagate_title_header` (`table.importTidyLong` in `crates/engine/src/cmd/table/import.rs:1389-1391` hardcodes `header_rows: 1, has_row_titles: false, parse_asterisk_exclusions: false`).
- **Failure scenario:** Selecting the `External CSV` source origin, typing a valid file path into `import_paste.input.path` with `Live Link` unchecked, and clicking Import always fails with `"Enter or paste CSV/TSV data in Source tab."`. Similarly, toggling `Parse * Exclusions` or `Has Row Titles` in `Tidy Long` reshape mode is silently ignored unless the user also sets a slice/filter option to bypass the fast path.
- **Evidence:**
```rust
// crates/ui-egui/src/views/import_paste.rs:897-921, 945-954
        // TidyLongPivot: honor extra slicing/row-wrap if enabled, else standard importTidyLong intent.
        if state.row_wrap_width == 0
            && state.row_start.is_empty()
            && state.row_end.is_empty()
            && state.col_start.is_empty()
            && state.col_end.is_empty()
            && state.decimate_every_k <= 1
            && state.filter_pattern.trim().is_empty()
            && state.exclude_predicate.trim().is_empty()
            && !state.transpose
            && state.target_mode == "new_sheet"
        {
            let intent = app.grid.import_reshape.build_import_intent()?;
            let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            maybe_append_provenance_to_info(app, state, out.payload.get("sheet_id").and_then(Value::as_str));
            return Ok(out.payload);
        }
    }

    let preview = compute_import_preview(app, state);
    if let Some(err) = preview.validation_error {
        return Err(err);
    }

    if state.paste_as_live_link && matches!(state.source_origin.as_str(), "internal_sheet" | "external_csv") {
...
        } else {
            let path = effective_text(app, WID_PATH, &state.external_path);
            if path.trim().is_empty() {
                return Err(String::from("live external CSV link requires a file path"));
            }
            params.insert(String::from("source_path"), json!(path.trim()));

// crates/ui-egui/src/views/import_paste.rs:1111-1120
    let raw = effective_source_text(app, state);
    let clean = normalize_text_for_encoding(&raw, &state.encoding);
    if clean.trim().is_empty() {
        return ImportPreviewGrid {
            headers: Vec::new(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            validation_error: Some(String::from("Enter or paste CSV/TSV data in Source tab.")),
        };
    }
```
- **Suggested fix:** When `state.source_origin == "external_csv"` and `state.paste_as_live_link == false` (and `WID_CSV` is empty), read or forward `"path"` (`effective_text(app, WID_PATH, &state.external_path)`) to `table.importCsv` (or load via host/format helper with the same filtering options), and guard the `TidyLongPivot` fast path on `!state.has_row_titles && !state.parse_asterisk_exclusions && !state.skip_contiguous_blanks && state.has_header_row && state.propagate_title_header`.
- **Related:** `crates/engine/src/cmd/table/import.rs:610-611, 1389-1391`.

---

### FA-SEC15-05

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/inspector.rs:1084-1096`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** In `dispatch_mv_encoding_action`, the action `"mv.metric:Count"` maps `"Count"` directly to `SummaryMeasure::GeometricMean` (`crates/doc/src/graph.rs:509-517` only defines `Mean`, `Median`, and `GeometricMean`).
- **Failure scenario:** Triggering `"mv.metric:Count"` in the Multivariable Graph Inspector configures bars/dots to display the geometric mean instead of counts, causing misleading summary plots on positive data and missing/undefined summaries when values are zero or negative.
- **Evidence:**
```rust
// crates/ui-egui/src/views/inspector.rs:1084-1091
    } else if let Some(m_raw) = sub.strip_prefix("metric:") {
        let next_m = match m_raw {
            "Mean" => SummaryMeasure::Mean,
            "Median" => SummaryMeasure::Median,
            "Count" => SummaryMeasure::GeometricMean,
            _ => return Ok(UiEventOutcome::default()),
        };
```
- **Suggested fix:** Expose `"GeometricMean"` (mapping to `SummaryMeasure::GeometricMean`) in the MV inspector metric action, and either remove `"Count"` from UI metric options or implement a dedicated `SummaryMeasure::Count` variant in `curvebook_doc` and the graph renderer.
- **Related:** `crates/doc/src/graph.rs:509-517`.

---

### FA-SEC15-06

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/dialogs_graph_misc.rs:231-268`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `handle_graph_hook_export_action` stores `selected_option_id`, `summary_measure`, and `error_bar_metric` on `app.canvas_dialogs.change_graph_type` (`231-255`), but `"graph_type:apply"` (`257-268`) executes `graph.setKind` with only `{ "sheet_id": gid.as_str(), "kind": graph_kind }`. It omits `"option_id"` (which `crates/engine/src/cmd/graph.rs:1532-1536` explicitly consumes to set grouped arrangement and GDO family preset) and never applies `summary_measure` or `error_bar_metric`.
- **Failure scenario:** Choosing a specific grouped bar variant (e.g. interleaved vs. stacked vs. floating bar), switching summary metric to Median/GeoMean, or switching error bars to 95% CI in the Change Graph Type dialog and clicking Apply (`graph_type:apply`) resets to default options for the bare `GraphKind`.
- **Evidence:**
```rust
// crates/ui-egui/src/views/dialogs_graph_misc.rs:257-268
    if payload == "graph_type:apply" {
        if let Some(gid) = app.canvas_dialogs.change_graph_type.graph_sheet_id.clone() {
            let opt_id = app.canvas_dialogs.change_graph_type.selected_option_id.clone();
            let graph_kind = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == opt_id).map_or(GraphKind::Xy, |o| o.graph_kind);
            let res = app.execute_command("graph.setKind", json!({ "sheet_id": gid.as_str(), "kind": graph_kind }));
            return Some(match res {
                Ok(_) => handled_outcome(true),
                Err(err) => Err(err.to_string()),
            });
        }
        return Some(handled_outcome(false));
    }
```
- **Suggested fix:** Build intents using `app.canvas_dialogs.change_graph_type.build_intents(...)` (as implemented in `dialogs_canvas/graph_type.rs`) or pass `"option_id": opt_id` in `graph.setKind` alongside `graph.setSpec`/`graph.setSeries` updates for `summary_measure` and `error_bar_metric`.
- **Related:** `crates/ui-egui/src/dialogs_canvas/graph_type.rs`, `crates/engine/src/cmd/graph.rs:1532-1536`.

---

### FA-SEC15-07

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/dialogs_graph_misc.rs:202-218`
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** `handle_command_palette_action` `"palette:run"` (`202-218`) executes every chosen `cmd_id` via `app.execute_command(&cmd_id, json!({}))` even though the command palette indexes both engine commands and `UI_COMMANDS` (`ui.*` / `app.*`). Engine `execute_command` only knows registered engine commands and rejects all `UI_COMMANDS`.
- **Failure scenario:** Typing a UI command (e.g., `ui.pane.cycle`, `ui.zoom.in`, `ui.help.about`) in the Command Palette and dispatching `"palette:run"` fails with an unknown engine command error instead of invoking `dispatch_ui_command`.
- **Evidence:**
```rust
// crates/ui-egui/src/views/dialogs_graph_misc.rs:202-218
        "palette:run" => {
            let idx = app.canvas_dialogs.command_palette.selected_index;
            let chosen = matches.get(idx).or_else(|| matches.first()).map(|e| e.id.to_owned());
            if let Some(cmd_id) = chosen {
                app.canvas_dialogs.close_dialog();
                app.focus_widget_id = None;
                return Some(
                    app.execute_command(&cmd_id, json!({}))
                        .map(|_| UiEventOutcome {
                            handled: true,
                            action: format!("palette.run:{cmd_id}"),
                            command_id: Some(cmd_id),
                            ..UiEventOutcome::default()
                        })
                        .map_err(|err| err.to_string()),
                );
            }
            Some(handled_outcome(false))
        }
```
- **Suggested fix:** Check whether `cmd_id` belongs to `UI_COMMANDS` (or starts with `"ui."` / `"app."`) and route it through `crate::control_io::dispatch_ui_command(app, &cmd_id, &json!({}))`, keeping engine commands on `app.execute_command` (and forwarding command `params_doc` if required by parameterized UI commands).
- **Related:** `crates/ui-egui/src/dialogs_canvas/command_palette.rs`, `SEC-14` `FA-SEC14-08`.

---

### FA-SEC15-08

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/dialogs_graph_misc.rs:102-117, 132-135, 151-159`
- **Category:** `no-panic` / `wrong-result` (non-finite canvas state poisoning)
- **Defect:** `handle_zoom_pan_action` parses numeric zoom percentages (`132`) and pan deltas (`155`) via `.parse::<f64>()` without checking `.is_finite()`. `apply_zoom_and_pan` (`103-116`) passes those numbers to `.clamp(0.25, 4.0)` and `.clamp(-1200.0, 1200.0)`, which in Rust return `f64::NAN` on `NaN` inputs.
- **Failure scenario:** Sending `"zoom:set:sheet1:NaN"` or `"pan:nudge:sheet1:NaN:0"` poisons `canvas_dialogs.canvas.{zoom, pan_pt}` and `pan_zoom.{zoom, pan_pt}` with `NaN`, breaking all relative `zoom:step` and `pan:scroll_*` operations (`NaN + delta = NaN`) until manually reset via `"zoom:set:*:fit"` or `"pan:reset:*"`.
- **Evidence:**
```rust
// crates/ui-egui/src/views/dialogs_graph_misc.rs:102-116, 132-135, 154-159
fn apply_zoom_and_pan(app: &mut UiApp, zoom: Option<f64>, reset_pan: bool, pan_delta: Option<[f64; 2]>) {
    if let Some(z) = zoom {
        let clamped = z.clamp(0.25, 4.0);
        app.canvas_dialogs.canvas.zoom = clamped;
        app.canvas_dialogs.pan_zoom.zoom = clamped;
    }
    if reset_pan {
        app.canvas_dialogs.canvas.pan_pt = PointPt::new(0.0, 0.0);
        app.canvas_dialogs.pan_zoom.pan_pt = [0.0, 0.0];
    } else if let Some([dx, dy]) = pan_delta {
        let nx = (app.canvas_dialogs.canvas.pan_pt.x + dx).clamp(-1200.0, 1200.0);
        let ny = (app.canvas_dialogs.canvas.pan_pt.y + dy).clamp(-1200.0, 1200.0);
...
        if let Ok(pct) = preset.parse::<f64>() {
            apply_zoom_and_pan(app, Some(pct / 100.0), false, None);
...
        if let (Some(dx_s), Some(dy_s)) = (parts.next(), parts.next())
            && let (Ok(dx), Ok(dy)) = (dx_s.parse::<f64>(), dy_s.parse::<f64>())
        {
            apply_zoom_and_pan(app, None, false, Some([dx, dy]));
```
- **Suggested fix:** Guard `pct.is_finite()`, `dx.is_finite()`, `dy.is_finite()` at the parse sites and ignore non-finite values inside `apply_zoom_and_pan`.

---

### FA-SEC15-09

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/fit_params.rs:778-817, 915-933, 960-1013, 1178-1181` (vs `:1187-1229`)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Nonlinear fit text inputs in `fit_params.rs` parse numbers with `.parse::<f64>()` (`782`, `796`, `806`, `926`, `991`, `1180`) instead of `parse_localized_number_input(..., &app.locale_preset)` (which `read_linear_range_inputs` at `1191` and `read_r_squared_threshold` at `1225` correctly use). In addition, malformed values in those nonlinear fields are silently ignored while still returning `true` (`handled: true`), whereas `handle_confidence_action` returns `false` (`handled: false`) without diagnostics when `"ci_method:profile"` or `"toggle_emit_cov"` is blocked in `RobustCauchy`/`ROUT` mode (`962-964`, `1008-1010`).
- **Failure scenario:** Under European comma-decimal locales (`de`, `fr`, `es`, `it`, `nl`, `pl`, `ru`, `pt-BR`), entering `"0,5"` or `"95,5"` into parameter constant value, lower/upper bounds, initial override, or confidence level fields is silently discarded without error feedback.
- **Evidence:**
```rust
// crates/ui-egui/src/views/fit_params.rs:781-800, 924-932, 990-997, 1178-1181
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.const.input")
            && let Ok(val) = buf.text.trim().parse::<f64>()
            && val.is_finite()
        {
            rule.scope = ParameterScope::Constant(val);
        }
...
            if txt.is_empty() || txt.eq_ignore_ascii_case("auto") {
                rule.initial_override = None;
            } else if let Ok(val) = txt.parse::<f64>()
                && val.is_finite()
            {
                rule.initial_override = Some(val);
            }
...
    if payload == "apply_ci_level" {
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.ci.lvl.input")
            && let Ok(pct) = buf.text.trim().parse::<f64>()
            && pct.is_finite()
        {
            set_percent_clamped(confidence_level, pct);
        }
        return true;
    }
...
fn parse_optional_bound(app: &UiApp, id: &str) -> Option<f64> {
    let txt = app.interaction.text_field_buffers.get(id)?.text.trim();
    if txt.is_empty() { None } else { txt.parse::<f64>().ok().filter(|v| v.is_finite()) }
}
```
- **Suggested fix:** Switch all numeric text-field parsers in `fit_params.rs` to `parse_localized_number_input(..., &app.locale_preset)` (with localized error feedback matching `read_linear_range_inputs`), and return `Err(...)` (or a clear status notice) when `RobustCauchy`/`ROUT` mode blocks profile CI or covariance tab output.
- **Related:** `crates/ui-egui/src/views/fit_params.rs:1187-1229`.

---

### FA-SEC15-10

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/grid_core.rs:762-786` and `crates/ui-egui/src/views/toolbar_menus.rs:631-642`
- **Category:** `data-loss` / `contract-drift`
- **Defect:** `grid_core` `"link:unlink:{region_id}"` binds `region_id` from the payload prefix and immediately discards it (`let _ = region_id;` at `767`), sending `table.clearCells` with `"cells": [], "unlink": true` (`770-775`). `toolbar_menus` `"ctx.unlink_region"` (`631-634`) sends the identical empty `"cells": [], "unlink": true` payload. On the engine side (`crates/engine/src/cmd/table.rs:2410-2412`), empty `"cells"` combined with `"unlink": true` calls `ds.linked_regions.clear()`, erasing every linked region on the sheet instead of just the targeted region.
- **Failure scenario:** On a datasheet containing two live pasted links (e.g., columns `A` and `B`), clicking `Unlink` in the popover for region `A` or invoking `ctx.unlink_region` inside region `A` silently detaches both region `A` and region `B`.
- **Evidence:**
```rust
// crates/ui-egui/src/views/grid_core.rs:762-777
    if let Some(region_id) = payload.strip_prefix("link:unlink:") {
        let Some(ds) = active_data_sheet(app) else {
            return Ok(Some(UiEventOutcome::default()));
        };
        let sid = ds.id.to_string();
        let _ = region_id;
        let out = app
            .execute_command(
                "table.clearCells",
                json!({
                    "sheet_id": sid,
                    "cells": [],
                    "unlink": true,
                }),
            )
            .map_err(|e| e.to_string())?;

// crates/ui-egui/src/views/toolbar_menus.rs:631-635
        "ctx.unlink_region" => {
            let doc = app.document().clone();
            let ds = active_data_sheet(&doc).ok_or_else(|| String::from("no active data sheet"))?;
            app.execute_command("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "unlink": true, "cells": [] })).map_err(|e| e.to_string())?;
            state.last_toolbar_notice = Some(String::from("Detached linked region"));
```
- **Suggested fix:** Resolve the specific linked region (`region_id` in `grid_core.rs`, or the region containing the active cell/selection in `toolbar_menus.rs`) and pass its cell coordinates (or add `region_id` support to `table.clearCells`) rather than sending `"cells": []` when targeting one region.
- **Related:** `crates/engine/src/cmd/table.rs:2410-2412`.

---

### FA-SEC15-11

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/welcome.rs:532-557`
- **Category:** `data-loss` / `wrong-result`
- **Defect:** `instantiate_sample_via_commands` creates a sample project sheet via `sheet.createData` (`544`) followed by up to four separate follow-up `execute_command` mutations (`populate_sample_x_values`, `populate_sample_titles`, `populate_sample_cells`, `populate_sample_mv_roles_and_formulas` at `546-549`) without wrapping them in an atomic command/transaction or rolling back on partial failure.
- **Failure scenario:** If any later step returns `Err` (for instance cell validation, MV role setup, or sheet budget enforcement), the previously committed empty/half-populated `DataSheet` (and any `auto_graph`/`auto_info` sheets) remain permanently in the project. On normal completion, creating one sample pushes multiple separate entries onto the undo stack.
- **Evidence:**
```rust
// crates/ui-egui/src/views/welcome.rs:543-550
    let create_params = build_sample_create_params(&fixture.archetype, sid, &title, auto_graph, auto_info);
    let create_out = app.execute_command("sheet.createData", create_params).map_err(|e| e.to_string())?;

    populate_sample_x_values(app, sid, &fixture.archetype)?;
    populate_sample_titles(app, sid, &fixture.archetype)?;
    populate_sample_cells(app, sid, &fixture.archetype)?;
    populate_sample_mv_roles_and_formulas(app, sid, &fixture.archetype)?;
```
- **Suggested fix:** Support initial cells/titles/MV roles inside a single atomic `sheet.createData` (or batch command) payload, or undo the created sheet if any `populate_sample_*` step fails.

---

### FA-SEC15-12

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/inspector.rs:747-810, 939-946, 1272-1293, 1462-1482` and `crates/ui-egui/src/views/dialogs_graph_misc.rs:752-766`
- **Category:** `wrong-result` / `data-loss`
- **Defect:** Five subtle numeric/layout state bugs across Inspector and Simulate Distributions helpers:
  1. `build_segments_for_count` (`inspector.rs:747-809`) resets `major_tick_interval: None` whenever setting 2 or 3 axis breaks (`769, 777, 788, 796, 804`) and falls back to `[0.0, 100.0]` when axis bounds are `Auto` (`749-751`).
  2. `apply_range` (`inspector.rs:942-946`) returns `Ok(UiEventOutcome::default())` (`handled: false`) silently when `hi <= lo` instead of reporting a validation error.
  3. `dispatch_pairwise_action` `"sig_only:toggle"` (`inspector.rs:1272-1292`) returns `outcome_ok(sub, Some("graph.addComparisonBrackets"))` unconditionally even when `find_pairwise_result_sheet_id` returns `None` or `gs.spec.comparisons.is_empty()` and no command ran.
  4. `rearrange_layout_grid` (`inspector.rs:1475`) wraps row index via `(panel_idx / usize::from(cols)) % usize::from(rows)`, placing panels beyond `rows * cols` directly on top of row `0` panels instead of expanding rows or rejecting overflow.
  5. `apply_sim_dist_param_step` (`dialogs_graph_misc.rs:754, 766`) mutates `Uniform::min += s` without clamping `<= col.max - 0.5`, allowing `min > max` (while `p2_` enforces `max >= min + 0.5`).
- **Evidence:**
```rust
// crates/ui-egui/src/views/inspector.rs:749-752, 769, 942-946, 1272-1293, 1475
    let lo = base.min.unwrap_or(0.0);
    let hi = base.max.unwrap_or(100.0);
...
                major_tick_interval: None,
...
        if let (Some(lo), Some(hi)) = (next_min, next_max)
            && hi <= lo
        {
            return Ok(UiEventOutcome::default());
        }
...
        "sig_only:toggle" => {
            pw.hide_non_significant = !pw.hide_non_significant;
            app.canvas_dialogs.pairwise = pw.clone();
            if let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs)
                && !gs.spec.comparisons.is_empty()
            {
                app.execute_command("graph.addComparisonBrackets", ...).map_err(|e| e.to_string())?;
            }
            return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
        }
...
            let r = (panel_idx / usize::from(cols)) % usize::from(rows);

// crates/ui-egui/src/views/dialogs_graph_misc.rs:753-755, 766
                crate::dialogs_r9::SimDistKind::Uniform => col.min += s,
...
                crate::dialogs_r9::SimDistKind::Uniform => col.max = (col.max + s).max(col.min + 0.5),
```
- **Suggested fix:** Preserve `base.major_tick_interval` across axis segment splits; return `Err(...)` on `hi <= lo`; omit `command_id` when `"sig_only:toggle"` did not execute `graph.addComparisonBrackets`; avoid overlapping `% rows` stacking in `rearrange_layout_grid` (place extra rows below or clamp/expand `rows`); and clamp `col.min = (col.min + s).min(col.max - 0.5)` in `apply_sim_dist_param_step`.

---

### FA-SEC15-13

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/views/results.rs:233-260` and `crates/ui-egui/src/views/power_wizard.rs:146-148`
- **Category:** `wrong-result` / `data-loss`
- **Defect:**
  1. `results.rs::handle_copy_action` (`238-246`) indexes `rs.tabs.get(app.canvas_dialogs.result_view.active_tab_index)` instead of using `state.bound_tab_idx` when `state.bound_sheet_id.as_deref() == Some(sheet_id)`. If a caller selects cells/rows/columns on tab `N` via `results.select_cell` / `select_row` / `select_col` while `canvas_dialogs.result_view.active_tab_index` still points to tab `0` (or out of bounds), `results.copy_selection` copies from the wrong tab (`rs.tabs.first()`) using tab `N`'s selection coordinates.
  2. `power_wizard.rs::handle_stage_action` (`148`) computes `PowerWizardStage::from_index(idx.saturating_sub(1))`, silently coalescing an invalid `"stage:0"` payload to stage index `0` (`Stage 1`) rather than rejecting `0`.
- **Evidence:**
```rust
// crates/ui-egui/src/views/results.rs:238-246
        let active_idx = app.canvas_dialogs.result_view.active_tab_index;
        let fallback_tab =
            rs.tabs.is_empty().then(|| rs.analyzable_data.as_ref().map(|ds| datasheet_to_preview_tab(ds, &app.locale_preset))).flatten();
        let tab_ref = rs.tabs.get(active_idx).or_else(|| rs.tabs.first()).or(fallback_tab.as_ref());
        let Some(tab) = tab_ref else {
            return UiEventOutcome::default();
        };
        let sel = if force_full { None } else { state.selection_bounds(sheet_id, active_idx) };

// crates/ui-egui/src/views/power_wizard.rs:146-148
fn handle_stage_action(app: &mut UiApp, state: &mut PowerWizardViewState, raw_stage: &str) -> Result<UiEventOutcome, String> {
    let idx: usize = raw_stage.trim().parse().map_err(|_| format!("invalid power stage `{raw_stage}`"))?;
    let target = PowerWizardStage::from_index(idx.saturating_sub(1)).ok_or_else(|| format!("unknown power stage `{idx}`"))?;
```
- **Suggested fix:** Prefer `state.bound_tab_idx` in `handle_copy_action` when `state.bound_sheet_id.as_deref() == Some(sheet_id)` (syncing `result_view.active_tab_index` on selection), and reject `idx == 0` via `idx.checked_sub(1).and_then(PowerWizardStage::from_index)` in `handle_stage_action`.

---

### FA-SEC15-14

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/i18n/core_menus.rs:1051-1068`, `crates/ui-egui/src/i18n/core_cmd_ui.rs:351-368`, `crates/ui-egui/src/i18n/w5_power_wizard.rs:160-213, 304-321`, and `crates/ui-egui/src/i18n/w5_import_paste.rs:368-386`
- **Category:** `wrong-result` (localization quality / untranslated terms & milestone leakage)
- **Defect:** Active Beta translations retain untranslated English technical strings and internal development tags:
  1. `core_menus.rs:1051-1068` (`menu.file.import_reshape`) leaves raw English `"Tidy Long"` across **all 15** non-English locales (`de` through `he`), whereas `core_cmd_table.rs:438-455` (`cmd.table.importTidyLong`) translates the concept in all 16 locales.
  2. `core_cmd_ui.rs:351-368` (`cmd.ui.result.applyRemediation`) embeds internal milestone identifier `"QOL-7"` in user-facing localized labels across all 16 locales.
  3. `w5_power_wizard.rs:160-213, 304-321` (`pw.section.sensitivity`, `pw.param.alpha`, `pw.param.target_power`, `pw.plot.title`) leaves English `"Alpha x Power"`, `"1 - beta"`, `"Power vs."` / `"vs."` in European Beta locales (`de`, `es`, `fr`, `pt-BR`, `it`, `nl`, `pl`, `tr`) even after CJK/RU rows were upgraded to `α`/`β`.
  4. `w5_import_paste.rs:368-386` (`import_paste.notes.record_info`) leaves untranslated `"Info"` / `"Info-Blatt"` in `de`, `es`, `fr`, `pt-BR`, `it`, `pl` after CJK/RU/RTL rows were localized.
- **Evidence:**
```rust
// crates/ui-egui/src/i18n/core_menus.rs:1051-1068
    entry!("menu.file.import_reshape" => [
        en: "Plate / Tidy Long Import...",
        de: "Platten- / Tidy-Long-Import...",
        es: "Importar placa / Tidy Long...",
        fr: "Importation plaque / Tidy Long...",
        ja: "プレート / Tidy Long インポート...",
        ko: "플레이트 / Tidy Long 가져오기...",
...

// crates/ui-egui/src/i18n/core_cmd_ui.rs:351-368
    entry!("cmd.ui.result.applyRemediation" => [
        en: "Diagnostic Remediation Disabled (QOL-7 Diagnostics-Only)",
        de: "Diagnose-Korrektur deaktiviert (QOL-7 nur Diagnose)",
...

// crates/ui-egui/src/i18n/w5_power_wizard.rs:171, 207, 308, 315
        "Gevoeligheidsmatrix (Alpha x Power)",
        "Doelpower (1 - bèta)",
        "Puissance vs Taille d'échantillon",
        "Power vs. Steekproefgrootte",
```
- **Suggested fix:** Remove the internal `"QOL-7"` milestone tag from `cmd.ui.result.applyRemediation`, translate `"Tidy Long"` in `menu.file.import_reshape` consistently with `cmd.table.importTidyLong`, localize `"Info"`/`"Info-Blatt"` in `import_paste.notes.record_info`, and standardize `α`/`β`/native `"Power"` & `"vs."` terminology across European locales in `w5_power_wizard.rs`.
- **Related:** `crates/ui-egui/src/i18n/core_cmd_table.rs:438-455`.

---

### FA-SEC15-15

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/i18n/cjk_rtl.rs:1-45, 335-633`, `crates/ui-egui/src/i18n/eu.rs:1-25, 16-1515`, and `crates/ui-egui/src/i18n/shard_extra.rs:1-8`
- **Category:** `cleanup` / `architecture`
- **Defect:** Residual Wave 6 sharding scaffolding leaves dead keys, unused helpers, and stale module documentation in `crates/ui-egui/src/i18n/`:
  1. `cjk_rtl.rs` defines `try_tr_staged` (`31-35`) and `14` `w6.cjk_rtl.*` entries in `STAGED_ENTRIES` (`38-333`) with **zero callers** anywhere in `crates/` outside `i18n/cjk_rtl.rs` (duplicating existing inspector/dialog/power keys such as `w6.cjk_rtl.inspector.panel_title` vs `w6.view.insp.panel_title`).
  2. `eu.rs` mounts `83` `w6.view.*` entries (`16-1515`), of which `62` have zero references outside `i18n/`.
  3. Module doc comments in `cjk_rtl.rs:3-11` and `eu.rs:3-5` still state that `ENTRIES` remains empty (`653`-key baseline), contradicting `pub const ENTRIES: &[CatalogEntry] = STAGED_ENTRIES;` (`cjk_rtl.rs:16`, `eu.rs:13`) and the active `867`-key catalog (`i18n/tests.rs:60`), while `cjk_rtl.rs:336-633` keeps `37` `REPLACEMENT_PROPOSALS` that have already been applied in their target files (and still cite `"crates/ui-egui/src/i18n.rs"` for keys residing in `core_*.rs`).
  4. `shard_extra.rs` (`1-8`) holds only an empty `&[]` slice (`ENTRIES = &[]`).
- **Evidence:**
```rust
// crates/ui-egui/src/i18n/cjk_rtl.rs:3-16, 31-35
//! - [`ENTRIES`] stays empty during parallel sharding so `xtask i18n-report` remains green on
//!   branches without editing `i18n.rs`.
//! - [`STAGED_ENTRIES`] stages new `w6.cjk_rtl.*` [`CatalogEntry`] rows (complete across all 16
//!   locales) ready for `r10-w6-integ` to mount into `EXTRA_SHARD_CATALOGS` / [`ENTRIES`] alongside
//!   updated `catalog_len()`.
...
pub const ENTRIES: &[CatalogEntry] = STAGED_ENTRIES;
...
pub fn try_tr_staged(locale: &str, key: &str) -> Option<&'static str> {

// crates/ui-egui/src/i18n/eu.rs:3-13
//! During Wave 6 parallel sharding, [`ENTRIES`] intentionally stays empty so `i18n::catalog_len()`
//! preserves the shared `653`-key baseline until `r10-w6-integ` mounts [`STAGED_ENTRIES`] alongside
...
pub const ENTRIES: &[CatalogEntry] = STAGED_ENTRIES;
```
- **Suggested fix:** Remove unused `try_tr_staged`, already-applied `REPLACEMENT_PROPOSALS`, unreferenced duplicate `w6.cjk_rtl.*` / `w6.view.*` staging rows, and the empty `shard_extra.rs` stub (or wire the intended `w6.*` keys into views), and update `cjk_rtl.rs`/`eu.rs` module docs and `tests.rs` expected catalog length accordingly.
- **Related:** `crates/ui-egui/src/i18n/tests.rs:60`, `crates/ui-egui/src/i18n.rs`.
