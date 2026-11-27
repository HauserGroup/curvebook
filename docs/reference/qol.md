# Calculators and everyday tools

<!-- Audience: Tier 2 Technical Reference — exhaustive quick-statistics formulas, quality-of-life commands, and guard rules belong here; link to Tier 1 guides (desktop-gui.md) for scientist walkthroughs. -->

This technical reference describes CurveBook's live selection statistics, its everyday workflow commands, and the guards that keep those workflows non-destructive. For a quick tour of these tools in the workspace, see the [Desktop GUI guide](desktop-gui.md).

Alongside its statistical and graphing engine, CurveBook provides workflow tools and live diagnostics. These tools never alter raw measurements, they keep a record of where each value came from, and they report on statistical assumptions without making recommendations.

---

## Selection quick statistics {#quick-statistics}

When you select a single cell or a rectangular range in any data table, CurveBook computes live summary statistics over the active numeric values in the selection:

- **Counts:** `selected_cells` is the total number of selected cells, `n` is the number of active finite numbers, and `excluded` is the number of excluded cells in the selection.
- **Moments and quantiles:** `sum`, `mean`, `median`, `min`, and `max`. The sample `sd` (with an $n - 1$ denominator) and the `sem` ($\text{SD}/\sqrt{n}$) are omitted when $n < 2$.
- **Filtering rules:** Excluded cells, blank cells, text cells, and non-finite values do not count toward `n` or any summary moment. The source table is never modified.

---

## Everyday tools in this release {#active-qol}

| Capability | Commands / UI entry | Summary |
|---|---|---|
| Non-destructive archetype conversion | `table.convertArchetype` | Converts a data table between the Column, Grouped, XY, Multiple Variables, and Contingency archetypes, with a preview that changes nothing. |
| Microplate and tidy-long reshaping | `table.importPlateMap`, `table.importTidyLong` | Maps 96-well or 384-well plates, sheets, or analyzable results into typed tables, and pivots tidy long-format rows. |
| Exclusion reasons for audits | `table.excludeCellsWithReason`, `table.exclusionAuditReport` | Attaches optional reason tags, notes, and timestamps to excluded cells while keeping their plate well origins. |
| Side-by-side split workspace | `ui.split.toggle` | Docks a live graph or results pane beside the active sheet, so you see updates as soon as the data change. |
| Two-way linking between data and graph | Interactive canvas and grid | Links plotted points to their source cells. You can hover for details, see highlights in the split pane, jump to the cell, or exclude the point quickly. |
| One-click methods report | `result.generateReport` | Writes reproducible methods paragraphs aligned with ICMJE and ARRIVE guidance, in `plain`, `markdown`, or `latex` format. |
| Assumption banners (diagnostics only) | `ui.result.inspectDiagnostics` | Summarizes the assumption checks that ran, without recommending replacement tests or editing anything automatically. |
| Journal figure presets and preflight | `graph.applyCanvasPreset`, `layout.applyCanvasPreset`, `graph.preflightCheck` | Resizes figures to journal column widths while keeping text at fixed point sizes, and warns about tiny text or thin strokes. |
| Live colour-vision (CVD) and grayscale preview | `ui.previewFilter.set` | Simulates colour-vision deficiency and grayscale printing on the canvas without changing stored colours or exports. |
| Expanded statistical coverage | [Analysis Kinds Registry](registry/tables.html#analysis-kinds) | Adds post-hoc tables for factorial ANOVA, weighted and Fleiss' $\kappa$, NNTB/NNTH intervals, exact Binomial and Sign tests, and *t*-tests from summary data. |
| Seeded randomization and simulation | `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions` | Generates reproducible treatment schedules, stratified row samples, and simulated univariate distributions. |
| Live $\TeX$ model equations | `graph.addAnnotation`, `ui.graph.insertModelEquation` | Places $\TeX$ equations on fitted XY graphs, either in symbolic form or with the live coefficient values substituted. |

### Details and safeguards

- **Archetype conversion:** The preview reports which cells are preserved, which are dropped, which exclusions are preserved, any warnings, and whether the conversion is lossy. Exclusion flags and exclusion notes are preserved and moved to their new positions. A lossy conversion requires `allow_lossy: true`.
- **Interactive microplate editor:** The desktop **Import / Reshape** dialog offers a colour-coded grid editor for 8×12 (96-well) and 16×24 (384-well) plates. In the grid you can:
  - drag to select wells;
  - assign a role, group, or constant $X$;
  - fill a linear or multiplicative dilution series with **Fill Dilution Series**;
  - paste a grid or load one from a sheet.
- **Plate readings and layouts:** Readings can come from direct values, from data tables (using `row_index`, `x_window_mean`, `min`, or `max`), or from up-to-date analyzable results (such as the `value_column` of an Area Under the Curve result). Layouts are saved atomically as `.cbplate` files (see [`plate-map.schema.json`](../schemas/plate-map.schema.json)).
- **Exclusions and well traceability:** Toggling an exclusion with plain **Cmd/Ctrl+E** never asks for a reason. On a plate-mapped table, hovering over or selecting a cell shows its origin as `Source well: <well>` (for example, `Source well: G12`). Well origins stay intact through exclusions, row shifts, transforms whose coordinates can be remapped, and reloads of the `.curvebook` file.
- **Diagnostics-only policy:** Assumption banners cover normality, homogeneity of variance, sphericity, proportional hazards, separation, collinearity, lack of fit, and runs checks. Each item reports a status (pass, warn, fail, inconclusive, or not applicable), its statistics, and its methodological limits. CurveBook never recommends a replacement test and never edits your analysis automatically.
- **Journal presets and preflight:** The standard targets are 88 mm, 130 mm, and 180 mm columns, a full page, a 16:9 slide, and poster panels. Locking point typography keeps font sizes and stroke weights unchanged when a figure is resized. Export preflight warns about text smaller than 5.0 pt and strokes thinner than 0.25 pt.
- **Colour-vision preview:** The viewport filters simulate deuteranopia, protanopia, and tritanopia (Machado 2009) and grayscale printing. Stored figure colours and exported vector files are not changed.
- **Expanded statistical routines:** One-sample and two-group *t*-tests now also accept pre-summarized data entered as mean/SD/N or mean/SEM/N. This release also adds:
  - multiple-comparison tabs for factorial ANOVA;
  - Cohen's $\kappa$ with unweighted, linear, or quadratic weights, and Fleiss' $\kappa$;
  - NNTB/NNTH bounds obtained by inverting Newcombe risk-difference intervals (Altman's method);
  - McNemar odds-ratio intervals based on discordant pairs;
  - custom expected proportions for the parts-of-whole $\chi^2$ goodness-of-fit test;
  - the exact one-sample Binomial test and the exact paired Sign test;
  - exact $\chi^2$ intervals for the SD.
- **Randomization and simulation:** Complete and permuted-block randomization write ARRIVE metadata into an info sheet. Uniform and stratified $k$-of-$N$ row sampling draws without replacement. Generated univariate distributions are reproducible from the recorded 64-bit seed.
- **Live $\TeX$ equations:** Equations are typeset with two-dimensional math boxes (Knuth, *The TeXbook*, Appendix G) on screen and in SVG, PDF, and EMF exports. Substituted coefficients update automatically when the upstream fit recalculates.

> [!NOTE]
> Standalone calculators that do not use a table (distribution, Bayesian, molarity, dilution, and radiochemical calculators) and the browser-folder collaboration tools are outside the scope of the Beta release. They are not registered as active commands.

See [Graphs & Page Layouts](graphs.md), the [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and the [Command Reference](../commands.md) for command arguments and rendering details.
