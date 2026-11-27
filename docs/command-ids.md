# Command ID Specification

Every command executed by `curvebook-engine` (`Session::execute`), bound to UI shortcuts/menus (`UI_COMMANDS`), exposed through `curvebook-mcp`, recorded in crash-recovery journals (`ops.journal`), or scripted in `curvebook.batch.v1` manifests uses a canonical **command ID**.

The authoritative generated list of all current engine commands is maintained at [`docs/commands.md`](commands.md) (generated via `cargo xtask commands` and verified in CI via `cargo xtask commands --check`); UI-only commands are listed in the generated `crates/mcp/src/ui_catalog.rs` (`cargo xtask ui-commands --check`). Never duplicate full command tables manually in prose.

---

## 1. Canonical Format: `<area>.<camelCaseAction>`

- **Syntax:** `<area>.<camelCaseAction>` matching `^[a-z]+\.[a-z][a-zA-Z0-9]*$` (single dot separator, no underscores, no hyphens, no spaces). This applies to engine commands and is enforced by `is_valid_command_id_format` in `crates/engine/tests/registry_and_errors.rs`; UI-only IDs follow the dotted form described in the `ui` / `app` row below.
- **`<area>`:** Lowercase domain prefix identifying the subsystem owning the command.
- **`<camelCaseAction>`:** Verb-first (or established action noun/verb) in strict lowerCamelCase starting with a lowercase ASCII letter.

### Valid Area Prefixes

| Area | Scope | Examples (representative pattern) |
| :--- | :--- | :--- |
| `project` *(and legacy `file`)* | Project creation, open, save, templates, checkpoints | `project.new`, `project.save`, `project.createCheckpoint` *(keep `file.new` alias for replay compat)* |
| `edit` | Session history traversal | `edit.undo`, `edit.redo` |
| `sheet` | Workbook sheet management & table format creation | `sheet.createData`, `sheet.duplicate`, `sheet.rename` |
| `table` | Data grid cells, titles, subcolumns, imports, exclusions | `table.setCell`, `table.setCells`, `table.setSubcolumnTitles` |
| `info` | Info sheet constants and notes | `info.setConstant`, `info.removeConstant`, `info.setNote` |
| `analyze` / `transform` / `stats` / `fit` | Running quantitative pipelines | `analyze.run`, `transform.run`, `stats.run`, `fit.run` |
| `analysis` | Existing analysis sheet lifecycle & parameters | `analysis.add`, `analysis.updateParams`, `analysis.run` |
| `result` | Structured report / summary generation | `result.generateReport` |
| `graph` | Plot/chart sheets, axes, legends, per-graph exports | `graph.create`, `graph.setAxes`, `graph.exportSvg` |
| `layout` | Multi-panel page layouts & layout exports | `layout.create`, `layout.addPanel`, `layout.exportEmf` |
| `export` | Whole-figure / batch vector & raster export | `export.svg`, `export.pdf`, `export.png`, `export.tiff` |
| `ui` / `app` | UI-only commands (`UI_COMMANDS` / `UiCommandSpec`); may use a dotted `ui.<object>.<action>` form | `ui.palette.open`, `ui.zoom.fitSheet`, `ui.navigator.toggle`, `app.about` |

Introducing a brand-new `<area>` prefix requires maintainer review and adding it to both the command registry validation test (`valid_areas` in `crates/engine/tests/core_suite/registry_and_errors.rs`) and this table.

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
2. **Batch automation (`curvebook.batch.v1`):** Headless CLI manifests (`apps/curvebook-cli/src/cmd_run_batch.rs`) execute serialized command arrays in production pipelines.
3. **Keybindings & MCP tooling:** User `shortcuts.json` bindings and external MCP workflows (`curvebook-mcp`) persist IDs across upgrades.

If a command's canonical area or verb must evolve (for example, `file.new` superseded by `project.new`), register a compatibility alias that continues accepting and replaying historical payloads cleanly until an explicit major version migration rewrites stored records.
