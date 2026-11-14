# LumenCraft Command Reference

Registered commands: 90

## Project & Files (`project`)

### `file.new` — New Project

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/New Project`
- **Shortcut:** `Cmd+N`
- **Summary:** Create a fresh empty LumenCraft project.
- **Beginner note:** Replaces the active workspace with an empty project document (preserving undo history in this session).
- **Parameters:** `title` (any)

### `project.createCheckpoint` — Create Checkpoint

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `File/Checkpoints/Create Checkpoint...`
- **Summary:** Save a named project snapshot checkpoint inside the document.
- **Beginner note:** Captures a named milestone copy of all sheets so you can restore or branch back to it later.
- **Parameters:** `id` (any), `name*` (string), `note` (any), `timestamp_utc` (any)

### `project.deleteCheckpoint` — Delete Checkpoint

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/Checkpoints/Delete Checkpoint...`
- **Summary:** Remove a named checkpoint from the project.
- **Beginner note:** Deletes a stored checkpoint by ID or name without altering active data sheets.
- **Parameters:** `id` (any), `name` (any)

### `project.inspect` — Inspect Project

- **Effects:** `IDEMPOTENT`
- **Menu:** `View/Inspect Project`
- **Summary:** Inspect sheet structure, metadata, and upstream dependency edges.
- **Beginner note:** Returns a structured read-only summary of all sheets, constants, checkpoints, and optional single-sheet dependencies.
- **Parameters:** `sheet_id` (any)

### `project.instantiateTemplate` — New Project From Template

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/New From Template...`
- **Summary:** Create a new project from a `.lctemplate` archive.
- **Beginner note:** Loads a saved template with its analysis pipelines, graphs, and layouts ready for new experimental data.
- **Parameters:** `bytes_base64` (any), `path` (any), `title` (any)

### `project.listCheckpoints` — List Checkpoints

- **Effects:** `IDEMPOTENT`
- **Menu:** `File/Checkpoints/List Checkpoints`
- **Summary:** List named checkpoints stored in the project.
- **Beginner note:** Lists each checkpoint's ID, name, note, and UTC timestamp without dumping full snapshot payloads.
- **Parameters:** (none)

### `project.new` — New Project

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/New Project`
- **Summary:** Create a fresh empty LumenCraft project.
- **Beginner note:** Creates a clean empty project document with an optional title; alias for `file.new`.
- **Parameters:** `title` (any)

### `project.open` — Open Project

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Open...`
- **Shortcut:** `Cmd+O`
- **Summary:** Load a `.lumencraft` archive or unpacked `.lumencraft.d/` directory.
- **Beginner note:** Opens a project from a filesystem path or base64 archive payload and replaces the current session document.
- **Parameters:** `bytes_base64` (any), `path` (any), `unpacked` (any)

### `project.recalculate` — Recalculate All

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | IDEMPOTENT`
- **Menu:** `Analyze/Recalculate All`
- **Summary:** Refresh live links and recalculate all downstream analyses and graphs.
- **Beginner note:** Re-evaluates linked regions, calculated variables, statistical analyses, curve fits, and derived tables in dependency order.
- **Parameters:** `include_external_links` (any)

### `project.restoreCheckpoint` — Restore Checkpoint

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `File/Checkpoints/Restore Checkpoint...`
- **Summary:** Restore the document to a previously saved named checkpoint.
- **Beginner note:** Replaces the current sheets with the chosen checkpoint snapshot while keeping existing checkpoints intact.
- **Parameters:** `id` (any), `name` (any)

### `project.save` — Save Project

- **Effects:** `WRITES_FILESYSTEM`
- **Menu:** `File/Save`
- **Shortcut:** `Cmd+S`
- **Summary:** Save the active project atomically to `.lumencraft`, unpacked directory, or bytes.
- **Beginner note:** Serializes the project document using atomic filesystem writes when a path is provided, or returns base64 bytes.
- **Parameters:** `path` (any), `return_bytes` (any), `unpacked` (any)

### `project.saveTemplate` — Save Template

- **Effects:** `WRITES_FILESYSTEM`
- **Menu:** `File/Save Template...`
- **Summary:** Export the project structure as a data-stripped `.lctemplate` file.
- **Beginner note:** Strips raw data values while preserving table schemas, analyses, graphs, and layouts, then writes atomically to disk or base64.
- **Parameters:** `path` (any), `return_bytes` (any)

## Edit & History (`edit`)

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

### `sheet.clonePipeline` — Clone Pipeline

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Change/Clone Pipeline...`
- **Summary:** Clone a table structure and downstream analysis/graph/layout pipeline for a new experimental run.
- **Beginner note:** Creates a new table preserving X coordinates, analyses, graphs, and layouts while clearing Y replicates for new measurements.
- **Parameters:** `clear_column_titles` (any), `clear_data` (any), `clear_results` (any), `clear_row_titles` (any), `clear_y_values` (any), `clone_analyses` (any), `clone_graphs` (any), `clone_layouts` (any), `keep_x_values` (any), `new_sheet_id` (any), `override_replicates` (any), `prefix` (any), `sheet_id*` (string), `title` (any)

### `sheet.createData` — New Data Table

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/New Data Table`
- **Shortcut:** `Cmd+T`
- **Summary:** Create a typed data table sheet and optional paired info/graph sheets.
- **Beginner note:** Creates one of the eight scientific table archetypes (XY, Column, Grouped, Contingency, Survival, Parts of Whole, Multiple Variables, or Nested).
- **Parameters:** `auto_family` (any), `auto_graph` (any), `auto_info` (any), `cols` (any), `column_factor_title` (any), `design_hint` (any), `graph_sheet_id` (any), `info_sheet_id` (any), `kind` (any), `replicates` (any), `row_factor_title` (any), `rows` (any), `sheet_id` (any), `subcolumn_mode` (any), `time_title` (any), `title` (any), `variables` (any), `x_mode` (any), `x_title` (any)

### `sheet.delete` — Delete Sheet

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA`
- **Menu:** `Edit/Delete Sheet`
- **Summary:** Delete a sheet and optionally cascade to its dependent family.
- **Beginner note:** Removes a sheet from the project, blocking deletion on frozen sheets or when `cascade: false` with active dependents.
- **Parameters:** `cascade` (any), `sheet_id*` (string)

### `sheet.duplicate` — Duplicate Sheet

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Duplicate Sheet`
- **Summary:** Duplicate a single sheet within its section.
- **Beginner note:** Creates an independent copy of one Data, Info, Result, Graph, or Layout sheet.
- **Parameters:** `frozen` (any), `new_sheet_id` (any), `sheet_id*` (string), `title` (any)

### `sheet.duplicateFamily` — Duplicate Family

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Duplicate Family`
- **Summary:** Duplicate a data sheet together with its paired info, analyses, graphs, and layouts.
- **Beginner note:** Clones a complete linked family of sheets so all formulas, analyses, and figures remain wired to the copied table.
- **Parameters:** `clear_data` (any), `new_sheet_id` (any), `prefix` (any), `sheet_id*` (string), `title` (any)

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
- **Parameters:** `folder` (any), `from_index` (any), `index` (any), `order` (any), `section` (any), `sheet_id` (any), `to_index` (any)

### `sheet.select` — Select Sheet

- **Effects:** `MUTATES_DOCUMENT | CHANGES_UI_STATE | IDEMPOTENT`
- **Menu:** `View/Select Sheet`
- **Summary:** Select the active sheet in the navigator or family pane.
- **Beginner note:** Focuses a sheet in the navigator (`mode: "navigator"`) or sub-sheet family view (`mode: "family"`).
- **Parameters:** `mode` (any), `sheet_id*` (string)

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
- **Parameters:** `all` (boolean), `cells` (array), `clear_x` (boolean), `col` (integer), `col_end` (integer), `col_start` (integer), `coord` (object), `range` (object), `row` (integer), `row_end` (integer), `row_start` (integer), `sheet_id*` (string), `subcol` (integer), `subcol_end` (integer), `subcol_start` (integer), `unlink` (boolean)

### `table.convertArchetype` — Convert Table Archetype

- **Effects:** `MUTATES_DOCUMENT | MAY_DISCARD_DATA | IDEMPOTENT`
- **Menu:** `Change/Convert Table Structure...`
- **Summary:** Preview or convert a data table between Column, Grouped, XY, and MultipleVariables archetypes.
- **Beginner note:** Reshape a data table into another structure, previewing any lossy changes before applying.
- **Parameters:** `allow_lossy` (boolean), `preview_only` (boolean), `sheet_id*` (string), `target_kind*` (string)

### `table.excludeCells` — Exclude Cells

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Exclude Values`
- **Shortcut:** `Cmd+E`
- **Summary:** Toggle the exclusion mask on single cells, cell lists, or ranges (including X/time cells).
- **Beginner note:** Exclude or include selected values so analyses ignore them without deleting data.
- **Parameters:** `cells` (array), `col` (integer), `col_end` (integer), `col_start` (integer), `coord` (object), `excluded` (boolean), `range` (object), `row` (integer), `row_end` (integer), `row_start` (integer), `sheet_id*` (string), `subcol` (integer), `subcol_end` (integer), `subcol_start` (integer), `target` (string)

### `table.excludeCellsWithReason` — Exclude Cells With Reason

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Change/Exclude Values With Reason...`
- **Summary:** Exclude cells and attach a structured audit reason tag, note, and UTC timestamp.
- **Beginner note:** Exclude values while recording a reason tag and audit timestamp for review.
- **Parameters:** `col` (integer), `coord` (object), `coords` (array), `note` (any), `range` (object), `reason_tag*` (string), `row` (integer), `sheet_id*` (string), `subcol` (integer), `timestamp_utc` (any)

### `table.exclusionAuditReport` — Exclusion Audit Report

- **Effects:** `IDEMPOTENT`
- **Menu:** `Data/Exclusion Audit Report...`
- **Summary:** Generate a read-only audit report of excluded cells and their reason annotations.
- **Beginner note:** Inspect all excluded values and audit reasons across one sheet or the entire project.
- **Parameters:** `sheet_id` (any)

### `table.exportCsv` — Export Table CSV / TSV

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Table CSV...`
- **Summary:** Export a data or result sheet to CSV/TSV text and optionally write atomically to disk.
- **Beginner note:** Save a data or result table as CSV or TSV without changing the project.
- **Parameters:** `decimal_separator` (any), `delimiter` (any), `path` (any), `sheet_id*` (string)

### `table.importCsv` — Import CSV / TSV

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/CSV or TSV...`
- **Summary:** Import CSV or TSV data from a file path or inline text into a new or existing data table.
- **Beginner note:** Create a new data table or populate an existing one from a CSV or TSV file.
- **Parameters:** `anchor` (object), `archetype` (any), `csv` (any), `decimal_separator` (any), `delimiter` (any), `filter` (any), `has_header` (boolean), `has_row_titles` (boolean), `link` (boolean), `mode` (string), `normalize_decimal` (boolean), `path` (any), `propagate_title_header` (boolean), `replace` (boolean), `replicates` (integer), `sheet_id` (any), `skip_contiguous_blanks` (boolean), `start_col` (integer), `start_row` (integer), `start_subcol` (integer), `subcolumn_mode` (any), `title` (any), `transpose` (boolean)

### `table.importPlateMap` — Import Plate Map

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `File/Import/Plate Map...`
- **Summary:** Map 96-well or 384-well microplate readings and well roles into a structured data sheet.
- **Beginner note:** Import 96-well or 384-well plate measurements using a plate template or role map.
- **Parameters:** `geometry` (any), `roles` (any), `sheet_id` (any), `subtract_blank_mean` (boolean), `target_kind` (any), `template` (any), `template_name` (any), `title` (any), `values*` (any), `x_title` (any)

### `table.importPrism` — Import External Archive

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/External Archive...`
- **Summary:** Import an external ZIP-based project archive from disk, base64, or raw byte array.
- **Beginner note:** Open or merge an external ZIP-format statistical archive into the project.
- **Parameters:** `bytes` (array), `bytes_base64` (any), `merge` (boolean), `path` (any)

### `table.importPzfx` — Import XML Project

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM | MAY_DISCARD_DATA`
- **Menu:** `File/Import/XML Project...`
- **Summary:** Import an XML project document from disk, inline XML string, or base64 payload.
- **Beginner note:** Open or merge an XML-format project file into the workspace.
- **Parameters:** `bytes_base64` (any), `merge` (boolean), `path` (any), `xml` (any)

### `table.importTidyLong` — Import Tidy Long Table

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `File/Import/Tidy Long Table...`
- **Summary:** Pivot a long-format table (rows/headers or CSV/TSV file) into a Column, Grouped, XY, or MultipleVariables sheet.
- **Beginner note:** Reshape long/tidy tabular records into a grouped or XY data table using column mappings.
- **Parameters:** `csv` (any), `decimal_sep` (any), `delimiter` (any), `excluded_col` (any), `group_col` (any), `headers` (any), `mapping` (any), `normalize_decimal` (boolean), `path` (any), `replicate_col` (any), `rows` (any), `sheet_id` (any), `target_kind` (any), `title` (any), `value_col` (any), `x_col` (any)

### `table.paste` — Paste Special

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Edit/Paste Special...`
- **Shortcut:** `Cmd+V`
- **Summary:** Paste delimited clipboard text or a 2D cell grid into a table with placement, transpose, and filter options.
- **Beginner note:** Paste tabular data into a table starting at an anchor cell, optionally transposing or filtering.
- **Parameters:** `anchor` (object), `decimal_separator` (any), `delimiter` (any), `filter` (any), `mode` (string), `normalize_decimal` (boolean), `propagate_title_header` (boolean), `rows` (array), `sheet_id*` (string), `skip_contiguous_blanks` (boolean), `start_col` (integer), `start_row` (integer), `start_subcol` (integer), `text` (string), `transpose` (boolean)

### `table.pasteLink` — Paste Link

- **Effects:** `MUTATES_DOCUMENT | READS_FILESYSTEM`
- **Menu:** `Edit/Paste Link`
- **Summary:** Attach a live linked region from an internal sheet or external delimited file and sync immediately.
- **Beginner note:** Link a range from another sheet or CSV/TSV file so updates flow through automatically.
- **Parameters:** `anchor` (object), `baseline` (any), `decimal_separator` (any), `delimiter` (any), `exclude_row_predicate` (any), `filter` (any), `link_id` (any), `link_spec` (any), `mode` (string), `normalize_decimal` (boolean), `propagate_title_header` (boolean), `provenance` (any), `skip_contiguous_blanks` (boolean), `source_kind` (any), `source_path` (any), `source_range` (any), `source_sheet_id` (any), `start_col` (integer), `start_row` (integer), `start_subcol` (integer), `tab_id` (any), `target_sheet_id*` (string), `transpose` (boolean)

### `table.recodeLevels` — Recode Categorical Levels

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Data/Recode Levels...`
- **Summary:** Map and rewrite categorical values in a MultipleVariables column.
- **Beginner note:** Rename or merge category labels within a MultipleVariables column.
- **Parameters:** `col` (any), `mappings*` (any), `sheet_id*` (string), `title` (any)

### `table.setCalculatedVariable` — Add / Update Calculated Variable

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Calculated Variable...`
- **Summary:** Create or update a formula-driven variable column in a MultipleVariables table and evaluate it.
- **Beginner note:** Add a computed column whose values update automatically from other variables via a formula.
- **Parameters:** `col` (any), `formula*` (string), `sheet_id*` (string), `title` (any)

### `table.setCategoricalOrder` — Set Categorical Order

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Categorical Order...`
- **Summary:** Configure display/reference ordering of categorical levels in a MultipleVariables column.
- **Beginner note:** Choose how categories are ordered (explicit list, alphabetical, entry order, or frequency).
- **Parameters:** `col` (any), `levels` (array), `order` (any), `order_spec` (any), `reference_level` (any), `reversed` (boolean), `sheet_id*` (string), `title` (any)

### `table.setCell` — Set Cell Value

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Cell Value`
- **Summary:** Set or update a single data cell value and optional exclusion flag, then recalculate downstream sheets.
- **Beginner note:** Enter or edit one cell in a data table. Connected analyses and graphs refresh automatically.
- **Parameters:** `col` (integer), `coord` (object), `decimal_separator` (any), `excluded` (boolean), `parse_asterisk_exclusion` (boolean), `row` (integer), `sheet_id*` (string), `subcol` (integer), `value` (any)

### `table.setCells` — Set Multiple Cells

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Multiple Cells`
- **Summary:** Apply a batch of cell edits or a 2D row/column block to a data table, then recalculate.
- **Beginner note:** Update multiple cells in a data table at once from a list of edits or a 2D grid.
- **Parameters:** `edits` (array), `rows` (array), `sheet_id*` (string), `start_col` (integer), `start_row` (integer), `start_subcol` (integer)

### `table.setGroupTitles` — Set Group Column Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Column Titles`
- **Summary:** Set group/dataset titles, X/time title, factor titles, or row titles on a data table.
- **Beginner note:** Rename columns, X/time headers, or factor titles in a data table.
- **Parameters:** `col` (integer), `column_factor_title` (any), `row_factor_title` (any), `row_titles` (array), `sheet_id*` (string), `start_col` (integer), `title` (any), `titles` (array), `x_title` (any)

### `table.setRowTitles` — Set Row Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Row Titles`
- **Summary:** Set or clear one or more row titles (or PartsOfWhole slice labels) in a data table.
- **Beginner note:** Label rows along the left edge of a data table.
- **Parameters:** `row` (integer), `sheet_id*` (string), `start_row` (integer), `title` (any), `titles` (array)

### `table.setSubcolumnTitles` — Set Subcolumn Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Subcolumn Titles`
- **Summary:** Set explicit subcolumn titles on a Nested data table across one or all groups.
- **Beginner note:** Rename nested replicate subcolumns for a specific group or all groups.
- **Parameters:** `col` (any), `sheet_id*` (string), `start_subcol` (integer), `subcol` (integer), `title` (string), `titles` (array)

### `table.setTitles` — Set Column Titles

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set Column Titles`
- **Summary:** Set dataset column titles, X/time title, factor titles, or row titles on a data table.
- **Beginner note:** Rename column headers, X/time title, or factor titles in a data table.
- **Parameters:** `col` (integer), `column_factor_title` (any), `row_factor_title` (any), `row_titles` (array), `sheet_id*` (string), `start_col` (integer), `title` (any), `titles` (array), `x_title` (any)

### `table.setVariableRole` — Set Variable Role

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Variable Properties...`
- **Summary:** Lock or re-infer the analytical role of a column in a MultipleVariables table.
- **Beginner note:** Mark a variable column as continuous, categorical, label, calculated, or auto-inferred.
- **Parameters:** `col` (any), `role*` (string), `sheet_id*` (string), `title` (any)

### `table.setXCells` — Set X / Time Column

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Data/Set X Values`
- **Summary:** Set one or more X or elapsed-time column cells on an XY or Survival table.
- **Beginner note:** Fill or edit values in the X (or Survival time) column.
- **Parameters:** `decimal_separator` (any), `edits` (array), `excluded` (boolean), `parse_asterisk_exclusion` (boolean), `row` (integer), `sheet_id*` (string), `start_row` (integer), `subcol` (integer), `value` (any), `values` (array)

### `table.simulateXy` — Simulate XY Data

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/Simulate Data...`
- **Summary:** Generate a synthetic XY data sheet from a built-in equation, parameter map, and noise model.
- **Beginner note:** Create synthetic XY curves with optional Gaussian, relative CV, or Poisson noise.
- **Parameters:** `auto_info` (boolean), `cv_percent` (number), `equation_id` (string), `graph_sheet_id` (any), `noise_kind` (string), `noise_sd` (number), `parameters` (any), `replicates` (integer), `row_count` (integer), `seed` (integer), `sheet_id` (any), `title` (any), `x_max` (number), `x_min` (number)

## Info & Constants (`info`)

### `info.removeConstant` — Remove Info Constant

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Info/Remove Constant`
- **Summary:** Remove a named constant from an InfoSheet and recalculate dependents.
- **Beginner note:** Deletes a constant from an InfoSheet and re-evaluates any sheets that depended on it.
- **Parameters:** `if_exists` (any), `name*` (string), `sheet_id*` (string)

### `info.setConstant` — Set Info Constant

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Info/Set Constant`
- **Summary:** Create or update a named InfoSheet constant and recalculate dependents.
- **Beginner note:** Sets a hookable constant (number, date, or text with optional units) used by formulas, fits, and graph titles.
- **Parameters:** `kind` (any), `name*` (string), `sheet_id*` (string), `units` (any), `value*` (any)

### `info.setNote` — Set Info Note

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Info/Edit Note`
- **Summary:** Set or append markdown notes on an InfoSheet.
- **Beginner note:** Updates experiment notes on an InfoSheet (or the InfoSheet linked to a given DataSheet ID).
- **Parameters:** `append` (any), `sheet_id*` (string), `text*` (string)

## Analysis & Reports (`analysis`)

### `analysis.add` — Add Analysis

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Add Analysis...`
- **Summary:** Attach and compute a new analysis of any domain (transform, stats, or regression) for a data sheet.
- **Beginner note:** Adds a new live analysis sheet linked to an input table and computes its initial results.
- **Parameters:** `comparison` (any), `confidence_percent` (any), `custom_dsl` (any), `equation_id` (any), `kind` (any), `multiplicity` (any), `overwrite` (any), `p_value_format` (any), `parameter_rules` (any), `selected_data_sets` (any), `sheet_id` (any), `source_sheet_id` (any), `spec` (any), `title` (any), `weighting` (any), `x_range_filter` (any), `y_range_filter` (any)

### `analysis.exportCsv` — Export Result Sheet CSV

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Results CSV...`
- **Summary:** Export a specific tab or tidy combined CSV representation of a result sheet and optionally save it atomically.
- **Beginner note:** Exports result tables to CSV/TSV text with configurable delimiter and decimal separator.
- **Parameters:** `decimal_separator` (any), `delimiter` (any), `path` (any), `sheet_id*` (string), `tab_id` (any), `tab_index` (any)

### `analysis.exportJson` — Export Result Sheet JSON

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Results JSON...`
- **Summary:** Serialize a result sheet (tables, scalars, parameters, checklist) to JSON and optionally save it atomically.
- **Beginner note:** Exports an analysis result sheet as structured JSON text or saves it to a `.json` file.
- **Parameters:** `path` (any), `pretty` (any), `sheet_id*` (string)

### `analysis.retargetData` — Change Data Analyzed

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Data Analyzed...`
- **Summary:** Rebind an existing analysis result sheet to a different compatible data sheet or subset of datasets.
- **Beginner note:** Points an existing analysis at a different data table and recalculates it immediately.
- **Parameters:** `clear_selected_data_sets` (any), `selected_data_sets` (any), `sheet_id*` (string), `source_sheet_id*` (string)

### `analysis.run` — New / Run Analysis

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Analyze Data...`
- **Summary:** Create and run a new analysis sheet, recalculate an existing result sheet by ID, or recalculate the project when empty.
- **Beginner note:** Runs a new analysis when source and kind are provided, or refreshes an existing analysis sheet.
- **Parameters:** `comparison` (any), `confidence_percent` (any), `custom_dsl` (any), `equation_id` (any), `kind` (any), `multiplicity` (any), `overwrite` (any), `p_value_format` (any), `parameter_rules` (any), `selected_data_sets` (any), `sheet_id` (any), `source_sheet_id` (any), `spec` (any), `title` (any), `weighting` (any), `x_range_filter` (any), `y_range_filter` (any)

### `analysis.updateParams` — Change Analysis Parameters

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Analysis Parameters...`
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

### `fit.run` — Run Curve Fit / Regression

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Fit Curve...`
- **Summary:** Create and execute linear, Deming, built-in nonlinear, global shared-parameter, or custom DSL regression.
- **Beginner note:** Fits a linear or nonlinear dose-response/kinetic curve with confidence bands and model comparisons.
- **Parameters:** `comparison` (any), `confidence_percent` (any), `custom_dsl` (any), `equation_id` (any), `kind` (any), `multiplicity` (any), `overwrite` (any), `p_value_format` (any), `parameter_rules` (any), `selected_data_sets` (any), `sheet_id` (any), `source_sheet_id` (any), `spec` (any), `title` (any), `weighting` (any), `x_range_filter` (any), `y_range_filter` (any)

### `result.generateReport` — Generate Methods Report

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Methods Report...`
- **Summary:** Generate a publication-ready statistical methods prose report in `plain`, `markdown`, or `latex`, optionally saving to disk.
- **Beginner note:** Creates a narrative methods paragraph describing the statistical test, sample sizes, and options used.
- **Parameters:** `format` (any), `path` (any), `result_sheet_id*` (string)

### `stats.run` — Run Statistical Test

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Statistical Test...`
- **Summary:** Create and execute a hypothesis test, ANOVA, contingency, survival, ROC, GLM, or multivariate analysis.
- **Beginner note:** Runs a statistical test on a compatible data sheet and stores tables, P values, and assumption checks.
- **Parameters:** `confidence_percent` (any), `kind` (any), `multiplicity` (any), `overwrite` (any), `p_value_format` (any), `selected_data_sets` (any), `sheet_id` (any), `source_sheet_id` (any), `spec` (any), `title` (any)

### `transform.run` — Run Transformation

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Analyze/Transform...`
- **Summary:** Create and execute a mathematical, normalization, baseline, transpose, prune, select, or simulation transformation sheet.
- **Beginner note:** Runs a reproducible data transformation pipeline on a data sheet and writes the output to a linked result sheet.
- **Parameters:** `confidence_percent` (any), `kind` (any), `overwrite` (any), `p_value_format` (any), `selected_data_sets` (any), `sheet_id` (any), `source_sheet_id` (any), `spec` (any), `title` (any)

## Graphs & Styling (`graph`)

### `graph.addAnnotation` — Add Graph Annotation

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Draw/Annotation...`
- **Summary:** Add a text box, line/arrow, shape, or live embedded result table annotation to a graph.
- **Beginner note:** Places labels, arrows, geometric callouts, or embedded result summaries onto a graph.
- **Parameters:** `annotation` (any), `bounds_pt` (any), `kind` (any), `result_sheet_id` (any), `sheet_id*` (string), `text` (any)

### `graph.addComparisonBrackets` — Add Pairwise Significance Brackets

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Draw/Pairwise Comparison Brackets...`
- **Summary:** Add statistical comparison brackets (stars or numeric P/q values) driven by a result sheet's pairwise comparisons.
- **Beginner note:** Draws significance brackets above compared groups using P values from your statistical test.
- **Parameters:** `bracket_step_pt` (any), `clear_existing` (any), `font_size_pt` (any), `group_a` (any), `group_b` (any), `hide_if_non_significant` (any), `label_mode` (any), `result_sheet_id*` (string), `sheet_id*` (string), `significant_only` (any), `stroke_width_pt` (any), `style` (any)

### `graph.applyCanvasPreset` — Apply Graph Canvas Preset

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Canvas Size Preset...`
- **Summary:** Resize a graph canvas to a journal column, slide, or poster preset (`KeepTypographyPt` or `ProportionalScale`).
- **Beginner note:** Resizes a graph to standard journal figure widths and runs a publication preflight check.
- **Parameters:** `preset*` (string), `resize_mode` (any), `sheet_id*` (string)

### `graph.applyStyleFrom` — Apply Graph Style From...

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Apply Style From...`
- **Summary:** Copy visual styling (palette, frame, axis styling, legend, and series appearances) from a donor graph.
- **Beginner note:** Copies the visual style of another graph sheet onto this graph without changing its data.
- **Parameters:** `clone_error_bars_and_legend` (any), `clone_frame_and_axes` (any), `clone_palette` (any), `donor_sheet_id*` (string), `sheet_id*` (string)

### `graph.create` — New Graph

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/New Graph...`
- **Summary:** Create a new graph sheet bound to an input data sheet with optional kind, title, and initialization state.
- **Beginner note:** Creates a new graph linked to one of your data tables.
- **Parameters:** `data_sheet_id*` (string), `initialized` (any), `kind` (any), `sheet_id` (any), `title` (any)

### `graph.initialize` — Initialize Graph

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Graph/Initialize...`
- **Summary:** Initialize an uninitialized graph sheet, applying its graph kind and default series styling.
- **Beginner note:** Chooses an initial plot type for a newly created graph sheet.
- **Parameters:** `kind` (any), `sheet_id*` (string), `title` (any)

### `graph.preflightCheck` — Preflight Check Figure

- **Effects:** `IDEMPOTENT`
- **Menu:** `Analyze/Preflight Check`
- **Summary:** Compile a graph or layout sheet and check typography, hairlines, clipping, and overlap readiness.
- **Beginner note:** Audits a graph or layout for publication print issues like tiny fonts or clipped labels.
- **Parameters:** `sheet_id*` (string)

### `graph.setAdditionalTicks` — Set Additional Axis Ticks

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Additional Ticks...`
- **Summary:** Set custom callout ticks and optional gridlines along the `x`, `y`, or `y2` axis.
- **Beginner note:** Places custom reference ticks and gridlines at specific axis coordinates.
- **Parameters:** `axis*` (string), `sheet_id*` (string), `ticks*` (array)

### `graph.setAxes` — Format Axes

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Axes...`
- **Summary:** Configure X, Y, and right-hand Y2 axes including log/probit/logit scales, segmented breaks, thresholds, and frame presets.
- **Beginner note:** Adjusts axis limits, log scales, titles, broken-axis gaps, and frame styling.
- **Parameters:** `axis` (any), `frame_preset` (any), `sheet_id*` (string), `x_axis` (any), `y2_axis` (any), `y_axis` (any)

### `graph.setKind` — Change Graph Type

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Change/Graph Type...`
- **Summary:** Switch a graph's visualization type (XY, column, grouped, survival, multivariate, diagnostic) with optional style reset.
- **Beginner note:** Changes the chart type of an existing graph without recreating the sheet.
- **Parameters:** `kind*` (any), `option_id` (any), `reset_series_style` (any), `sheet_id*` (string)

### `graph.setRegressionOverlay` — Overlay Regression Curve

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Graph/Regression Overlay...`
- **Summary:** Bind or clear a fitted curve overlay with optional confidence/prediction bands on one or all graph series.
- **Beginner note:** Overlays a curve fit line and confidence ribbons from a result sheet onto your graph.
- **Parameters:** `all_series` (any), `clear` (any), `data_set_index` (any), `result_sheet_id` (any), `sample_segments` (any), `series_index` (any), `shaded_ribbon` (any), `sheet_id*` (string), `show_confidence_band` (any), `show_prediction_band` (any)

### `graph.setSeries` — Format Data Series

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Data on Graph...`
- **Summary:** Style one or all data series: colors, markers, connecting lines, error bars, right-Y binding, or multivariate encoding.
- **Beginner note:** Customizes markers, connecting lines, error bars, and colors for curves or bars.
- **Parameters:** `data_set_index` (any), `multivariate` (any), `series` (any), `series_index` (any), `sheet_id*` (string)

### `graph.setSpec` — Update Graph Specification

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Graph...`
- **Summary:** Patch a graph's full GraphSpec, palette, frame, legend, title, survival options, or multivariate encoding.
- **Beginner note:** Updates visual graph settings such as palette, frame style, legend, or multivariate channels.
- **Parameters:** `initialized` (any), `multivariate` (any), `sheet_id*` (string), `spec` (any), `survival_number_at_risk` (any), `title` (any)

## Page Layouts (`layout`)

### `layout.addPanel` — Add Panel to Layout

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/Layout Panel...`
- **Summary:** Place a live graph panel, live table panel, or markdown text callout onto a layout sheet.
- **Beginner note:** Adds a graph, data or result table, or label callout to an explicit rectangle or grid slot on a layout.
- **Parameters:** `badge_override` (any), `bounds_pt` (any), `col` (any), `font_size_pt` (any), `graph_sheet_id` (any), `gutter_pt` (any), `kind` (any), `margin_pt` (any), `markdown` (any), `panel_id` (any), `row` (any), `scale_percent` (any), `sheet_id*` (string), `source_sheet_id` (any), `tab_id` (any)

### `layout.alignOrigins` — Align Panel Origins

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Arrange/Align Plot Origins`
- **Summary:** Enable or disable plot-area origin alignment across layout panels and optionally normalize live graph bounds.
- **Beginner note:** Lines up the X and Y axis origins of neighboring panels so multi-panel figures align cleanly.
- **Parameters:** `align` (any), `normalize_bounds` (any), `sheet_id*` (string)

### `layout.applyCanvasPreset` — Apply Layout Canvas Preset

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Layout Canvas Size Preset...`
- **Summary:** Resize a layout page to a standard publication canvas preset while optionally preserving typography sizes.
- **Beginner note:** Resizes a whole layout figure to a journal column width or slide preset and runs preflight checks.
- **Parameters:** `preset*` (string), `resize_mode` (any), `sheet_id*` (string)

### `layout.create` — New Layout

- **Effects:** `MUTATES_DOCUMENT`
- **Menu:** `Insert/New Layout...`
- **Summary:** Create a new publication layout sheet with a journal width preset, page orientation, and grid template.
- **Beginner note:** Creates a multi-panel figure page where you can arrange graphs, tables, and text callouts.
- **Parameters:** `align_plot_origins` (any), `background_fill` (any), `grid_template` (any), `height` (any), `orientation` (any), `panel_badges` (any), `preset` (any), `sheet_id` (any), `title` (any), `unit` (any), `width` (any)

### `layout.setSpec` — Format Layout

- **Effects:** `MUTATES_DOCUMENT | IDEMPOTENT`
- **Menu:** `Format/Layout...`
- **Summary:** Update a layout sheet's page dimensions, journal preset, grid template, origin alignment, badge style, or background.
- **Beginner note:** Changes the page setup, grid arrangement, or panel lettering of an existing figure layout.
- **Parameters:** `align_plot_origins` (any), `background_fill` (any), `grid_template` (any), `height` (any), `orientation` (any), `panel_badges` (any), `preset` (any), `sheet_id*` (string), `spec` (any), `title` (any), `unit` (any), `width` (any)

## Vector & Raster Export (`export`)

### `export.pdf` — Export PDF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PDF...`
- **Summary:** Render one or more graph or layout sheets to publication PDF (sRGB or print CMYK) and optionally write it atomically to disk.
- **Beginner note:** Creates a single-page or multi-page vector PDF from one or more graphs and layouts.
- **Parameters:** `color_space` (any), `compress_streams` (any), `outline_text` (any), `path` (any), `sheet_id` (any), `sheet_ids` (any), `title` (any)

### `export.png` — Export PNG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PNG...`
- **Summary:** Rasterize a graph or layout sheet to a PNG image at the requested pixel dimensions or DPI.
- **Beginner note:** Renders a graph or layout to a high-DPI PNG image, returning base64 bytes and optionally saving to disk.
- **Parameters:** `dpi` (any), `height_px` (any), `path` (any), `sheet_id*` (string), `width_px` (any)

### `export.svg` — Export SVG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/SVG...`
- **Summary:** Render a graph or layout sheet to scalable vector graphics (SVG) and optionally write it atomically to disk.
- **Beginner note:** Exports the selected graph or layout as crisp SVG text, optionally saving it to a file path.
- **Parameters:** `embed_fonts` (any), `include_metadata` (any), `include_xml_declaration` (any), `outline_text` (any), `path` (any), `precision` (any), `pretty` (any), `sheet_id*` (string)

### `export.tiff` — Export TIFF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/TIFF...`
- **Summary:** Rasterize a graph or layout sheet to a TIFF image with selectable compression (`none`, `lzw`, `deflate`).
- **Beginner note:** Renders a publication TIFF image at custom DPI or resolution with lossless compression.
- **Parameters:** `compression` (any), `dpi` (any), `height_px` (any), `path` (any), `sheet_id*` (string), `width_px` (any)

### `graph.exportEmf` — Export Graph EMF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Graph EMF...`
- **Summary:** Export a graph sheet as a Windows Enhanced Metafile (EMF) vector graphic.
- **Beginner note:** Exports a graph sheet to EMF for insertion into Office documents.
- **Parameters:** `path` (any), `sheet_id*` (string)

### `graph.exportPdf` — Export Graph PDF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PDF`
- **Summary:** Export a graph sheet (or sheet list) to vector PDF.
- **Beginner note:** Exports the active graph to publication PDF format.
- **Parameters:** `color_space` (any), `compress_streams` (any), `outline_text` (any), `path` (any), `sheet_id` (any), `sheet_ids` (any), `title` (any)

### `graph.exportPng` — Export Graph PNG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/PNG`
- **Summary:** Export a graph sheet to a raster PNG image.
- **Beginner note:** Exports the active graph to a PNG bitmap at configurable DPI and size.
- **Parameters:** `dpi` (any), `height_px` (any), `path` (any), `sheet_id*` (string), `width_px` (any)

### `graph.exportSvg` — Export Graph SVG

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/SVG`
- **Summary:** Export a graph sheet to SVG using the unified SVG renderer.
- **Beginner note:** Exports the active graph to vector SVG format.
- **Parameters:** `embed_fonts` (any), `include_metadata` (any), `include_xml_declaration` (any), `outline_text` (any), `path` (any), `precision` (any), `pretty` (any), `sheet_id*` (string)

### `layout.exportEmf` — Export Layout EMF

- **Effects:** `WRITES_FILESYSTEM | IDEMPOTENT`
- **Menu:** `File/Export/Layout EMF...`
- **Summary:** Export a multi-panel layout sheet as a Windows Enhanced Metafile (EMF) vector graphic.
- **Beginner note:** Exports a multi-panel layout sheet to EMF for insertion into Office documents.
- **Parameters:** `path` (any), `sheet_id*` (string)

