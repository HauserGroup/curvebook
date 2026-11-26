# Data tables, calculated variables & transforms

CurveBook separates experimental structure from visual styling using eight strictly typed table archetypes, an AST-verified formula language for `=` Calculated Variables, and non-destructive transform and reshaping pipelines.

---

## The eight table archetypes {#table-archetypes}

Every data table belongs to one of eight archetypes:

| Archetype | Primary purpose | Column & subcolumn layout |
|---|---|---|
| **XY** (`Xy`) | Continuous predictor $X$ vs. response $Y$ (regression, curves, correlation, Bland–Altman, ROC, AUC). | Optional $X$ column plus datasets A, B, … with raw replicates or summary subcolumns. |
| **Column** (`Column`) | One-factor comparison across independent or matched groups (*t*-tests, one-way ANOVA, descriptive stats, normality, outliers, *P*-value stacks). | One column per group (A, B, …) with single values, raw replicates, or summary subcolumns. |
| **Grouped** (`Grouped`) | Two-way, three-way, or multi-factor cell designs (`Row Factor × Column Factor`, optional C-factor slice). | Row labels as Factor A and dataset columns as Factor B (or B × C) with replicates or summaries. |
| **Contingency** (`Contingency`) | Cross-tabulated non-negative integer counts ($\chi^2$, Fisher exact, $2 \times 2$ risk/odds ratios, McNemar, Cochran–Armitage, $\kappa$). | Non-negative whole-number counts arranged by category rows and columns. |
| **Survival** (`Survival`) | Right-censored time-to-event outcomes (Kaplan–Meier, log-rank, Gehan–Breslow–Wilcoxon, trend, cohort HR). | Shared follow-up time column plus cohort columns with event/censor indicators (`1` = event, `0` = censored by default). |
| **Parts of Whole** (`PartsOfWhole`) | Proportions summing to a total (parts-of-whole charts, goodness-of-fit $\chi^2$, exact binomial tests). | Single value column paired with slice labels for category counts or percentages. |
| **Multiple Variables** (`MultipleVariables`) | Tidy observational tables (rows = subjects, columns = variables) for GLMs, Cox PH, PCA, clustering, and multifactor ANOVA. | Typed columns with explicit roles (continuous, categorical, label, or calculated) and level ordering. |
| **Nested** (`Nested`) | Hierarchical cluster designs (fixed treatment groups containing random subjects, litters, or wells). | Treatment groups containing subcluster columns with technical replicate subcolumns. |

Additional structural options by archetype:

- **XY predictor modes:** The optional $X$ column accepts numeric values, dates, sequence progression, or elapsed times, or can be omitted.
- **Column designs:** Supports unpaired replicates, paired/repeated measures, or precomputed error entry, with optional row titles.
- **Multiple Variables columns:** Each variable column can hold entered values or a live `=` calculated formula, with automatic role inference or explicit role and level-order settings.

### Subcolumn modes & pre-summarized entry {#subcolumn-modes}

Continuous tables (XY, Column, Grouped, and Nested) support eight subcolumn modes:

| Mode | Subcolumns | Interpretation | Analysis compatibility |
|---|---|---|---|
| `Single` | 1 | Single value per row or cell. | All standard analyses. |
| `RawReplicates(n)` | 1 to 512 | Individual raw technical or biological replicates. | Full parametric, nonparametric, normality, residuals, and robust/ROUT fits. |
| `MeanSdN` | 3 (Mean, SD, N) | Pre-summarized normal moments. | Parametric *t*-tests, one-way/Welch/Brown–Forsythe ANOVA, regression, and descriptive moments. |
| `MeanSemN` | 3 (Mean, SEM, N) | Mean, standard error ($\text{SD} = \text{SEM}\sqrt{N}$), and sample size. | Same summary-capable routines as `MeanSdN`. |
| `MeanCvN` | 3 (Mean, %CV, N) | Mean, coefficient of variation ($\text{SD} = |\text{Mean}|\cdot\%\text{CV}/100$), and N. | Same parametric moment reconstruction and plotting as `MeanSdN`. |
| `MeanLowHigh` | 3 (Mean, −Error, +Error) | Mean with lower and upper error spans. | Plotting and central-value extraction. |
| `UpperLowerLimits` | 3 (Mean, Lower, Upper) | Central mean with explicit lower and upper bounds. | Plotting and explicit error bounds. |
| `MedianIqr` | 3 (Median, Q1, Q3) | Median and interquartile bounds. | Box and error plotting. |

Subcolumn compatibility and layout notes:

- **Raw-replicate requirement:** Normality tests, ROUT/Grubbs outlier detection, and rank tests (such as Kruskal–Wallis and Mann–Whitney) reject pre-summarized modes with `RequiresRawReplicates`. `MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr` are also rejected where raw replicates or `(Mean, SD, N)` moments are required.
- **Replicate stacking:** Replicate subcolumns can be treated side by side or stacked during plotting and summary extraction.

---

## `=` calculated variables & formula language {#calculated-variables}

Data tables support live formula evaluation (`table.setCalculatedVariable`) through an AST-verified parser, evaluator, and dependency graph.

### Formula references & constants {#formula-references}

- **Prefix:** Formulas begin with `=` (the leading `=` is stripped before parsing).
- **Variable and cell references:**
  - Bare identifiers (`Conc`, `logConc`, `Ctrl_1`) or bracketed names (`[Blank Sub]`) matching column or variable titles.
  - Explicit index or quoted title lookups: `col("Title")` or `col(idx)` (0-based index or column letter).
  - Experiment metadata constants from linked info sheets: `Info("Key")`.
  - Row index builtins: `$row` (1-based row number) and `$index` (0-based row index).
  - Archetype aliases: `X` and `Y` on XY and Survival tables.
- **Built-in constants:** `pi`, `e`, `tau` ($2\pi$), `true` (`1.0`), `false` (`0.0`), and `nan` (missing). Matching column titles take precedence over `pi`, `e`, and `tau` (exact case-sensitive match is checked first for `e`).
- **Circular dependency guard:** Cross-column references form a dependency DAG. Self-references and multi-column cycles fail cleanly with `CycleDetected`.

### Operators & three-valued logic {#formula-operators}

| Precedence | Operators | Semantics |
|---:|---|---|
| Highest | `^` | Exponentiation (right-associative; domain errors propagate as missing). |
| Unary | `+`, `-`, `!` | Numeric sign flip; `!` maps missing to missing, `0.0` to `1.0`, nonzero to `0.0`. |
| Multiplicative | `*`, `/`, `%` | Multiplication, division (`0` divisor yields missing), floating remainder. |
| Additive | `+`, `-` | Addition and subtraction. |
| Comparison | `<`, `<=`, `>`, `>=`, `==`, `=`, `!=`, `<>` | Numeric ordering and numeric/text equality (missing yields missing). |
| Unary word | `NOT` | Word-form negation (binds looser than comparisons: `NOT x == 0` parses as `NOT (x == 0)`). |
| Conjunction | `&&`, `AND` | Strong Kleene 3-valued AND (`false && Missing` → `false`; `true && Missing` → `Missing`). |
| Disjunction | `\|\|`, `OR` | Strong Kleene 3-valued OR (`true \|\| Missing` → `true`; `false \|\| Missing` → `Missing`). |

### Supported functions & clauses {#formula-functions}

- **Arithmetic, rounding & clamping:** `abs`, `sqrt`, `cbrt`, `sqr`, `pow(x, p)`, `clamp(x, lo, hi)`, `floor`, `ceil`, `round(x[, digits])`, `trunc`, `int` (alias of `floor`), `sign` / `sgn`, `min(...)`, `max(...)`, `sum(...)`, `mean(...)`.
- **Logarithmic, exponential & trigonometric:** `ln`, `log(x[, base])`, `log10`, `log2`, `exp`, `exp10`, `sin`, `cos`, `tan`, `asin` / `arcsin`, `acos` / `arccos`, `atan` / `arctan`, `atan2` / `arctan2`, `sinh`, `cosh`, `tanh`.
- **Special & statistical transforms:** `erf`, `erfc`, `lgamma`, `logit(p)` (for $p \in (0, 1)$), `probit(p)` ($5 + \Phi^{-1}(p)$, Finney probit scale), `norm_ppf` / `norm_inv` / `qnorm` / `inv_norm`, `arcsin_sqrt(p)` ($\arcsin\sqrt{p}$ for $p \in [0, 1]$).
- **Logical & missing-value handling:** `if(cond, then_expr, else_expr)` (evaluates only the chosen branch), `isnan(x)`, `isvalid(x)` / `is_defined(x)`, `coalesce(a, b, ...)`, variadic `and(...)`, `or(...)`, `not(x)`.
- **Whole-column aggregates:** `sum`, `mean`, `count` (active numeric cells), `counta` (active non-empty numeric or text cells), `sd` / `stdev`, `sem` / `stderr`, `var`, `median`, `iqr`, `min_col` / `col_min`, `max_col` / `col_max`, `geo_mean`, `percentile(col, p)` / `quantile(col, p)`.
- **Conditional `*if` aggregates:** `countif`, `countaif`, `sumif`, `meanif`, `minif`, `maxif`, `stdevif` / `sdif`, `stderrif` / `semif`. Argument order is `(values, condition)` (or `countif(condition)` / `countif(values, condition)`), matching rows where `condition` is finite and nonzero. `countaif` counts active non-empty numeric or text cells matching the predicate.
- **Window functions:** `row_number()`, `rank()`, `dense_rank()`, `center(expr)`, `standardize(expr)` / `zscore(expr)`, `cumsum(expr)`, `cumprod(expr)`, `cummean(expr)`, `lag(expr[, offset[, default]])`, `lead(expr[, offset[, default]])`, `diff(expr[, offset])`, `rolling_mean(expr, window)` / `moving_avg(expr, window)`, `rolling_sum(expr, window)`.
- **Filter & window clauses:** `FILTER (WHERE <cond>)` / `WHERE <cond>` (inside aggregates and `OVER (...)`), plus `OVER ([PARTITION BY ...] [ORDER BY <expr> [ASC|DESC] [NULLS FIRST|LAST]] [ROWS <frame>])` supporting `UNBOUNDED PRECEDING`, `<k> PRECEDING`, `CURRENT ROW`, `<k> FOLLOWING`, and `UNBOUNDED FOLLOWING`.
- **Evaluator scope & error propagation:** Values are 64-bit numbers, text, or missing. String-manipulation and date/calendar arithmetic functions are not present in Beta. Empty cells, excluded cells, division by zero, out-of-domain inputs (`ln(-1)`, `sqrt(-1)`, `probit(0)`), non-finite intermediates (`NaN`, `±Inf`), and type mismatches propagate deterministically as missing (stored as an empty cell).

---

## Table transforms, reshaping & simulation {#transforms-and-reshaping}

CurveBook executes derived transforms without modifying upstream raw cells (`transform.run`, `table.convertArchetype`, `table.importPlateMap`, `table.importTidyLong`, `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions`, `table.simulateXy`):

| Transform / operation | Purpose |
|---|---|
| **Math transforms** | Element-wise transforms over $X$ or $Y$ (logs, powers, trig/logit/probit, linear scaling, concentration to $-\log\text{M}$). |
| **Normalize** | Scales each dataset between 0% and 100% (or 0.0 to 1.0) using selectable anchor definitions. |
| **Remove baseline** | Subtracts or ratios designated baseline columns as difference, ratio, percent change, or fractional change. |
| **Transpose & select-and-transform** | Swaps rows and columns, subsets or reorders rows/columns, filters by predicate, or transforms columns. |
| **Curve calculus & AUC** | Smooths, differentiates, integrates, or computes trapezoidal Area Under the Curve on continuous curves. |
| **Archetype conversion** | Converts tables among Column, Grouped, XY, Multiple Variables, and Contingency with a non-mutating preview. |
| **Microplate mapping** | Maps 96-well or 384-well grids, sheets, or analyzable results into typed tables with well provenance. |
| **Tidy long-to-wide pivot** | Reshapes long-format rows into typed XY, Column, or Grouped replicate subcolumns. |
| **Randomization & row sampling** | Runs seeded treatment-arm allocation and uniform or stratified $k$-of-$N$ row sampling. |
| **Distribution & XY simulation** | Generates seeded random columns or synthetic noisy XY curves deterministically. |
| **4-stage linked import pipeline** | Reconciles linked sheets or text through filter → reshape/transpose → baseline → exclusions. |

Options, edge cases, and safety guards:

- **Math transforms:** Built-in presets cover natural log, $\log_{10}$, $\log_2$, $10^x$, $e^x$, reciprocal, square root, square, logit, probit, $\arcsin\sqrt{p}$, linear scaling (`slope`, `intercept`), and concentration to $-\log\text{M}$ (`molar_unit_factor`), alongside arbitrary `=` formulas (such as `2^-y` qPCR pipelines). Non-positive inputs to log/reciprocal, negative inputs to square root, or probabilities outside $(0, 1)$ for logit/probit yield missing cells or typed errors rather than `NaN`.
- **Normalize:** `zero_anchor` and `hundred_anchor` accept column min, max, or sum; first or last row; a fixed scalar; a control column mean; or a hooked value. Equal anchors ($y_{100} = y_0$) fail cleanly to avoid division by zero.
- **Remove baseline:** Zero baseline divisors in ratio, percent change, or fractional change modes omit undefined entries or return an error; excluded cells stay skipped.
- **Transpose and selection:** Enforces row, dataset, and 10,000,000 imported-cell caps while remapping exclusion masks atomically.
- **Curve calculus and AUC:** Smoothing supports moving average, Savitzky–Golay, and Cleveland (1979) LOWESS (bisquare $6 \times \text{MAD}$ robustness, `span` in $(0, 1]$). Differentiation uses 3-point Lagrange quadratic first/second derivatives on sorted uniform or non-uniform $X$ (duplicate $X$ values fail with `InvalidInput`). Integration computes cumulative trapezoidal area above `baseline`, and AUC computes total, net, and baseline-corrected peaks (Yeh & Kwan 1978).
- **Archetype conversion (`table.convertArchetype`):** Preview reports `preserved_cells`, `dropped_cells`, `preserved_exclusions`, `warnings`, and `lossy`. Exclusion flags and notes are preserved and remapped. Lossy conversions (such as negative or fractional numbers targeting Contingency, or collapsed subcolumns) are refused unless `allow_lossy` is `true`.
- **Microplate mapping (`table.importPlateMap`):** Validates strict 8×12 or 16×24 grids, up-to-date freshness on results sources, non-empty assignments, and cell budgets; saves reusable `.cbplate` layouts (see [Composable microplate mapping](#microplate-mapping)).
- **Tidy long-to-wide pivot (`table.importTidyLong`):** Accepts columns for X or row factor, group, optional replicate index, value, and optional exclusion flag. Preserves exclusion flags and rejects out-of-range column indices or expansions exceeding 512 subcolumns.
- **Randomization and sampling:** `table.randomizeGroups` supports seeded complete or permuted-block allocation across $k \ge 2$ arms and writes ARRIVE metadata into an info sheet (block sizes must be positive multiples of the summed integer allocation weights). `table.randomSampleRows` performs partial Fisher–Yates sampling without replacement and rejects empty strata or $k > N$.
- **Simulation:** `table.simulateDistributions` generates uniform, normal, log-normal, binomial, Poisson, or exponential columns, and `table.simulateXy` generates noisy XY curves with Gaussian, uniform, or multiplicative log-normal noise. Both are deterministic across native and WebAssembly targets for a given 64-bit seed.
- **Linked import pipelines:** Topological cycle detection rejects circular sheet links before mutating the project.

---

## Multi-block tabular import (`.xlsx` & `.csv` / `.tsv`) {#tabular-import-options}

CurveBook provides explicit range slicing, exact OOXML merge expansion, discriminator filtering and splitting, and preamble metadata extraction across `.xlsx` and delimited text imports without fragile layout heuristics.

### `.xlsx` workbook import options (`table.importXlsx`) {#xlsx-import-options}

All advanced `.xlsx` controls are opt-in and apply in deterministic order: `fill_merged_cells` → `cell_range` crop → `skip_rows` (plus optional `extract_preamble_info`) → `header_rows` → `row_match` or `split_by_column` → archetype inference.

- **`cell_range` (single-sheet rule):** Optional A1 rectangle (for example `"B4:M11"`) preserving exact rectangle dimensions, including trailing empty cells. `cell_range` requires a single selected worksheet; passing it with multiple sheets fails and asks you to select one sheet via `sheets` or `sheet_indices`.
- **`skip_rows`:** Number of leading rows skipped before `header_rows` (counted relative to `cell_range` when specified).
- **`fill_merged_cells` (default `false`):** Expands each merged region's top-left value across its exact OOXML `<mergeCell>` bounds before cropping and header extraction. Unmerged empty cells remain empty (CurveBook never carries values right into unmerged blanks).
- **`split_by_column`:** Zero-based discriminator column index. Groups rows by trimmed text in first-seen block order (handling stacked channels and interleaved cycles), drops the discriminator column, skips blank separator rows, and warns on non-blank rows with an empty discriminator. Emits one data table per block with deterministic IDs (`{base_id}_{block_slug}` with `_2`, `_3`, … collision suffixes), titles `"{sheet} — {block}"`, and independent archetype inference. Mutually exclusive with `row_match`.
- **`row_match`:** Filter `{ column, equals, drop_column }` (`drop_column` defaults to `true`) retaining only rows whose trimmed value in zero-based `column` equals `equals`. Mutually exclusive with `split_by_column`.
- **`extract_preamble_info` (default `false`):** When `true` and `skip_rows > 0`, parses `Key: Value` and `Key = Value` constants from skipped leading rows. Free-text preamble lines go into `notes_markdown` on one shared info sheet linked to every imported table.
- **Importing bare plate grids into an open project:** `table.importXlsx` does not take `sheet_id`, `link`, or `filter`. To slice a bare 8×12 or 16×24 layout or concentration grid (such as `cell_range: "B4:M11"`) into an open study, pass `merge: true` and `has_header: false`. Add `fill_merged_cells: true` when labels span merged cells so the existing project stays intact and row A is not used as headers.

### `.csv` / `.tsv` import windowing, splitting & preamble extraction (`table.importCsv`) {#csv-import-options}

- **Windowing via `filter`:** `table.importCsv` honors `filter` (`row_window` or `row_start`/`row_end`, `col_window` or `col_start`/`col_end`, `decimate_every_k`, `parse_asterisk_exclusions`, and `row_match`) on new sheets, updated sheets, and live paste-link pipelines. Use `filter.row_start` or `filter.row_window` to skip leading rows.
- **`row_match`:** Retains only rows whose trimmed value in `column` matches `equals`, dropping that column by default (`drop_column: true`).
- **`split_by_column`:** Splits stacked or interleaved delimited blocks into one data table per distinct trimmed discriminator value using the same `{base_id}_{block_slug}` ID scheme (`_2`, `_3`, … collision suffixes) as `.xlsx` import. Blank separator rows are skipped and non-blank rows with an empty discriminator emit a warning. Mutually exclusive with `filter.row_match`, and rejected when targeting an existing `sheet_id` or `link: true` (use `filter.row_match` instead).
- **`extract_preamble_info` (default `false`):** When `true` and leading rows are skipped (`row_start > 0`), extracts `Key: Value` and `Key = Value` constants using `decimal_separator` (including delimiter-padded lines like `RunId: 42,,,,`). Free-text preamble lines go into `notes_markdown` on one shared info sheet linked to all emitted tables.

### Numeric-X contract for kinetic imports {#numeric-x-contract}

> [!IMPORTANT]
> CurveBook's `.xlsx` and `.csv`/`.tsv` importers do **not** parse Excel `numFmt` day-fraction time serials or text timestamps such as `hh:mm:ss`. For kinetic analyses ([Area Under the Curve](registry/tables.html#analysis-kind-area-under-curve)) and kinetic plate reducers (`x_window_mean`), the `X` column must contain **finite numeric values in the desired integration units** (for example, elapsed seconds or minutes).

- **Spreadsheet time-formatted numeric cells:** Spreadsheets commonly store times as numeric fractions of a 24-hour day (such as `0.00069444...` formatted as `00:01:00`). Because `.xlsx` import reads the stored number directly without interpreting display `numFmt`, integrating those cells yields AUC in **day units** unless converted to seconds or minutes first.
- **Text timestamps (`hh:mm:ss`):** Strings such as `"00:05:00"` import as text cells, are skipped by numeric `X` extraction, and fail with a diagnostic explaining the numeric-X requirement rather than integrating arbitrary numbers.

---

## Composable microplate mapping (`table.importPlateMap`) {#microplate-mapping}

`table.importPlateMap` maps 8×12 (96-well) or 16×24 (384-well) microplate readings into typed Column, XY, Grouped, or Multiple Variables tables while preserving per-cell well traceability.

### Reading sources: direct values vs. analyzable sheets & kinetic reducers {#plate-sources}

Provide either `values` (a direct flat 96/384 or 2D 8×12 / 16×24 numeric array) or `source` (mutually exclusive):

- **`source`:** Reads measurements from an existing data table or an analyzable results sheet (such as [Area Under the Curve](registry/tables.html#analysis-kind-area-under-curve)):
  - **Well-per-row tables (`value_column`):** For tables with one row per well (such as AUC analyzable output), specify `value_column` matching the numeric column header (trimmed, case-insensitive; e.g., `"Net Area"`, `"Positive Area"`, `"Negative Area"`, `"Total Area"`).
  - **Well-per-column kinetic or column tables (`reducer`):** For XY or Column tables where each column is a well, choose one of four kinetic reducers (all skip excluded cells):
    - `{"row_index": n}`: active numeric value at zero-based row `n`.
    - `{"x_window_mean": {"x_min": a, "x_max": b}}`: arithmetic mean of active values with finite numeric `X` in inclusive `[a, b]` (requires an XY table with numeric `X`).
    - `"min"` or `"max"`: minimum or maximum active numeric value in the well column.
- **`match_wells`:**
  - `by_well_id` *(default)*: matches canonical or unpadded well IDs (`"A01"`, `"A1"`) or leading well-ID tokens before delimiters (such as `"A01 - Sample X1"`). Rejects duplicate collisions or zero matches; missing wells remain empty with non-blocking warnings.
  - `positional`: maps source rows or columns sequentially in row-major plate order (`A01..H12` or `A01..P24`).
- **Static snapshot & freshness guard:** `table.importPlateMap` creates an independent static data table snapshot (later edits to upstream kinetic sheets do not overwrite it). When `source.sheet_id` names a results sheet, CurveBook refuses the read (`Conflict`) unless the results sheet is up to date, and records the upstream `source_sheet_id` and `value_column` or reducer in a floating note.

### Plate layouts: grids, layout sheets, `x_grid` & controls {#plate-layouts}

Supply well assignments via either `template` / `roles` or `label_grid` / `layout_sheet_id` (mutually exclusive):

- **`label_grid` / `layout_sheet_id`:** Strictly 8×12 (96-well) or 16×24 (384-well). When reading `layout_sheet_id` or `x_grid_sheet_id` from a data table, CurveBook extracts only data cells (excluding column and row titles). Dimension mismatches like 9×13 or 17×25 fail with a header-exclusion hint, and 7×12 or 15×24 grids remind you to re-import with `has_header: false`. Labels are strictly literal (no inline concentration parsing); empty cells become excluded; `blank_labels` and `positive_control_labels` match trimmed case-insensitively; grid layouts assign 0-based replicate indices (`0`, `1`, …).
- **Optional `x_grid` / `x_grid_sheet_id`:** Matching 8×12 or 16×24 numeric grid supplying per-well `dilution_x` (for example, shifted concentration ladders). When omitted, treatment wells default to `dilution_x = 0.0` (standard for Column screening tables; emits a non-blocking single-X warning for XY and Grouped tables). When `x_grid` or `x_grid_sheet_id` is supplied, any treatment well with an empty or non-finite X value triggers a hard validation error naming the well and label.
- **Controls & reference arms:** Setting `subtract_blank_mean: true` subtracts the mean of active blank wells from readings. On Column targets, wells matching `positive_control_labels` pool into one `"Positive Control"` column, and blank wells appear as a `"Blank"` column when `subtract_blank_mean` is `false`. To keep separate named reference arms for multiple comparisons (such as distinct vehicle and agonist controls for Dunnett's test), assign them as regular treatment groups and apply non-destructive **Remove Baseline** or **Normalize** (control column mean) transforms.
- **Versioned `.cbplate` persistence:** Reusable layouts save and load as single-extension `.cbplate` UTF-8 JSON (`format_version = 1`; see [`plate-map.schema.json`](../schemas/plate-map.schema.json) and [Persisted File Formats](../file-formats.md)), written atomically with forward-version refusal.

### Well provenance lifecycle & `well_cell_map` {#plate-well-origins}

`table.importPlateMap` records per-cell well origins mapping `(row, col, subcol)` to well ID across Column, XY, Grouped, and Multiple Variables outputs and returns the ordered `well_cell_map` in its response:

- **Preserved:** Kept intact across `table.excludeCells` and `table.excludeCellsWithReason` (for well-ID inspection during outlier and QC audits) and saved inside `.curvebook` schema v8 archives.
- **Shifted & remapped:** Shifted automatically on row insertions and remapped 1:1 across coordinate-preserving operations (`table.convertArchetype`, transpose, select-and-transform row/column subsets, random sampling, every-Nth row pruning, math transforms, normalization, and baseline removal).
- **Invalidated & stripped:** Cleared whenever a cell is manually overwritten, cleared, or deleted (and restored on Undo), dropped on aggregating or formula-derived outputs (bin-averaged pruning, calculated columns), and stripped when saving `.cbtemplate` project templates.

See [Concepts & Document Architecture](concepts.md), [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Command Reference](../commands.md) for parameter schemas.
