# Command ID Specification

Every command executed by `lumencraft-engine` (`Session::execute`), bound to UI shortcuts/menus (`UI_COMMANDS`), exposed through `lumencraft-mcp`, recorded in crash-recovery journals (`ops.journal`), or scripted in `lumencraft.batch.v1` manifests uses a canonical **command ID**.

The authoritative generated list of all current engine and UI commands is maintained at `docs/commands.md` (generated via `cargo xtask commands` and verified in CI via `cargo xtask commands --check`). Never duplicate full command tables manually in prose.

---

## 1. Canonical Format: `<area>.<camelCaseAction>`

- **Syntax:** `<area>.<camelCaseAction>` matching `^[a-z]+\.[a-z][a-zA-Z0-9]*$` (single dot separator, no underscores, no hyphens, no spaces).
- **`<area>`:** Lowercase domain prefix identifying the subsystem owning the command.
- **`<camelCaseAction>`:** Verb-first (or established action noun/verb) in strict lowerCamelCase starting with a lowercase ASCII letter.

### Valid Area Prefixes

| Area | Scope | Examples (representative pattern) |
| :--- | :--- | :--- |
| `project` *(and legacy `file`)* | Project creation, open, metadata, info pages | `project.new`, `project.setAuthor` *(keep `file.new` alias for replay compat)* |
| `edit` | Session history traversal | `edit.undo`, `edit.redo` |
| `sheet` | Workbook sheet management & table format creation | `sheet.addDataTable`, `sheet.duplicate`, `sheet.rename` |
| `table` | Data grid cells, rows, columns, subcolumns, sorting | `table.setCell`, `table.insertRow`, `table.setColumnTitle` |
| `info` | Info sheet constants and notes | `info.setConstant`, `info.deleteConstant`, `info.setNotes` |
| `analyze` / `transform` / `stats` / `fit` | Running quantitative pipelines | `analyze.run`, `transform.run`, `stats.run`, `fit.run` |
| `analysis` | Existing analysis sheet lifecycle & parameters | `analysis.editParameters`, `analysis.recalculate`, `analysis.freeze` |
| `result` | Structured report / summary generation | `result.generateReport` |
| `graph` | Plot/chart sheets, axes, legends, per-graph exports | `graph.new`, `graph.addPlot`, `graph.exportSvg` |
| `layout` | Multi-panel page layouts & layout exports | `layout.new`, `layout.addPanel`, `layout.exportEmf` |
| `export` | Whole-figure / batch vector & raster export | `export.svg`, `export.pdf`, `export.png`, `export.tiff` |
| `ui` / `app` | UI-only commands (`UI_COMMANDS` / `UiCommandSpec`) | `ui.openSettings`, `ui.zoomFit`, `ui.commandPalette`, `app.about` |

Introducing a brand-new `<area>` prefix requires maintainer review and adding it to both the command registry validation test and this table.

---

## 2. `snake_case` to `camelCase` Rules

When deriving a command ID from Rust identifiers or CLI/JSON names:

1. Split the action words on underscores (`_`):
   - `add_data_table` -> `addDataTable`
   - `insert_row` -> `insertRow`
   - `set_column_title` -> `setColumnTitle`
2. Lowercase the first word completely (`set`, `add`, `export`, `toggle`).
3. Capitalize **only the first character** of each subsequent word, including short acronyms inside multi-word actions:
   - `export_svg` -> `exportSvg` *(not `exportSVG`)*
   - `export_pdf` -> `exportPdf` *(not `exportPDF`)*
   - `set_x_title` -> `setXTitle`
   - `zoom_100` -> `zoom100`
4. Keep single-letter mathematical axes clear and standard (`setXLabel`, `setYBounds`, `addYErrorColumn`).

---

## 3. Immutability & Stability Guarantee

**Once a command ID ships in a release, it MUST NOT be renamed, deleted without compatibility handling, or reassigned to different semantics.**

Reasons:
1. **Crash recovery (`ops.journal`):** `crates/format/src/recovery.rs` logs executed engine commands verbatim to NDJSON (`command_json`) and replays them through `Session::execute` after an unclean exit. Renaming an ID breaks recovery of existing user journals.
2. **Batch automation (`lumencraft.batch.v1`):** Headless CLI manifests (`apps/lumencraft-cli/src/cmd_run_batch.rs`) execute serialized command arrays in production pipelines.
3. **Keybindings & MCP tooling:** User `shortcuts.json` bindings and external MCP workflows (`lumencraft-mcp`) persist IDs across upgrades.

If a command's canonical area or verb must evolve (for example, `file.new` superseded by `project.new`), register a compatibility alias that continues accepting and replaying historical payloads cleanly until an explicit major version migration rewrites stored records.
