# Data tables, calculated variables and transforms

<!-- Audience: Tier 2 Technical Reference — exhaustive table archetypes, subcolumn schemas, formula syntax, transforms, and import options belong here; link to Tier 1 guides (desktop-gui.md) for scientist walkthroughs. -->

This technical reference covers CurveBook's eight typed table archetypes, subcolumn modes, the formula language for `=` Calculated Variables, non-destructive transforms, import options for tabular files, and microplate mapping. For a quick walkthrough of creating tables and entering data in the app, see the [Desktop GUI guide](desktop-gui.md).

---

## The eight table archetypes {#table-archetypes}

Every data table belongs to one of eight strictly typed archetypes:

| Archetype | Primary purpose | Column and subcolumn layout |
|---|---|---|
| **XY** (`Xy`) | A continuous predictor $X$ against a response $Y$ (regression, curves, correlation, Bland–Altman, ROC, AUC). | An optional $X$ column, plus datasets A, B, … holding raw replicates or summary subcolumns. |
| **Column** (`Column`) | Comparing one factor across independent or matched groups (*t*-tests, one-way ANOVA, descriptive statistics, normality, outliers, *P*-value stacks). | One column per group (A, B, …) holding single values, raw replicates, or summary subcolumns. |
| **Grouped** (`Grouped`) | Designs with two, three, or more factors (`Row Factor × Column Factor`, with an optional slice for a third factor, C). | Row labels form Factor A and dataset columns form Factor B (or B × C), holding replicates or summaries. |
| **Contingency** (`Contingency`) | Cross-tabulated counts of non-negative integers ($\chi^2$, Fisher's exact test, $2 \times 2$ risk and odds ratios, McNemar, Cochran–Armitage, $\kappa$). | Non-negative whole-number counts arranged in category rows and columns. |
| **Survival** (`Survival`) | Right-censored time-to-event outcomes (Kaplan–Meier, log-rank, Gehan–Breslow–Wilcoxon, trend, cohort hazard ratios). | A shared follow-up time column, plus one column per cohort holding event/censoring indicators (by default, `1` = event and `0` = censored). |
| **Parts of Whole** (`PartsOfWhole`) | Proportions that add up to a total (parts-of-whole charts, $\chi^2$ goodness of fit, exact binomial tests). | A single value column paired with slice labels, holding category counts or percentages. |
| **Multiple Variables** (`MultipleVariables`) | Tidy observational tables, with one row per subject and one column per variable (GLMs, Cox proportional hazards, PCA, clustering, and multifactor ANOVA). | Typed columns with explicit roles (continuous, categorical, label, or calculated) and a level order. |
| **Nested** (`Nested`) | Hierarchical designs in which fixed treatment groups contain random clusters such as subjects, litters, or wells. | Treatment groups containing subcluster columns, each with subcolumns for technical replicates. |

Some archetypes have additional structural options:

- **XY predictor modes:** The optional $X$ column accepts numeric values, dates, a sequence, or elapsed times. You can also leave out $X$ entirely.
- **Column designs:** A Column table can hold unpaired replicates, paired or repeated measures, or precomputed error values. Row titles are optional.
- **Multiple Variables columns:** Each variable column holds either entered values or a live `=` calculated formula. CurveBook can infer each column's role, or you can set the role yourself and choose a custom level order.

### Subcolumn modes and pre-summarized entry {#subcolumn-modes}

Tables of continuous data (XY, Column, Grouped, and Nested) support these subcolumn modes:

| Mode | Subcolumns | Interpretation | Analysis compatibility |
|---|---|---|---|
| `Single` | 1 | One value per row or cell. | All standard analyses. |
| `RawReplicates(n)` | 1 to 512 | Individual raw technical or biological replicates. | Full support for parametric and nonparametric analyses, normality tests, residuals, and robust/ROUT fits. |
| `MeanSd` | 2 (Mean, SD) | Pre-summarized mean and SD, without the sample size $N$. | Plotting and extracting central values. |
| `MeanSem` | 2 (Mean, SEM) | Pre-summarized mean and SEM, without the sample size $N$. | Plotting and extracting central values. |
| `MeanSdN` | 3 (Mean, SD, N) | Pre-summarized moments of a normal distribution. | Parametric *t*-tests, one-way, Welch, and Brown–Forsythe ANOVA, regression, and descriptive moments. |
| `MeanSemN` | 3 (Mean, SEM, N) | Mean, standard error ($\text{SD} = \text{SEM}\sqrt{N}$), and sample size. | The same routines that accept `MeanSdN` summaries. |
| `MeanCvN` | 3 (Mean, %CV, N) | Mean, coefficient of variation ($\text{SD} = |\text{Mean}|\cdot\%\text{CV}/100$), and $N$. | The same moment reconstruction for parametric tests, and the same plotting, as `MeanSdN`. |
| `MeanLowHigh` | 3 (Mean, −Error, +Error) | Mean with lower and upper error spans. | Plotting and extracting central values. |
| `UpperLowerLimits` | 3 (Mean, Lower, Upper) | Mean with explicit lower and upper bounds. | Plotting with explicit error bounds. |
| `MedianIqr` | 3 (Median, Q1, Q3) | Median and the bounds of the interquartile range. | Box plots and error bars. |

Notes on compatibility and layout:

- **Analyses that need raw replicates:** Normality tests, ROUT and Grubbs outlier detection, and rank tests (such as Kruskal–Wallis and Mann–Whitney) reject pre-summarized modes with `RequiresRawReplicates`.
- **Analyses that need moments:** When an analysis requires either raw replicates or `(Mean, SD, N)` moments, it also rejects `MeanSd`, `MeanSem`, `MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr`.
- **Replicate stacking:** For plotting and summary extraction, replicate subcolumns can be treated side by side or stacked.

---

## `=` calculated variables and formula language {#calculated-variables}

Data tables can hold live formulas (`table.setCalculatedVariable`). CurveBook parses each formula into a verified syntax tree, evaluates it, and tracks the columns it depends on. To turn computed cells into ordinary editable values, run `table.unlinkCalculatedVariable`. Unlinking keeps the undo/redo history and marks downstream analyses for recalculation.

### Formula references and constants {#formula-references}

- **Prefix:** Formulas begin with `=`. CurveBook removes the leading `=` before parsing.
- **Column and cell references:**
  - Use bare identifiers (`Conc`, `logConc`, `Ctrl_1`) or bracketed names (`[Blank Sub]`) to refer to columns or variables by title.
  - Use `col("Title")` or `col(idx)` to look up a column explicitly by quoted title, by 0-based index, or by column letter.
  - Use `Info("Key")` to read an experiment constant from a linked info sheet.
  - Use `$row` (the 1-based row number) and `$index` (the 0-based row index) for row positions.
  - On XY and Survival tables, `X` and `Y` are aliases for the archetype's columns.
- **Built-in constants:** `pi`, `e`, `tau` ($2\pi$), `true` (`1.0`), `false` (`0.0`), and `nan` (missing). A column with a matching title takes precedence over `pi`, `e`, or `tau`. For `e`, CurveBook checks for an exact, case-sensitive title match first.
- **Circular dependencies:** References between columns form a dependency DAG. A formula that refers to itself, or a cycle across several columns, fails with `CycleDetected`.

### Operators and three-valued logic {#formula-operators}

| Precedence | Operators | Meaning |
|---:|---|---|
| Highest | `^` | Exponentiation (right-associative; domain errors propagate as missing). |
| Unary | `+`, `-`, `!` | Numeric sign change. `!` maps missing to missing, `0.0` to `1.0`, and any nonzero value to `0.0`. |
| Multiplicative | `*`, `/`, `%` | Multiplication, division (a `0` divisor gives missing), and floating-point remainder. |
| Additive | `+`, `-` | Addition and subtraction. |
| Comparison | `<`, `<=`, `>`, `>=`, `==`, `=`, `!=`, `<>` | Numeric ordering, and equality for numbers or text (a missing operand gives missing). |
| Unary word | `NOT` | Negation written as a word. It binds more loosely than comparisons, so `NOT x == 0` parses as `NOT (x == 0)`. |
| Conjunction | `&&`, `AND` | Strong Kleene three-valued AND (`false && Missing` → `false`; `true && Missing` → `Missing`). |
| Disjunction | `\|\|`, `OR` | Strong Kleene three-valued OR (`true \|\| Missing` → `true`; `false \|\| Missing` → `Missing`). |

### Supported functions and clauses {#formula-functions}

| Category | Supported functions and syntax |
|---|---|
| Arithmetic, rounding and clamping | `abs`, `sqrt`, `cbrt`, `sqr`, `pow(x, p)`, `mod(x, y)`, `clamp(x, lo, hi)`, `floor`, `ceil`, `round(x[, digits])`, `trunc`, `int`, `sign` / `sgn`, `rad(deg)`, `deg(rad)`, `perc(x)`, `fact(n)`, `pi([n])`, `min(...)`, `max(...)`, `sum(...)`, `mean(...)`. |
| Logarithmic, exponential, trigonometric and hyperbolic | `ln`, `log(x[, base])`, `log10`, `log2`, `exp`, `exp10`, `sin`, `cos`, `tan`, `asin` / `arcsin`, `acos` / `arccos`, `atan` / `arctan`, `atan2` / `arctan2`, `sinh`, `cosh`, `tanh`, `asinh` / `arcsinh`, `acosh` / `arccosh`, `atanh` / `arctanh`. |
| Special and statistical | `erf`, `erfc`, `gamma`, `lgamma` / `gammaln` / `lngamma`, `igamma(a, x)`, `igammac(a, x)`, `beta(a, b)`, `ibeta(a, b, x)`, `psi` / `digamma`, `besselj(n, x)`, `bessely(n, x)`, `besseli(n, x)`, `besselk(n, x)`, `hypgeometricm` / `hyp1f1`, `hypgeometricu` / `confluent_u`, `hypgeometricf` / `hyp2f1`, `logit(p)` ($p \in (0, 1)$), `probit(p)` ($5 + \Phi^{-1}(p)$, Finney scale), `norm_ppf` / `norm_inv` / `qnorm` / `inv_norm`, `arcsin_sqrt(p)` ($\arcsin\sqrt{p}$ for $p \in [0, 1]$). |
| Text, seeded random values and missing values | `concatenate(...)` / `concat(...)` (text concatenation with a length limit), `gauss(...)` and `rnd(...)` (random values from a seeded generator, reproducible from the seed), `nan` / `null`. |
| Logic and missing values | `if(cond, then_expr, else_expr)` (evaluates only the branch that is chosen), `isnan(x)`, `isvalid(x)` / `is_defined(x)`, `coalesce(a, b, ...)`, and the variadic `and(...)`, `or(...)`, and `not(x)`. |
| Whole-column aggregates | `sum`, `mean`, `count` (active numeric cells), `counta` (active non-empty numeric or text cells), `sd` / `stdev`, `sem` / `stderr`, `var`, `median`, `iqr`, `min_col` / `col_min`, `max_col` / `col_max`, `geo_mean`, `percentile(col, p)` / `quantile(col, p)`. |
| Conditional `*if` aggregates | `countif`, `countaif`, `sumif`, `meanif`, `minif`, `maxif`, `stdevif` / `sdif`, `stderrif` / `semif`. |
| Window functions | `row_number()`, `rank()`, `dense_rank()`, `center(expr)`, `standardize(expr)` / `zscore(expr)`, `cumsum(expr)`, `cumprod(expr)`, `cummean(expr)`, `lag(expr[, offset[, default]])`, `lead(expr[, offset[, default]])`, `diff(expr[, offset])`, `rolling_mean(expr, window)` / `moving_avg(expr, window)`, `rolling_sum(expr, window)`. |

Clause syntax and evaluation rules:

- **Argument order for `*if` aggregates:** Pass `(values, condition)`. `countif` also accepts `countif(condition)` or `countif(values, condition)`. A row matches when `condition` is finite and nonzero. `countaif` counts the active non-empty numeric or text cells that match the condition.
- **Filter clauses:** Use `FILTER (WHERE <cond>)` or `WHERE <cond>` inside aggregates and `OVER (...)` expressions.
- **Window frames:** `OVER ([PARTITION BY ...] [ORDER BY <expr> [ASC|DESC] [NULLS FIRST|LAST]] [ROWS <frame>])` supports `UNBOUNDED PRECEDING`, `<k> PRECEDING`, `CURRENT ROW`, `<k> FOLLOWING`, and `UNBOUNDED FOLLOWING`.
- **Types and missing values:** A value is a 64-bit number, text, or missing. The Beta release has no date or calendar arithmetic functions. The following always propagate as missing, which is stored as an empty cell:
  - empty cells and excluded cells;
  - division by zero;
  - out-of-domain inputs (`ln(-1)`, `sqrt(-1)`, `probit(0)`);
  - non-finite intermediate results (`NaN`, `±Inf`);
  - type mismatches.

---

## Table transforms, reshaping and simulation {#transforms-and-reshaping}

CurveBook runs transforms as derived operations, so the raw cells upstream are never modified. The main commands are `transform.run`, `table.convertArchetype`, `table.importPlateMap`, `table.importTidyLong`, `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions`, and `table.simulateXy`.

| Transform or operation | Purpose |
|---|---|
| Math transforms | Applies a transform to each value of $X$ or $Y$ (logs, powers, trigonometric, logit or probit functions, linear scaling, or concentration to $-\log\text{M}$). |
| Normalize | Scales each dataset to run from 0% to 100% (or from 0.0 to 1.0), using anchor definitions that you choose. |
| Remove baseline | Removes a baseline by subtraction or by ratio. The baseline can come from row-wise columns, column means, a constant, the mean of a row range, a line through the first and last points, an OLS trend against $X$, or paired controls. The result is expressed as a difference, ratio, percent change, or fractional change. |
| Row statistics and fraction of total | Row statistics collapse the replicates in each row into a total, a mean (with SD, SEM, %CV, CI, or min–max), a median (with quartiles or percentiles), or a geometric mean. Fraction of total computes proportions and percentages of the row, column, or grand total, with Wilson or Clopper–Pearson CIs. |
| Transpose and select-and-transform | Swaps rows and columns, takes a subset of rows or columns or reorders them, filters rows by a condition, or transforms columns. |
| Curve calculus and AUC | Computes first or second derivatives, cumulative integrals, smoothing (Savitzky–Golay, moving average, or LOWESS), or the trapezoidal area under the curve for continuous XY curves. |
| Archetype conversion | Converts a table between the Column, Grouped, XY, Multiple Variables, and Contingency archetypes, with a preview that changes nothing. |
| Microplate mapping | Maps 96-well or 384-well grids, sheets, or analyzable results into typed tables that record the source well of each value. |
| Tidy long-to-wide pivot | Reshapes long-format rows into the replicate subcolumns of a typed XY, Column, or Grouped table. |
| Randomization and row sampling | Performs seeded allocation to treatment arms, and draws uniform or stratified samples of $k$ out of $N$ rows. |
| Simulation and Monte Carlo | Generates seeded random columns, noisy XY curves, or $2 \times 2$ contingency tables reproducibly, and chains them into Monte Carlo studies of limited size (`MonteCarloSimulation`). |
| 4-stage linked import pipeline | Processes linked sheets or text in four stages: filter → reshape/transpose → baseline → exclusions. |

Options, edge cases, and safeguards:

- **Math transforms:** The built-in presets are natural log, $\log_{10}$, $\log_2$, $10^x$, $e^x$, reciprocal, square root, square, logit, probit, and $\arcsin\sqrt{p}$. CurveBook also supports linear scaling (`slope`, `intercept`), concentration to $-\log\text{M}$ (`molar_unit_factor`), and any `=` formula (such as `2^-y` for qPCR pipelines). Domain errors (logs or reciprocals of non-positive values, square roots of negative values, or probabilities outside $(0, 1)$) produce missing cells or typed errors rather than `NaN`.
- **Normalize:** `zero_anchor` and `hundred_anchor` each accept one of the anchors listed below. If the two anchors are equal ($y_{100} = y_0$), normalization fails with an error instead of dividing by zero.
  - the column minimum, maximum, or sum;
  - the first or last row;
  - a fixed scalar;
  - the mean of a control column;
  - a linked (hook) value.
- **Remove baseline:** In ratio, percent change, or fractional change modes, a zero baseline divisor either leaves the undefined entries out or returns an error. Excluded cells remain skipped.
- **Transpose and selection:** These operations enforce limits on rows, datasets, and imported cells (10,000,000). Exclusion masks are remapped atomically.
- **Curve calculus and AUC:**
  - Smoothing offers moving average, Savitzky–Golay, and Cleveland (1979) LOWESS (bisquare robustness weights at $6 \times \text{MAD}$, `span` in $(0, 1]$).
  - Differentiation uses 3-point Lagrange quadratic formulas for first and second derivatives on sorted $X$ values, whether uniformly spaced or not. Duplicate $X$ values fail with `InvalidInput`.
  - Integration computes the cumulative trapezoidal area above `baseline`. AUC computes total, net, and baseline-corrected peak areas (Yeh & Kwan 1978).
- **Archetype conversion (`table.convertArchetype`):** The preview reports `preserved_cells`, `dropped_cells`, `preserved_exclusions`, `warnings`, and `lossy`. Exclusion flags and notes are preserved and remapped. A lossy conversion is refused unless `allow_lossy` is `true`. Examples of lossy conversions are negative or fractional values targeting a Contingency table, and collapsed subcolumns.
- **Microplate mapping (`table.importPlateMap`):** The command checks that grids are exactly 8×12 or 16×24, that results sources are up to date, that assignments are not empty, and that cell limits are respected. Reusable layouts are saved as `.cbplate` files (see [Composable microplate mapping](#microplate-mapping)).
- **Tidy long-to-wide pivot (`table.importTidyLong`):** You identify the columns that hold X or the row factor, the group, an optional replicate index, the value, and an optional exclusion flag. The pivot keeps exclusion flags. It rejects column indices that are out of range and any expansion beyond 512 subcolumns.
- **Randomization and sampling:**
  - `table.randomizeGroups` performs seeded complete or permuted-block allocation across $k \ge 2$ arms and writes ARRIVE metadata into an info sheet. Block sizes must be positive multiples of the sum of the integer allocation weights.
  - `table.randomSampleRows` uses partial Fisher–Yates sampling without replacement. It rejects empty strata and requests where $k > N$.
- **Simulation:** `table.simulateDistributions` generates columns from uniform, normal, log-normal, binomial, Poisson, or exponential distributions. `table.simulateXy` generates noisy XY curves with Gaussian, uniform, or multiplicative log-normal noise. For a given 64-bit seed, both commands give identical results on native and WebAssembly builds.
- **Linked import pipelines:** CurveBook checks the sheet links for cycles and rejects circular links before it changes the project.

---

## Multi-block tabular import (`.xlsx` and `.csv` / `.tsv`) {#tabular-import-options}

For both `.xlsx` and delimited text files, CurveBook lets you set exactly which part of the file to import rather than relying on fragile guesses about the layout. You can:

- slice an explicit range;
- expand merged cells exactly as the OOXML file defines them;
- filter or split rows by a discriminator column;
- extract metadata from preamble rows.

### `.xlsx` workbook import options (`table.importXlsx`) {#xlsx-import-options}

All advanced `.xlsx` options are opt-in. CurveBook always applies them in this order:

1. `fill_merged_cells`
2. `cell_range` crop
3. `skip_rows` (plus optional `extract_preamble_info`)
4. `header_rows`
5. `row_match` or `split_by_column`
6. Archetype inference

Option details:

- **`cell_range` (single sheet only):** An optional A1-style rectangle (such as `"B4:M11"`). The import keeps the exact dimensions of the rectangle, including trailing empty cells. This option requires a single selected worksheet. If you pass it with several sheets, the import fails and asks you to select one with `sheets` or `sheet_indices`.
- **`skip_rows`:** The number of leading rows to skip before `header_rows`. When `cell_range` is set, rows are counted from the top of that range.
- **`fill_merged_cells` (default `false`):** Copies the top-left value of each merged region across the exact `<mergeCell>` bounds in the OOXML file. This happens before cropping and header extraction. Empty cells that are not merged stay empty; CurveBook never carries a value to the right into an unmerged blank.
- **`split_by_column`:** A zero-based index of a discriminator column. It cannot be combined with `row_match`.
  - Rows are grouped by the trimmed text in that column, with blocks in the order they first appear. This handles stacked channels and interleaved cycles.
  - The discriminator column is dropped, and blank separator rows are skipped. A non-blank row with an empty discriminator produces a warning.
  - Each block becomes its own data table, with its own archetype inference. Table IDs follow the pattern `{base_id}_{block_slug}`, with `_2`, `_3`, … appended if IDs collide. Titles follow the pattern `"{sheet} — {block}"`.
- **`row_match`:** A filter of the form `{ column, equals, drop_column }`, where `drop_column` defaults to `true`. It cannot be combined with `split_by_column`. Only rows whose trimmed value in the zero-based `column` equals `equals` are kept.
- **`extract_preamble_info` (default `false`):** Takes effect only when it is `true` and `skip_rows > 0`. CurveBook reads `Key: Value` and `Key = Value` constants from the skipped leading rows. Free-text preamble lines go into `notes_markdown` on one shared info sheet, which is linked to every imported table.
- **Importing a bare plate grid into an open project:** `table.importXlsx` does not accept `sheet_id`, `link`, or `filter`. To slice a bare 8×12 or 16×24 layout or concentration grid (such as `cell_range: "B4:M11"`) into an open study, pass `merge: true` so existing sheets stay intact, and `has_header: false` so row A is not used as headers. Add `fill_merged_cells: true` when labels span merged cells.

### `.csv` / `.tsv` import windowing, splitting and preamble extraction (`table.importCsv`) {#csv-import-options}

- **Windowing with `filter`:** `table.importCsv` applies `filter` to new sheets, updated sheets, and live paste-link pipelines. The supported fields are `row_window` (or `row_start`/`row_end`), `col_window` (or `col_start`/`col_end`), `decimate_every_k`, `parse_asterisk_exclusions`, and `row_match`. To skip leading rows, use `filter.row_start` or `filter.row_window`.
- **`row_match`:** Keeps only rows whose trimmed value in `column` matches `equals`. By default, that column is then dropped (`drop_column: true`).
- **`split_by_column`:** Splits stacked or interleaved blocks of delimited text into one data table for each distinct trimmed value of the discriminator column.
  - Table IDs use the same `{base_id}_{block_slug}` scheme as `.xlsx` import, with `_2`, `_3`, … appended if IDs collide.
  - Blank separator rows are skipped, and a non-blank row with an empty discriminator produces a warning.
  - It cannot be combined with `filter.row_match`, and it is rejected when the import targets an existing `sheet_id` or uses `link: true`. In those cases, use `filter.row_match` instead.
- **`extract_preamble_info` (default `false`):** Takes effect only when it is `true` and leading rows are skipped (`row_start > 0`). CurveBook reads `Key: Value` and `Key = Value` constants using `decimal_separator`, including lines padded with delimiters such as `RunId: 42,,,,`. Free-text preamble lines go into `notes_markdown` on one shared info sheet, which is linked to every table the import creates.

### Numeric-X requirement for kinetic imports {#numeric-x-contract}

> [!IMPORTANT]
> CurveBook's `.xlsx` and `.csv`/`.tsv` importers do **not** interpret Excel `numFmt` time serials (fractions of a day) or text timestamps such as `hh:mm:ss`. For kinetic analyses ([Area Under the Curve](registry/tables.html#analysis-kind-area-under-curve)) and kinetic plate reducers (`x_window_mean`), the `X` column must contain **finite numeric values in the units you want to integrate over** (for example, elapsed seconds or minutes).

- **Numeric cells formatted as times in a spreadsheet:** Spreadsheets often store a time as a fraction of a 24-hour day (for example, `0.00069444...` displayed as `00:01:00`). Because `.xlsx` import reads the stored number and ignores the display `numFmt`, integrating those cells gives an AUC in **days**, unless you first convert the values to seconds or minutes.
- **Text timestamps (`hh:mm:ss`):** Strings such as `"00:05:00"` are imported as text cells. Numeric `X` extraction skips them and fails with a message explaining that X must be numeric, rather than integrating arbitrary numbers.

---

## Composable microplate mapping (`table.importPlateMap`) {#microplate-mapping}

`table.importPlateMap` maps readings from an 8×12 (96-well) or 16×24 (384-well) microplate into a typed Column, XY, Grouped, or Multiple Variables table. Every imported cell keeps the ID of the well it came from.

### Reading sources: direct values, analyzable sheets and kinetic reducers {#plate-sources}

Provide either `values` or `source`, but not both. `values` is a direct numeric array, either flat (96 or 384 values) or two-dimensional (8×12 or 16×24).

- **`source`:** Reads measurements from an existing data table or from an analyzable results sheet (such as [Area Under the Curve](registry/tables.html#analysis-kind-area-under-curve)).
  - **Tables with one row per well (`value_column`):** Use this for tables such as AUC analyzable output. The numeric column header is matched after trimming and ignoring case (for example, `"Net Area"`, `"Positive Area"`, `"Negative Area"`, or `"Total Area"`).
  - **Kinetic or column tables with one column per well (`reducer`):** Use this for XY or Column tables in which each column is a well. All four reducers skip excluded cells:
    - `{"row_index": n}`: the active numeric value at zero-based row `n`.
    - `{"x_window_mean": {"x_min": a, "x_max": b}}`: the arithmetic mean of the active values whose finite numeric `X` lies in the inclusive range `[a, b]`. This requires an XY table with numeric `X`.
    - `"min"` or `"max"`: the minimum or maximum active numeric value in the well's column.
- **`match_wells`:**
  - `by_well_id` *(default)*: matches standard or unpadded well IDs (`"A01"`, `"A1"`), or a well ID at the start of a label followed by a delimiter (such as `"A01 - Sample X1"`). Duplicate matches or no matches at all cause an error. Missing wells stay empty and produce warnings that do not stop the import.
  - `positional`: maps source rows or columns in order, following row-major plate order (`A01..H12` or `A01..P24`).
- **Static snapshot and freshness check:** `table.importPlateMap` creates an independent, static snapshot as a data table, so later edits to upstream kinetic sheets do not overwrite it. When `source.sheet_id` names a results sheet, CurveBook refuses to read it (`Conflict`) unless that results sheet is up to date. A floating note records the upstream `source_sheet_id` and the `value_column` or reducer that was used.

### Plate layouts: grids, layout sheets, `x_grid` and controls {#plate-layouts}

Supply well assignments with either `template` / `roles` or `label_grid` / `layout_sheet_id`, but not both:

- **`label_grid` / `layout_sheet_id`:** Must be exactly 8×12 (96-well) or 16×24 (384-well). When CurveBook reads `layout_sheet_id` or `x_grid_sheet_id` from a data table, it uses only the data cells and leaves out the column and row titles. A grid with the wrong dimensions, such as 9×13 or 17×25, fails with a hint to exclude headers. Similarly, a 7×12 or 15×24 grid prompts you to re-import with `has_header: false`.
- **Literal labels and replicate indexing:** Labels are taken literally; concentrations written inside a label are not parsed. Empty cells become excluded. `blank_labels` and `positive_control_labels` are matched after trimming and ignoring case. Grid layouts assign 0-based replicate indices (`0`, `1`, …).
- **Optional `x_grid` / `x_grid_sheet_id`:** A matching 8×12 or 16×24 numeric grid that supplies a `dilution_x` value for each well (for example, a shifted concentration ladder).
  - When it is omitted, treatment wells default to `dilution_x = 0.0`. That default is standard for Column screening tables. For XY and Grouped tables, it produces a warning that all wells share a single X; the warning does not stop the import.
  - When `x_grid` or `x_grid_sheet_id` is supplied, any treatment well with an empty or non-finite X value causes a validation error that names the well and its label.
- **Controls and reference arms:**
  - Setting `subtract_blank_mean: true` subtracts the mean of the active blank wells from the readings.
  - On Column targets, wells that match `positive_control_labels` are pooled into one `"Positive Control"` column.
  - When `subtract_blank_mean` is `false`, blank wells appear as a `"Blank"` column.
  - To keep separate named reference arms for multiple comparisons (such as vehicle and agonist controls for Dunnett's test), assign them as regular treatment groups. Then apply the non-destructive **Remove Baseline** or **Normalize** (control column mean) transform.
- **Versioned `.cbplate` files:** Reusable layouts are saved and loaded as single-extension `.cbplate` files in UTF-8 JSON (`format_version = 1`). Writes are atomic, and CurveBook refuses to overwrite a file saved in a newer format version (see [`plate-map.schema.json`](../schemas/plate-map.schema.json) and [Persisted File Formats](../file-formats.md)).

### Well origins and `well_cell_map` {#plate-well-origins}

For every cell in a Column, XY, Grouped, or Multiple Variables output, `table.importPlateMap` records which well it came from, mapping `(row, col, subcol)` to a well ID. The command also returns the ordered `well_cell_map` in its response. These well origins behave as follows:

- **Preserved:** Well origins stay intact through `table.excludeCells` and `table.excludeCellsWithReason`, so you can still see each well ID during outlier and QC audits. They are saved inside `.curvebook` archives (schema v8 and later).
- **Shifted and remapped:** Well origins shift automatically when rows are inserted. They are remapped one-to-one by operations that keep cell coordinates: `table.convertArchetype`, transpose, select-and-transform subsets of rows or columns, random sampling, keeping every Nth row, math transforms, normalization, and baseline removal.
- **Cleared and dropped:** A cell's well origin is cleared whenever you overwrite, clear, or delete the cell by hand, and Undo restores it. Well origins are dropped from outputs that aggregate or derive values by formula (pruning by bin averages, calculated columns), and removed when a project is saved as a `.cbtemplate` template.

See [Concepts and document structure](concepts.md), the [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and the [Command Reference](../commands.md) for parameter schemas.
