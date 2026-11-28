# CurveBook Command Reference

Registered commands: 116

## Project & Files (`project`)

### `file.new` — New Project

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/New Project`
- **Shortcut:** `Cmd+N`
- **Summary:** Create a fresh empty CurveBook project.
- **Beginner note:** Replaces the active workspace with an empty project document (preserving undo history in this session).
- **Parameters:** `title` (string|null)

### `project.createCheckpoint` — Create Checkpoint

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `File/Checkpoints/Create Checkpoint`
- **Summary:** Save a named project snapshot checkpoint inside the document.
- **Beginner note:** Captures a named milestone copy of all sheets so you can restore or branch back to it later.
- **Parameters:** `id` (string|null), `name*` (string), `note` (string|null), `timestamp_utc` (string|null)

### `project.deleteCheckpoint` — Delete Checkpoint

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/Checkpoints/Delete Checkpoint`
- **Summary:** Remove a named checkpoint from the project.
- **Beginner note:** Deletes a stored checkpoint by ID or name without altering active data sheets.
- **Parameters:** `id` (string|null), `name` (string|null)

### `project.exportPrism` — Export .prism File (Experimental)

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Export .prism File (Experimental)...`
- **Summary:** Experimental `.prism` archive export (disabled by default; requires `experimental: true`).
- **Beginner note:** Requires explicit `experimental: true` opt-in because `.prism` is import-only at Beta; use `project.exportPzfx` for standard data interchange. Never changes the active file path or unsaved state.
- **Parameters:** `experimental` (boolean|null), `path` (string|null), `return_bytes` (boolean|null)

### `project.exportPzfx` — Export .pzfx File

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Export .pzfx File...`
- **Summary:** Export data tables and info sheets as a deterministic `.pzfx` XML file.
- **Beginner note:** Saves data tables and info sheets only (analyses, graphs, and layouts stay in your `.curvebook` project; written to match the `.pzfx` structure of real files and not tested in other applications) without changing the active file path or unsaved state.
- **Parameters:** `path` (string|null), `return_bytes` (boolean|null)

### `project.inspect` — Inspect Project

- **Effects:** `IDEMPOTENT`
- **Menu:** `View/Inspect Project`
- **Summary:** Inspect sheet structure, metadata, and upstream dependency edges.
- **Beginner note:** Returns a structured read-only summary of all sheets, constants, checkpoints, and optional single-sheet dependencies.
- **Parameters:** `sheet_id` (string|null)

### `project.instantiateTemplate` — New Project From Template

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/New From Template`
- **Summary:** Create a new project from a `.cbtemplate` archive.
- **Beginner note:** Loads a saved template with its analysis pipelines, graphs, and layouts ready for new experimental data.
- **Parameters:** `bytes_base64` (string|null), `path` (string|null), `title` (string|null)

### `project.listCheckpoints` — List Checkpoints

- **Effects:** `IDEMPOTENT`
- **Menu:** `File/Checkpoints/List Checkpoints`
- **Summary:** List named checkpoints stored in the project.
- **Beginner note:** Lists each checkpoint's ID, name, note, and UTC timestamp without dumping full snapshot payloads.
- **Parameters:** (none)

### `project.merge` — Merge Project

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `File/Merge Project...`
- **Summary:** Merge sheets and referenced image assets from another `.curvebook`, `.curvebook.d/`, or `.cbtemplate` project into the active project.
- **Beginner note:** Brings data tables, info sheets, results, graphs, layouts, and embedded images from another project into this project in one undoable step without changing the open project's file path.
- **Parameters:** `bytes_base64` (string|null), `merged_at` (string|null), `path` (string|null), `source_label` (string|null), `unpacked` (boolean|null)

### `project.new` — New Project

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/New Project`
- **Summary:** Create a fresh empty CurveBook project.
- **Beginner note:** Creates a clean empty project document with an optional title; alias for `file.new`.
- **Parameters:** `title` (string|null)

### `project.open` — Open Project

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Open`
- **Shortcut:** `Cmd+O`
- **Summary:** Load a `.curvebook` archive or unpacked `.curvebook.d/` directory.
- **Beginner note:** Opens a project from a filesystem path or base64 archive payload and replaces the current session document.
- **Parameters:** `bytes_base64` (string|null), `path` (string|null), `unpacked` (boolean|null)

### `project.recalculate` — Recalculate All

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/Recalculate All`
- **Summary:** Refresh live links and recalculate all downstream analyses and graphs.
- **Beginner note:** Re-evaluates linked regions, calculated variables, statistical analyses, curve fits, and derived tables in dependency order.
- **Parameters:** `include_external_links` (boolean|null)

### `project.restoreCheckpoint` — Restore Checkpoint

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/Checkpoints/Restore Checkpoint`
- **Summary:** Restore the document to a previously saved named checkpoint.
- **Beginner note:** Replaces the current sheets with the chosen checkpoint snapshot while keeping existing checkpoints intact.
- **Parameters:** `id` (string|null), `name` (string|null)

### `project.save` — Save Project

- **Effects:** `WRITES_FILESYSTEM`
- **Menu:** `File/Save`
- **Shortcut:** `Cmd+S`
- **Summary:** Save the active project atomically to `.curvebook`, unpacked directory, or bytes.
- **Beginner note:** Serializes the project document using atomic filesystem writes when a path is provided, or returns base64 bytes.
- **Parameters:** `path` (string|null), `return_bytes` (boolean|null), `unpacked` (boolean|null)

### `project.saveTemplate` — Save Template

- **Effects:** `WRITES_FILESYSTEM`
- **Menu:** `File/Save Template`
- **Summary:** Export the project structure as a data-stripped `.cbtemplate` file.
- **Beginner note:** Strips raw data values while preserving table schemas, analyses, graphs, and layouts, then writes atomically to disk or base64.
- **Parameters:** `path` (string|null), `return_bytes` (boolean|null)

## Edit & History (`edit`)

### `edit.pasteImage` — Paste Image

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `Edit/Paste Image`
- **Summary:** Paste or insert a PNG, JPEG, or TIFF raster image onto a graph annotation or multi-panel layout.
- **Beginner note:** Pastes an image from bytes, local file path, or existing project asset onto a graph or layout page.
- **Parameters:** `asset_id` (string|null), `badge_override` (string|null), `border` (any), `bounds_pt` (any), `bytes_base64` (string|null), `crop` (any), `lock_aspect_ratio` (boolean|null), `panel_id` (string|null), `path` (string|null), `rotation_deg` (number|null), `target_sheet_id*` (string)

### `edit.pasteResultTable` — Paste Result Table

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Paste Result Table`
- **Summary:** Paste a live result or data table onto a graph annotation or multi-panel layout.
- **Beginner note:** Places a formatted, auto-updating result table onto the selected graph or layout page.
- **Parameters:** `bounds_pt` (any), `panel_id` (string|null), `selected_cols` (array|null), `selected_rows` (array|null), `source_sheet_id*` (string), `style` (object|null), `tab_id` (string|null), `target_sheet_id*` (string), `transpose` (boolean|null)

### `edit.redo` — Redo

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Redo`
- **Shortcut:** `Cmd+Shift+Z`
- **Summary:** Redo the most recently undone document edit.
- **Beginner note:** Re-applies the project snapshot reverted by the last undo command; succeeds with mutated=false when the redo stack is empty.
- **Parameters:** (none)

### `edit.undo` — Undo

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Undo`
- **Shortcut:** `Cmd+Z`
- **Summary:** Undo the most recent document-mutating command.
- **Beginner note:** Restores the project snapshot captured immediately before the last edit; succeeds with mutated=false when the undo stack is empty.
- **Parameters:** (none)

## Sheets & Pipelines (`sheet`)

### `sheet.clearOrigin` — Clear Origin Badge

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Clear Origin Badge`
- **Summary:** Clear the merge/import origin badge from one sheet or every sheet from the same source file.
- **Beginner note:** Removes the navigator provenance badge from a single sheet or from all sheets merged from the same source file without changing titles or data.
- **Parameters:** `all_from_same_source` (boolean|null), `sheet_id` (string|null), `source_label` (string|null)

### `sheet.clonePipeline` — Clone Pipeline

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Change/Clone Pipeline`
- **Summary:** Clone a table structure and downstream analysis/graph/layout pipeline for a new experimental run.
- **Beginner note:** Creates a new table preserving X coordinates, analyses, graphs, and layouts while clearing Y replicates for new measurements.
- **Parameters:** `clear_column_titles` (boolean|null), `clear_data` (boolean|null), `clear_results` (boolean|null), `clear_row_titles` (boolean|null), `clear_y_values` (boolean|null), `clone_analyses` (boolean|null), `clone_graphs` (boolean|null), `clone_layouts` (boolean|null), `keep_x_values` (boolean|null), `new_sheet_id` (string|null), `override_replicates` (integer|null), `prefix` (string|null), `sheet_id*` (string), `title` (string|null)

### `sheet.createData` — New Data Table

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/New Data Table`
- **Shortcut:** `Cmd+Shift+T`
- **Summary:** Create a typed data table sheet and optional paired info/graph sheets.
- **Beginner note:** Creates one of the eight scientific table archetypes (XY, Column, Grouped, Contingency, Survival, Parts of Whole, Multiple Variables, or Nested).
- **Parameters:** `auto_family` (boolean|null), `auto_graph` (boolean|null), `auto_info` (boolean|null), `censored_code` (integer|null), `cols` (integer|null), `column_factor_title` (string|null), `design_hint` (any), `event_code` (integer|null), `graph_sheet_id` (string|null), `info_sheet_id` (string|null), `kind` (string|null), `replicates` (integer|null), `row_factor_title` (string|null), `rows` (integer|null), `sheet_id` (string|null), `subcolumn_mode` (any), `time_title` (string|null), `title` (string|null), `variables` (array|null), `x_mode` (any), `x_title` (string|null)

### `sheet.delete` — Delete Sheet

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `Edit/Delete Sheet`
- **Summary:** Delete a sheet and optionally cascade to its dependent family.
- **Beginner note:** Removes a sheet from the project, blocking deletion on frozen sheets or when `cascade: false` with active dependents.
- **Parameters:** `cascade` (boolean|null), `sheet_id*` (string)

### `sheet.duplicate` — Duplicate Sheet

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Duplicate Sheet`
- **Summary:** Duplicate a single sheet within its section.
- **Beginner note:** Creates an independent copy of one Data, Info, Result, Graph, or Layout sheet.
- **Parameters:** `frozen` (boolean|null), `new_sheet_id` (string|null), `sheet_id*` (string), `title` (string|null)

### `sheet.duplicateFamily` — Duplicate Family

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Duplicate Family`
- **Summary:** Duplicate a data sheet together with its paired info, analyses, graphs, and layouts.
- **Beginner note:** Clones a complete linked family of sheets so all formulas, analyses, and figures remain wired to the copied table.
- **Parameters:** `clear_data` (boolean|null), `new_sheet_id` (string|null), `prefix` (string|null), `sheet_id*` (string), `title` (string|null)

### `sheet.extractResultTab` — Extract Result Tab to Data Table

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/Extract Result Tab`
- **Summary:** Create a reactive linked DataSheet from a ResultSheet sub-tab (such as PC scores, residuals, or cleaned data).
- **Beginner note:** Copies a computed result table tab into a new linked data table so you can analyze or graph it.
- **Parameters:** `archetype` (any), `auto_graph` (boolean|null), `result_sheet_id*` (string), `sheet_id` (string|null), `tab_id` (string|null), `title` (string|null)

### `sheet.freeze` — Freeze Sheet

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Freeze Sheet`
- **Summary:** Lock a DataSheet against accidental cell, link, or structural edits.
- **Beginner note:** Prevents mutations, link refreshes, renaming, and deletion on a DataSheet while allowing downstream reads and graphs.
- **Parameters:** `sheet_id*` (string)

### `sheet.rename` — Rename Sheet

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Rename Sheet`
- **Summary:** Rename any sheet in Data, Info, Results, Graphs, or Layouts.
- **Beginner note:** Updates the display title of a sheet (refusing frozen DataSheets) and refreshes dependent labels.
- **Parameters:** `sheet_id*` (string), `title*` (string)

### `sheet.reorder` — Reorder Sheet

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Reorder Sheet`
- **Summary:** Reorder sheets within a project navigator folder.
- **Beginner note:** Moves a sheet to a new index by (`sheet_id`, `index`), by (`folder`, `from_index`, `to_index`), or by explicit `order` array.
- **Parameters:** `folder` (string|null), `from_index` (integer|null), `index` (integer|null), `order` (array|null), `section` (string|null), `sheet_id` (string|null), `to_index` (integer|null)

### `sheet.select` — Select Sheet

- **Effects:** `CHANGES_UI_STATE | IDEMPOTENT`
- **Menu:** `View/Select Sheet`
- **Summary:** Select the active sheet in the navigator or family pane.
- **Beginner note:** Focuses a sheet in the navigator (`mode: "navigator"`) or sub-sheet family view (`mode: "family"`).
- **Parameters:** `mode` (string|null), `sheet_id*` (string)

### `sheet.unfreeze` — Unfreeze Sheet

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Unfreeze Sheet`
- **Summary:** Unlock a frozen DataSheet and recalculate linked dependents.
- **Beginner note:** Restores mutability to a frozen DataSheet and triggers immediate link/DAG recalculation.
- **Parameters:** `sheet_id*` (string)

## Data Tables (`table`)

### `table.clearCells` — Clear Cells

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA | IDEMPOTENT`
- **Menu:** `Edit/Clear Cells`
- **Summary:** Clear single cells, lists, ranges, or an entire table, optionally breaking live links.
- **Beginner note:** Erase values from selected cells or clear the entire data table.
- **Parameters:** `all` (boolean|null), `cells` (array|null), `clear_x` (boolean|null), `col` (integer|null), `col_end` (integer|null), `col_start` (integer|null), `coord` (any), `range` (any), `region_id` (string|null), `row` (integer|null), `row_end` (integer|null), `row_start` (integer|null), `sheet_id*` (string), `subcol` (integer|null), `subcol_end` (integer|null), `subcol_start` (integer|null), `unlink` (boolean|null)

### `table.convertArchetype` — Convert Table Format

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA | IDEMPOTENT`
- **Menu:** `Change/Convert Table Structure`
- **Summary:** Preview or convert a data table between Column, Grouped, XY, and MultipleVariables archetypes.
- **Beginner note:** Reshape a data table into another structure, previewing any lossy changes before applying.
- **Parameters:** `allow_lossy` (boolean|null), `preview_only` (boolean|null), `sheet_id*` (string), `target_kind*` (string)

### `table.excludeCells` — Exclude Cells

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Exclude Values`
- **Shortcut:** `Cmd+E`
- **Summary:** Toggle the exclusion mask on single cells, cell lists, or ranges (including X/time cells).
- **Beginner note:** Exclude or include selected values so analyses ignore them without deleting data.
- **Parameters:** `cells` (array|null), `col` (integer|null), `col_end` (integer|null), `col_start` (integer|null), `coord` (any), `excluded` (boolean|null), `range` (any), `row` (integer|null), `row_end` (integer|null), `row_start` (integer|null), `sheet_id*` (string), `subcol` (integer|null), `subcol_end` (integer|null), `subcol_start` (integer|null), `target` (string|null)

### `table.excludeCellsWithReason` — Exclude Cells With Reason

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Change/Exclude Values With Reason`
- **Summary:** Exclude cells and attach a structured audit reason tag, note, and UTC timestamp.
- **Beginner note:** Exclude values while recording a reason tag and audit timestamp for review.
- **Parameters:** `col` (integer|null), `coord` (any), `coords` (array|null), `note` (string|null), `range` (any), `reason_tag*` (string), `row` (integer|null), `sheet_id*` (string), `subcol` (integer|null), `timestamp_utc` (string|null)

### `table.exclusionAuditReport` — Exclusion Audit Report

- **Effects:** `IDEMPOTENT`
- **Menu:** `Data/Exclusion Audit Report`
- **Summary:** Generate a read-only audit report of excluded cells and their reason annotations.
- **Beginner note:** Inspect all excluded values and audit reasons across one sheet or the entire project.
- **Parameters:** `sheet_id` (string|null)

### `table.exportCsv` — Export Table CSV / TSV

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Table CSV`
- **Summary:** Export a data or result sheet to CSV/TSV text and optionally write atomically to disk.
- **Beginner note:** Save a data or result table as CSV or TSV without changing the project.
- **Parameters:** `decimal_separator` (string|null), `delimiter` (string|null), `path` (string|null), `sheet_id*` (string)

### `table.importCsv` — Import CSV / TSV

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/CSV or TSV`
- **Summary:** Import CSV or TSV data from a file path or inline text into a new or existing data table.
- **Beginner note:** Create a new data table or populate an existing one from a CSV or TSV file.
- **Parameters:** `anchor` (any), `archetype` (string|null), `csv` (string|null), `decimal_separator` (string|null), `delimiter` (string|null), `extract_preamble_info` (boolean|null), `filter` (any), `has_header` (boolean|null), `has_row_titles` (boolean|null), `header_rows` (integer|null), `link` (boolean|null), `mode` (string|null), `normalize_decimal` (boolean|null), `path` (string|null), `propagate_title_header` (boolean|null), `replace` (boolean|null), `replicates` (integer|null), `sheet_id` (string|null), `skip_contiguous_blanks` (boolean|null), `split_by_column` (integer|null), `start_col` (integer|null), `start_row` (integer|null), `start_subcol` (integer|null), `subcolumn_mode` (any), `title` (string|null), `transpose` (boolean|null)

### `table.importPlateMap` — Import Plate Map

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `File/Import/Plate Map`
- **Summary:** Map 96-well or 384-well microplate readings and well roles into a structured data sheet.
- **Beginner note:** Import 96-well or 384-well plate measurements using a plate template or role map.
- **Parameters:** `blank_labels` (array|null), `geometry` (any), `label_grid` (array|null), `layout_sheet_id` (string|null), `positive_control_labels` (array|null), `roles` (array|null), `sheet_id` (string|null), `source` (any), `subtract_blank_mean` (boolean|null), `target_kind` (string|null), `template` (any), `template_name` (string|null), `title` (string|null), `values` (any), `x_grid` (array|null), `x_grid_sheet_id` (string|null), `x_title` (string|null)

### `table.importPrism` — Import .prism files

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/.prism files`
- **Summary:** Import .prism files from disk, base64, or raw byte array.
- **Beginner note:** Open or merge .prism files into the project.
- **Parameters:** `bytes` (array|null), `bytes_base64` (string|null), `merge` (boolean|null), `merged_at` (string|null), `path` (string|null)

### `table.importPzfx` — Import .pzfx files

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/.pzfx files`
- **Summary:** Import .pzfx files from disk, inline XML string, or base64 payload.
- **Beginner note:** Open or merge .pzfx files into the project.
- **Parameters:** `bytes_base64` (string|null), `merge` (boolean|null), `merged_at` (string|null), `path` (string|null), `xml` (string|null)

### `table.importTidyLong` — Import Tidy Long Table

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `File/Import/Tidy Long Table`
- **Summary:** Pivot a long-format table (rows/headers or CSV/TSV file) into a Column, Grouped, XY, or MultipleVariables sheet.
- **Beginner note:** Reshape long/tidy tabular records into a grouped or XY data table using column mappings.
- **Parameters:** `csv` (string|null), `decimal_sep` (string|null), `delimiter` (string|null), `excluded_col` (any), `group_col` (any), `headers` (array|null), `mapping` (any), `normalize_decimal` (boolean|null), `path` (string|null), `replicate_col` (any), `rows` (array|null), `sheet_id` (string|null), `target_kind` (string|null), `title` (string|null), `value_col` (any), `x_col` (any)

### `table.importXlsx` — Import XLSX Workbook

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/XLSX Workbook`
- **Summary:** Import one or more worksheets (or list available sheets) from an .xlsx workbook file, base64, or byte array.
- **Beginner note:** Open or merge worksheets from a spreadsheet workbook (.xlsx) into the project.
- **Parameters:** `archetype` (string|null), `bytes` (array|null), `bytes_base64` (string|null), `cell_range` (string|null), `extract_preamble_info` (boolean|null), `fill_merged_cells` (boolean|null), `has_header` (boolean|null), `has_row_titles` (boolean|null), `header_rows` (integer|null), `include_hidden` (boolean|null), `list_sheets` (boolean|null), `merge` (boolean|null), `merged_at` (string|null), `path` (string|null), `row_match` (any), `sheet_indices` (array|null), `sheets` (array|null), `skip_rows` (integer|null), `split_by_column` (integer|null), `subcolumn_mode` (any), `title` (string|null)

### `table.paste` — Paste Special

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Paste Special`
- **Shortcut:** `Cmd+V`
- **Summary:** Paste delimited clipboard text or a 2D cell grid into a table with placement, transpose, and filter options.
- **Beginner note:** Paste tabular data into a table starting at an anchor cell, optionally transposing or filtering.
- **Parameters:** `anchor` (any), `decimal_separator` (string|null), `delimiter` (string|null), `filter` (any), `mode` (string|null), `normalize_decimal` (boolean|null), `propagate_title_header` (boolean|null), `rows` (array|null), `sheet_id*` (string), `skip_contiguous_blanks` (boolean|null), `start_col` (integer|null), `start_row` (integer|null), `start_subcol` (integer|null), `text` (string|null), `transpose` (boolean|null)

### `table.pasteLink` — Paste Link

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `Edit/Paste Link`
- **Summary:** Attach a live linked region from an internal sheet or external delimited file and sync immediately.
- **Beginner note:** Link a range from another sheet or CSV/TSV file so updates flow through automatically.
- **Parameters:** `anchor` (any), `baseline` (any), `decimal_separator` (string|null), `delimiter` (string|null), `exclude_row_predicate` (string|null), `filter` (any), `link_id` (string|null), `link_spec` (object|null), `mode` (string|null), `normalize_decimal` (boolean|null), `propagate_title_header` (boolean|null), `provenance` (string|null), `skip_contiguous_blanks` (boolean|null), `source_kind` (string|null), `source_path` (string|null), `source_range` (any), `source_sheet_id` (string|null), `start_col` (integer|null), `start_row` (integer|null), `start_subcol` (integer|null), `tab_id` (string|null), `target_sheet_id*` (string), `transpose` (boolean|null)

### `table.randomSampleRows` — Random Sample Rows

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Data/Random Sample Rows`
- **Summary:** Draw a reproducible Fisher-Yates random sample of rows (optional stratification) into a new data sheet.
- **Beginner note:** Creates a new table containing a seeded simple or stratified random subset of rows from a source sheet.
- **Parameters:** `info_sheet_id` (string|null), `proportional_allocation` (boolean|null), `sample_size*` (integer), `seed` (integer|null), `sheet_id` (string|null), `source_sheet_id*` (string), `stratify_column` (integer|null), `title` (string|null)

### `table.randomizeGroups` — Randomize Subjects to Groups

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Data/Randomize Groups`
- **Summary:** Deterministically allocate subjects across treatment groups (complete-balanced or permuted-block) and upsert ARRIVE constants on an InfoSheet.
- **Beginner note:** Randomizes subjects into balanced or blocked treatment arms and stores reproducibility metadata on an Info sheet.
- **Parameters:** `block_sizes` (any), `groups*` (array), `info_sheet_id` (string|null), `scheme` (any), `seed` (integer|null), `sheet_id` (string|null), `subject_count` (integer|null), `subject_ids` (any), `title` (string|null)

### `table.recodeLevels` — Recode Categorical Levels

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Data/Recode Levels`
- **Summary:** Map and rewrite categorical values in a MultipleVariables column.
- **Beginner note:** Rename or merge category labels within a MultipleVariables column.
- **Parameters:** `col` (any), `mappings*` (any), `sheet_id*` (string), `title` (string|null)

### `table.setCalculatedVariable` — Add / Update Calculated Variable

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Calculated Variable`
- **Summary:** Create or update a formula-driven variable column in a MultipleVariables table and evaluate it.
- **Beginner note:** Add a computed column whose values update automatically from other variables via a formula.
- **Parameters:** `col` (any), `formula*` (string), `sheet_id*` (string), `title` (string|null)

### `table.setCategoricalOrder` — Set Categorical Order

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Categorical Order`
- **Summary:** Configure display/reference ordering of categorical levels in a MultipleVariables column.
- **Beginner note:** Choose how categories are ordered (explicit list, alphabetical, entry order, or frequency).
- **Parameters:** `col` (any), `levels` (array|null), `order` (any), `order_spec` (object|null), `reference_level` (string|null), `reversed` (boolean|null), `sheet_id*` (string), `title` (string|null)

### `table.setCell` — Set Cell Value

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Cell Value`
- **Summary:** Set or update a single data cell value and optional exclusion flag, then recalculate downstream sheets.
- **Beginner note:** Enter or edit one cell in a data table. Connected analyses and graphs refresh automatically.
- **Parameters:** `col` (integer|null), `coord` (any), `decimal_separator` (string|null), `excluded` (boolean|null), `parse_asterisk_exclusion` (boolean|null), `row` (integer|null), `sheet_id*` (string), `subcol` (integer|null), `value` (any)

### `table.setCells` — Set Multiple Cells

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Multiple Cells`
- **Summary:** Apply a batch of cell edits or a 2D row/column block to a data table, then recalculate.
- **Beginner note:** Update multiple cells in a data table at once from a list of edits or a 2D grid.
- **Parameters:** `edits` (array|null), `rows` (array|null), `sheet_id*` (string), `start_col` (integer|null), `start_row` (integer|null), `start_subcol` (integer|null)

### `table.setGroupTitles` — Set Group Column Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Column Titles`
- **Summary:** Set group/dataset titles, X/time title, factor titles, or row titles on a data table.
- **Beginner note:** Rename columns, X/time headers, or factor titles in a data table.
- **Parameters:** `col` (integer|null), `column_factor_title` (string|null), `row_factor_title` (string|null), `row_titles` (array|null), `sheet_id*` (string), `start_col` (integer|null), `title` (string|null), `titles` (array|null), `x_title` (string|null)

### `table.setRowTitles` — Set Row Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Row Titles`
- **Summary:** Set or clear one or more row titles (or PartsOfWhole slice labels) in a data table.
- **Beginner note:** Label rows along the left edge of a data table.
- **Parameters:** `row` (integer|null), `sheet_id*` (string), `start_row` (integer|null), `title` (string|null), `titles` (array|null)

### `table.setSubcolumnTitles` — Set Subcolumn Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Subcolumn Titles`
- **Summary:** Set explicit subcolumn titles on a Nested data table across one or all groups.
- **Beginner note:** Rename nested replicate subcolumns for a specific group or all groups.
- **Parameters:** `col` (integer|null), `sheet_id*` (string), `start_subcol` (integer|null), `subcol` (integer|null), `title` (string|null), `titles` (array|null)

### `table.setTitles` — Set Column Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Column Titles`
- **Summary:** Set dataset column titles, X/time title, factor titles, or row titles on a data table.
- **Beginner note:** Rename column headers, X/time title, or factor titles in a data table.
- **Parameters:** `col` (integer|null), `column_factor_title` (string|null), `row_factor_title` (string|null), `row_titles` (array|null), `sheet_id*` (string), `start_col` (integer|null), `title` (string|null), `titles` (array|null), `x_title` (string|null)

### `table.setVariableRole` — Set Variable Role

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Variable Properties`
- **Summary:** Lock or re-infer the analytical role of a column in a MultipleVariables table.
- **Beginner note:** Mark a variable column as continuous, categorical, label, calculated, or auto-inferred.
- **Parameters:** `col` (any), `role*` (string), `sheet_id*` (string), `title` (string|null)

### `table.setXCells` — Set X / Time Column

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set X Values`
- **Summary:** Set one or more X or elapsed-time column cells on an XY or Survival table.
- **Beginner note:** Fill or edit values in the X (or Survival time) column.
- **Parameters:** `decimal_separator` (string|null), `edits` (array|null), `excluded` (boolean|null), `parse_asterisk_exclusion` (boolean|null), `row` (integer|null), `sheet_id*` (string), `start_row` (integer|null), `subcol` (integer|null), `value` (any), `values` (array|null)

### `table.simulateDistributions` — Simulate Distributions

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/Simulate Distributions`
- **Summary:** Generate a Column or MultipleVariables table with columns sampled from Uniform, Gaussian, LogNormal, Binomial, Poisson, or Exponential distributions.
- **Beginner note:** Creates a new table of seeded random variates drawn from parametric statistical distributions.
- **Parameters:** `columns*` (array), `n_rows*` (integer), `seed` (integer|null), `sheet_id` (string|null), `title` (string|null), `use_multiple_variables` (boolean|null)

### `table.simulateXy` — Simulate XY Data

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/Simulate Data`
- **Summary:** Generate a synthetic XY data sheet from a built-in equation, parameter map, and noise model.
- **Beginner note:** Create synthetic XY curves with optional Gaussian, relative CV, or Poisson noise.
- **Parameters:** `auto_info` (boolean|null), `cv_percent` (number|null), `equation_id` (string|null), `graph_sheet_id` (string|null), `noise_kind` (string|null), `noise_sd` (number|null), `parameters` (any), `replicates` (integer|null), `row_count` (integer|null), `seed` (integer|null), `sheet_id` (string|null), `title` (string|null), `x_max` (number|null), `x_min` (number|null)

### `table.unlinkCalculatedVariable` — Unlink Calculated Variable

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Unlink Calculated Variable`
- **Summary:** Freeze evaluated values of a calculated variable into an editable entered column and remove its live formula.
- **Beginner note:** Converts a formula-driven column into static entered values while preserving its cells, exclusions, and role.
- **Parameters:** `col` (any), `sheet_id*` (string), `title` (string|null)

## Info & Constants (`info`)

### `info.removeConstant` — Remove Info Constant

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Info/Remove Constant`
- **Summary:** Remove a named constant from an InfoSheet and recalculate dependents.
- **Beginner note:** Deletes a constant from an InfoSheet and re-evaluates any sheets that depended on it.
- **Parameters:** `if_exists` (boolean|null), `name*` (string), `sheet_id*` (string)

### `info.setConstant` — Set Info Constant

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Info/Set Constant`
- **Summary:** Create or update a named InfoSheet constant and recalculate dependents.
- **Beginner note:** Sets a hookable constant (number, date, or text with optional units) used by formulas, fits, and graph titles.
- **Parameters:** `kind` (string|null), `name*` (string), `sheet_id*` (string), `units` (string|null), `value*` (any)

### `info.setNote` — Set Info Note

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Info/Edit Note`
- **Summary:** Set or append markdown notes on an InfoSheet.
- **Beginner note:** Updates experiment notes on an InfoSheet (or the InfoSheet linked to a given DataSheet ID).
- **Parameters:** `append` (boolean|null), `sheet_id*` (string), `text*` (string)

## Analysis & Reports (`analysis`)

### `analysis.add` — Add Analysis

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Add Analysis`
- **Summary:** Attach and compute a new analysis of any domain (transform, stats, or regression) for a data sheet.
- **Beginner note:** Adds a new live analysis sheet linked to an input table and computes its initial results.
- **Parameters:** `cluster_direction` (any), `comparison` (any), `comparison_method` (any), `comparison_question` (any), `confidence_percent` (any), `consensus_optimal_k` (any), `create_summary_graph` (any), `custom_dsl` (any), `deming_error` (any), `df_method` (any), `downstream_result_sheet_id` (any), `emit_extra_gof` (any), `emit_qq_tabs` (any), `equation_id` (any), `exclude_flagged_fits` (any), `extend_line_to_x_intercept` (any), `fit_flag_rules` (any), `fixed_point` (any), `flag_r_squared_below` (any), `hit_rules` (any), `identifiability_mode` (any), `identify_unstable_parameters` (any), `iterations` (any), `kind` (any), `kmeans_algorithm` (any), `lognormal_mode` (any), `method` (any), `multiplicity` (any), `normality_only` (any), `output_significant_digits` (any), `overwrite` (any), `p_value_format` (any), `parameter_rules` (any), `plot_distribution` (any), `plot_segments` (any), `plot_x_max` (any), `plot_x_min` (any), `prefer_well_defined_fit` (any), `relational_constraints` (any), `reml_zero_variance_policy` (any), `repeated_measures` (any), `residual_covariance` (any), `retention_rule` (any), `save_as_default` (any), `seed` (any), `selected_data_sets` (any), `sheet_id` (any), `show_standard_errors` (any), `significant_digits` (any), `source_sheet_id` (any), `spec` (any), `sphericity_correction` (any), `standard_addition` (any), `summary_parameter` (any), `survival_method` (any), `title` (any), `tracked_scalars` (any), `visible_metrics` (any), `weighting` (any), `x_range_filter` (any), `y_range_filter` (any)

### `analysis.exportCsv` — Export Result Sheet CSV

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Results CSV`
- **Summary:** Export a specific tab or tidy combined CSV representation of a result sheet and optionally save it atomically.
- **Beginner note:** Exports result tables to CSV/TSV text with configurable delimiter and decimal separator.
- **Parameters:** `decimal_separator` (any), `delimiter` (any), `path` (any), `sheet_id*` (string), `tab_id` (any), `tab_index` (any)

### `analysis.exportJson` — Export Result Sheet JSON

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Results JSON`
- **Summary:** Serialize a result sheet (tables, scalars, parameters, checklist) to JSON and optionally save it atomically.
- **Beginner note:** Exports an analysis result sheet as structured JSON text or saves it to a `.json` file.
- **Parameters:** `path` (any), `pretty` (any), `sheet_id*` (string)

### `analysis.retargetData` — Change Data Analyzed

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Data Analyzed`
- **Summary:** Rebind an existing analysis result sheet to a different compatible data sheet or subset of datasets.
- **Beginner note:** Points an existing analysis at a different data table and recalculates it immediately.
- **Parameters:** `clear_selected_data_sets` (any), `selected_data_sets` (any), `sheet_id*` (string), `source_sheet_id*` (string)

### `analysis.run` — New / Run Analysis

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Analyze Data`
- **Summary:** Create and run a new analysis sheet, recalculate an existing result sheet by ID, or recalculate the project when empty.
- **Beginner note:** Runs a new analysis when source and kind are provided, or refreshes an existing analysis sheet.
- **Parameters:** `cluster_direction` (any), `comparison` (any), `comparison_method` (any), `comparison_question` (any), `confidence_percent` (any), `consensus_optimal_k` (any), `create_summary_graph` (any), `custom_dsl` (any), `deming_error` (any), `df_method` (any), `downstream_result_sheet_id` (any), `emit_extra_gof` (any), `emit_qq_tabs` (any), `equation_id` (any), `exclude_flagged_fits` (any), `extend_line_to_x_intercept` (any), `fit_flag_rules` (any), `fixed_point` (any), `flag_r_squared_below` (any), `hit_rules` (any), `identifiability_mode` (any), `identify_unstable_parameters` (any), `iterations` (any), `kind` (any), `kmeans_algorithm` (any), `lognormal_mode` (any), `method` (any), `multiplicity` (any), `normality_only` (any), `output_significant_digits` (any), `overwrite` (any), `p_value_format` (any), `parameter_rules` (any), `plot_distribution` (any), `plot_segments` (any), `plot_x_max` (any), `plot_x_min` (any), `prefer_well_defined_fit` (any), `relational_constraints` (any), `reml_zero_variance_policy` (any), `repeated_measures` (any), `residual_covariance` (any), `retention_rule` (any), `save_as_default` (any), `seed` (any), `selected_data_sets` (any), `sheet_id` (any), `show_standard_errors` (any), `significant_digits` (any), `source_sheet_id` (any), `spec` (any), `sphericity_correction` (any), `standard_addition` (any), `summary_parameter` (any), `survival_method` (any), `title` (any), `tracked_scalars` (any), `visible_metrics` (any), `weighting` (any), `x_range_filter` (any), `y_range_filter` (any)

### `analysis.updateParams` — Change Analysis Parameters

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Analysis Parameters`
- **Summary:** Update analysis kind, confidence level, dataset selection, P-value formatting, or parameter rules and recalculate.
- **Beginner note:** Modifies settings on an existing result sheet without recreating it and refreshes downstream sheets.
- **Parameters:** `confidence_percent` (any), `kind` (any), `p_value_format` (any), `params` (any), `selected_data_sets` (any), `sheet_id*` (string), `title` (any)

### `analyze.run` — Run Analyses

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Analyze/Run Analyses`
- **Shortcut:** `Cmd+Enter`
- **Summary:** Recalculate all dirty or upstream-dependent analysis result sheets across the project DAG.
- **Beginner note:** Refreshes every analysis sheet so all tables, stats, and curve fits match the latest data.
- **Parameters:** (none)

### `fit.cloneBuiltinEquation` — Clone Built-in Equation

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/User Equations/Clone Built-in...`
- **Summary:** Clone a built-in equation into an editable DSL template.
- **Beginner note:** Generates editable DSL source (with constraints and transforms) from any built-in curve-fitting equation.
- **Parameters:** `builtin_id*` (string), `dir` (any), `new_id` (any), `new_name` (any), `overwrite` (any), `save` (any)

### `fit.deleteUserEquation` — Delete User Equation

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/User Equations/Delete`
- **Summary:** Delete a saved custom equation by ID.
- **Beginner note:** Deletes a custom equation from the local user equation library.
- **Parameters:** `dir` (any), `id*` (string)

### `fit.getUserEquation` — Get User Equation

- **Effects:** `READS_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/User Equations/Get`
- **Summary:** Get a saved custom DSL equation by ID.
- **Beginner note:** Fetches the DSL source, parameter names, and metadata for one saved user equation.
- **Parameters:** `dir` (any), `id*` (string)

### `fit.listUserEquations` — List User Equations

- **Effects:** `READS_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/User Equations/List`
- **Summary:** List saved custom DSL equations.
- **Beginner note:** Lists all custom curve-fitting equations saved in the local user equation library.
- **Parameters:** `dir` (any)

### `fit.run` — Run Curve Fit / Regression

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Fit Curve`
- **Summary:** Create and execute linear, Deming, built-in nonlinear, global shared-parameter, or custom DSL regression.
- **Beginner note:** Fits a linear or nonlinear dose-response/kinetic curve with confidence bands and model comparisons.
- **Parameters:** `cluster_direction` (any), `comparison` (any), `comparison_method` (any), `comparison_question` (any), `confidence_percent` (any), `consensus_optimal_k` (any), `create_summary_graph` (any), `custom_dsl` (any), `deming_error` (any), `df_method` (any), `downstream_result_sheet_id` (any), `emit_extra_gof` (any), `emit_qq_tabs` (any), `equation_id` (any), `exclude_flagged_fits` (any), `extend_line_to_x_intercept` (any), `fit_flag_rules` (any), `fixed_point` (any), `flag_r_squared_below` (any), `hit_rules` (any), `identifiability_mode` (any), `identify_unstable_parameters` (any), `iterations` (any), `kind` (any), `kmeans_algorithm` (any), `lognormal_mode` (any), `method` (any), `multiplicity` (any), `normality_only` (any), `output_significant_digits` (any), `overwrite` (any), `p_value_format` (any), `parameter_rules` (any), `plot_distribution` (any), `plot_segments` (any), `plot_x_max` (any), `plot_x_min` (any), `prefer_well_defined_fit` (any), `relational_constraints` (any), `reml_zero_variance_policy` (any), `repeated_measures` (any), `residual_covariance` (any), `retention_rule` (any), `save_as_default` (any), `seed` (any), `selected_data_sets` (any), `sheet_id` (any), `show_standard_errors` (any), `significant_digits` (any), `source_sheet_id` (any), `spec` (any), `sphericity_correction` (any), `standard_addition` (any), `summary_parameter` (any), `survival_method` (any), `title` (any), `tracked_scalars` (any), `visible_metrics` (any), `weighting` (any), `x_range_filter` (any), `y_range_filter` (any)

### `fit.saveUserEquation` — Save User Equation

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/User Equations/Save...`
- **Summary:** Validate and atomically save a custom DSL equation.
- **Beginner note:** Compiles and saves a custom curve-fitting equation for reuse across projects.
- **Parameters:** `category` (any), `cloned_from_builtin` (any), `description` (any), `dir` (any), `dsl_source*` (string), `id*` (string), `name*` (string), `overwrite` (any)

### `result.generateReport` — Generate Methods Report

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Methods Report`
- **Summary:** Generate a publication-ready statistical methods prose report in `plain`, `markdown`, or `latex`, optionally saving to disk.
- **Beginner note:** Creates a narrative methods paragraph describing the statistical test, sample sizes, and options used.
- **Parameters:** `format` (any), `path` (any), `result_sheet_id*` (string), `scope` (any)

### `stats.pvalueStack` — Analyze a Stack of P Values

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Column Analyses/Analyze a Stack of P Values`
- **Summary:** Adjust a column stack of raw P values across 7 FWER/FDR multiplicity procedures and report discoveries.
- **Beginner note:** Adjusts a column of P values using Bonferroni, Holm, Šídák, Holm-Šídák, Benjamini-Hochberg, BKY two-stage, or Benjamini-Yekutieli.
- **Parameters:** `column` (any), `method` (any), `overwrite` (any), `p_value_format` (any), `selected_data_sets` (any), `sheet_id` (any), `source_sheet_id*` (string), `threshold` (any), `title` (any)

### `stats.run` — Run Statistical Test

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Statistical Test`
- **Summary:** Create and execute a hypothesis test, ANOVA, contingency, survival, ROC, GLM, or multivariate analysis.
- **Beginner note:** Runs a statistical test on a compatible data sheet and stores tables, P values, and assumption checks.
- **Parameters:** `cluster_direction` (any), `confidence_percent` (any), `consensus_optimal_k` (any), `df_method` (any), `downstream_result_sheet_id` (any), `emit_qq_tabs` (any), `exclude_flagged_fits` (any), `hit_rules` (any), `iterations` (any), `kind` (any), `kmeans_algorithm` (any), `lognormal_mode` (any), `multiplicity` (any), `normality_only` (any), `overwrite` (any), `p_value_format` (any), `reml_zero_variance_policy` (any), `repeated_measures` (any), `residual_covariance` (any), `retention_rule` (any), `save_as_default` (any), `seed` (any), `selected_data_sets` (any), `sheet_id` (any), `significant_digits` (any), `source_sheet_id` (any), `spec` (any), `sphericity_correction` (any), `survival_method` (any), `title` (any), `tracked_scalars` (any), `visible_metrics` (any)

### `transform.run` — Run Transformation

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Transform`
- **Summary:** Create and execute a mathematical, normalization, baseline, transpose, prune, select, or simulation transformation sheet.
- **Beginner note:** Runs a reproducible data transformation pipeline on a data sheet and writes the output to a linked result sheet.
- **Parameters:** `ci_method` (any), `confidence_percent` (any), `create_graph` (any), `denominator` (any), `graph_sheet_id` (any), `kind` (any), `operation` (any), `output_percent` (any), `overwrite` (any), `p_value_format` (any), `selected_data_sets` (any), `sheet_id` (any), `simulation_mode` (any), `source_sheet_id` (any), `spec` (any), `summary` (any), `title` (any), `upper_percentile` (any)

## Graphs & Styling (`graph`)

### `graph.addAnnotation` — Add Graph Annotation

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Draw/Annotation`
- **Summary:** Add a text box, line/arrow, shape, live embedded result table, or live fitted model equation annotation to a graph.
- **Beginner note:** Places labels, arrows, geometric callouts, embedded result summaries, or model equations onto a graph.
- **Parameters:** `annotation` (any), `bordered` (any), `bounds_pt` (any), `data_set_index` (any), `font_size_pt` (any), `kind` (any), `mode` (any), `result_sheet_id` (any), `sheet_id*` (string), `text` (any)

### `graph.addComparisonBrackets` — Add Pairwise Significance Brackets

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Draw/Pairwise Comparison Brackets`
- **Summary:** Add statistical comparison brackets (stars or numeric P/q values) driven by a result sheet's pairwise comparisons.
- **Beginner note:** Draws significance brackets above compared groups using P values from your statistical test.
- **Parameters:** `bracket_step_pt` (any), `clear_existing` (any), `font_size_pt` (any), `group_a` (any), `group_b` (any), `hide_if_non_significant` (any), `label_mode` (any), `result_sheet_id*` (string), `sheet_id*` (string), `significant_only` (any), `stroke_width_pt` (any), `style` (any)

### `graph.applyCanvasPreset` — Apply Graph Canvas Preset

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Canvas Size Preset`
- **Summary:** Resize a graph canvas to a journal column, slide, or poster preset (`KeepTypographyPt` or `ProportionalScale`).
- **Beginner note:** Resizes a graph to standard journal figure widths and runs a publication preflight check.
- **Parameters:** `preset*` (string), `resize_mode` (any), `sheet_id*` (string)

### `graph.applyStyleFrom` — Apply Graph Style From

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Apply Style From`
- **Summary:** Copy visual styling (palette, frame, axis styling, legend, and series appearances) from a donor graph.
- **Beginner note:** Copies the visual style of another graph sheet onto this graph without changing its data.
- **Parameters:** `clone_error_bars_and_legend` (any), `clone_frame_and_axes` (any), `clone_palette` (any), `donor_sheet_id*` (string), `sheet_id` (any), `target_sheet_ids` (any)

### `graph.create` — New Graph

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/New Graph`
- **Summary:** Create a new graph sheet bound to an input data sheet with optional kind, title, and initialization state.
- **Beginner note:** Creates a new graph linked to one of your data tables.
- **Parameters:** `data_sheet_id*` (string), `initialized` (any), `kind` (any), `one_per_data_set` (any), `sheet_id` (any), `title` (any), `y_axis_title` (any)

### `graph.formatSelectedPoints` — Format Selected Points

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Selected Points`
- **Summary:** Apply, merge, replace, or clear per-point visual overrides across one or more selected graph points.
- **Beginner note:** Highlights or hides specific data points (changing color, symbol, size, border, or visibility) without modifying the whole series.
- **Parameters:** `border_width_pt` (any), `clear` (boolean), `color` (any), `fill_color` (any), `fill_mode` (any), `hidden` (boolean), `mode` (string), `overrides` (array), `points` (array), `sheet_id*` (string), `size_pt` (any), `symbol` (any)

### `graph.initialize` — Initialize Graph

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Graph/Initialize`
- **Summary:** Initialize an uninitialized graph sheet, applying its graph kind and default series styling.
- **Beginner note:** Chooses an initial plot type for a newly created graph sheet.
- **Parameters:** `kind` (any), `sheet_id*` (string), `title` (any)

### `graph.insertImage` — Insert Image on Graph

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `Insert/Image on Graph`
- **Summary:** Embed a PNG, JPEG, or TIFF raster image annotation onto a graph sheet.
- **Beginner note:** Inserts a microscopy micrograph, Western blot, or schematic photo onto a graph.
- **Parameters:** `asset_id` (any), `border` (any), `bounds_pt` (any), `bytes_base64` (any), `crop` (any), `lock_aspect_ratio` (any), `path` (any), `rotation_deg` (any), `sheet_id*` (string)

### `graph.preflightCheck` — Preflight Check Figure

- **Effects:** `IDEMPOTENT`
- **Menu:** `Analyze/Preflight Check`
- **Summary:** Compile a graph or layout sheet and check typography, hairlines, clipping, and overlap readiness.
- **Beginner note:** Audits a graph or layout for publication print issues like tiny fonts or clipped labels.
- **Parameters:** `sheet_id*` (string)

### `graph.setAdditionalTicks` — Set Additional Axis Ticks

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Additional Ticks`
- **Summary:** Set custom callout ticks and optional gridlines along the `x`, `y`, or `y2` axis.
- **Beginner note:** Places custom reference ticks and gridlines at specific axis coordinates.
- **Parameters:** `axis*` (string), `sheet_id*` (string), `ticks*` (array)

### `graph.setAxes` — Format Axes

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Axes`
- **Summary:** Configure X, Y, and right-hand Y2 axes including log/probit/logit scales, segmented breaks, thresholds, and frame presets.
- **Beginner note:** Adjusts axis limits, log scales, titles, broken-axis gaps, and frame styling.
- **Parameters:** `axis` (any), `frame_preset` (any), `sheet_id*` (string), `x_axis` (any), `y2_axis` (any), `y_axis` (any)

### `graph.setCompactLetterDisplay` — Compact Letter Display

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Draw/Compact Letter Display`
- **Summary:** Configure or toggle Piepho (2004) compact letter display significance grouping letters and pairwise display mode on a graph.
- **Beginner note:** Places shared grouping letters (a, b, ab) above graph groups to summarize all-pairwise statistical comparisons.
- **Parameters:** `alpha_override` (any), `color` (any), `enabled` (any), `font_size_pt` (any), `offset_pt` (any), `ordering` (any), `pairwise_display_mode` (any), `result_sheet_id` (any), `sheet_id*` (string), `uppercase` (any)

### `graph.setKind` — Change Graph Type

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Graph Type`
- **Summary:** Switch a graph's visualization type (XY, column, grouped, survival, multivariate, diagnostic) with optional style reset.
- **Beginner note:** Changes the chart type of an existing graph without recreating the sheet.
- **Parameters:** `kind*` (any), `option_id` (any), `reset_series_style` (any), `sheet_id*` (string)

### `graph.setRegressionOverlay` — Overlay Regression Curve

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Graph/Regression Overlay`
- **Summary:** Bind or clear a fitted curve overlay with optional confidence/prediction bands on one or all graph series.
- **Beginner note:** Overlays a curve fit line and confidence ribbons from a result sheet onto your graph.
- **Parameters:** `all_series` (any), `clear` (any), `data_set_index` (any), `result_sheet_id` (any), `sample_segments` (any), `series_index` (any), `shaded_ribbon` (any), `sheet_id*` (string), `show_confidence_band` (any), `show_prediction_band` (any)

### `graph.setSeries` — Format Data Series

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Data on Graph`
- **Summary:** Style one or all data series: colors, markers, connecting lines, error bars, right-Y binding, or multivariate encoding.
- **Beginner note:** Customizes markers, connecting lines, error bars, and colors for curves or bars.
- **Parameters:** `all_series` (any), `data_set_index` (any), `mode` (any), `multivariate` (any), `series` (any), `series_index` (any), `sheet_id*` (string)

### `graph.setSpec` — Update Graph Specification

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Graph`
- **Summary:** Patch a graph's full GraphSpec, palette, frame, legend, title, survival options, or multivariate encoding.
- **Beginner note:** Updates visual graph settings such as palette, frame style, legend, or multivariate channels.
- **Parameters:** `forest_show_pooled_diamond` (any), `heatmap_labels` (any), `histogram` (any), `initialized` (any), `multivariate` (any), `orientation` (any), `paper_fill` (any), `sheet_id*` (string), `spec` (any), `survival_number_at_risk` (any), `title` (any)

### `graph.updateImage` — Format Embedded Graph Image

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Embedded Image`
- **Summary:** Update placement bounds, aspect-ratio lock, rotation, crop insets, or border on an embedded graph image.
- **Beginner note:** Crops, rotates, resizes, or frames an embedded image on a graph.
- **Parameters:** `annotation_index*` (integer), `border` (any), `bounds_pt` (any), `crop` (any), `height_pt` (any), `lock_aspect_ratio` (any), `rotation_deg` (any), `sheet_id*` (string), `width_pt` (any)

## Page Layouts (`layout`)

### `layout.addPanel` — Add Panel to Layout

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/Layout Panel`
- **Summary:** Place a live graph panel, live table panel, or markdown text callout onto a layout sheet.
- **Beginner note:** Adds a graph, data or result table, or label callout to an explicit rectangle or grid slot on a layout.
- **Parameters:** `badge_override` (string|null), `bounds_pt` (any), `col` (integer|null), `font_size_pt` (number|null), `graph_sheet_id` (string|null), `gutter_pt` (number|null), `kind` (string|null), `margin_pt` (number|null), `markdown` (string|null), `panel_id` (string|null), `row` (integer|null), `scale_percent` (number|null), `sheet_id*` (string), `source_sheet_id` (string|null), `tab_id` (string|null)

### `layout.alignOrigins` — Align Panel Origins

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Arrange/Align Plot Origins`
- **Summary:** Enable or disable plot-area origin alignment across layout panels and optionally normalize live graph bounds.
- **Beginner note:** Lines up the X and Y axis origins of neighboring panels so multi-panel figures align cleanly.
- **Parameters:** `align` (boolean|null), `normalize_bounds` (boolean|null), `sheet_id*` (string)

### `layout.applyCanvasPreset` — Apply Layout Canvas Preset

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Layout Canvas Size Preset`
- **Summary:** Resize a layout page to a standard publication canvas preset while optionally preserving typography sizes.
- **Beginner note:** Resizes a whole layout figure to a journal column width or slide preset and runs preflight checks.
- **Parameters:** `preset*` (string), `resize_mode` (string|null), `sheet_id*` (string)

### `layout.create` — New Layout

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/New Layout`
- **Summary:** Create a new publication layout sheet with a journal width preset, page orientation, and grid template.
- **Beginner note:** Creates a multi-panel figure page where you can arrange graphs, tables, and text callouts.
- **Parameters:** `align_plot_origins` (boolean|null), `background_fill` (any), `grid_template` (array|null), `height` (number|null), `orientation` (any), `panel_badges` (any), `preset` (any), `sheet_id` (string|null), `title` (string|null), `unit` (any), `width` (number|null)

### `layout.equalizeScaling` — Equalize Scaling

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Arrange/Equalize Scaling`
- **Summary:** Equalize rendered inner plot scaling across live graph panels on a layout sheet.
- **Beginner note:** Resizes selected or all graph panels on a layout to match the smallest, largest, or an explicit scale percentage.
- **Parameters:** `mode` (any), `panel_ids` (array|null), `panel_indices` (array|null), `percent` (number|null), `sheet_id*` (string)

### `layout.insertImage` — Insert Image on Layout

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `Insert/Image on Layout`
- **Summary:** Embed a PNG, JPEG, or TIFF raster image item onto a publication layout sheet.
- **Beginner note:** Places a microscopy image, gel photo, or schematic panel onto a layout page.
- **Parameters:** `asset_id` (string|null), `badge_override` (string|null), `border` (any), `bounds_pt` (any), `bytes_base64` (string|null), `crop` (any), `lock_aspect_ratio` (boolean|null), `panel_id` (string|null), `path` (string|null), `rotation_deg` (number|null), `sheet_id*` (string)

### `layout.reorderPanel` — Reorder Panel

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Arrange/Reorder Panel`
- **Summary:** Change the z-order stacking depth of a panel on a layout sheet.
- **Beginner note:** Moves a graph, table, or text panel forward or backward in the layout paint order.
- **Parameters:** `action*` (string), `panel_id` (string|null), `panel_index` (integer|null), `sheet_id*` (string)

### `layout.setSpec` — Format Layout

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Layout`
- **Summary:** Update a layout sheet's page dimensions, journal preset, grid template, origin alignment, badge style, or background.
- **Beginner note:** Changes the page setup, grid arrangement, or panel lettering of an existing figure layout.
- **Parameters:** `align_plot_origins` (boolean|null), `background_fill` (any), `grid_template` (array|null), `height` (number|null), `orientation` (any), `panel_badges` (any), `preset` (any), `sheet_id*` (string), `spec` (object|null), `title` (string|null), `unit` (any), `width` (number|null)

### `layout.updateImage` — Format Embedded Layout Image

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Layout Image`
- **Summary:** Update placement bounds, aspect-ratio lock, rotation, crop insets, border, or badge override on a layout image.
- **Beginner note:** Crops, rotates, resizes, frames, or labels an embedded image panel on a layout sheet.
- **Parameters:** `badge_override` (string|null), `border` (any), `bounds_pt` (any), `crop` (any), `height_pt` (number|null), `item_index` (integer|null), `lock_aspect_ratio` (boolean|null), `panel_id` (string|null), `rotation_deg` (number|null), `sheet_id*` (string), `width_pt` (number|null)

## Vector & Raster Export (`export`)

### `export.pdf` — Export PDF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PDF`
- **Summary:** Render one or more graph or layout sheets to publication PDF (sRGB, print CMYK, or grayscale) and optionally write it atomically to disk.
- **Beginner note:** Creates a single-page or multi-page vector PDF from one or more graphs and layouts.
- **Parameters:** `color_space` (string|null), `compress_streams` (boolean|null), `outline_text` (boolean|null), `path` (string|null), `sheet_id` (string|null), `sheet_ids` (array|null), `title` (string|null)

### `export.png` — Export PNG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PNG`
- **Summary:** Rasterize a graph or layout sheet to a PNG image at the requested pixel dimensions or DPI.
- **Beginner note:** Renders a graph or layout to a high-DPI PNG image, returning base64 bytes and optionally saving to disk.
- **Parameters:** `dpi` (integer|null), `height_px` (integer|null), `path` (string|null), `sheet_id*` (string), `transparent_bg` (boolean|null), `width_px` (integer|null)

### `export.svg` — Export SVG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/SVG`
- **Summary:** Render a graph or layout sheet to scalable vector graphics (SVG) and optionally write it atomically to disk.
- **Beginner note:** Exports the selected graph or layout as crisp SVG text, optionally saving it to a file path.
- **Parameters:** `embed_fonts` (boolean|null), `include_metadata` (boolean|null), `include_xml_declaration` (boolean|null), `outline_text` (boolean|null), `path` (string|null), `precision` (integer|null), `pretty` (boolean|null), `sheet_id*` (string), `transparent_bg` (boolean|null)

### `export.tiff` — Export TIFF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/TIFF`
- **Summary:** Rasterize a graph or layout sheet to a TIFF image with selectable compression (`none`, `lzw`, `deflate`).
- **Beginner note:** Renders a publication TIFF image at custom DPI or resolution with lossless compression.
- **Parameters:** `compression` (string|null), `dpi` (integer|null), `height_px` (integer|null), `path` (string|null), `sheet_id*` (string), `transparent_bg` (boolean|null), `width_px` (integer|null)

### `graph.exportEmf` — Export Graph EMF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Graph EMF`
- **Summary:** Export a graph sheet as a Windows Enhanced Metafile (EMF) vector graphic.
- **Beginner note:** Exports a graph sheet to EMF for insertion into Office documents.
- **Parameters:** `path` (string|null), `sheet_id*` (string)

### `graph.exportPdf` — Export Graph PDF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PDF`
- **Summary:** Export a graph sheet (or sheet list) to vector PDF.
- **Beginner note:** Exports the active graph to publication PDF format.
- **Parameters:** `color_space` (string|null), `compress_streams` (boolean|null), `outline_text` (boolean|null), `path` (string|null), `sheet_id` (string|null), `sheet_ids` (array|null), `title` (string|null)

### `graph.exportPng` — Export Graph PNG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PNG`
- **Summary:** Export a graph sheet to a raster PNG image.
- **Beginner note:** Exports the active graph to a PNG bitmap at configurable DPI and size.
- **Parameters:** `dpi` (integer|null), `height_px` (integer|null), `path` (string|null), `sheet_id*` (string), `transparent_bg` (boolean|null), `width_px` (integer|null)

### `graph.exportSvg` — Export Graph SVG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/SVG`
- **Summary:** Export a graph sheet to SVG using the unified SVG renderer.
- **Beginner note:** Exports the active graph to vector SVG format.
- **Parameters:** `embed_fonts` (boolean|null), `include_metadata` (boolean|null), `include_xml_declaration` (boolean|null), `outline_text` (boolean|null), `path` (string|null), `precision` (integer|null), `pretty` (boolean|null), `sheet_id*` (string), `transparent_bg` (boolean|null)

### `layout.exportEmf` — Export Layout EMF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Layout EMF`
- **Summary:** Export a multi-panel layout sheet as a Windows Enhanced Metafile (EMF) vector graphic.
- **Beginner note:** Exports a multi-panel layout sheet to EMF for insertion into Office documents.
- **Parameters:** `path` (string|null), `sheet_id*` (string)

