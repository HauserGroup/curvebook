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
4. **Freeze state.** A `DataSheet` exposes `frozen: bool` (matching the Prism 11.0 Freeze/Unfreeze table workflow). When `frozen` is `true`, cell-edit, paste, row/column insertion, and formula-edit commands return `DocError::SheetFrozen` (surfaced as `ErrorCode::FrozenSheet`, `"frozen_sheet"`) while read-only inspection, downstream analyses, and graph styling remain active.

---

## 2. The Eight Data Table Archetypes (`lumencraft_doc::TableArchetype`)

| Archetype | Row Semantics | X Column | Data-Set Columns (`A`, `B`, …) | Subcolumn Modes | Primary Downstream Analyses |
|---|---|---|---|---|---|
| **`Xy`** | Continuous independent point or time step | Numeric (`f64`), date/elapsed-time series, or generated arithmetic/geometric sequence (`X0`, `ΔX`), with optional error subcolumns | Continuous response series | `Single`, `RawReplicates(n)`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr` | Simple linear/logistic regression, nonlinear curve fitting, correlation, AUC, smooth/differentiate/integrate, XY graphs |
| **`Column`** | Unmatched replicate observation (or matched subject row when paired) | Optional row labels only | One treatment/group per column | `Single` (replicates stacked down rows) or pre-summarized `MeanSdN` / `MeanSemN` / `MeanCvN` | Unpaired/paired/Welch/one-sample *t*-tests, Mann–Whitney, Wilcoxon, ordinary/Welch/RM one-way ANOVA, Kruskal–Wallis, Friedman, descriptive stats, normality, Grubbs/ROUT outliers |
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
- **Categorical ordering (`LevelOrderSpec`, Prism 11.1 parity):** Each `Categorical` column stores a `LevelOrderSpec { order: LevelOrderKind, reversed }` (`crates/doc/src/archetypes.rs`) where `LevelOrderKind` is one of:
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

`lumencraft-engine` validates all constant hooks through its topological DAG before execution; circular parameter/constant references are rejected with `ErrorCode::CycleDetected` (`"cycle_detected"`, from `FormatError::DependencyCycle` / link-cycle detection in `crates/engine/src/dag.rs`) and never recurse infinitely (preventing the cyclic-hook corruption class).

---

## 4. In-Table Calculated Variables & Formula Engine (`MultipleVariables`)

In `MultipleVariables` sheets, a column may declare `VariableSource::Calculated { formula: String }`, evaluated column-wise across all rows (`1..=row_count`) whenever upstream columns change.

### 4.1 Formula Grammar & Evaluation Rules

- Formulas begin with `=` in UI entry (stored normalized without leading `=`).
- Variable references resolve column names (`[Body Mass] / ([Height_m]^2)` or bare identifiers when unambiguous). Self-references and multi-column cycles are rejected during DAG compilation.
- Missing (`Empty`), excluded, or non-numeric cells propagate `Empty` (`NaN` internally mapped back to `CellValue::Empty` with a diagnostic badge, never panicking on `0.0 / 0.0` or `ln(-1.0)`).
- Arithmetic & scalar functions are evaluated by the `lumencraft-transform` formula engine (`crates/transform/src/formula/`; the curve-fit equation DSL in `crates/fit/src/dsl.rs` is a separate compiler with an overlapping function set) (`abs`, `sqrt`, `sqr`, `ln`, `log`/`log10`, `log2`, `exp`, `sin`, `cos`, `tan`, `arcsin`, `arccos`, `arctan`, `arctan2(x, y)`, `logit`, `probit` [$5 + \Phi^{-1}(p)$], `round`, `floor`, `ceil`, `int`, `sgn`, `min`, `max`, `if`, `and`, `or`, `not`, `is_defined`).
- **Aggregate & conditional column functions** (available in Calculated Variables and Select-and-Transform, disallowed inside nonlinear regression models):
  - `count(var)`, `sum(var)`, `mean(var)`, `sd(var)`, `sem(var)`, `median(var)`, `iqr(var)`, `min_col(var)`, `max_col(var)`, `percentile(var, p)`
  - `countif`, `sumif`, `meanif`
  - `zscore(var)` $= (x_i - \bar{x}) / s_x$
- **Window functions** (partitioned optionally by a grouping column and ordered by a sort column):
  - `row_number()`, `rank()`, `dense_rank()`, `lag(var, k)`, `lead(var, k)`, `cumsum(var)`, `cummean(var)`, `diff(var, k)`, `moving_avg(var, window)`.

---

## 5. Derived Transforms (`crates/transform`)

Transforms never overwrite a `DataSheet` in place; they execute as pure functions (`execute_transform(&DataSheet, &TransformAnalysisSpec, &TransformContext) -> Result<TransformOutput, TransformError>` and `execute_analysis_transform(…) -> Result<ResultSheet, TransformError>` in `crates/transform/src/ops/mod.rs`) inside the reactive DAG so downstream analyses and graphs can bind to transformed outputs while raw data stay untouched:

1. **Mathematical transforms (`TransformAnalysisSpec::Math`, `MathTransformPreset`):**
   - Standard presets on $X$ and/or $Y$: $\ln(v)$, $\log_{10}(v)$, $\log_2(v)$, $10^v$, $e^v$, $1/v$, $\sqrt{v}$, $v^2$, $\text{logit}(v) = \ln(v/(1-v))$, $\text{probit}(v) = 5 + \Phi^{-1}(v)$, $\arcsin(\sqrt{v})$, linear scale $a v + b$, pharmacology concentration conversions ($\text{M} \leftrightarrow \text{mM}, \mu\text{M}, \text{nM}, \text{pM}$ and $-\log_{10}[\text{M}] = \text{pEC}_{50}$).
   - Error propagation when input is `Mean + SD/SEM`: exact transformation of replicate endpoints or first-order delta-method $|f'(M)| \cdot \text{SD}$ depending on user setting.
2. **Normalize (`TransformAnalysisSpec::Normalize`):**
   - Per-dataset or global `0%` anchor (minimum, `0.0`, first/last row, mean of control column, or hooked constant) and `100%` anchor (maximum, sum, last/first row, or hooked constant), outputting percentages or fractions `[0, 1]` with optional clamping/stretching.
3. **Remove Baseline and Column Math (`TransformAnalysisSpec::RemoveBaseline`):**
   - Subtract or divide by baseline column(s), linear baseline interpolated between first/last $k$ rows, or mean of replicate subcolumns (`A - B`, `A / B`, `% change from baseline`).
4. **Transpose, Prune Rows & Binning (`Transpose`, `PruneRows`, `SelectAndTransform`):**
   - Swap rows $\leftrightarrow$ data sets while preserving titles; decimate every $k$-th row or average $k$-row windows for high-frequency time-series traces; Select & Transform subsetting (`MultipleVariables` $\to$ `XY` / `Column` / `Grouped` / `Survival`).
5. **Simulate XY / Column / Contingency Data (`TransformAnalysisSpec::Simulate`):**
   - Deterministic seeded PRNG (`lumencraft_math::rng::DeterministicRng`) parameterized by `seed`, `row_count`, `replicates`, `x_min`, `x_max`, `equation_id`, `parameters`, and `noise` (`SimulationNoise`: `None`, `Gaussian`, `RelativePercent`, or `Poisson`) to generate synthetic curves or distributions for Monte Carlo validation, power exploration, and teaching workflows.

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
   - Any upstream cell edit, value exclusion, or title rename invalidates the `LinkedDataRegion` in `lumencraft-engine`'s DAG and automatically recomputes all downstream `Column` analyses and graphs.

---

## 7. Engine Command Contracts (`crates/engine/src/cmd/{sheet,table,info,analysis}.rs`)

Landed command IDs (all JSON-serializable, undoable in one step, reachable from GUI, CLI, `--control`, and MCP). Key parameters are listed below; the generated [`docs/commands.md`](commands.md) is the authoritative parameter schema:

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
- `info.setConstant` — `{ sheet_id, name, value, units, kind }`; constants are hooked into fits through `parameter_rules` entries with scope `hooked_constant` (`HookSource`) on `analysis.updateParams` / `fit.run` (there is no separate `analysis.hookConstant` command)
- `analysis.updateParams` — `{ sheet_id, params, kind, confidence_percent, p_value_format, selected_data_sets, title }` (in-place parameter reconfiguration + auto-title refresh)
- `analysis.retargetData` — `{ sheet_id, source_sheet_id, selected_data_sets, clear_selected_data_sets }` (`Change Data Analyzed` across families)
- `transform.run` — `{ source_sheet_id, spec | kind, sheet_id, title, overwrite, … }`
