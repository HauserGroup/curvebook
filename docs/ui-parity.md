# Desktop & Web UI Parity (`lumencraft-ui-egui`)

> Related specifications: [`data-tables-and-transforms.md`](data-tables-and-transforms.md), [`graphing-and-layouts.md`](graphing-and-layouts.md), [`development.md`](development.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G6`, `G7`)

## Project Workspace & Navigation
- [ ] Five-folder Project Navigator tree (`Data Tables`, `Info`, `Results`, `Graphs`, `Layouts`) with search filter, sheet freeze badges, drag-reorder, and color tags
- [ ] **Family Pane** displaying the live upstream/downstream dependency DAG of the active sheet (`Data -> Transform -> Stats/Fit -> Graph -> Layout`)
- [ ] Welcome / New Table & Graph dialog (fully keyboard-navigable with visible focus rings) choosing among the 8 table archetypes and subcolumn modes
- [ ] "Clone Family / New Data for Existing Pipeline" workflow and `.lctemplate` launcher
- [ ] Multi-sheet gallery view and `F6` / `Shift+F6` pane-cycling keyboard accessibility

## Structured Data Grid & Calculated Variables UX
- [ ] Virtualized 60+ fps spreadsheet grid with two-tier column + replicate subcolumn headers and WCAG AA ($\ge 4.5:1$) header/cell contrast
- [ ] Typed validation for continuous, censoring (`1`/`0`), categorical (with level-order editor), and grouped subcolumns
- [ ] In-table `=` **Calculated Variable** formula bar with autocomplete and live column evaluation on `MultipleVariables` tables (Prism 11.0+)
- [ ] Non-destructive **Exclude Values** toggle (`Cmd+E` / `Ctrl+E`) and **Freeze / Unfreeze Data Table** command (Prism 11.0+)
- [ ] Boundary-aware range navigation (`Shift+Cmd/Ctrl+Arrow` stopping cleanly at the first empty cell boundary)
- [ ] Clipboard interoperability with Excel/TSV/CSV including Paste Special (transpose, skip blanks, decimal separator override)
- [ ] Inline selection quick-stats footer ($n$, mean, SD, SEM, median, sum)

## Analysis, Graphing & Inspection UX
- [ ] Searchable `Analyze…` dialog filtering compatible analyses for the active table archetype (with explicit empty-search feedback)
- [ ] `MultipleVariables` Data-tab variable picker with **variable-type filter (`Categorical` / `Continuous`) and color-coded badges** (Prism 11.1)
- [ ] Multi-tab Result sheet viewer (Tabular results, Multiple comparisons, Residuals, Model comparison, and plain-language Analysis Checklist) + floating diagnostic notes
- [ ] Live parameter re-entry (`Change › Analysis Parameters…`) and graceful orphaned-result reassignment
- [ ] Interactive Format Graph / Format Axes / Pairwise Comparisons inspector with instant canvas preview
- [ ] Multi-panel Page Layout canvas with snap guides, plot-origin alignment, physical rulers (mm / in), and Export Figure dialog
- [ ] Command palette (`Cmd+K` / `Ctrl+K`), customizable shortcuts, and light/dark UI chrome (keeping figure paper crisp white by default)
