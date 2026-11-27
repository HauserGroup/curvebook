# Calculators & quality-of-life capabilities

<!-- Audience: Tier 2 Technical Reference — exhaustive quick-statistics formulas, quality-of-life commands, and guard rules belong here; link to Tier 1 guides (desktop-gui.md) for scientist walkthroughs. -->

This technical reference specifies CurveBook's live selection statistics, quality-of-life commands, and non-destructive workflow guards. For a quick tour of using these tools in the workspace, see the [Desktop GUI guide](desktop-gui.md).

CurveBook augments its statistical and graphing engine with non-destructive workflow tools and live diagnostics. These tools keep raw measurements intact, preserve provenance, and report statistical assumptions objectively.

---

## Selection quick statistics {#quick-statistics}

Selecting a single cell or rectangular range in any data table computes live summary statistics over active numeric values inside the selection:

- **Counts:** `selected_cells` (total selected cells), `n` (active finite numbers), and `excluded` (excluded cells in the selection).
- **Moments & quantiles:** `sum`, `mean`, `median`, `min`, and `max`. Sample `sd` ($n - 1$ denominator) and `sem` ($\text{SD}/\sqrt{n}$) are omitted when $n < 2$.
- **Filtering rules:** Excluded cells, blank cells, text cells, and non-finite values are skipped from `n` and every summary moment. The source table is never modified.

---

## Active quality-of-life capabilities {#active-qol}

| Capability | Commands / UI entry | Summary |
|---|---|---|
| **Non-destructive archetype conversion** | `table.convertArchetype` | Converts data tables among Column, Grouped, XY, Multiple Variables, and Contingency with a non-mutating preview. |
| **Microplate & tidy-long reshaping** | `table.importPlateMap`, `table.importTidyLong` | Maps 96-well or 384-well plates, sheets, or analyzable results into typed tables and pivots tidy long rows. |
| **Auditable cell exclusion reasons** | `table.excludeCellsWithReason`, `table.exclusionAuditReport` | Attaches optional reason tags, notes, and timestamps to excluded cells while preserving plate well origins. |
| **Side-by-side split workspace** | `ui.split.toggle` | Docks a live graph or results pane beside the active sheet for immediate reactive updates. |
| **Two-way data-to-graph brushing** | Interactive canvas & grid | Links plotted points to source cells for hover details, split-pane highlighting, jump-to-cell, and quick exclusion. |
| **One-click methods report** | `result.generateReport` | Produces deterministic ICMJE/ARRIVE-aligned methods paragraphs in `plain`, `markdown`, or `latex`. |
| **Diagnostics-only assumption banners** | `ui.result.inspectDiagnostics` | Summarizes executed assumption checks objectively without recommending replacement tests or automatic edits. |
| **Journal figure presets & preflight** | `graph.applyCanvasPreset`, `layout.applyCanvasPreset`, `graph.preflightCheck` | Resizes figures to journal column widths while locking point typography and warning on tiny text or strokes. |
| **Live CVD & grayscale preview** | `ui.previewFilter.set` | Simulates color-vision deficiency and grayscale print on the canvas without changing stored colours or exports. |
| **Expanded statistical coverage** | [Analysis Kinds Registry](registry/tables.html#analysis-kinds) | Adds factorial ANOVA post-hoc tables, weighted/Fleiss' $\kappa$, NNTB/NNTH intervals, exact Binomial/Sign tests, and summary-data *t*-tests. |
| **Seeded randomization & simulation** | `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions` | Generates deterministic treatment schedules, stratified row samples, and simulated univariate distributions. |
| **Live $\TeX$ model equations** | `graph.addAnnotation`, `ui.graph.insertModelEquation` | Places symbolic or live coefficient-substituted $\TeX$ equations on fitted XY graphs. |

### Capability details & guards

- **Archetype conversion:** Preview reports preserved cells, dropped cells, preserved exclusions, warnings, and whether the conversion is lossy. Exclusion flags and exclusion notes are preserved and remapped. Lossy conversions require `allow_lossy: true`.
- **Interactive microplate editor:** The desktop **Import / Reshape** dialog offers a colour-coded 8×12 (96-well) and 16×24 (384-well) grid editor. It supports drag selection, role/group/constant-$X$ assignment, linear or multiplicative **Fill Dilution Series**, grid pasting, and sheet loading. Readings accept direct values, data tables (via `row_index`, `x_window_mean`, `min`, or `max`), or up-to-date analyzable results (such as Area Under the Curve `value_column`). Layouts save atomically as `.cbplate` files (see [`plate-map.schema.json`](../schemas/plate-map.schema.json)).
- **Exclusion and well traceability:** Plain **Cmd/Ctrl+E** exclusion toggles never require a reason. On plate-mapped tables, hovering or selecting a cell displays `Source well: <well>` (such as `Source well: G12`). Well origins stay intact across exclusions, row shifts, coordinate-remappable transforms, and `.curvebook` reloads.
- **Diagnostics-only policy:** Assumption banners cover normality, variance homogeneity, sphericity, proportional hazards, separation, collinearity, lack of fit, and runs checks. Each item reports status (pass, warn, fail, inconclusive, or not applicable), statistics, and methodological limits. CurveBook never recommends replacement tests or applies automatic edits.
- **Journal presets and preflight:** Standard targets cover 88 mm, 130 mm, 180 mm, full page, 16:9 slide, and poster panels. Locking point typography keeps font sizes and stroke weights intact during resizing. Export preflight warns on text below 5.0 pt or strokes below 0.25 pt.
- **Colour-vision preview:** Applies Machado (2009) deuteranopia, protanopia, tritanopia, and grayscale-print viewport filters. Stored figure colours and exported vector files stay untouched.
- **Expanded statistical routines:** Adds factorial ANOVA multiple-comparison tabs, weighted Cohen's $\kappa$ (unweighted, linear, quadratic), Fleiss' $\kappa$, and inverted Altman NNTB/NNTH bounds from Newcombe risk-difference intervals. Also includes McNemar discordant-pair OR intervals, custom expected proportions for parts-of-whole $\chi^2$ goodness-of-fit, exact one-sample Binomial and paired Sign tests, and exact $\chi^2$ SD intervals. One-sample and two-group *t*-tests also accept pre-summarized mean/SD/N or mean/SEM/N entry.
- **Randomization and simulation:** Complete and permuted-block randomization writes ARRIVE metadata into an info sheet. Uniform and stratified $k$-of-$N$ row sampling runs without replacement. Univariate distribution generation is deterministic given the recorded 64-bit seed.
- **Live $\TeX$ equations:** Uses two-dimensional math box typesetting (Knuth TeXbook Appendix G) across screen, SVG, PDF, and EMF exports. Substituted coefficients update automatically when upstream fits recalculate.

> [!NOTE]
> Standalone table-free distribution, Bayesian, molarity, dilution, and radiochemical calculators—along with browser-folder collaboration tools—are outside the Beta release scope and are not registered as active commands.

See [Graphs & Page Layouts](graphs.md), [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Command Reference](../commands.md) for command arguments and rendering details.
