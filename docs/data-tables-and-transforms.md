# Data Tables, Calculated Variables & Transforms — Implementation Guidelines

Governing specification for `crates/doc` (`L1`), `crates/transform` (`L2`), and data-sheet commands in `crates/engine` (`L5`). Parity checklist: [`file-format-parity.md`](file-format-parity.md) and [`ui-parity.md`](ui-parity.md).

Public behavioural references:
- [Data tables overview](https://www.graphpad.com/guides/prism/latest/user-guide/using_prism_tables.htm)
- [Eight kinds of data tables](https://www.graphpad.com/guides/prism/latest/user-guide/eight_kinds_of_data_tables.htm)
- [Multiple variables tables & calculated variables](https://www.graphpad.com/guides/prism/latest/user-guide/multiple_variables_data_tables.htm)
- [Prism 11.0 & 11.1 release notes](https://www.graphpad.com/updates/prism-11-0-0-release-notes)

---

## 1. Core Architectural Invariants

1. **Typed experimental semantics, never untyped A1 spreadsheets.** Every `DataSheet` in `lumencraft-doc` carries an explicit `TableArchetype` variant that governs valid subcolumn layouts, compatible analyses, and default graph types.
2. **Non-destructive exclusions and missingness.** Every cell is an explicit enum (`CellValue::Number(f64)`, `CellValue::Text(String)`, `CellValue::Empty`) paired with an `excluded: bool` mask. Excluded cells remain visible in the grid (styled in blue italics with an `*` badge) and remain stored in `.lumencraft` files, while downstream transforms, statistical tests, curve fits, and graphs treat them as masked observations without mutating raw values.
3. **Bounded untrusted dimensions.** Project loaders, CSV importers, clipboard paste handlers, and command parameters enforce explicit caps (`MAX_ROWS = 1_000_000`, `MAX_DATA_SETS = 1_024`, `MAX_SUBCOLUMNS = 512`, `MAX_TITLE_BYTES = 4_096`) and return `DocError` / `EngineError` rather than panicking or exhausting memory.
4. **Freeze state.** A `DataSheet` exposes `frozen: bool` (matching the Prism 11.0 Freeze/Unfreeze table workflow). When `frozen` is `true`, cell-edit, paste, row/column insertion, and formula-edit commands return `EngineError::SheetFrozen` while read-only inspection, downstream analyses, and graph styling remain active.

---

## 2. The Eight Data Table Archetypes (`lumencraft_doc::TableArchetype`)

| Archetype | Row Semantics | X Column | Data-Set Columns (`A`, `B`, …) | Subcolumn Modes | Primary Downstream Analyses |
|---|---|---|---|---|---|
| **`Xy`** | Continuous independent point or time step | Numeric (`f64`), date/elapsed-time series, or generated arithmetic/geometric sequence (`X0`, `ΔX`), with optional error subcolumns | Continuous response series | `Single`, `RawReplicates(n)`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr` | Simple linear/logistic regression, nonlinear curve fitting, correlation, AUC, smooth/differentiate/integrate, XY graphs |
| **`Column`** | Unmatched replicate observation (or matched subject row when paired) | Optional row labels only | One treatment/group per column | `Single` (one column per group, replicates stacked down rows) | Unpaired/paired/Welch/one-sample *t*-tests, Mann–Whitney, Wilcoxon, ordinary/Welch/RM one-way ANOVA, Kruskal–Wallis, Friedman, descriptive stats, normality, Grubbs/ROUT outliers |
| **`Grouped`** | Factor A (row factor, e.g. time or dose tier) | Optional row titles / numeric spacing | Factor B (column factor, e.g. treatment or genotype) | `RawReplicates(n)` (stacked or side-by-side) or pre-summarized `MeanSdN` / `MeanSemN` | Two-way & three-way ANOVA (ordinary, repeated-measures, mixed-effects REML), multiple *t*-tests (one per row) |
| **`Contingency`** | Categorical outcome / exposure row | Row category title | Categorical outcome / exposure column | `Single` non-negative integer counts (`u64` / integral `f64 >= 0`) | Pearson $\chi^2$, Fisher's exact test, McNemar's test, Cochran–Armitage trend, RR/OR/sensitivity/specificity CIs |
| **`Survival`** | Individual subject / event time | Elapsed time $t \ge 0$ | One cohort column per group (`1` = event/death, `0` = censored; configurable codes) | `Single` status code per cohort column | Kaplan–Meier product-limit survival, log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon, trend log-rank |
| **`PartsOfWhole`** | Mutually exclusive slice / category | Slice label | Total / fractional series | `Single` non-negative values | Fraction of total, binomial / $\chi^2$ goodness-of-fit, pie / donut / waffle / stacked-bar graphs |
| **`MultipleVariables`** | One observational unit (tidy row) | None (all columns are named variables) | Typed variables (`Continuous`, `Categorical`, `Label`, or `Calculated`) | `Single` per variable; explicit categorical level-ordering metadata | Multiple linear/logistic/Poisson regression, Cox PH, PCA, correlation matrix,clustering, plus Prism 11 unified *t*-tests, Multifactor ($N$-way) ANOVA, nonlinear regression, and simple survival |
| **`Nested`** | Replicate within subcolumn (technical replicate) | Optional row label | Experimental group (`A`, `B`, …) | Biological units (`A:1`, `A:2`, …; e.g. animals/donors) containing technical replicates down rows | Nested *t*-test, nested one-way ANOVA, variance-component partitioning (mixed model) |

### 2.1 Pre-Summarized Error Subcolumn Rules

When a user enters pre-averaged data (`MeanSdN`, `MeanSemN`, `MeanCvN`) instead of raw replicates:
- Statistical tests and nonlinear regression reconstruct the within-point sum of squares rigorously from $(M_i, \text{SD}_i, n_i)$:
  $$\text{SS}_{\text{within}, i} = (n_i - 1)\,\text{SD}_i^2, \qquad \text{SD}_i = \begin{cases} \text{SEM}_i \sqrt{n_i} & \text{if entered as SEM}, \\ \frac{\%\text{CV}_i}{100}\,|M_i| & \text{if entered as \%CV}. \end{cases}$$
- Points with $n_i < 1$, negative SD/SEM, or missing $n_i$ when degrees of freedom depend on $n_i$ are flagged as invalid for parametric inference rather than silently treated as $n_i = 1$.
- Nonparametric rank tests, outlier tests, normality tests, and robust regression **refuse** pre-summarized `Mean/SD/N` tables with a clear `StatsError::RequiresRawReplicates` diagnostic, because rank order and heavy-tailed residuals cannot be recovered from sample moments.

### 2.2 Categorical Level Ordering (`MultipleVariables`)

Matching the Prism 11.1 categorical-variable model, each `Categorical` column in a `MultipleVariables` sheet stores a `LevelOrderSpec`:
- `EntryOrder` (order of first appearance in the column),
- `Alphabetical` (natural case-insensitive UTF-8 sort),
- `FrequencyDescending` / `FrequencyAscending` (count of non-excluded rows),
- `Explicit(Vec<String>)` (manual drag-ordered sequence; unseen levels append deterministically in entry order),
- `reversed: bool` modifier.

Graphs referencing a `MultipleVariables` sheet may either inherit the table's `LevelOrderSpec` or override display order per axis/legend without mutating the source table's contrast reference level.

---

## 3. Info Sheets & Hookable Constants

Every `DataSheet` is paired with (or shares) an `InfoSheet` containing:
1. Freeform Rich/Markdown experimental notes,
2. Structured key–value **Info Constants** (`name: String`, `value: ConstantValue`, `units: Option<String>`).

Info constants and upstream `ResultSheet` scalars (such as a fitted $K_d$ from a saturation-binding sheet or a control mean from a descriptive-stats sheet) participate in the engine's **Constant Hooking** graph (`HookSource`):
- Nonlinear regression constraints (e.g., fixing radioligand concentration `HotNM` or Michaelis constant `Km` in a competitive-inhibition fit),
- Standard-curve dilution factors and baseline offsets,
- Dynamic axis titles, callout text boxes, and layout headers (`{{info:Ligand}}`, `{{result:fit-1:EC50}}`).

`lumencraft-engine` validates all constant hooks through its topological DAG before execution; circular parameter/constant references return `EngineError::DependencyCycle` and never recurse infinitely (preventing the cyclic-hook corruption class).

---

## 4. In-Table Calculated Variables & Formula Engine (`MultipleVariables`)

In `MultipleVariables` sheets, a column may declare `VariableSource::Calculated { formula: String }`, evaluated column-wise across all rows (`1..=row_count`) whenever upstream columns change.

### 4.1 Formula Grammar & Evaluation Rules

- Formulas begin with `=` in UI entry (stored normalized without leading `=`).
- Variable references resolve column names (`[Body Mass] / ([Height_m]^2)` or bare identifiers when unambiguous). Self-references and multi-column cycles are rejected during DAG compilation.
- Missing (`Empty`), excluded, or non-numeric cells propagate `Empty` (`NaN` internally mapped back to `CellValue::Empty` with a diagnostic badge, never panicking on `0.0 / 0.0` or `ln(-1.0)`).
- Arithmetic & scalar functions share the unified AST evaluator with `lumencraft-transform` and `lumencraft-fit` (`abs`, `sqrt`, `sqr`, `ln`, `log`/`log10`, `log2`, `exp`, `sin`, `cos`, `tan`, `arcsin`, `arccos`, `arctan`, `arctan2(x, y)`, `logit`, `probit` [$5 + \Phi^{-1}(p)$], `round`, `floor`, `ceil`, `int`, `sgn`, `min`, `max`, `if`, `and`, `or`, `not`, `is_defined`).
- **Aggregate & conditional column functions** (available in Calculated Variables and Select-and-Transform, disallowed inside nonlinear regression models):
  - `count(var)`, `sum(var)`, `mean(var)`, `sd(var)`, `sem(var)`, `median(var)`, `iqr(var)`, `min_col(var)`, `max_col(var)`, `percentile(var, p)`
  - `countif`, `sumif`, `meanif`
  - `zscore(var)` $= (x_i - \bar{x}) / s_x$
- **Window functions** (partitioned optionally by a grouping column and ordered by a sort column):
  - `row_number()`, `rank()`, `dense_rank()`, `lag(var, k)`, `lead(var, k)`, `cumsum(var)`, `cummean(var)`, `diff(var, k)`, `moving_avg(var, window)`.

---

## 5. Derived Transforms (`crates/transform`)

Transforms never overwrite a `DataSheet` in place; they execute as pure functions `(DataSheet, TransformSpec) -> Result<ResultSheet, TransformError>` inside the reactive DAG so downstream analyses and graphs can bind to transformed outputs while raw data stay untouched:

1. **Mathematical transforms (`TransformMath`):**
   - Standard presets on $X$ and/or $Y$: $\ln(v)$, $\log_{10}(v)$, $\log_2(v)$, $10^v$, $e^v$, $1/v$, $\sqrt{v}$, $v^2$, $\text{logit}(v) = \ln(v/(1-v))$, $\text{probit}(v) = 5 + \Phi^{-1}(v)$, $\arcsin(\sqrt{v})$, linear scale $a v + b$, pharmacology concentration conversions ($\text{M} \leftrightarrow \text{mM}, \mu\text{M}, \text{nM}, \text{pM}$ and $-\log_{10}[\text{M}] = \text{pEC}_{50}$).
   - Error propagation when input is `Mean + SD/SEM`: exact transformation of replicate endpoints or first-order delta-method $|f'(M)| \cdot \text{SD}$ depending on user setting.
2. **Normalize (`TransformNormalize`):**
   - Per-dataset or global `0%` anchor (minimum, `0.0`, first/last row, mean of control column, or hooked constant) and `100%` anchor (maximum, sum, last/first row, or hooked constant), outputting percentages or fractions `[0, 1]` with optional clamping/stretching.
3. **Remove Baseline and Column Math (`TransformBaseline`):**
   - Subtract or divide by baseline column(s), linear baseline interpolated between first/last $k$ rows, or mean of replicate subcolumns (`A - B`, `A / B`, `% change from baseline`).
4. **Transpose, Prune Rows & Binning:**
   - Swap rows $\leftrightarrow$ data sets while preserving titles; decimate every $k$-th row or average $k$-row windows for high-frequency time-series traces; Select & Transform subsetting (`MultipleVariables` $\to$ `XY` / `Column` / `Grouped` / `Survival`).
5. **Simulate XY / Column / Contingency Data (`TransformSimulate`):**
   - Deterministic seeded PRNG (`lumencraft-math::rng`) generating synthetic curves or distributions with Gaussian, Poisson, or relative-percentage scatter for Monte Carlo validation, power exploration, and teaching workflows.

---

## 6. Engine Command Contracts (`crates/engine/src/cmd/table.rs`)

Planned canonical command IDs (all JSON-serializable, undoable in one step, reachable from GUI, CLI, `--control`, and MCP):

- `sheet.createData` — `{ archetype, title, subcolumnMode, replicates }`
- `sheet.freeze` / `sheet.unfreeze` — `{ sheetId }`
- `sheet.clonePipeline` — `{ sheetId, keepXValues, clearYValues, title }` ("Clone family / New data for existing graph & analysis")
- `table.setCells` — `{ sheetId, edits: [{ row, dataSet, subcolumn, value }] }`
- `table.excludeCells` — `{ sheetId, selection, excluded: bool }`
- `table.setTitles` — `{ sheetId, xTitle, dataSetTitles, rowTitles }`
- `table.setCategoricalOrder` — `{ sheetId, column, orderSpec }`
- `table.setCalculatedVariable` — `{ sheetId, column, title, formula }`
- `info.setConstant` / `analysis.hookConstant` — `{ sheetId, name, value, units }`
- `transform.run` — `{ sourceSheetId, spec }`
