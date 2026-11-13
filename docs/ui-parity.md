# Desktop & Web UI Parity (`lumencraft-ui-egui`)

> Related specifications: [`data-tables-and-transforms.md`](data-tables-and-transforms.md), [`graphing-and-layouts.md`](graphing-and-layouts.md), [`development.md`](development.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G6`, `G7`)
>
> **Architecture & Verification Note:** `lumencraft-ui-egui` (`L6`), `lumencraft` (desktop `--control <port>` / `--headless-once`), and `lumencraft-web` (`init_web_app`) implement a pure-Rust immediate/retained workspace shell (`UiApp`) paired with a deterministic CPU software RGBA/PNG compositor (`render_workspace_rgba` / `render_workspace_frame_png`) and JSONL control channel (`41 passed` in `lumencraft-ui-egui`, `6 passed` in `lumencraft` including `control_loopback`, `1 passed` in `lumencraft-web`, all `0 failed` under `--offline` and `--no-default-features` with `0` Clippy warnings). Because external windowing/GPU crates (`eframe`/`egui`/`wgpu`) are intentionally outside the minimal offline dependency set, state machines, tool intents, themes, localization, and rendered workspace PNG regions are verified deterministically headlessly, while native OS window-event / pointer-drag loops remain unchecked (`[ ]`) below.

## Risk Spike `S1` Decision (Spreadsheet Grid at 60 fps): **GO**

- **Decision:** **GO** — virtualized two-tier spreadsheet engine (`lumencraft-tools::GridToolState`) + sparse window formatter (`lumencraft-ui-egui::GridViewState`). No redesign required.
- **Measured Evidence (`1,000 × 1,000 = 1,000,000 cells`):**
  - `lumencraft-tools` (`spike_s1_grid_perf`, dev `opt-level=1`, `120` frames): `min = 0.0340 ms`, `p50 = 0.0390 ms`, `p95 = 0.0432 ms`, `max = 0.0612 ms`, `mean = 0.0392 ms`; `1,000,000`-row tail jump `elapsed = 0.0150 ms` (`62` visible cells).
  - `lumencraft-ui-egui` (`grid_perf_s1`): `SPIKE_S1_UI_1M` grouped scroll (`120` frames) -> `p50 = 0.061 ms`, `p95 = 0.507 ms`, `mean = 0.245 ms`, `max = 7.795 ms` (`37 × 14` visible cells); `Select All (1,000,000 cells)` + batched `table.excludeCells` -> `0.044 ms` (`intents = 1`, `quick_stats` capped at `50,000` cells); `multivar 30 frames` -> `mean = 0.069 ms`, `max = 1.246 ms`.
  - Paste (`ImportWizardState`) and `table.excludeCells` round-trips dispatch cleanly through `Engine` commands well inside the `16.67 ms` 60-fps frame budget.
- **Target Coverage Note:** Desktop (`apps/lumencraft`) and browser WASM (`apps/lumencraft-web::init_web_app`) share identical pure-Rust `UiApp` state/command routing; `wasm32-unknown-unknown` sysroot compilation (`cargo xtask wasm`) skipped locally when `rustup`'s `wasm32` sysroot is not installed (`CI=true` enforces it).

## Project Workspace & Navigation
- [ ] Five-folder Project Navigator tree (`Data Tables`, `Info`, `Results`, `Graphs`, `Layouts`) supporting **dual display modes** (nested `Data with Results` tree vs. split folders), **bold transitive dependency highlighting**, search filter, sheet freeze badges, drag-reorder, and color tags *(five folders, dual nesting modes, transitive bold highlights, search filter, freeze badges, and `color_tag` implemented & tested in `workspace_nav.rs`; pointer drag-reorder pending)*
- [x] **Family Pane** displaying the live upstream/downstream dependency DAG (`Data -> Transform -> Stats/Fit -> Graph -> Layout`) with **stable `family_anchor_sheet_id` browsing** (clicking inside Family Pane inspects sibling sheets without collapsing or re-rooting the family; clicking in Project Navigator re-anchors)
- [ ] Two-pane `InfoSheet` workspace (standard laboratory metadata keys + hookable custom constants on the left, rich-text experimental notes on the right) and collapsible **`FloatingNote`** sticky callouts on any sheet *(two-pane `InfoSheet` compositor view `draw_info_view` and Navigator `floating_note_count` badges implemented; interactive in-sheet Info/FloatingNote editing & collapse widgets pending)*
- [x] Welcome / Project Initialization dialog (keyboard-navigable with visible focus rings) supporting empty typed table creation (including `Unpaired` vs. `Paired` `Column` design hints), curated self-guided tutorial datasets, `.lctemplate` templates, and **`Clone a Graph / Pipeline`** (with optional data/row-title/column-title clearing and replicate-count $N$ override)
- [ ] Multi-sheet gallery view and `F6` / `Shift+F6` pane-cycling keyboard accessibility *(not yet implemented)*

## Structured Data Grid & Calculated Variables UX
- [x] Virtualized 60+ fps spreadsheet grid with two-tier column + replicate subcolumn headers, non-destructive `Row Titles` column expand/collapse, and WCAG AA ($\ge 4.5:1$) header/cell contrast *(verified at `UiApp` + software compositor layer in `grid_perf_s1` & `i18n_theme`; live OS window hosting pending)*
- [x] `MultipleVariables` column-role header strip (`Continuous`, `Categorical`, `Label`, `Calculated`) distinguishing auto-inferred vs. user-locked roles, plus bulk **`Variables…` level recoding / typo-merging** and level-order editor
- [x] Typed validation for continuous, censoring (`1`/`0`), categorical, and grouped `(Row × Column × Subcolumn)` cells
- [x] In-table `=` **Calculated Variable** formula bar with autocomplete and live column evaluation on `MultipleVariables` tables (Prism 11.0+)
- [x] Non-destructive **Exclude Values** toggle (`Cmd+E` / `Ctrl+E`) and **Freeze / Unfreeze Data Table** command (Prism 11.0+)
- [x] Boundary-aware range navigation (`Shift+Cmd/Ctrl+Arrow` stopping cleanly at the first empty cell boundary)
- [x] 4-stage **`Import & Paste Special` / `Paste Link`** pipeline (`Source`, `Filter` [row/col slicing, decimation, `Unstack`, `*`-tagged excluded cells], `Placement` [including reactive cross-table `Transpose`], `Provenance & Notes`) with color-coded `LinkedDataRegion` grid borders and double-click `Format Link` inspector
- [x] Inline selection quick-stats footer ($n$, mean, SD, SEM, median, sum)

## Analysis, Graphing & Inspection UX
- [x] Searchable `Analyze…` dialog filtering compatible analyses for the active table archetype (with explicit empty-search feedback) and **duplicate-analysis disambiguation prompt** (modify existing vs. create new analysis when re-analyzing the same sheet)
- [x] `MultipleVariables` Data-tab variable picker with **variable-type filter (`Categorical` / `Continuous`) and color-coded badges** (Prism 11.1)
- [x] Multi-tab Result sheet viewer (Tabular results, Multiple comparisons, Residuals, Model comparison, and contextual **Interpret / Analysis Checklist** toolbar panel) *(`ResultSheetViewState` tab switching + checklist badge summaries verified in `dialogs_canvas`)*
- [x] Live parameter re-entry (`Change › Analysis Parameters…`, `Cmd/Ctrl+T`) with **dynamic auto-derived Result sheet title updates** (preserving custom user-edited names) and **`Change › Data Analyzed…` cross-family dataset retargeting**
- [x] First-visit `Change Graph Type` visual schematic chooser + non-modal Format Graph / Format Axes / Bubble-Plot Variable Assignment / Pairwise Comparisons inspectors with instant `Apply` canvas preview
- [ ] Multi-panel Page Layout canvas (live linked graphs + live embedded tables) with snap guides, plot-origin alignment, physical rulers (mm / in), and Export Figure dialog *(`PageLayoutComposerState`, `CanvasToolState` `SnapGuide`s, `layout.alignOrigins`, physical page presets, and `ExportFigureState` verified in `lumencraft-tools`/`dialogs_canvas`; interactive canvas ruler chrome bars pending)*
- [ ] Command palette (`Cmd+K` / `Ctrl+K`), customizable shortcuts, and light/dark UI chrome (keeping figure paper crisp white by default) *(`CommandPaletteState` + `Light`/`Dark`/`HighContrast` themes verified in `dialogs_canvas` & `i18n_theme`; user-rebindable shortcut customization pending)*
