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
| **Microplate Mapper (`QOL-2`, `table.importPlateMap`)** | Maps `8×12` (96-well) or `16×24` (384-well) reader grids or analyzable sheet / `AUC` results into typed `Xy`, `Column`, `Grouped`, or `MultipleVariables` tables with maintained `plate_well_origins` provenance and reusable `.cbplate` layouts (see [Microplate Mapping](#microplate-mapping)). | Validates strict `8×12`/`16×24` grids, `UpToDate` freshness on `ResultSheet` sources, non-empty assignments, and cell budget limits. |
| **Tidy Long-to-Wide Pivot (`QOL-2`, `table.importTidyLong`)** | Reshapes tidy long rows `(X/Row Factor, Group Column, Optional Replicate Index, Value, Optional Exclusion Flag)` directly into typed `Xy`, `Column`, or `Grouped` replicate subcolumns. | Preserves exclusion flags from indicator column; rejects out-of-range column indices and excessive subcolumn expansions above `MAX_SUBCOLUMNS = 512`. |
| **Randomization & Row Sampling (`QOL-14`)** | `table.randomizeGroups`: seeded `Complete` or `PermutedBlock` allocation across $k \ge 2$ arms over `DeterministicRng`, emitting a schedule sheet + `InfoSheet` ARRIVE metadata. `table.randomSampleRows`: uniform or stratified $k$-of-$N$ sampling without replacement via partial Fisher–Yates. | Block sizes must be positive multiples of the summed integer allocation weights; empty strata and $k > N$ fail cleanly with typed validation errors. |
| **Monte Carlo & Distribution Simulation (`QOL-14`, `table.simulateDistributions`, `table.simulateXy`)** | Generates independent seeded columns (`Uniform`, `Normal`, `LogNormal`, `Binomial`, `Poisson`, `Exponential`) or synthetic `Xy` noisy curves (`Gaussian`, `Uniform`, `LogNormal` / multiplicative noise). | Deterministic across native and `wasm32` targets given the recorded 64-bit PRNG seed. |
| **4-Stage `ImportLinkSpec` Pipeline** | Reconciles upstream `DataSheet`, `ResultSheet`, or delimited text payloads across `filter` (including `row_match`) `-> reshape/transpose -> baseline -> exclude/*` while maintaining `LinkedDataRegion` provenance. | Cycle detection via Kahn's topological sort (`detect_link_cycles`) aborts circular sheet links before mutation. |

---

## Multi-Block Tabular Import (`.xlsx` & `.csv` / `.tsv`) {#tabular-import-options}

CurveBook provides explicit, vendor-neutral range slicing, exact OOXML merge expansion, discriminator filtering/splitting, and preamble metadata extraction across `.xlsx` and delimited text imports without fragile layout heuristics.

### `.xlsx` Workbook Import Options (`table.importXlsx`) {#xlsx-import-options}

All advanced `.xlsx` parsing controls are opt-in and applied in deterministic order: **`fill_merged_cells` -> `cell_range` crop -> `skip_rows` (+ optional `extract_preamble_info`) -> `header_rows` -> `row_match` / `split_by_column` -> table archetype inference**:

- **`cell_range` (single-sheet rule):** Optional A1 rectangle (for example `"B4:M11"`). Preserves exact rectangle dimensions including trailing empty cells. `cell_range` requires a **single selected worksheet**; attempting to pass `cell_range` when multiple worksheets are selected fails with an error asking you to select one sheet via `sheets` or `sheet_indices`.
- **`skip_rows`:** Number of leading rows skipped before `header_rows` (counted relative to `cell_range` when a range is specified).
- **`fill_merged_cells` (default `false`):** Expands each merged region's top-left value across its exact OOXML `<mergeCell>` XML bounds before cropping and header extraction. Unmerged empty cells remain empty (CurveBook never performs carry-right guessing into unmerged blanks).
- **`split_by_column`:** Zero-based discriminator column index. Groups data rows by trimmed cell text while preserving first-seen block order and row order within each block (handling both stacked multi-channel blocks and interleaved reading cycles), drops the discriminator column, skips fully blank separator rows, warns on non-blank rows whose discriminator cell is empty, emits one `DataSheet` per block using deterministic sheet IDs (`{base_id}_{block_slug}` with `_2`, `_3`, ... collision suffixes) and titles (`"{sheet} — {block}"`), and infers each block's table archetype independently. Mutually exclusive with `row_match`.
- **`row_match` (`ColumnEqualsFilter`):** `{ column, equals, drop_column }` (`drop_column` defaults to `true`). Retains only data rows whose trimmed value in zero-based `column` equals `equals`. Mutually exclusive with `split_by_column`.
- **`extract_preamble_info` (default `false`):** When `true` and `skip_rows > 0`, inspects skipped leading rows and attaches **one shared `InfoSheet`** (parsed `Key: Value` / `Key = Value` constants plus free-text preamble lines in `notes_markdown`) linked to every `DataSheet` produced by that import.
- **Importing bare plate grids into an existing project:** `table.importXlsx` does not take `sheet_id`/`link`/`filter`; when slicing a bare `8×12` or `16×24` plate layout or concentration grid (for example `cell_range: "B4:M11"`) into an open study, pass **`merge: true`** and **`has_header: false`** (together with `fill_merged_cells: true` when replicate labels span merged cells) so the existing project is preserved and row `A` is not consumed as column headers.

### `.csv` / `.tsv` Import Windowing, Splitting & Preamble Extraction (`table.importCsv`) {#csv-import-options}

- **Unified `ImportFilterSpec` windowing on new sheets:** `table.importCsv` honors `filter` (`row_window` / `row_start`/`row_end`, `col_window` / `col_start`/`col_end`, `decimate_every_k`, `parse_asterisk_exclusions`, and `row_match`) when creating **new** sheets as well as when updating existing sheets or running live `Paste Link` pipelines (avoiding a separate CSV `skip_rows` option — set `filter.row_start` / `filter.row_window` instead).
- **`row_match` (`ColumnEqualsFilter`):** Keeps only rows matching a trimmed value in `column` and drops that column by default (`drop_column: true`).
- **`split_by_column`:** Splits stacked or interleaved multi-block delimited files into one `DataSheet` per distinct trimmed discriminator value using the same deterministic `{base_id}_{block_slug}` ID scheme (`_2`, `_3`, ... collision suffixes) as `.xlsx` import, skipping blank separator rows and warning on non-blank rows with an empty discriminator. Mutually exclusive with `filter.row_match`. `split_by_column` is rejected when targeting an existing single `sheet_id` or `link: true` because a single target or live-linked sheet cannot host multiple split blocks; pass `filter.row_match` instead to filter one block into a targeted or linked sheet.
- **`extract_preamble_info` (default `false`):** When `true` and leading rows are skipped (`row_start > 0`), extracts `Key: Value` / `Key = Value` metadata (including trailing-delimiter padded lines such as `RunId: 42,,,,`) using the active `decimal_separator` for numeric constants and collects non-KV preamble lines into `notes_markdown` on **one shared `InfoSheet`** linked to all emitted `DataSheet`s.

### Numeric-X Contract for Kinetic Imports {#numeric-x-contract}

> [!IMPORTANT]
> CurveBook's `.xlsx` and `.csv`/`.tsv` importers do **not** parse Excel `numFmt` day-fraction time serials or text timestamps such as `hh:mm:ss`. For kinetic analyses ([Area Under the Curve](registry/tables.html#analysis-kind-area-under-curve)) and kinetic plate reducers (`x_window_mean`), the `X` column must contain **finite numeric values in the desired integration units** (for example elapsed seconds or minutes).

- **Spreadsheet time-formatted numeric cells:** Spreadsheets commonly store times as numeric fractions of a 24-hour day (for example `0.00069444...` formatted as `00:01:00`). Because `.xlsx` import reads the stored numeric value directly without interpreting display `numFmt`, integrating those values produces AUC in **day units** unless converted to seconds or minutes first.
- **Text timestamps (`hh:mm:ss`):** Delimited or worksheet text strings such as `"00:05:00"` import as text cells, are ignored by numeric `X` extraction, and are rejected cleanly (with a diagnostic warning/error explaining the numeric-X requirement) rather than silently integrated as arbitrary numbers.

---

## Composable Microplate Mapping (`table.importPlateMap`) {#microplate-mapping}

`table.importPlateMap` maps `8×12` (96-well) or `16×24` (384-well) microplate readings into typed `Column`, `Xy`, `Grouped`, or `MultipleVariables` tables while preserving per-cell well traceability.

### Reading Sources: Direct Values vs. Analyzable Sheets & Kinetic Reducers {#plate-sources}

Provide either **`values`** (a direct flat 96/384 or 2D `8×12`/`16×24` numeric array) or **`source`** (mutually exclusive):

- **`source` (`require_analyzable_data_sheet`):** Reads measurements from an existing `DataSheet` or an analyzable `ResultSheet` (such as [`AreaUnderCurve`](registry/tables.html#analysis-kind-area-under-curve)'s `analyzable_data` table):
  - **Well-per-row tables (`value_column`):** For tables with one row per well (for example `AUC` `analyzable_data`), pass `value_column` naming the numeric column header to extract (trimmed, case-insensitive; e.g., `"Net Area"`, `"Positive Area"`, `"Negative Area"`, `"Total Area"`).
  - **Well-per-column kinetic/column tables (`reducer`):** For `Xy` or `Column` tables where each dataset column represents a well, supply one of four explicit kinetic reducers (all skip `excluded == true` cells):
    - `{"row_index": n}`: active numeric value at zero-based row index `n`.
    - `{"x_window_mean": {"x_min": a, "x_max": b}}`: arithmetic mean of active numeric values whose finite numeric `X` lies within inclusive `[a, b]` (requires an `Xy` table with numeric `X`).
    - `"min"` / `"max"`: minimum or maximum active numeric value across the well column.
- **`match_wells`:**
  - `by_well_id` *(default)*: matches exact canonical or unpadded well IDs (`"A01"`, `"A1"`) or leading well-ID tokens before delimiters (such as `"A01 - Sample X1"`). Fails cleanly on duplicate well-ID collisions or zero matches; missing or unavailable wells remain `None` with non-blocking warnings.
  - `positional`: maps source rows or columns sequentially in row-major plate order (`A01..H12` or `A01..P24`).
- **Static snapshot & `UpToDate` freshness guard:** `table.importPlateMap` produces an independent **static `DataSheet` snapshot** (no reactive DAG link node — later edits to upstream kinetic tables do not overwrite the mapped plate sheet). When `source.sheet_id` names a `ResultSheet`, CurveBook refuses the read (`ErrorCode::Conflict`) unless `status == ResultStatus::UpToDate`, and records the upstream `source_sheet_id` (and `value_column` or reducer) in the mapped sheet's `FloatingNote`.

### Plate Layouts: Grids, Layout Sheets, `x_grid` & Controls {#plate-layouts}

Supply well assignments via either **`template` / `roles`** (legacy/JSON template or per-well role array) or **`label_grid` / `layout_sheet_id`** (mutually exclusive):

- **`label_grid` / `layout_sheet_id`:** Strictly **`8×12`** (96-well) or **`16×24`** (384-well). When reading `layout_sheet_id` or `x_grid_sheet_id` from a `DataSheet`, CurveBook extracts strictly the sheet's **data cells** (excluding column titles and row titles). Dimension mismatches such as `9×13` or `17×25` fail with an explicit header-exclusion hint (`7×12` or `15×24` grids remind you to re-import with `has_header: false`). Labels are strictly **literal** (zero inline concentration parsing); empty cells become `Excluded`; `blank_labels` and `positive_control_labels` match trimmed case-insensitively; grid-constructed templates assign zero-based replicate indices (`0, 1, ...`).
- **Optional `x_grid` / `x_grid_sheet_id`:** Strictly matching `8×12`/`16×24` numeric grid supplying per-well `dilution_x` (for example shifted concentration-response ladders). When omitted, every `Treatment` well defaults to **`dilution_x = 0.0`** (valid for `Column` screening tables; emits a non-blocking single-X warning for `Xy`/`Grouped`). When `x_grid` or `x_grid_sheet_id` **is** supplied, any non-Blank, non-Control, non-Excluded treatment well with an empty or non-finite X cell triggers a **hard validation error** naming the well and label.
- **Controls, `PositiveControl` pooling & Dunnett reference arms:** `subtract_blank_mean: true` subtracts the mean of non-empty `Blank` wells from readings. On `Column` targets, all wells matching `positive_control_labels` pool into a single `"Positive Control"` column (and `Blank` wells append as a `"Blank"` column when `subtract_blank_mean` is `false`). If a screen requires separate named reference arms for downstream multiple comparisons (for example distinct vehicle/buffer and reference-agonist controls for Dunnett's test), keep them as regular **`Treatment`** groups and apply reactive `Transform -> RemoveBaseline` or `Transform -> Normalize` (`ControlColumnMean`) non-destructively.
- **Versioned `.cbplate` layout persistence:** Reusable layouts save and load as single-extension **`.cbplate`** UTF-8 JSON (`format_version = 1`, `docs/schemas/plate-map.schema.json`, atomic writes via `curvebook-platform`, forward-version refusal; distinct from `.cbtemplate` ZIP project templates and `.json` result exports; see [Persisted File Formats](../file-formats.md)).

### Well Provenance (`plate_well_origins`) Lifecycle & `well_cell_map` {#plate-well-origins}

`table.importPlateMap` populates `DataSheet::plate_well_origins` (`Vec<PlateCellOrigin>`, mapping `(row, col, subcol) -> well`) across `Column`, `Xy`, `Grouped`, and `MultipleVariables` outputs and returns the ordered `well_cell_map` in the command payload:

- **Preserved:** Kept intact across `table.excludeCells` / `table.excludeCellsWithReason` (enabling well-ID inspection during `QOL-3` outlier/QC audits) and persisted through `.curvebook` Schema `v8` streaming archive save/reopen.
- **Shifted & remapped:** Shifted automatically on row insertions (`InsertRows` / `shift_rows_down`) and remapped 1:1 across coordinate-preserving operations (`table.convertArchetype`, `Transpose`, `SelectAndTransform` row/column subsets, `RandomSample`, `PruneRows(EveryNth)`, `Math`, `Normalize`, and `RemoveBaseline`).
- **Invalidated & stripped:** Removed whenever a cell is manually overwritten, cleared, or deleted (and restored on Undo), dropped on aggregating or formula-derived outputs (`PruneRows(AverageBins)`, calculated columns), and cleared by `strip_project_data_for_template` when saving `.cbtemplate` project templates.

See [Concepts & Document Architecture](concepts.md), [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Command Reference](../commands.md) for parameter schemas.
