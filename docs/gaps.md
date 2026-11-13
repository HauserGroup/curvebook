# Gaps

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** ranked `G1`–`G14` clean-room work list established from Prism 11.1 public guides · **Target:** GraphPad Prism 11 (11.1.x)

Every shortfall between LumenCraft (`M0`) and GraphPad Prism 11.1, **ranked by how much it blocks a working scientist from switching**. Unless the user assigns a specific task, pick the highest-ranked unblocked gap below, read its linked implementation guide first, implement and test it end to end, and update this file, [`target-app-parity.md`](target-app-parity.md), the domain parity checklist, and [`ROADMAP.md`](../ROADMAP.md) in the same PR.

Gap IDs (`G1`–`G14`) are permanent; ranks and hour estimates adjust as work lands. **Alpha** marks gaps required to pass the [Alpha gate](roadmap.md#alpha-gate); **Beta** marks gaps required for beta.

## Ranked

| Rank | Gap | Hours | Gate | Implementation guide & checklist |
|---:|---|---:|:---:|---|
| 1 | [G1 Numerical foundations & NIST StRD oracle](#g1-numerical-foundations-and-nist-strd-oracle) | 20–30 | Alpha | [statistics-implementation.md](statistics-implementation.md) |
| 2 | [G2 Typed data tables, reactive DAG & `.lumencraft` / CSV I/O](#g2-typed-data-tables-reactive-dag-and-lumencraft--csv-io) | 25–40 | Alpha | [data-tables-and-transforms.md](data-tables-and-transforms.md), [file-format-parity.md](file-format-parity.md) |
| 3 | [G3 Core biostatistics, ANOVA/REML, effect sizes & multiplicity](#g3-core-biostatistics-anovareml-effect-sizes-and-multiplicity) | 45–70 | Alpha | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 4 | [G4 Nonlinear regression solver, profile CIs & built-in equations](#g4-nonlinear-regression-solver-profile-cis-and-built-in-equations) | 50–78 | Alpha | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 5 | [G5 Publication chart compiler, broken/log axes & SVG/PDF export](#g5-publication-chart-compiler-brokenlog-axes-and-svgpdf-export) | 40–62 | Alpha | [graphing-and-layouts.md](graphing-and-layouts.md), [graphing-parity.md](graphing-parity.md) |
| 6 | [G6 Desktop & WASM shell: Navigator, subcolumn grid & inspectors](#g6-desktop-and-wasm-shell-navigator-subcolumn-grid-and-inspectors) | 45–70 | Alpha | [ui-parity.md](ui-parity.md) |
| 7 | [G7 Prism 11 Multiple Variables workflows, Calculated Variables & Multifactor ANOVA](#g7-prism-11-multiple-variables-workflows-calculated-variables-and-multifactor-anova) | 30–45 | Beta | [data-tables-and-transforms.md](data-tables-and-transforms.md), [statistics-implementation.md](statistics-implementation.md) |
| 8 | [G8 Survival (Kaplan–Meier, Cox PH), contingency CIs, ROC, GLMs & PCA](#g8-survival-kaplanmeier-cox-ph-contingency-cis-roc-glms-and-pca) | 35–55 | Beta | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 9 | [G9 Custom equation DSL, implicit/ODE models & derived transforms](#g9-custom-equation-dsl-implicitode-models-and-derived-transforms) | 25–40 | Beta | [curve-fitting-implementation.md](curve-fitting-implementation.md), [data-tables-and-transforms.md](data-tables-and-transforms.md) |
| 10 | [G10 Clean-room `.prism` & `.pzfx` project import](#g10-clean-room-prism-and-pzfx-project-import) | 20–35 | Beta | [file-format-parity.md](file-format-parity.md) |
| 11 | [G11 Multi-panel Page Layouts, style cloning & print/TIFF pipeline](#g11-multi-panel-page-layouts-style-cloning-and-printtiff-pipeline) | 18–28 | Beta | [graphing-and-layouts.md](graphing-and-layouts.md), [hardware-parity.md](hardware-parity.md) |
| 12 | [G12 Performance budgets, large-table virtualization & fuzzing](#g12-performance-budgets-large-table-virtualization-and-fuzzing) | 15–25 | Beta | [development.md](development.md) |
| 13 | [G13 Localization & regional scientific number formatting](#g13-localization-and-regional-scientific-number-formatting) | 35–55 | 1.0 | [localization-parity.md](localization-parity.md) |
| 14 | [G14 Headless batch CLI, `.lctemplate` pipelines & full MCP catalog](#g14-headless-batch-cli-lctemplate-pipelines-and-full-mcp-catalog) | 15–25 | Beta | [scripting-and-automation.md](scripting-and-automation.md), [mcp.md](mcp.md) |

---

## Details by gap

### G1 Numerical foundations and NIST StRD oracle
**Crates:** `crates/math`, `crates/testkit` · **Spec:** [`statistics-implementation.md` §1](statistics-implementation.md#1-numerical-foundations-cratesmath)
- **Done:** M0 crate structure and strict no-panic lint gates.
- **Missing:**
  - Regularized incomplete beta $I_x(a,b)$ and gamma $P(a,x), Q(a,x)$ accurate to $\le 10^{-13}$ relative error without `1.0 - cdf(x)` tail cancellation.
  - Exact tail/quantile evaluators for Student *t* (real df), Snedecor *F*, $\chi^2$, normal, noncentral *t*/*F*/$\chi^2$ (for standardized effect-size CIs), Studentized range $q(k,\nu)$, and multivariate Dunnett *t*.
  - Neumaier compensated summation (`NumAcc1`–`NumAcc4`), column-pivoted Householder QR, Golub–Kahan SVD, Cholesky, and seeded deterministic PRNG.
  - `cargo xtask corpus` downloader/verifier + `lumencraft-testkit` NIST StRD parser.

### G2 Typed data tables, reactive DAG and `.lumencraft` / CSV I/O
**Crates:** `crates/doc`, `crates/format`, `crates/engine` · **Spec:** [`data-tables-and-transforms.md`](data-tables-and-transforms.md)
- **Done:** M0 stubs.
- **Missing:**
  - All 8 `TableArchetype` data models, replicate subcolumn modes (`RawReplicates`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `MedianIqr`), non-destructive cell exclusions, freeze/unfreeze state, and `InfoSheet` hookable constants.
  - Topological dependency DAG (`DataSheet -> Transform/Analysis -> ResultSheet -> Graph -> Layout`) with cycle rejection, dirty-sub-DAG recalculation, and atomic undo/redo command journal in `lumencraft-engine`.
  - Lossless `.lumencraft` ZIP+JSON+CSV persistence (`prop_format` round-trip tests) and RFC-4180 CSV/TSV import/export.

### G3 Core biostatistics, ANOVA/REML, effect sizes and multiplicity
**Crates:** `crates/stats` · **Spec:** [`statistics-implementation.md` §2–§4](statistics-implementation.md#2-descriptive-statistics-normality--outlier-detection)
- **Missing:**
  - Descriptive statistics, normality/lognormality battery (D'Agostino–Pearson $K^2$, Shapiro–Wilk $W$, Anderson–Darling $A^{*2}$, KS), Grubbs & ROUT outlier identification.
  - Unpaired, Welch, paired, ratio-paired, lognormal, and one-sample *t*-tests + Cohen's $d$, Hedges' $g$, Glass's $\Delta$ with noncentral-$t$ CIs; Mann–Whitney $U$, Wilcoxon signed-rank, and two-sample KS (exact small-sample + tie-corrected asymptotic + Hodges–Lehmann CIs); decimal confidence levels (`0.01%..=99.99%`).
  - Ordinary, Welch/Brown–Forsythe, and RM one-way ANOVA (Geisser–Greenhouse $\hat{\varepsilon}$), Kruskal–Wallis, Friedman; two-way & three-way ANOVA (Type I/III SS) + $\eta^2, \eta_p^2$, Cohen's $f$; REML linear mixed-effects solver for missing repeated measures & nested tables.
  - Multiplicity: Tukey, Dunnett, Šídák, Bonferroni, Holm–Šídák, Dunnett T3, Games–Howell, Dunn, linear trend, and BH/BY/BKY two-stage FDR + original Analysis Checklists.

### G4 Nonlinear regression solver, profile CIs and built-in equations
**Crates:** `crates/fit` · **Spec:** [`curve-fitting-implementation.md` §1–§3](curve-fitting-implementation.md#1-nonlinear-solver-architecture-cratesfit)
- **Missing:**
  - Moré trust-region Levenberg–Marquardt (QR augmented step, avoiding $J^\top J$ squaring), Nelder–Mead simplex warm-start, differential weights ($1/Y, 1/Y^2, 1/X, 1/\text{SD}^2$, Poisson), Lorentzian/Cauchy robust regression, and ROUT outlier masking.
  - Box constraints, constant hooks, shared global parameters across datasets, Venzon–Moolgavkar asymmetrical profile-likelihood CIs (`CiBound::Open` on flat plateaus), confidence/prediction bands, standard-curve inverse interpolation, extra-sum-of-squares *F*-test, AICc + Akaike weights, replicate/runs/homoscedasticity diagnostics.
  - Built-in equations across Dose–Response (3PL/4PL/5PL/Biphasic/Bell/Operational/Schild/$\text{EC}_F$/Absolute $\text{IC}_{50}$), Receptor Binding, Enzyme Kinetics (Michaelis–Menten, $k_{\text{cat}}$, allosteric, competitive/noncompetitive/uncompetitive/mixed inhibition, Morrison tight-binding $K_i$), Exponentials, Lines/Deming/Polynomials, Peaks/Waves, Growth, and Linear–Quadratic radiation survival, verified against all 27 NIST StRD nonlinear datasets.

### G5 Publication chart compiler, broken/log axes and SVG/PDF export
**Crates:** `crates/chart`, `crates/text`, `crates/palette`, `crates/render`, `crates/svg`, `crates/pdf` · **Spec:** [`graphing-and-layouts.md`](graphing-and-layouts.md)
- **Missing:**
  - Deterministic `GraphSpec -> ChartScene` compiler for XY, Column (bar + beeswarm replicates, box, violin, before–after), Grouped, Contingency/Parts-of-Whole, and Kaplan–Meier Survival (with number-at-risk table).
  - Linear, $\log_{10}$, $\log_2$, $\ln$, $\text{logit}$, $\text{probit}$, and reciprocal axes; 2- and 3-segment discontinuous axes; solid and dashed error bars; scientific superscripts/subscripts & Greek symbols.
  - Live pairwise comparison brackets auto-stacked and hooked to upstream `ResultSheet` *p*/*q*-values.
  - Vector SVG (`crates/svg`), PDF 1.7 (`crates/pdf`), and 72–1200 DPI PNG raster (`crates/render`) export.

### G6 Desktop and WASM shell: Navigator, subcolumn grid and inspectors
**Crates:** `crates/ui-egui`, `crates/tools`, `apps/lumencraft`, `apps/lumencraft-web` · **Spec:** [`ui-parity.md`](ui-parity.md)
- **Done:** M0 desktop/web shell skeletons, i18n registry, and UI command sync.
- **Missing:**
  - Five-folder Project Navigator + Family dependency pane + Welcome / New Table dialog.
  - Virtualized 60 fps two-tier subcolumn spreadsheet grid with cell exclusions (`Cmd/Ctrl+E`), Excel/TSV paste-special, keyboard range selection stopping cleanly at empty boundaries, and WCAG AA contrast compliance.
  - Analyze parameter dialogs, multi-tab Result viewer with Analysis Checklist, interactive Graph canvas + Format Graph / Axes / Pairwise-Comparisons inspector.

### G7 Prism 11 Multiple Variables workflows, Calculated Variables and Multifactor ANOVA
**Crates:** `crates/doc`, `crates/transform`, `crates/stats`, `crates/fit`, `crates/chart`, `crates/ui-egui` · **Spec:** [`data-tables-and-transforms.md` §2.2 & §4](data-tables-and-transforms.md#22-categorical-level-ordering-multiplevariables)
- **Missing:**
  - In-table `=` Calculated Variables + aggregate/conditional/window functions with live column recalculation.
  - Categorical level ordering (entry, alphabetical, frequency, manual, reversed; independent graph level order).
  - Running classic *t*-tests, nonlinear regression, simple Kaplan–Meier survival, and $N$-way **Multifactor ANOVA** directly from `MultipleVariables` tables via typed/color-coded variable role assignment.
  - Categorical $\times$ continuous `MultipleVariables` graph types (bars, box-and-whisker, violin, column scatter) + bubble/convex-hull/confidence-ellipse multivariate plots.

### G8 Survival (Kaplan–Meier, Cox PH), contingency CIs, ROC, GLMs and PCA
**Crates:** `crates/stats` · **Spec:** [`statistics-implementation.md` §5](statistics-implementation.md#5-contingency-survival-roc--multivariate-analysis)
- **Missing:** Kaplan–Meier + log-rank / Gehan–Breslow–Wilcoxon / trend + Cox PH (Breslow/Efron ties, Schoenfeld residuals); Contingency $\chi^2$, Fisher's exact, Baptista–Pike OR CIs, Koopman RR CIs, Wilson binomial CIs, $\phi$ & Cramér's $V$ CIs; ROC (Hanley–McNeil / DeLong AUC CIs); Multiple linear, logistic & Poisson GLMs; PCA (Horn parallel analysis) & K-means / hierarchical clustering.

### G9 Custom equation DSL, implicit/ODE models and derived transforms
**Crates:** `crates/fit`, `crates/transform` · **Spec:** [`curve-fitting-implementation.md` §4](curve-fitting-implementation.md#4-user-defined-equation-language-cratesfitdsl)
- **Missing:** Safe bytecode VM for multiline/piecewise/dataset-scoped (`<A>`) custom equations, implicit root-solved equations, RK45 ODE integration, and standard derived transforms (`Normalize`, `Baseline`, `Transpose`, `Prune`, `Simulate`, `Smooth/Differentiate/AUC`).

### G10 Clean-room `.prism` and `.pzfx` project import
**Crates:** `crates/format` · **Spec:** [`file-format-parity.md`](file-format-parity.md)
- **Missing:** Clean-room importers built strictly from public format documentation (`.prism` ZIP/JSON/CSV archive and legacy `.pzfx` XML tables, subcolumns, titles, exclusions, and Info constants) with bounded decompression/XML depth and synthetic test builders in `lumencraft-testkit`.

### G11 Multi-panel Page Layouts, style cloning and print/TIFF pipeline
**Crates:** `crates/chart`, `crates/render`, `crates/pdf`, `crates/ui-egui` · **Spec:** [`graphing-and-layouts.md` §5–§6](graphing-and-layouts.md#5-consistent-styling-wand--clone-style--palettes)
- **Missing:** Multi-graph `LayoutSheet` composer with plot-origin alignment across panels, auto-lettering (`A`, `B`, `C`), `graph.applyStyleFrom` ("Wand"), uncompressed/LZW TIFF export, native vector clipboard copy, and OS print spooling.

### G12 Performance budgets, large-table virtualization and fuzzing
**Crates:** `crates/engine`, `crates/format`, `crates/mcp`, `crates/ui-egui` · **Spec:** [`development.md`](development.md#fuzzing)
- **Missing:** `lumencraft-cli bench` / `perf` budget suite (`100k`-row `MultipleVariables` formula eval $< 25\text{ ms}$, 96-curve 4PL batch fit $< 50\text{ ms}$, 60 fps grid scroll) and property/fuzz harnesses (`import_fuzz`, `command_sweep`, `prop_format`, `protocol_props`).

### G13 Localization and regional scientific number formatting
**Crates:** `crates/ui-egui`, `crates/format`, `crates/text` · **Spec:** [`localization-parity.md`](localization-parity.md)
- **Done:** Locale registry and `cargo xtask i18n-report`.
- **Missing:** Tier-1 translations, decimal comma vs. period formatting decoupled from OS locale, European `;`-CSV presets, RTL layout.

### G14 Headless batch CLI, `.lctemplate` pipelines and full MCP catalog
**Crates:** `apps/lumencraft-cli`, `crates/mcp`, `crates/engine` · **Spec:** [`scripting-and-automation.md`](scripting-and-automation.md)
- **Done:** CLI/MCP/control crate skeletons and `ui-commands` sync.
- **Missing:** `.lctemplate` instantiation, `lumencraft-cli batch` runner, and full MCP tool handlers (`doc_inspect`, `command_run`, `command_batch`, `render_preview`, `export`).
