# Data Tables, Calculated Variables & Transforms — Implementation Guidelines

Governing specification for `crates/doc` (`L1`), `crates/transform` (`L2`), and data-sheet commands in `crates/engine` (`L5`). Parity checklist: [`evidence/file-formats.md`](../evidence/file-formats.md) and [`evidence/user-interface.md`](../evidence/user-interface.md).

Clean-room data-model & interchange specifications:
- [`crates/doc`](../../crates/doc): Eight typed table archetypes (`TableArchetype`), `SubcolumnMode`, non-destructive cell exclusions, and freeze state.
- [`crates/transform`](../../crates/transform): Calculated variable formula engine, baseline/normalization transforms, and plate/long-wide reshaping.
- [`docs/file-formats.md`](../file-formats.md): Native `.curvebook` / `.curvebook.d/` containers, CSV/TSV, and bounded `.pzfx` / `.prism` interchange rules.

---

## 1. Core Architectural Invariants

1. **Typed experimental semantics, never untyped A1 spreadsheets.** Every `DataSheet` in `curvebook-doc` carries an explicit `TableArchetype` variant that governs valid subcolumn layouts, compatible analyses, and default graph types.
2. **Non-destructive exclusions and missingness.** Every cell is an explicit enum (`CellValue::Number(f64)`, `CellValue::Text(String)`, `CellValue::Empty`) paired with an `excluded: bool` mask. Excluded cells remain visible in the grid (styled in blue italics with an `*` badge) and remain stored in `.curvebook` files, while downstream transforms, statistical tests, curve fits, and graphs treat them as masked observations without mutating raw values.
3. **Bounded untrusted dimensions.** Project loaders, CSV importers, clipboard paste handlers, and command parameters enforce explicit caps (`MAX_ROWS = 1_000_000`, `MAX_DATA_SETS = 1_024`, `MAX_SUBCOLUMNS = 512`, `MAX_TITLE_BYTES = 4_096`) and return `DocError` / `EngineError` rather than panicking or exhausting memory.
4. **Freeze state.** A `DataSheet` exposes `frozen: bool` (the Freeze/Unfreeze table workflow). When `frozen` is `true`, cell-edit, paste, row/column insertion, and formula-edit commands return `DocError::SheetFrozen` (surfaced as `ErrorCode::FrozenSheet`, `"frozen_sheet"`) while read-only inspection, downstream analyses, and graph styling remain active.

---

## 2. The Eight Data Table Archetypes (`curvebook_doc::TableArchetype`)

| Archetype | Row Semantics | X Column | Data-Set Columns (`A`, `B`, …) | Subcolumn Modes | Primary Downstream Analyses |
|---|---|---|---|---|---|
| **`Xy`** | Continuous independent point or time step | Numeric (`f64`), numeric elapsed-time / serial series with `DateOrElapsedTime` unit-label metadata (no `chrono` calendar parsing), or generated arithmetic/geometric sequence (`X0`, `ΔX`), with optional error subcolumns | Continuous response series | `Single`, `RawReplicates(n)`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr` | Simple linear/logistic regression, nonlinear curve fitting, correlation, AUC, spline/LOWESS, XY graphs |
| **`Column`** | Unmatched replicate observation (or matched subject row when paired) | Optional row labels only | One treatment/group per column | `Single` (replicates stacked down rows) or pre-summarized `MeanSdN` / `MeanSemN` / `MeanCvN` | Unpaired/paired/Welch/one-sample *t*-tests, Mann–Whitney, Wilcoxon, ordinary/Welch/RM one-way ANOVA, Kruskal–Wallis, Friedman, descriptive stats, normality, Grubbs/ROUT outliers |
| **`Grouped`** | Factor A (row factor, e.g. time or dose tier) | Optional row titles / numeric spacing | Factor B (column factor, e.g. treatment or genotype) | `RawReplicates(n)` (stacked or side-by-side) or pre-summarized `MeanSdN` / `MeanSemN` | Two-way & three-way ANOVA (ordinary, repeated-measures, mixed-effects REML), multiple *t*-tests (one per row) |
| **`Contingency`** | Categorical outcome / exposure row | Row category title | Categorical outcome / exposure column | `Single` non-negative integer counts (`u64` / integral `f64 >= 0`) | Pearson $\chi^2$, Fisher's exact test, McNemar's test, Cochran–Armitage trend, RR/OR/sensitivity/specificity CIs |
| **`Survival`** | Individual subject / event time | Elapsed time $t \ge 0$ | One cohort column per group (`1` = event/death, `0` = censored; configurable codes) | `Single` status code per cohort column | Kaplan–Meier product-limit survival, log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon, trend log-rank |
| **`PartsOfWhole`** | Mutually exclusive slice / category | Slice label | Total / fractional series | `Single` non-negative values | Fraction of total, binomial / $\chi^2$ goodness-of-fit, pie / donut / stacked-bar graphs |
| **`MultipleVariables`** | One observational unit (tidy row) | None (all columns are named variables) | Typed variables (`Continuous`, `Categorical`, `Label`, or `Calculated`) | `Single` per variable; explicit categorical level-ordering metadata | Multiple linear/logistic/Poisson regression, Cox PH, PCA, correlation matrix, clustering, plus unified *t*-tests, Multifactor ($N$-way) ANOVA, nonlinear regression, and simple survival |
| **`Nested`** | Replicate within subcolumn (technical replicate) | Optional row label | Experimental group (`A`, `B`, …) | Biological units (`A:1`, `A:2`, …; e.g. animals/donors) containing technical replicates down rows | Nested *t*-test, nested one-way ANOVA, variance-component partitioning (mixed model) |

### 2.1 Pre-Summarized Error Subcolumn Rules

When a user enters pre-averaged data (`MeanSdN`, `MeanSemN`, `MeanCvN`) instead of raw replicates:
- Statistical tests and nonlinear regression reconstruct the within-point sum of squares rigorously from $(M_i, \text{SD}_i, n_i)$:
  $$\text{SS}_{\text{within}, i} = (n_i - 1)\,\text{SD}_i^2, \qquad \text{SD}_i = \begin{cases} \text{SEM}_i \sqrt{n_i} & \text{if entered as SEM}, \\ \frac{\%\text{CV}_i}{100}\,|M_i| & \text{if entered as \%CV}. \end{cases}$$
- Points with $n_i < 1$, negative SD/SEM, or missing $n_i$ when degrees of freedom depend on $n_i$ are flagged as invalid for parametric inference rather than silently treated as $n_i = 1$.
- Nonparametric rank tests, outlier tests, normality tests, and robust regression **refuse** pre-summarized `Mean/SD/N` tables with a clear `StatsError::RequiresRawReplicates` diagnostic, because rank order and heavy-tailed residuals cannot be recovered from sample moments.

### 2.2 `Column` Design Semantics & `Grouped` 3D Tensor Coordinates

- **`Column` unpaired vs. matched row-index semantics (`ColumnDesignHint`):** Although unpaired and matched `Column` sheets share the same 2D `[row][column]` grid, `ColumnDesignHint` (`UnpairedReplicates`, `PairedRepeatedMeasures`, `PrecomputedError`) governs both UI layout and default downstream defaults:
  - In `UnpairedReplicates` (default), row index $r$ is an arbitrary vertical stack slot: permuting observations within column `A` alone leaves all unpaired test statistics invariant, unequal column lengths ($n_A \ne n_B$) are ordinary unbalanced groups, the `Row Titles` column starts collapsed, and the auto-created graph defaults to a summary bar/column-scatter plot.
  - In `PairedRepeatedMeasures`, row index $r$ identifies a shared experimental block/subject measured across columns `A, B, …`: permuting a single column changes paired-*t* / RM-ANOVA statistics, missing cells indicate incomplete blocks (handled by pair dropping in two-group tests or REML mixed-effects in $\ge 3$ groups), the `Row Titles` column starts expanded for subject identifiers, and the auto-created graph defaults to a before–after individual-value line plot. Expanding or collapsing the `Row Titles` divider is always non-destructive and reversible on live tables.
- **`Grouped` 3D coordinate tensor:** Every observation $y_{r, c, s}$ in a `Grouped` table is addressed by `(Row Factor r, Column Factor c, Subcolumn Replicate s)`:
  - Two-Way ANOVA main effects ($F_{\text{Row}}, F_{\text{Col}}$) and interaction ($F_{\text{Row}\times\text{Col}}$) are mathematically symmetric under swapping Row and Column factors, whereas default graph encoding binds `Column Factor c` to series color/legend and `Row Factor r` to X-axis cluster position.
  - Three-Way ANOVA (`2 × 2 × K`) maps two binary factors across four column groups (`(F1_0, F2_0)`, `(F1_0, F2_1)`, `(F1_1, F2_0)`, `(F1_1, F2_1)`) and the $K$-level factor across rows; higher-level multi-way designs ($3 \times 3 \times 3$, etc.) route to `MultipleVariables` Multifactor GLM.

### 2.3 `MultipleVariables` Role Inference, Bulk Recoding & Categorical Level Ordering

Each column in a `MultipleVariables` sheet stores a `VariableRole` (`Continuous`, `Categorical`, `Label`, or `Calculated`) paired with `role_inferred: bool` (displaying a muted badge when auto-inferred on paste/import vs. an accented badge once explicitly confirmed by the user):
- **Bulk categorical level recoding (`table.recodeLevels`):** Renaming a category token in the Variables inspector (for instance correcting `"Hzel"` to `"Hazel"`) rewrites matching non-excluded and excluded cells across the column atomically and merges the two levels' frequency counts in a single undoable transaction.
- **Categorical ordering (`LevelOrderSpec`):** Each `Categorical` column stores a `LevelOrderSpec { order: LevelOrderKind, reversed }` (`crates/doc/src/archetypes.rs`) where `LevelOrderKind` is one of:
  - `EntryOrder` (order of first appearance in the column),
  - `Alphabetical` (natural case-insensitive UTF-8 sort),
  - `FrequencyDescending` / `FrequencyAscending` (count of non-excluded rows),
  - `Explicit(Vec<String>)` (manual drag-ordered sequence; unseen levels append deterministically in entry order),
  - plus the `reversed: bool` modifier.

Graphs referencing a `MultipleVariables` sheet may either inherit the table's `LevelOrderSpec` or override display order per axis/legend without mutating the source table's contrast reference level.

---

## 3. Info Sheets, Floating Notes & Hookable Constants

Every `DataSheet` is paired with (or shares) a two-pane `InfoSheet` containing:
1. Structured key–value **Info Constants** (`name: String`, `value: ConstantValue`, `units: Option<String>`) on the left pane, pre-seeded on new projects with standard laboratory keys (`Experiment Date`, `Experiment ID`, `Notebook ID`, `Project`, `Experimenter`, `Protocol`) alongside user-appended rows,
2. Freeform Rich/Markdown experimental notes on the right pane.

In addition, any sheet in a project (`DataSheet`, `InfoSheet`, `ResultSheet`, `GraphSheet`, `LayoutSheet`) may carry zero or more collapsible **`FloatingNote`** callouts (`id`, `fill_color`, `bounds_pt`, `minimized: bool`, rich-text body) for inline methodological reminders and outlier audit trails.

Info constants and upstream `ResultSheet` scalars (such as a fitted $K_d$ from a saturation-binding sheet or a control mean from a descriptive-stats sheet) participate in the engine's **Constant Hooking** graph (`HookSource`):
- Nonlinear regression constraints (e.g., fixing radioligand concentration `HotNM` or Michaelis constant `Km` in a competitive-inhibition fit),
- Standard-curve dilution factors and baseline offsets,
- Dynamic axis titles, callout text boxes, and layout headers (`{{info:Ligand}}`, `{{result:fit-1:EC50}}`).

`curvebook-engine` validates all constant hooks through its topological DAG before execution; circular parameter/constant references are rejected with `ErrorCode::CycleDetected` (`"cycle_detected"`, from `FormatError::DependencyCycle` / link-cycle detection in `crates/engine/src/dag.rs`) and never recurse infinitely (preventing the cyclic-hook corruption class).

---

## 4. In-Table Calculated Variables & Formula Engine (`MultipleVariables`)

In `MultipleVariables` sheets, a column may declare `VariableSource::Calculated { formula: String }`, evaluated column-wise across all rows (`1..=row_count`) whenever upstream columns change.

### 4.1 Formula Grammar & Evaluation Rules

- Formulas begin with `=` in UI entry (stored normalized without leading `=`).
- Variable references resolve column names (`[Body Mass] / ([Height_m]^2)` or bare identifiers when unambiguous). Self-references and multi-column cycles are rejected during DAG compilation.
- Missing (`Empty`), excluded, or non-numeric cells propagate `Empty` (`NaN` internally mapped back to `CellValue::Empty` with a diagnostic badge, never panicking on `0.0 / 0.0` or `ln(-1.0)`).
- Arithmetic, scalar, special, text & seeded random functions are evaluated by the `curvebook-transform` formula engine (`crates/transform/src/formula/`; the curve-fit equation DSL in `crates/fit/src/dsl/` is a separate compiler with an overlapping function set): `abs`, `sqrt`, `cbrt`, `sqr`, `pow`, `mod`, `ln`, `log`/`log10`, `log2`, `exp`, `exp10`, `pi([n])`, `sin`, `cos`, `tan`, `asin`/`arcsin`, `acos`/`arccos`, `atan`/`arctan`, `atan2(y, x)`/`arctan2(y, x)`, `sinh`, `cosh`, `tanh`, `asinh`/`arcsinh`, `acosh`/`arccosh`, `atanh`/`arctanh`, `rad`, `deg`, `perc`, `fact`, `erf`, `erfc`, `gamma`, `lgamma`/`gammaln`/`lngamma`, `igamma`, `igammac`, `beta`, `ibeta`, `psi`/`digamma`, `besselj`, `bessely`, `besseli`, `besselk`, `hypgeometricm`/`hyp1f1`, `hypgeometricu`/`confluent_u`, `hypgeometricf`/`hyp2f1`, `logit`, `probit` [$5 + \Phi^{-1}(p)$], `norm_ppf`/`norm_inv`/`qnorm`/`inv_norm`, `arcsin_sqrt`, `round`, `floor`, `ceil`, `trunc`, `int`, `sign`/`sgn`, `clamp`, `min`, `max`, `concatenate`/`concat` (with bounded materialized text budget), `gauss` / `rnd` (deterministic seeded PRNG), `nan`/`null`, `if`, `and`, `or`, `not`, `isnan`, `isvalid`/`is_defined`, `coalesce`.
- **Unlinking calculated variables (`table.unlinkCalculatedVariable`):** Freezes evaluated cells of a `VariableSource::Calculated` column into `VariableSource::Entered` (preserving computed values as static editable cells with full undo/redo and reactive DAG invalidation via `curvebook_transform::formula::unlink_calculated_variable`).
- **Aggregate & conditional column functions** (available in Calculated Variables and `SelectAndTransform`, disallowed inside nonlinear regression models):
  - `count(var)` / `counta(var)`, `sum(var)`, `mean(var)`, `sd(var)` / `stdev(var)`, `sem(var)` / `stderr(var)`, `var(var)`, `median(var)`, `iqr(var)`, `min_col(var)`, `max_col(var)`, `geo_mean(var)`, `percentile(var, p)` / `quantile(var, q)`
  - Conditional aggregations: `countif(condition_expr)` or `countif(value_expr, condition_expr)`, `countaif(value_expr, condition_expr)`, `sumif(value_expr, condition_expr)`, `meanif(value_expr, condition_expr)`, `minif(value_expr, condition_expr)`, `maxif(value_expr, condition_expr)`, `stdevif(value_expr, condition_expr)` / `sdif(value_expr, condition_expr)`, `stderrif(value_expr, condition_expr)` / `semif(value_expr, condition_expr)` (backwards-compatible `(condition_expr, value_expr)` argument ordering is also accepted when unambiguous)
- **Row-wise standardization, rank & window functions:**
  - `center(var)` ($x_i - \bar{x}$), `standardize(var)` / `zscore(var)` ($(x_i - \bar{x}) / s_x$)
  - `row_number()`, `rank([var])`, `dense_rank([var])`, `lag(var [, k])`, `lead(var [, k])`, `cumsum(var)`, `cummean(var)`, `cumprod(var)`, `diff(var [, k])`, `moving_avg(var, window)` / `rolling_mean(var, window)`, `rolling_sum(var, window)`
- **Window clauses (`OVER`, `WHERE`, `ORDER BY`, `ROWS`):**
  - Any aggregate, conditional aggregate, or row-wise window/standardization/rank function accepts optional non-destructive row filtering (`FILTER (WHERE cond)` or `WHERE cond`) and/or `OVER ([PARTITION BY expr, ...] [WHERE cond] [ORDER BY expr [ASC|DESC] [NULLS FIRST|LAST], ...] [ROWS frame])`.
  - `ROWS` supports `ROWS <bound>` and `ROWS BETWEEN <start> AND <end>` with bounds `UNBOUNDED PRECEDING`, `<n> PRECEDING`, `CURRENT ROW`, `<n> FOLLOWING`, and `UNBOUNDED FOLLOWING`. For aggregates, `ORDER BY` without an explicit `ROWS` clause defaults to SQL cumulative peer framing (`RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`), while omitting both `ORDER BY` and `ROWS` evaluates over the full partition.
  - Non-destructive `WHERE` filtering restricts which rows contribute to aggregates (preserving output at all rows in the partition for expressions such as `Y / mean(Y) WHERE Role == "Ctrl" OVER (PARTITION BY Batch)`) and masks filtered-out rows to `Empty` in row-wise window functions (`center`, `standardize`, `zscore`, `rank`, `dense_rank`, `row_number`, `cumsum`, `cummean`, `lag`, `lead`, `diff`, `moving_avg`).
  - `crates/transform/src/formula/dag.rs` tracks all column and `InfoSheet` dependencies across `PARTITION BY`, `ORDER BY`, `WHERE`/`FILTER`, and `ROWS` offset expressions for topological ordering and cycle detection.

---

## 5. Derived Transforms (`crates/transform`)

Transforms never overwrite a `DataSheet` in place; they execute as pure functions (`execute_transform(&DataSheet, &TransformAnalysisSpec, &TransformContext) -> Result<TransformOutput, TransformError>` and `execute_analysis_transform(…) -> Result<ResultSheet, TransformError>` in `crates/transform/src/ops/mod.rs`) inside the reactive DAG so downstream analyses and graphs can bind to transformed outputs while raw data stay untouched:

1. **Mathematical transforms (`TransformAnalysisSpec::Math`, `MathTransformPreset`):**
   - Standard presets on $X$ and/or $Y$: $\ln(v)$, $\log_{10}(v)$, $\log_2(v)$, $10^v$, $e^v$, $1/v$, $\sqrt{v}$, $v^2$, $\text{logit}(v) = \ln(v/(1-v))$, $\text{probit}(v) = 5 + \Phi^{-1}(v)$, $\arcsin(\sqrt{v})$, linear scale $a v + b$, pharmacology concentration conversions ($\text{M} \leftrightarrow \text{mM}, \mu\text{M}, \text{nM}, \text{pM}$ and $-\log_{10}[\text{M}] = \text{pEC}_{50}$).
   - Error propagation preserves table subcolumn schemas: `MeanSdN`, `MeanSemN`, and `MeanCvN` keep `[Mean, Spread, N]` (evaluating first-order delta-method $|f'(M)| \cdot \text{SD}$ or `ReplicateEndpoints` symmetric half-width $\tfrac{1}{2}|f(M+\text{SD}) - f(M-\text{SD})|$, converting `MeanCvN` via $\text{SD} = (\text{CV}/100)|M|$); `MeanLowHigh` retains asymmetric offsets (`-Error`, `+Error`) and swaps them whenever $f'(M) < 0$ or transformed endpoints reverse order; `UpperLowerLimits` and `MedianIqr` transform center and bounds independently, sorting or swapping lower/upper bounds on negative slope while keeping exclusion flags/notes synchronized. Whenever an endpoint, derivative, zero CV denominator (`mean_out == 0`), or limit lies outside the domain and blanks an active spread/limit cell, a non-fatal warning is emitted.
2. **Normalize (`TransformAnalysisSpec::Normalize`):**
   - Per-dataset or global `0%` anchor (minimum, `0.0`, first/last row, mean of control column, or hooked constant) and `100%` anchor (maximum, sum, last/first row, or hooked constant), outputting percentages or fractions `[0, 1]` with optional clamping/stretching, reversing asymmetric error bounds (`MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`) and their exclusion notes whenever normalization slope is negative.
3. **Remove Baseline and Column Math (`TransformAnalysisSpec::RemoveBaseline`, `BaselineKind`):**
   - Supports all seven baseline definitions end-to-end (`ColumnsRowWise`, `ColumnsMean`, `Constant`, `RowRangeMean`, `FirstAndLastLinear { first_k, last_k }`, `OlsLinearWithX`, and `PairedEveryOtherColumn`) across difference, ratio, percent-change, and fractional-change operations. Under `FirstAndLastLinear`, the initial anchor $(x_{\text{start}}, y_{\text{start}})$ is the arithmetic centroid of $(r, y)$ across all active numeric observations in rows $0 \dots \text{first\_k}-1$, and $(x_{\text{end}}, y_{\text{end}})$ is the centroid across rows $(R - \text{last\_k}) \dots R-1$ (falling back to constant $\tfrac{1}{2}(y_{\text{start}} + y_{\text{end}})$ when $x_{\text{end}} = x_{\text{start}}$). Negative baseline ratios swap asymmetric lower/upper bounds and their exclusion notes.
4. **Row Statistics (`AnalysisKind::RowStatistics`), Fraction of Total (`AnalysisKind::FractionOfTotal`), and Curve Calculus (`AnalysisKind::CurveCalculus`):**
   - `RowStatistics` collapses per-row replicates into chainable `analyzable_data` tables supporting row totals, mean + SD/SEM/%CV/CI/min-max, median + quartiles/custom Hyndman–Fan Type-6 & Type-7 percentiles, and geometric mean + geometric SD factor / CI.
   - `FractionOfTotal` computes row-, column-, grand-total, or all-three proportions/percentages with optional Wilson score or exact Clopper–Pearson binomial confidence intervals and chainable `analyzable_data` output.
   - `CurveCalculus` promotes `crates/transform/src/curve.rs` operations into a first-class XY analysis (`FirstDerivative`, `SecondDerivative` via shifted Lagrange differentiation on uniform and non-uniform grids, `CumulativeIntegral` with configurable baseline, `SmoothSavitzkyGolay`, `SmoothMovingAverage`, and `SmoothLowess`), exposing `analyzable_data` for downstream regression or AUC chaining.
5. **Transpose, Prune Rows, Select & Transform, and Stratified Randomization:**
   - Swap rows $\leftrightarrow$ data sets while preserving titles; decimate or bin rows (`Transpose`, `PruneRows`) and filter/derive/project columns (`SelectAndTransform`) while remapping cell exclusion notes (carrying `X` / `Time` as the first projected column when converting `Xy` or `Survival` tables into general tables, or warning for count/proportion targets).
   - Seeded subject randomization and stratified row sampling (`crates/transform/src/randomize.rs`): Hamilton largest-remainder apportionment distributes leftovers in descending exact remainder order with **ascending arm/stratum-index tie-breaking**; stratified proportional allocation enforces min-1 clamping ($l_h = 1$ when $S \ge H$ and $N_h \ge 1$) and capacity capping ($n_h \le N_h$) iteratively before re-apportioning active strata; equal allocation fills strata via water-filling with ascending-index remainder tie-breaks.
6. **Simulate XY / Column / Contingency Data (`TransformAnalysisSpec::Simulate`) & L5 Chained Monte Carlo (`AnalysisKind::MonteCarloSimulation`):**
   - Deterministic seeded PRNG (`curvebook_math::rng::DeterministicRng`) supports XY curve, multi-group column, and $2 \times 2$ contingency table simulation via `simulation_mode: SimulationMode::{XyCurve, Column, Contingency2x2}` across `SimulationNoise` variants (`None`, `Gaussian`, `RelativePercent`, `Poisson`) for validation, power exploration, and teaching workflows.
   - `crates/engine/src/monte_carlo.rs` orchestrates bounded deterministic chained Monte Carlo loops (`MAX_MC_ITERATIONS = 100_000`, step/cancellation guard) that re-simulate upstream tables per iteration, execute any downstream `Transform` / `Stats` / `Fit` analysis, collect hooked scalar outputs, apply `FitFlagRules::exclude_flagged_from_summary` (excluding ambiguous/non-converged fits cleanly), evaluate `"Hit"` predicates, and emit `Hits`, `Not Hits`, and `All` summary and per-iteration result tabs plus chainable `analyzable_data`.
7. **AUC & Spline/LOWESS Curve Fitting (`crates/transform/src/curve.rs`):**
   - `curve.rs` backs first-class `AnalysisKind::AreaUnderCurve` (`R17.7`, trapezoidal AUC with partial-window interpolation and Bailer SE), `AnalysisKind::CurveCalculus` (`R23.3`), and XY `FitSplineLowess` (`R18.7`; Natural Cubic Spline, Akima subspline, knot-controlled Smoothing Spline, Coarse/Medium/Fine LOWESS, log-aware point-to-point curves, and nonparametric standard-curve unknown interpolation). Theoretical distribution plotting (`Plot a function` / `plot_distribution`) and model-free spline/LOWESS fitting are specified in [`curve-fitting-implementation.md`](curve-fitting-implementation.md#6-model-free-curve-fitting-spline--lowess--theoretical-distribution-plotting) §6.

---

## 6. Live Linked Data Regions, Import Filtering & Reactive Transposition

Beyond derived `ResultSheet` transforms, a `DataSheet` may embed live, provenance-tracked **`LinkedDataRegion`** cell ranges (`table.pasteLink` and the 4-stage `Import & Paste Special` pipeline):

1. **Four-stage import & link specification (`ImportLinkSpec`):**
   - **Source:** Clipboard buffer, internal upstream `DataSheet` / `ResultSheet` range, or external delimited text file (`CSV` / `TSV` path + delimiter + decimal locale `,` vs `.`).
   - **Filter & Reshape:** Row/column window slicing (`start..=end`), periodic row decimation (every $k$-th row), long-to-wide `Unstack` grouping, and **inline asterisk exclusion parsing** (tokens formatted as `45.6*` or `*45.6` parse directly as `CellValue::Number(45.6)` with `excluded = true`).
   - **Placement & Orientation:** Overwrite, insert, or append at anchor cell `(row, col, subcol)`, optional `skip_contiguous_blanks`, and optional **`transpose: bool`**.
   - **Provenance & Notes:** Automatic audit entry in the target `InfoSheet` recording source path/sheet, filter slice, and timestamp.
2. **Color-coded provenance borders in the grid:**
   - **Blue border:** Live link to an internal `DataSheet` cell range.
   - **Green border:** Live link to an analyzable `ResultSheet` (such as `Transform`, `Normalize`, or `Transpose`).
   - **Red border:** Live link to a terminal statistical/regression `ResultSheet` parameter cell.
   - **Charcoal border:** Live link to an external CSV/TSV file on disk.
3. **Reactive cross-table transposition (`Grouped` $\leftrightarrow$ `Column`):**
   - Linking a single row's horizontal replicate subcolumns (`1 × N`) from a `Grouped` sheet into a `Column` sheet with `transpose: true` reshapes the slice into `(N × 1)` vertical replicates; anchoring the link in the target's column-title header row also propagates the upstream row title as the target column title.
   - Any upstream cell edit, value exclusion, or title rename invalidates the `LinkedDataRegion` in `curvebook-engine`'s DAG and automatically recomputes all downstream `Column` analyses and graphs.

---

## 7. Engine Command Contracts (`crates/engine/src/cmd/{sheet,table,info,analysis}.rs`)

Landed command IDs (all JSON-serializable, undoable in one step, reachable from GUI, CLI, `--control`, and MCP). Key parameters are listed below; the generated [`docs/commands.md`](../commands.md) is the authoritative parameter schema:

- `sheet.createData` — `{ kind, title, design_hint, replicates, rows, cols, auto_info, auto_graph, auto_family, … }`
- `sheet.freeze` / `sheet.unfreeze` — `{ sheet_id }`
- `sheet.clonePipeline` — `{ sheet_id, keep_x_values, clear_y_values, clear_column_titles, clear_row_titles, clone_analyses, clone_graphs, clone_layouts, … }` ("Clone family / New data for existing graph & analysis")
- `table.setCells` — `{ sheet_id, edits | rows, start_row, start_col, start_subcol }`
- `table.pasteLink` — `{ sheet_id, anchor, link_spec, mode, filter, propagate_title_header, … }` (live `LinkedDataRegion` including reactive transpose)
- `table.excludeCells` — `{ sheet_id, cells | coord | range, excluded }`
- `table.setTitles` — `{ sheet_id, x_title, titles, row_titles, row_factor_title, column_factor_title, … }`
- `table.setVariableRole` — `{ sheet_id, col | title, role }` (locks `role_inferred = false`)
- `table.recodeLevels` — `{ sheet_id, col | title, mappings }`
- `table.setCategoricalOrder` — `{ sheet_id, col | title, order | order_spec, levels, reversed, reference_level }`
- `table.setCalculatedVariable` — `{ sheet_id, col | title, formula }`
- `table.unlinkCalculatedVariable` — `{ sheet_id, col | title }` (freezes calculated cells into static `Entered` values with full undo/redo and DAG invalidation)
- `info.setConstant` — `{ sheet_id, name, value, units, kind }`; constants are hooked into fits through `parameter_rules` entries with scope `hooked_constant` (`HookSource`) on `analysis.updateParams` / `fit.run` (there is no separate `analysis.hookConstant` command)
- `analysis.updateParams` — `{ sheet_id, params, kind, confidence_percent, p_value_format, selected_data_sets, title }` (in-place parameter reconfiguration + auto-title refresh)
- `analysis.retargetData` — `{ sheet_id, source_sheet_id, selected_data_sets, clear_selected_data_sets }` (`Change Data Analyzed` across families)
- `transform.run` — `{ source_sheet_id, spec | kind, sheet_id, title, overwrite, … }`
