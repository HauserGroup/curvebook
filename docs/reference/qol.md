# Calculators & Quality-of-Life Capabilities

CurveBook augments its statistical and graphing core with non-destructive workflow tools and live diagnostics designed to keep raw measurements intact, maintain reproducible provenance, and surface assumption evidence cleanly.

---

## Selection Quick Statistics {#quick-statistics}

Selecting a single cell or rectangular range `[r0..=r1, flat0..=flat1]` in any `DataSheet` computes live summary statistics over all included finite numeric values inside the selection (`SelectionQuickStats` / `GridQuickStats` in `crates/ui-egui/src/grid_view.rs`):

- **Computed fields:** `selected_cells` (total cell count in selection), `n` (active finite numeric values), `excluded` (count of excluded cells inside the selection), `sum`, `mean`, `sd` (sample standard deviation, $n - 1$ denominator; `None` when $n < 2$), `sem` ($\text{SD}/\sqrt{n}$; `None` when $n < 2$), `median`, `min`, and `max`.
- **Filtering semantics:** Excluded cells (`excluded: true`), blank cells, text cells, and non-finite values are excluded from `n` and every summary moment without altering document state.

---

## Active Quality-of-Life Capabilities (`R9`) {#active-qol}

| ID | Capability | Engine / UI Seam | Scope & Behavior |
|---|---|---|---|
| **QOL-1** | **Non-destructive table archetype conversion** | `table.convertArchetype` | Converts `DataSheet` layouts among `Column`, `Grouped`, `Xy`, `MultipleVariables`, and `Contingency` with a non-mutating `ConversionPreview` (`preserved_cells`, `dropped_cells`, `preserved_exclusions`, `warnings`, `lossy`), preserving `excluded` flags and remapping `CellExclusionNote`s. Lossy conversions require explicit `allow_lossy: true`. |
| **QOL-2** | **96/384-well microplate & tidy long-to-wide reshape** | `table.importPlateMap`, `table.importTidyLong` | Maps `8×12` and `16×24` plate grids or tidy long observational records directly into typed `Xy`, `Column`, or `Grouped` replicate subcolumns during import or paste. |
| **QOL-3** | **Auditable cell exclusion reasons & provenance tags** | `table.excludeCellsWithReason`, `table.exclusionAuditReport` | Attaches sparse `(reason_tag, note, timestamp_utc)` `CellExclusionNote` records alongside boolean `excluded` masks without changing numerical results or requiring reasons on plain `Cmd/Ctrl+E` toggles. |
| **QOL-4** | **Side-by-side split workspace** | `ui.view.splitRight` (and inspector/workspace layout) | Docks a live `Graph` or `Result` pane adjacent to the active `Data` or `Result` sheet so cell edits, exclusions, and analysis/fit parameter updates re-render immediately through the reactive DAG. |
| **QOL-5** | **Two-way data-to-graph point brushing** | `SourceCellId` metadata on `ChartScene` | Links plotted point glyphs (`sheet_id, row, col, subcol`) with source grid cells for hover inspection, split-pane highlighting, jumping to source coordinates, and context-menu observation exclusion/inclusion. |
| **QOL-6** | **One-click statistical reporting generator** | `result.generateReport` | Produces deterministic, read-only ICMJE/ARRIVE-aligned statistical methods and summary paragraphs in `plain`, `markdown`, or `latex` directly from stored `AnalysisSpec`, `DataSheet`, and `ResultSheet` values without modifying numbers. |
| **QOL-7** | **Diagnostics-only inline assumption banners** | `ResultSheet::checklist`, `ui.result.inspectDiagnostics` | Summarizes executed assumption checks (normality, variance homogeneity, sphericity, proportional hazards, separation, collinearity, lack of fit, runs) with status (`Pass`, `Warn`, `Fail`, `Inconclusive`, `NotApplicable`), diagnostic statistics, and methodological limits. Strictly **diagnostics-only**: never recommends alternative tests, ranks procedures, or triggers automatic exclusions/remediations. |
| **QOL-8** | **Journal figure dimension & typography lock presets** | `graph.applyCanvasPreset`, `layout.applyCanvasPreset`, `graph.preflightCheck` | Resizes graphs and page layouts to standard publication column targets (`88 mm`, `130 mm`, `180 mm`, full page, `16:9`, poster) with `ScaleGeometriesOnly` lock keeping point typography and hairline strokes intact, plus export preflight warnings below `5.0 pt` text or `0.25 pt` strokes. |
| **QOL-9** | **Live CVD & grayscale canvas preview** | `CanvasPreviewFilter` (`ui.canvas.previewFilter`) | Applies Machado (2009) `Deuteranopia`, `Protanopia`, `Tritanopia`, or `Grayscale` viewport filters non-destructively on the rendered canvas without modifying stored figure colours or exported files. |
| **QOL-12** | **Categorical & continuous analysis completions** | [`analyses.md`](analyses.md) | Adds `ResultTabKind::MultipleComparisons` post-hoc tabs inside `FactorialAnova`, weighted Cohen's $\kappa$ (`Unweighted`, `Linear`, `Quadratic`) & Fleiss' $\kappa$, inverted Altman `NNTB`/`NNTH` bounds from Newcombe RD CIs, McNemar discordant-pair OR CIs, custom expected proportions for `PartsOfWhole` $\chi^2$ GOF, exact one-sample Binomial & paired Sign tests, exact $\chi^2$ `sd_ci`, and direct `MeanSdN` / `MeanSemN` support in two-group and one-sample *t*-tests. |
| **QOL-14** | **Seeded randomization, subset sampling & distribution generation** | `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions` | Performs deterministic seeded complete or permuted-block group randomization (writing ARRIVE randomization keys into `InfoSheet`), uniform/stratified $k$-of-$N$ sampling without replacement, and seeded univariate distribution simulation over `DeterministicRng`. |
| **QOL-16** | **Live $\TeX$ model equation annotations** | `graph.addAnnotation` (`model_equation`), `ui.graph.insertModelEquation` | Places symbolic or live coefficient-substituted $\TeX$-subset regression equations on fitted `XY` graphs (`R10.19` Knuth TeXbook Appendix G 2D box typesetting across screen, SVG, PDF, and EMF), updating automatically when upstream fits recalculate. |

> [!NOTE]
> Instant table-free distribution/Bayesian calculators (`QOL-13`) and wet-lab molarity/dilution/radiochemical calculators (`QOL-15`), along with standalone browser-folder WASM collaboration extensions (`QOL-10`, `QOL-11`), are deferred outside the Beta release scope and are not registered as active commands.

See [Graphs & Page Layouts](graphs.md), [Analyses Reference](analyses.md), and [Command Reference](../commands.md) for command arguments and rendering details.
