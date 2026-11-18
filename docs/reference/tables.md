# Data Tables, Calculated Variables & Transforms

CurveBook separates experimental structure from visual styling through eight strictly typed table archetypes (`TableKind`), an AST-verified formula language for `=` Calculated Variables, and non-destructive transform and reshaping pipelines.

---

## The Eight Table Archetypes (`TableArchetype`) {#table-archetypes}

Every `DataSheet` belongs to one `TableKind` (`crates/doc/src/archetypes.rs`):

| Table Archetype | Primary Purpose | Column & Subcolumn Layout | Key Structural Fields |
|---|---|---|---|
| `Xy` | Continuous predictor $X$ vs. response $Y$ (regression, correlation, curves, Bland–Altman, ROC, AUC) | Optional $X$ column (`Numbers`, `Dates`, `SequenceProgression`, `ElapsedTimes`, or omitted) + datasets `A, B, ...` supporting raw replicates or summary subcolumns | `x_mode`, `x_title`, `data_sets`, `y_subcolumn_mode` (`SubcolumnMode`) |
| `Column` | One-factor comparison across independent or matched groups (`t`-tests, one-way ANOVA, descriptive stats, normality, outliers, *P*-value stacks) | One column per group (`A, B, ...`); single-replicate or subcolumn summary modes | `design_hint` (`UnpairedReplicates`, `PairedRepeatedMeasures`, `PrecomputedError`), `subcolumn_mode`, `row_titles` |
| `Grouped` | Two-way / three-way / multi-factor cell design (`Row Factor × Column Factor`, optional `C`-factor slice) | Row labels as levels of Factor A; dataset columns as Factor B (or B×C) with raw replicates or summary subcolumns per cell | `row_factor_title`, `column_factor_title`, `three_way_grouping` (`Option<ThreeWayColumnGrouping>`), `subcolumn_mode` |
| `Contingency` | Cross-tabulated non-negative integer counts ($\chi^2$, Fisher exact, $2 \times 2$ risk/odds ratios, McNemar, Cochran–Armitage, $\kappa$) | Integer counts in `[row, column]`; non-negative whole numbers (`u64` semantics on analysis) | `data_sets` (category columns), `row_titles` (category rows) |
| `Survival` | Right-censored time-to-event outcomes (Kaplan–Meier, log-rank, Gehan–Breslow–Wilcoxon, trend, cohort HR) | Shared follow-up time column + cohort columns recording event/censor indicator (`1` = event, `0` = censored by default) | `time_title`, `times`, `cohorts`, `event_code`, `censored_code` |
| `PartsOfWhole` | Proportions summing to a total (parts-of-whole charts, goodness-of-fit $\chi^2$, exact binomial proportion tests) | Single value column with `slice_labels` and category percentages/counts | `slice_labels`, `data_sets` |
| `MultipleVariables` | Tidy observational data (`rows = subjects/records`, `columns = typed variables`) for GLMs, Cox PH, PCA, clustering, multifactor ANOVA, direct multi-group tests | Typed `VariableColumn`s with explicit `VariableRole` (`Continuous`, `Categorical`, `Label`, `Calculated`) and `VariableSource` (`Entered` or `Calculated { formula }`) | `variables` (`role`, `role_inferred`, `level_order`, `source`), `row_titles` |
| `Nested` | Hierarchical cluster / nested designs (fixed treatment groups containing random biological subjects/litters/wells measured across technical replicates) | Top-level treatment groups containing `NestedGroupColumn` subclusters with technical replicate subcolumns | `groups` (`Vec<NestedGroupColumn>`), `row_titles`, `row_titles_expanded` |

### Subcolumn Modes & Pre-Summarized Entry (`SubcolumnMode`) {#subcolumn-modes}

Continuous tables (`Xy`, `Column`, `Grouped`, `Nested`) support eight `SubcolumnMode` representations (`crates/doc/src/cell.rs`):

| Mode | Subcolumns | Interpretation | Analysis Compatibility |
|---|---|---|---|
| `Single` | `1` | Single value per row/cell | All standard analyses |
| `RawReplicates(usize)` | `n` (`1..=512`, `MAX_SUBCOLUMNS = 512`) | Individual raw technical/biological replicates (`Mean ± SD/SEM/CI` computed dynamically) | Full parametric, nonparametric, normality, residual diagnostics, ROBUST/ROUT |
| `MeanSdN` | `3` (`Mean`, `SD`, `N`) | Pre-summarized normal moments | One/two-sample *t*, one-way/Welch/Brown–Forsythe ANOVA, OLS/WLS/Deming/Nonlinear LS, Descriptive moments; rejected with `RequiresRawReplicates` for normality, ROUT/Grubbs, Kruskal–Wallis/Mann–Whitney |
| `MeanSemN` | `3` (`Mean`, `SEM`, `N`) | Pre-summarized mean, standard error, sample size ($\text{SD} = \text{SEM}\sqrt{N}$) | Same summary-capable routines as `MeanSdN` |
| `MeanCvN` | `3` (`Mean`, `%CV`, `N`) | Mean paired with coefficient of variation and sample size ($\text{SD} = |\text{Mean}|\cdot\%\text{CV}/100$) | Same parametric moment reconstruction (`supports_parametric_moments`) and plotting |
| `MeanLowHigh` | `3` (`Mean`, `-Error`, `+Error`) | Mean with lower and upper error bounds | Plotting & central-value extraction; rejected where full replicate observations or `N`/`SD` moments are required |
| `UpperLowerLimits` | `3` (`Mean`, `LowerLimit`, `UpperLimit`) | Central mean with explicit lower and upper bounds | Plotting & `ExplicitErrorBounds`; rejected by replicate fit extraction |
| `MedianIqr` | `3` (`Median`, `Q1`, `Q3`) | Median and quartile bounds for box/error plotting | Plotting; rejected by replicate/parametric fit extraction |

`ReplicateStacking` controls how replicate columns stack or collapse during plotting and summary extraction (`SideBySide` vs `Stacked`).

---

## `=` Calculated Variables & Formula Language {#calculated-variables}

Tables support live formula evaluation (`table.setCalculatedVariable`) implemented cleanly in `crates/transform/src/formula/` (`parser.rs`, `eval.rs`, `dag.rs`).

### Formula References & Constants {#formula-references}

- **Prefix:** Formulas begin with `=` (leading `=` is stripped before AST parsing).
- **Variable & Cell References:**
  - Bare identifiers (`Conc`, `logConc`, `Ctrl_1`) or bracketed names (`[Blank Sub]`) matching column/variable titles.
  - Explicit index or quoted name lookups: `col("Title")` or `col(idx)` (0-based or column letter mapping).
  - Experiment metadata constants from linked `InfoSheet`s: `Info("Key")`.
  - Row index builtins: `$row` (1-based row number) and `$index` (0-based row index).
  - Archetype aliases: `X` and `Y` on `Xy` and `Survival` tables.
- **Built-in Constants:** `pi`, `e`, `tau` ($2\pi$), `true` (`1.0`), `false` (`0.0`), `nan` (`Missing`). Matching column titles take precedence over `pi`/`e`/`tau` (exact case-sensitive match checked first for `e`).
- **Circular Dependency Guard:** Cross-column formula references form an evaluation DAG (`formula/dag.rs`); self-references or multi-column cycles fail cleanly with `ErrorCode::CycleDetected`.

### Operators & Three-Valued Logic {#formula-operators}

| Precedence | Operators | Semantics |
|---:|---|---|
| Highest | `^` | Exponentiation (right-associative; domain errors propagate as `Missing`) |
| Unary | `+`, `-`, `!` | Numeric sign flip; logical `!` maps `Missing` to `Missing`, `0.0` to `1.0`, nonzero to `0.0` |
| Multiplicative | `*`, `/`, `%` | Multiplication, division (`0` divisor -> `Missing`), floating remainder |
| Additive | `+`, `-` | Addition, subtraction |
| Comparison | `<`, `<=`, `>`, `>=`, `==`, `=`, `!=`, `<>` | Numeric and text equality/ordering (relational `<`/`<=`/`>`/`>=` are numeric; `Missing` yields `Missing`) |
| Unary Word | `NOT` | Word-form logical negation (lower binding than comparisons so `NOT x == 0` parses as `NOT (x == 0)`) |
| Conjunction | `&&`, `AND` | Strong Kleene 3-valued AND (`false && Missing -> false`; `true && Missing -> Missing`) |
| Disjunction | `||`, `OR` | Strong Kleene 3-valued OR (`true || Missing -> true`; `false || Missing -> Missing`) |

### Supported Functions & Clauses {#formula-functions}

- **Arithmetic, Rounding & Clamping:** `abs`, `sqrt`, `cbrt`, `sqr`, `pow(x, p)`, `clamp(x, lo, hi)`, `floor`, `ceil`, `round(x[, digits])`, `trunc`, `int` (alias of `floor`), `sign` / `sgn`, `min(...)`, `max(...)`, `sum(...)`, `mean(...)`.
- **Logarithmic, Exponential & Trigonometric:** `ln`, `log(x[, base])`, `log10`, `log2`, `exp`, `exp10`, `sin`, `cos`, `tan`, `asin` / `arcsin`, `acos` / `arccos`, `atan` / `arctan`, `atan2` / `arctan2`, `sinh`, `cosh`, `tanh`.
- **Special & Statistical Transforms:** `erf`, `erfc`, `lgamma`, `logit(p)` (for $p \in (0, 1)$), `probit(p)` ($5 + \Phi^{-1}(p)$, Finney probit scale), `norm_ppf` / `norm_inv` / `qnorm` / `inv_norm`, `arcsin_sqrt(p)` ($\arcsin\sqrt{p}$ for $p \in [0, 1]$).
- **Logical & Missing-Value Handling:** `if(cond, then_expr, else_expr)` (short-circuiting only the chosen branch), `isnan(x)`, `isvalid(x)` / `is_defined(x)`, `coalesce(a, b, ...)`, variadic `and(...)`, `or(...)`, `not(x)`.
- **Whole-Column Aggregates:** `sum`, `mean`, `count` (numeric active cells), `counta` (non-empty active numeric/text cells), `sd` / `stdev`, `sem` / `stderr`, `var`, `median`, `iqr`, `min_col` / `col_min`, `max_col` / `col_max`, `geo_mean`, `percentile(col, p)` / `quantile(col, p)`.
- **Conditional `*if` Aggregates:** `countif`, `countaif`, `sumif`, `meanif`, `minif`, `maxif`, `stdevif` / `sdif`, `stderrif` / `semif`. Argument order is deterministically `(values, condition)` (or `countif(condition)` / `countif(values, condition)` where rows matching nonzero finite `condition` are included); `countaif` counts non-empty text/numeric cells matching the predicate.
- **Window Functions & SQL-Style Clauses:**
  - Window functions: `row_number()`, `rank()`, `dense_rank()`, `center(expr)`, `standardize(expr)` / `zscore(expr)`, `cumsum(expr)`, `cumprod(expr)`, `cummean(expr)`, `lag(expr[, offset[, default]])`, `lead(expr[, offset[, default]])`, `diff(expr[, offset])`, `rolling_mean(expr, window)` / `moving_avg(expr, window)`, `rolling_sum(expr, window)`.
  - Filter & Window clauses: `FILTER (WHERE <cond>)` / `WHERE <cond>` (applicable inside aggregates and `OVER (...)`), plus `OVER ([PARTITION BY ...] [ORDER BY <expr> [ASC|DESC] [NULLS FIRST|LAST]] [ROWS <frame>])` supporting `UNBOUNDED PRECEDING`, `<k> PRECEDING`, `CURRENT ROW`, `<k> FOLLOWING`, and `UNBOUNDED FOLLOWING`.
- **Evaluator Scope & Error Propagation:** Formula values are `Number(f64)`, `Text(String)`, or `Missing`. String-manipulation functions and date/calendar arithmetic functions are **not** present in the Beta formula evaluator. Empty cells, excluded cells, division by zero, out-of-domain operations (`ln(-1)`, `sqrt(-1)`, `probit(0)`), non-finite intermediates (`NaN`, `±Inf`), and unhandled type mismatches propagate deterministically as `FormulaValue::Missing` (stored as an empty `Cell`).

---

## Table Transforms, Reshaping & Simulation {#transforms-and-reshaping}

CurveBook executes derived transforms (`transform.run`, `table.convertArchetype`, `table.importPlateMap`, `table.importTidyLong`, `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions`, `table.simulateXy`) without altering upstream raw cells:

| Transform / Operation | Purpose & Options | Edge Cases & Guards |
|---|---|---|
| **Math Transforms (`MathTransformPreset`)** | Per-cell preset transforms over `X` or `Y`: `Ln`, `Log10`, `Log2`, `Exp10`, `Exp`, `Reciprocal`, `Sqrt`, `Square`, `Logit`, `Probit`, `ArcsinSqrt`, `LinearScale { slope, intercept }`, `ConcentrationToNegativeLogM { molar_unit_factor }` (and arbitrary `=` Calculated Variable formulas including `2^-y` qPCR pipelines). | Non-positive inputs for `Log*`/`Reciprocal`, negative inputs for `Sqrt`, or probabilities outside `(0, 1)` for `Logit`/`Probit` yield empty missing cells or typed errors rather than `NaN`. |
| **Normalize (`NormalizeAnchor`)** | Scales each dataset between `0%` and `100%` (or `0.0`..`1.0`) defined by `zero_anchor` and `hundred_anchor` (`ColumnMin`, `ColumnMax`, `ColumnSum`, `FirstRow`, `LastRow`, `FixedValue(v)`, `ControlColumnMean { column }`, `Hooked(HookSource)`). | Rejects zero span (`y_100 == y_0`) with an error to avoid division by zero. |
| **Baseline (`BaselineOperation`)** | Subtracts or ratios designated `baseline_columns`: `Difference`, `Ratio`, `PercentChange`, or `FractionalChange`. | Zero baseline divisors in `Ratio` / `PercentChange` / `FractionalChange` omit/error on undefined denominators; excluded cells remain skipped. |
| **Row Summary & Replicate Transposition (`Transpose`, `SelectAndTransform`)** | Swaps rows and columns (`transpose_cell_grid`), subsets/reorders rows or columns, filters rows by predicate, or applies column-wise transforms while preserving cell exclusion masks. | Enforces `MAX_ROWS`, `MAX_DATA_SETS`, `MAX_IMPORT_CELLS = 10,000,000`, and remaps exclusion masks atomically. |
| **Curve Calculus (`CurveSheetOp`, `compute_sheet_auc`)** | `Smooth` (`MovingAverage`, `SavitzkyGolay`, `Lowess` of Cleveland 1979 with bisquare `6*MAD` robustness), `Differentiate` (`First` / `Second` 3-point Lagrange quadratic derivative on sorted uniform or non-uniform `X`), `Integrate` (cumulative trapezoidal area above `baseline`), and trapezoidal AUC with total/net/baseline-corrected peaks (`Yeh & Kwan 1978`). | Duplicate `X` values reject differentiation with `InvalidInput`; LOWESS `span` must lie in `(0, 1]`. |
| **Non-Destructive Archetype Conversion (`QOL-1`, `table.convertArchetype`)** | Projects among `Column`, `Grouped`, `Xy`, `MultipleVariables`, and `Contingency` with non-mutating `ConversionPreview` (`preserved_cells`, `dropped_cells`, `preserved_exclusions`, `warnings`, `lossy`). | Preserves `excluded` masks and remaps `CellExclusionNote` coordinates. Refuses lossy projections (e.g., negative/fractional entries targeting `Contingency`, collapsed subcolumns) unless `allow_lossy = true`. |
| **Microplate Mapper (`QOL-2`, `table.importPlateMap`)** | Maps `8×12` (96-well) or `16×24` (384-well) reader grids into typed `Xy`, `Column`, or `Grouped` tables by assigning concentrations ($X$), treatment groups, technical replicates, and optional blank subtraction/tagging. | Validates well grid dimensions (`8×12` or `16×24`), non-empty assignments, and cell budget limits. |
| **Tidy Long-to-Wide Pivot (`QOL-2`, `table.importTidyLong`)** | Reshapes tidy long rows `(X/Row Factor, Group Column, Optional Replicate Index, Value, Optional Exclusion Flag)` directly into typed `Xy`, `Column`, or `Grouped` replicate subcolumns. | Preserves exclusion flags from indicator column; rejects out-of-range column indices and excessive subcolumn expansions above `MAX_SUBCOLUMNS = 512`. |
| **Randomization & Row Sampling (`QOL-14`)** | `table.randomizeGroups`: seeded `Complete` or `PermutedBlock` allocation across $k \ge 2$ arms over `DeterministicRng`, emitting a schedule sheet + `InfoSheet` ARRIVE metadata. `table.randomSampleRows`: uniform or stratified $k$-of-$N$ sampling without replacement via partial Fisher–Yates. | Block sizes must be positive multiples of the summed integer allocation weights; empty strata and $k > N$ fail cleanly with typed validation errors. |
| **Monte Carlo & Distribution Simulation (`QOL-14`, `table.simulateDistributions`, `table.simulateXy`)** | Generates independent seeded columns (`Uniform`, `Normal`, `LogNormal`, `Binomial`, `Poisson`, `Exponential`) or synthetic `Xy` noisy curves (`Gaussian`, `Uniform`, `LogNormal` / multiplicative noise). | Deterministic across native and `wasm32` targets given the recorded 64-bit PRNG seed. |
| **4-Stage `ImportLinkSpec` Pipeline** | Reconciles upstream `DataSheet`, `ResultSheet`, or delimited text payloads across `filter -> reshape/transpose -> baseline -> exclude/*` while maintaining `LinkedDataRegion` provenance. | Cycle detection via Kahn's topological sort (`detect_link_cycles`) aborts circular sheet links before mutation. |

See [Concepts & Document Architecture](concepts.md), [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Command Reference](../commands.md) for parameter schemas.
