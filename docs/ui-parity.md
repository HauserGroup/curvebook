# Desktop & Web UI Parity (`lumencraft-ui-egui`)

> Related specifications: [`data-tables-and-transforms.md`](data-tables-and-transforms.md), [`graphing-and-layouts.md`](graphing-and-layouts.md), [`development.md`](development.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G6`, `G7`)

## Project Workspace & Navigation
- [ ] Five-folder Project Navigator tree (`Data Tables`, `Info`, `Results`, `Graphs`, `Layouts`) supporting **dual display modes** (nested `Data with Results` tree vs. split folders), **bold transitive dependency highlighting**, search filter, sheet freeze badges, drag-reorder, and color tags
- [ ] **Family Pane** displaying the live upstream/downstream dependency DAG (`Data -> Transform -> Stats/Fit -> Graph -> Layout`) with **stable `family_anchor_sheet_id` browsing** (clicking inside Family Pane inspects sibling sheets without collapsing or re-rooting the family; clicking in Project Navigator re-anchors)
- [ ] Two-pane `InfoSheet` workspace (standard laboratory metadata keys + hookable custom constants on the left, rich-text experimental notes on the right) and collapsible **`FloatingNote`** sticky callouts on any sheet
- [ ] Welcome / Project Initialization dialog (keyboard-navigable with visible focus rings) supporting empty typed table creation (including `Unpaired` vs. `Paired` `Column` design hints), curated self-guided tutorial datasets, `.lctemplate` templates, and **`Clone a Graph / Pipeline`** (with optional data/row-title/column-title clearing and replicate-count $N$ override)
- [ ] Multi-sheet gallery view and `F6` / `Shift+F6` pane-cycling keyboard accessibility

## Structured Data Grid & Calculated Variables UX
- [ ] Virtualized 60+ fps spreadsheet grid with two-tier column + replicate subcolumn headers, non-destructive `Row Titles` column expand/collapse, and WCAG AA ($\ge 4.5:1$) header/cell contrast
- [ ] `MultipleVariables` column-role header strip (`Continuous`, `Categorical`, `Label`, `Calculated`) distinguishing auto-inferred vs. user-locked roles, plus bulk **`Variables…` level recoding / typo-merging** and level-order editor
- [ ] Typed validation for continuous, censoring (`1`/`0`), categorical, and grouped `(Row × Column × Subcolumn)` cells
- [ ] In-table `=` **Calculated Variable** formula bar with autocomplete and live column evaluation on `MultipleVariables` tables (Prism 11.0+)
- [ ] Non-destructive **Exclude Values** toggle (`Cmd+E` / `Ctrl+E`) and **Freeze / Unfreeze Data Table** command (Prism 11.0+)
- [ ] Boundary-aware range navigation (`Shift+Cmd/Ctrl+Arrow` stopping cleanly at the first empty cell boundary)
- [ ] 4-stage **`Import & Paste Special` / `Paste Link`** pipeline (`Source`, `Filter` [row/col slicing, decimation, `Unstack`, `*`-tagged excluded cells], `Placement` [including reactive cross-table `Transpose`], `Provenance & Notes`) with color-coded `LinkedDataRegion` grid borders and double-click `Format Link` inspector
- [ ] Inline selection quick-stats footer ($n$, mean, SD, SEM, median, sum)

## Analysis, Graphing & Inspection UX
- [ ] Searchable `Analyze…` dialog filtering compatible analyses for the active table archetype (with explicit empty-search feedback) and **duplicate-analysis disambiguation prompt** (modify existing vs. create new analysis when re-analyzing the same sheet)
- [ ] `MultipleVariables` Data-tab variable picker with **variable-type filter (`Categorical` / `Continuous`) and color-coded badges** (Prism 11.1)
- [ ] Multi-tab Result sheet viewer (Tabular results, Multiple comparisons, Residuals, Model comparison, and contextual **Interpret / Analysis Checklist** toolbar panel)
- [ ] Live parameter re-entry (`Change › Analysis Parameters…`, `Cmd/Ctrl+T`) with **dynamic auto-derived Result sheet title updates** (preserving custom user-edited names) and **`Change › Data Analyzed…` cross-family dataset retargeting**
- [ ] First-visit `Change Graph Type` visual schematic chooser + non-modal Format Graph / Format Axes / Bubble-Plot Variable Assignment / Pairwise Comparisons inspectors with instant `Apply` canvas preview
- [ ] Multi-panel Page Layout canvas (live linked graphs + live embedded tables) with snap guides, plot-origin alignment, physical rulers (mm / in), and Export Figure dialog
- [ ] Command palette (`Cmd+K` / `Ctrl+K`), customizable shortcuts, and light/dark UI chrome (keeping figure paper crisp white by default)
