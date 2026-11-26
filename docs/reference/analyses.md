# Statistical analyses and transforms reference

This reference documents every statistical test, data transform, and regression analysis in CurveBook, paired with the [Analysis Kinds Registry](registry/tables.html#analysis-kinds) and [Table Archetypes](tables.md#table-archetypes). Each entry covers compatible tables, commands and defaults, reported outputs, diagnostic checks, edge cases, and verification references.

> [!NOTE]
> CurveBook's diagnostic checklists report assumption metrics, sample-size limits, convergence statuses, and missing-data notes objectively. They never rank tests, recommend replacement procedures, or transform your data automatically (see [Quality-of-life capabilities](qol.md#active-qol)).

- **Global controls:** Every analysis stores `confidence_level` (`confidence_percent` in $(1.0, 99.99]$, default `95.0`; saved specs accept $[0.01, 99.99]$), `p_value_format`, and `selected_data_sets` (empty selects all compatible columns unless explicit indices are required). Excluded cells are always omitted before computation ([Cell exclusions](concepts.md#cell-exclusions)).
- **Running analyses:** Execute [`analysis.run`](../commands.md) (or `transform.run`, `stats.run`, `fit.run`), [`analysis.updateParams`](../commands.md), or [`analysis.retargetData`](../commands.md). Run [`analyze.run`](../commands.md) to recalculate pending sheets across the project. Unknown option keys return `bad_input` ([Engine error codes](registry/tables.html#error-codes)).
- **Multiplicity names (`multiplicity`):** Canonical values are `none` (default unadjusted / omnibus only), `tukey_hsd`, `dunnett` (with `control_column`), `sidak`, `bonferroni`, `holm_sidak`, `dunnett_t3`, `games_howell`, `tamhane_t2`, `dunn_rank_sum`, `linear_trend`, `benjamini_hochberg_fdr`, `benjamini_yekutieli_fdr`, `two_stage_bky_fdr`, `newman_keuls`, and `fisher_lsd`. FDR methods interpret `threshold` as false discovery rate $Q$; FWER methods interpret `threshold` as family-wise $\alpha$ (default `0.05`; see [Number formatting](concepts.md#number-and-pvalue-formatting)).

---

## Data transforms, reshaping, and simulation {#analysis-transform}

Applies non-destructive element-wise math transforms, normalizations, baseline removals, transpositions, row pruning, formula filters, synthetic XY simulation, or row sampling, producing a linked derived data table and summary sheet. Compatible with all [table archetypes](tables.md#table-archetypes) (see [Analysis Registry: `transform`](registry/tables.html#analysis-kind-transform)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `transform.run` with `kind` set to `math` (canonical), `transform`, `normalize`, `remove_baseline`, `baseline`, `transpose`, `prune`, `prune_rows`, `select`, `select_and_transform`, `simulate`, or `random_sample`.
- **Options & defaults:**
  - `math`: `x_transform`, `y_transform` (`none` or preset: `log10`, `ln`, `log2`, `exp10`, `exp`, `reciprocal`, `square`, `sqrt`, `logit`, `probit`, `arcsin_sqrt`, etc.), and `error_propagation` (`replicate_endpoints` default, or `first_order_delta` / `delta_method`).
  - `normalize`: `zero_anchor`, `hundred_anchor` (`column_min`, `column_max`, `column_sum`, `first_row`, `last_row`, fixed constant, control column mean, or hooked scalar), `output_percent` (boolean), and `per_data_set` (boolean).
  - `remove_baseline`: `baseline_columns` and `operation` (`difference`/`subtract`, `ratio`/`divide`, `percent_change`, `fractional_change`).
  - `transpose`: `swap_row_and_column_titles` (boolean).
  - `prune_rows`: `every_k_rows` ($\ge 1$) and `average_window` (boolean).
  - `select_and_transform`: `filter_formula`, `target_archetype`, and `derived_columns`.
  - `simulate`: `seed`, `row_count`, `replicates`, `x_min`, `x_max`, `equation_id`, `parameters`, and `noise`.
  - `random_sample`: `seed`, `sample_size`, `stratify_column`, and `proportional_allocation`.
- **Reported outputs:**
  - Emits a linked derived data table and a **Summary** tab (`summary`) recording transform metadata, anchor scalars (`ZeroAnchor`, `HundredAnchor`), or sampling fields (`sample_size`, `population_size`, `seed`).
  - Utility curve transforms (AUC, Savitzky–Golay / LOWESS smoothing, numerical derivatives, cumulative integrals) are documented under [Transforms and reshaping](tables.md#transforms-and-reshaping).
- **Diagnostic checks:** Emits non-fatal warnings and checklist notes when a domain guard blanks an endpoint or derivative. Common examples include `ln(u)` at $u \le 0$ or a zero transformed mean on Mean/%CV/N summaries.
- **Edge cases & limits:**
  - On [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N, Mean/Low/High, Median/IQR, Upper/Lower Limits), `first_order_delta` scales spread by $|g'(\text{mean})|$ and swaps asymmetric `-Error`/`+Error` bounds when $g' < 0$.
  - `replicate_endpoints` maps $[\text{mean} - \text{SD}, \text{mean} + \text{SD}]$ through $g$ and halves the width while preserving exclusion notes.
  - Equal anchors (`zero_anchor == hundred_anchor`) fail with an invalid-input error, and `random_sample` enforces stratified quotas deterministically from the recorded seed.
- **References & verification:**
  - **References:** JCGM 100:2008 / GUM §5.1.2 (delta-method propagation); Zar (2010) (logit, probit, angular $\arcsin\sqrt{u}$); Cleveland (1979) & Savitzky–Golay (1964) / Gorry (1990).
  - **Verified against:** `mpmath 1.3.0` (80- and 120-digit rational and transcendental benchmarks) and `R 4.5.2` ([statistics evidence](../evidence/statistics.md)).

---

## Column descriptive statistics, normality, and one-sample tests {#analysis-descriptive-stats}

Computes column sample size, location, dispersion, quantiles, geometric summaries, confidence intervals, skewness, excess kurtosis, Gaussian and lognormal normality tests, Normal Q–Q coordinates, and optional one-sample tests against a hypothetical value. Compatible with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `descriptive-stats`](registry/tables.html#analysis-kind-descriptive-stats)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `descriptive_stats` (canonical), `descriptive`, or `column_stats`.
- **Options & defaults:**
  - `aggregation`: `row_means` (default, collapses replicate subcolumns by row mean first) or `pooled_grand_sample` (pools all active replicates).
  - `custom_upper_percentile`: optional $p_{\text{hi}} \in [50.0, 100.0)$ reporting $(100 - p_{\text{hi}}, p_{\text{hi}})$ Hyndman–Fan Type-7 quantiles (default unset).
  - `run_normality_battery`: boolean (default `true`).
  - `hypothetical_value`: optional finite $\mu_0$ triggering one-sample Student *t* and Wilcoxon signed-rank tests (default unset).
- **Reported outputs:**
  - **Descriptive Summary** (`desc-summary` or `desc-pooled`): $N$, Minimum, 25% Percentile, Median, 75% Percentile, Maximum, Range, Mean, Std. Deviation, Std. Error of Mean, `conf%` CI of Mean, `conf%` CI of SD ($\chi^2$ normal-theory interval), exact/interpolated binomial CI of Median, %CV, Geometric Mean and Geometric SD Factor (when all values $> 0$), Harmonic Mean (when valid), Skewness ($G_1$), Excess Kurtosis ($G_2$), and lag-1 autocorrelation ($r_1$).
  - **Diagnostics** (`normality`): Shapiro–Wilk $W$ ($3 \le n \le 5000$), D'Agostino–Pearson $K^2$ ($n \ge 8$), Anderson–Darling $A^{2*}$ ($n \ge 5$), and Lilliefors Kolmogorov–Smirnov $D$ ($n \ge 5$) on both raw data and $\ln(Y)$ when strictly positive.
  - **Normal Q–Q** (`normal-qq`): Blom (1958) plotting positions $p_i = (i - 3/8)/(n + 1/4)$ and expected normal or lognormal $z$ quantiles.
  - **Summary** (`one-sample`): When `hypothetical_value` is set, one-sample *t*-test ($t, \text{df}, P$) and Wilcoxon signed-rank test ($W, P$, Pratt 1959 zero handling and Hodges–Lehmann pseudomedian/CI).
- **Diagnostic checks:** Reports Shapiro–Wilk normality status per column (pass if $P > \alpha$, warn if $P \le \alpha$) and logs any ignored non-numeric text cells.
- **Edge cases & limits:**
  - On [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N), reconstructs exact pooled Mean, SD, SEM, $N$, and CI of Mean; nonparametric quantiles, medians, Q–Q plots, and normality/Wilcoxon tests require raw values and are omitted.
  - Zero-variance columns omit *t*-statistics cleanly while reporting constant summaries; columns with no finite numeric values return an empty-input error.
- **References & verification:**
  - **References:** Hyndman & Fan (1996) Type-7 quantiles; Royston (1995) AS R94; D'Agostino, Belanger & D'Agostino (1990); Anscombe & Glynn (1983); Stephens (1986); Lilliefors (1967) & Dallal & Wilkinson (1986); Pratt (1959) & Hollander, Wolfe & Chicken (2014); Kirkwood (1979).
  - **Verified against:** NIST StRD Univariate benchmarks, `SciPy 1.18.1` (`normaltest`, `anderson`), and `statsmodels 0.15.0` (`normal_ad`, `lilliefors`) ([statistics evidence](../evidence/statistics.md)).

---

## Outlier detection (Grubbs and ROUT) {#analysis-outlier-detection}

Screens each column independently for single or multiple univariate outliers using normal-theory Grubbs Extreme Studentized Deviate (ESD) or robust ROUT false-discovery-rate screening. Compatible with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `outlier-detection`](registry/tables.html#analysis-kind-outlier-detection)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `outliers` (canonical), `outlier`, `rout`, or `grubbs`.
- **Options & defaults:**
  - **ROUT** (`method: "rout"`, default): `fdr_q_percent` in $(0.0, 100.0]$ (default `1.0` for $Q = 1\%$). Univariate column ROUT allows $Q \in (0, 100]$, whereas curve-fitting ROUT restricts $Q \in (0, 50]$.
  - **Grubbs** (`method: "grubbs"`): `alpha` in $(0.0, 1.0)$ (default `0.05`) and `iterative` boolean (default `false` for single outlier; `true` for sequential ESD).
- **Reported outputs:**
  - **Summary** (`outliers` — "Identified Outliers"): Column, 1-based Row Index, observed Value, test Statistic ($G$ or $t_i = |y_i - M| / \text{RSDR}$), and Threshold / P (Grubbs $G_{\text{crit}}$ or BH $q$-value).
  - **Scalars:** `OutlierCount` per column, plus `RobustCenter` and `RSDR` under ROUT.
- **Diagnostic checks:** Notes Grubbs normality assumptions, ROUT median and 68.27th-percentile RSDR screening over the furthest 30% of observations, and detected outlier counts (experimental provenance governs whether flagged cells should be excluded).
- **Edge cases & limits:**
  - Both methods require $n \ge 3$ active values. Grubbs returns zero outliers when $\text{SD} = 0$.
  - ROUT resists masking across multiple extreme outliers and protects against division by zero on zero-spread clean cores while flagging points off the constant mass. Non-numeric cells are ignored with a checklist note.
- **References & verification:**
  - **References:** Grubbs (1969); Rosner (1983) generalized ESD; Motulsky & Brown (2006) ROUT; Benjamini & Hochberg (1995).
  - **Verified against:** NIST/SEMATECH e-Handbook Grubbs benchmark and Motulsky & Brown (2006) ROUT reference cases ([statistics evidence](../evidence/statistics.md)).

---

## Two-group and one-sample comparisons {#analysis-two-group-comparison}

Compares two independent groups, matched pairs, or a single group against a hypothetical location using parametric, lognormal/ratio, or nonparametric rank and distribution tests. Compatible with Column and Multiple Variables tables (`mv_group_column` + `mv_response_column` split a two-level factor in long tables; see [Analysis Registry: `two-group-comparison`](registry/tables.html#analysis-kind-two-group-comparison)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `unpaired_t` (canonical), `t_test`, `unpaired_t_test`, `welch_t`, `paired_t`, `ratio_paired_t`, `lognormal_unpaired_t`, `mann_whitney`, `wilcoxon`, `wilcoxon_matched_pairs`, `kolmogorov_smirnov`, `ks_test`, `paired_sign_test`, `sign_test`, `one_sample_t`, or `one_sample_wilcoxon`.
- **Options & defaults:**
  - `method`: `unpaired_t` (default), `welch_t`, `paired_t`, `ratio_paired_t`, `lognormal_unpaired_t`, `mann_whitney`, `wilcoxon_signed_rank`, `kolmogorov_smirnov`, `paired_sign_test`, `one_sample_t`, or `one_sample_wilcoxon`.
  - `two_tailed`: boolean (default `true`). One-tailed rank tests evaluate the tail strictly in the observed sample direction.
  - `compute_effect_sizes`: boolean (default `true`). Reports Cohen's $d$, Hedges' $g$, Glass's $\Delta$ with noncentral-*t* CIs, $R^2$ ($\eta^2$), and rank-biserial correlation / Cliff's $\delta$ (`mann_whitney`, `wilcoxon_signed_rank`).
  - `mv_group_column`, `mv_response_column`, and `hypothetical` (for one-sample tests).
- **Reported outputs:**
  - **Summary** (`two-group`, `paired-t`, `mann-whitney`, etc.): group means/medians, difference or ratio of means or Hodges–Lehmann shift with CI, test statistic ($t, U, W, D_{\text{KS}}, S$), $\text{df}$, and *P* value.
  - **Additional diagnostics:** Variance-equality *F*-test and median-centered Levene / Brown–Forsythe test (unpaired tests), pairing correlation $r$ and *P* (`paired_t`, `ratio_paired_t`), and requested effect sizes.
  - For `ratio_paired_t` and `lognormal_unpaired_t`, geometric mean ratios $\exp(\Delta \bar{y}_{\ln})$ and CIs are back-transformed automatically.
- **Diagnostic checks:** Evaluates group Shapiro–Wilk normality, *F*-test / Levene homoscedasticity, pairing effectiveness (warns if $r \le 0$ or $|r| = 1$), exact Hodges–Lehmann CI achieved coverage for small $n$, and non-positive values in ratio/lognormal tests.
- **Edge cases & limits:**
  - [Pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N) work for `unpaired_t`, `welch_t`, and `one_sample_t` (retaining the exact *F*-test while omitting raw-only Levene checks).
  - Exact permutation and Wilcoxon distributions run for moderate sample sizes, switching to tie-corrected asymptotics and $O((m+n)\log(mn))$ Hodges–Lehmann selection when $m \cdot n > 40000$. Incomplete paired rows are skipped listwise with an audit note.
- **References & verification:**
  - **References:** Student (1908); Welch (1947); Satterthwaite (1946); Mann & Whitney (1947); Wilcoxon (1945); Pratt (1959); Hodges & Lehmann (1963); Smirnov (1948) & Hodges (1958); Dixon & Mood (1946); Cohen (1988), Hedges (1981) & Steiger & Fouladi (1997); Cliff (1993) & Wendt (1972) / Kerby (2014); Levene (1960) & Brown & Forsythe (1974a).
  - **Verified against:** `R 4.5.2` (`stats::ks.test`, `pt`, `pf`), `SciPy 1.18.1` (`scipy.stats.nct`), and published exact rank benchmarks ([statistics evidence](../evidence/statistics.md)).

---

## Bland–Altman method comparison {#analysis-bland-altman}

Evaluates agreement between two quantitative assays on matched subjects. Plots differences ($A - B$ or percentage difference $100 \cdot (A - B) / ((A + B)/2)$) against pair averages $(A + B) / 2$, reporting mean bias and Limits of Agreement ($\text{LoA} = \text{bias} \pm z_{1-\alpha/2} \cdot \text{SD}_{\text{diff}}$). Compatible with Column, XY, and Multiple Variables tables (see [Analysis Registry: `bland-altman`](registry/tables.html#analysis-kind-bland-altman)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `bland_altman` (canonical) or `stats.bland_altman`.
- **Options & defaults:**
  - `column_a`: 0-based index of method *A* (default `0`).
  - `column_b`: 0-based index of method *B* (default `1`).
  - `percent_difference`: boolean (default `false` for `A - B`; `true` for percentage difference).
  - `xy_compare_x_vs_y`: optional boolean on XY tables (`true` forces $X$ vs $Y_0$; `false` uses `Y[column_a]` vs `Y[column_b]`; unset defaults to $X$ vs $Y_0$ when one $Y$ dataset exists and $Y$ datasets otherwise).
- **Reported outputs:**
  - **Summary** (`bland-altman`): paired $n$, Mean Bias with SE and `conf%` *t*-CI, and SD of differences. Also reports Lower and Upper Limits of Agreement with Bland–Altman (1999) standard errors $\text{SD}\sqrt{1/n + z^2 / (2(n - 1))}$ and *t*-CIs, plus proportional-bias OLS slope, intercept, and *P* value.
  - **Residuals:** per-pair (Average, Difference) scatter coordinates.
- **Diagnostic checks:** Tests normality of paired differences (Shapiro–Wilk for $3 \le n \le 5000$), checks proportional bias via the difference-vs-mean slope *P* value, and logs complete-case pair filtering.
- **Edge cases & limits:** Fails under `percent_difference: true` if any pair has average $(A + B)/2 = 0$; skips incomplete rows cleanly; rejects pre-summarized tables because matched subject pairs are required.
- **References & verification:**
  - **References:** Bland & Altman (1986, 1999).
  - **Verified against:** `R 4.5.2` and Bland & Altman (1986) Table 1 ([statistics evidence](../evidence/statistics.md)).

---

## Multiple row-wise *t*-tests and nonparametric tests {#analysis-multiple-row-t-tests}

Runs independent or paired two-group comparisons separately on every row of a table (for screening, dose/time tables, or volcano plots) with FWER or FDR multiplicity control across testable rows. Compatible with Grouped, XY, Column (paired replicate column blocks), and Multiple Variables tables (see [Analysis Registry: `multiple-row-t-tests`](registry/tables.html#analysis-kind-multiple-row-t-tests)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `multiple_row_t_tests` (canonical), `stats.multiple_row_t_tests`, `multiple_row_tests`, or `multiple_t_tests`.
- **Options & defaults:**
  - `paired`: boolean (default `false`). Requires matched subcolumns on Grouped/XY tables or an even number ($\ge 4$) of selected columns on Column tables.
  - `nonparametric`: boolean (default `false`; runs Mann–Whitney $U$ or Wilcoxon signed-rank per row when `true`).
  - `assume_equal_sd`: boolean (default `true` for Student pooled variance; `false` for Welch–Satterthwaite when unpaired and parametric).
  - `multiplicity` (`benjamini_hochberg_fdr`, `two_stage_bky_fdr`, `benjamini_yekutieli_fdr`, `holm_sidak`, `bonferroni`, `sidak`, or `none`), `threshold` (default `0.05`), `report_confidence_intervals`, and `control_column`.
- **Reported outputs:**
  - **Summary** and **Multiple Comparisons** (`multiple-row-comparisons`): per-row $n_A, n_B$, Mean/Median difference, $\text{SE}$, test statistic, raw *P*, adjusted *P* or FDR *q*, $-\log_{10}(P)$ volcano coordinates, `Discovery?` flag, and optional CI of difference.
- **Diagnostic checks:** Reports evaluated vs. skipped row counts (insufficient replicates or degenerate zero variance) and summarizes the selected multiplicity family (`FWER` vs. `FDR`).
- **Edge cases & limits:** Strictly rejects [pre-summarized subcolumns](tables.md#subcolumn-modes) (`insufficient_data`) because raw replicates per row are required. Split-group Column inputs require at least 4 selected columns (even count when `paired: true`).
- **References & verification:**
  - **References:** Benjamini & Hochberg (1995); Benjamini, Krieger & Yekutieli (2006); Holm (1979); Šídák (1967).
  - **Verified against:** `R 4.5.2` and `statsmodels 0.15.0` (`multipletests(method="fdr_tsbky")`) ([statistics evidence](../evidence/statistics.md)).

---

## Passing–Bablok regression {#analysis-passing-bablok}

Fits Passing & Bablok (1983) nonparametric errors-in-variables linear regression $Y = \alpha + \beta X$, resilient to outliers and error in both *X* and *Y*. Compatible with Column, XY, and Multiple Variables tables (see [Analysis Registry: `passing-bablok`](registry/tables.html#analysis-kind-passing-bablok)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `passing_bablok` (canonical) or `stats.passing_bablok`.
- **Options & defaults:**
  - `x_column`: 0-based index of reference assay *X* (default `0`).
  - `y_column`: 0-based index of candidate assay *Y* (default `1`).
- **Reported outputs:**
  - **Summary** and **Parameters and CIs**: slope $\hat{\beta}$ (shifted median of valid slopes $S_{ij} = (y_i - y_j)/(x_i - x_j) \ne -1$ with offset $K = \text{count}(S_{ij} < -1)$), exact rank CIs, and intercept $\hat{\alpha} = \text{median}(y_i - \hat{\beta}x_i)$ with CIs.
  - **Scalars:** `Slope`, `Slope CI lower/upper`, `Intercept`, `Intercept CI lower/upper`, `CUSUM H`, `CUSUM statistic`, `CUSUM P value`, `N`, `Valid slopes`, and `K offset`.
  - **Residuals:** fitted values and perpendicular/vertical residuals.
- **Diagnostic checks:** Runs Passing & Bablok's Kolmogorov-type CUSUM linearity test ($H$, statistic, *P* value) and notes whether the 95% slope CI excludes 1 (proportional bias) or intercept CI excludes 0 (systematic bias).
- **Edge cases & limits:** Requires $n \ge 3$ complete pairs. Drops exact $(x_i, y_i) = (x_j, y_j)$ ties and $S_{ij} = -1$ symmetrically; fails if zero valid slopes or vertical-only ties remain; rejects pre-summarized XY subcolumns.
- **References & verification:**
  - **References:** Passing & Bablok (1983).
  - **Verified against:** `Python 3.13` and `R 4.5.2` exact rank-order and CUSUM reference values ([statistics evidence](../evidence/statistics.md)).

---

## One-way ANOVA and nonparametric equivalents {#analysis-one-way-anova}

Compares $k \ge 2$ groups across independent columns (ordinary ANOVA, Brown–Forsythe and Welch ANOVA, Kruskal–Wallis) or matched subject rows (repeated-measures ANOVA, REML mixed model, Friedman). Includes post-hoc tests, trend tests, and Compact Letter Display (CLD). Compatible with Column and Multiple Variables tables (see [Analysis Registry: `one-way-anova`](registry/tables.html#analysis-kind-one-way-anova)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `one_way_anova` (canonical), `anova`, `welch_anova`, `brown_forsythe`, `rm_one_way_anova`, `repeated_measures_anova`, `kruskal_wallis`, or `friedman`.
- **Options & defaults:**
  - `method`: `ordinary_anova` (default), `brown_forsythe_and_welch`, `repeated_measures`, `kruskal_wallis`, or `friedman`.
  - `geisser_greenhouse`: boolean (applies to `repeated_measures`).
  - `mixed_effects_reml`: boolean (default `false`; engages REML automatically when RM subjects have missing cells, or explicitly on complete tables).
  - `df_method`: `kenward_roger` (default) or `satterthwaite`.
  - `residual_covariance`: `compound_symmetry` (default) or `unstructured` (Pinheiro–Bates log-Cholesky REML with automatic fallback to compound symmetry when under-identified).
  - `compute_effect_sizes`: boolean reporting $\eta^2, \omega^2$, Cohen's $f$ with noncentral-*F* CIs, or $\varepsilon^2$ / Kendall's $W$.
  - `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, and `control_column`.
- **Reported outputs:**
  - **ANOVA Table** (`anova-table`, `unequal-var-anova`, `rm-one-way-anova`, `kruskal-wallis`, `friedman`): Treatment/Residual/Total SS, df, MS, statistic ($F$, Welch $W$, Brown–Forsythe $F^*$, Kruskal–Wallis $H$, or Friedman $\chi^2_F$), and *P* value.
  - **Diagnostics:** Bartlett and Brown–Forsythe median-Levene variance tests (unpaired ANOVA), or Mauchly $W$, Greenhouse–Geisser $\hat{\varepsilon}$, and Huynh–Feldt $\tilde{\varepsilon}$ sphericity (RM ANOVA).
  - **Parameters and CIs** & **Covariance Matrix**: REML variance components or estimated unstructured covariance matrix.
  - **Multiple Comparisons:** pairwise mean or rank differences, SEs, raw and adjusted *P* or *q*, simultaneous CIs (omitted for `newman_keuls` where CIs are undefined), and Piepho (2004) CLD letter groupings.
- **Diagnostic checks:** Checks per-group Shapiro–Wilk normality, variance homogeneity, sphericity (warns when $\hat{\varepsilon} < 0.75$), REML zero-boundary or upper-bound variance estimates, and listwise row drops in non-REML RM/Friedman.
- **Edge cases & limits:**
  - Supports [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N) for `ordinary_anova` and `brown_forsythe_and_welch`, omitting raw-only normality and Levene checks.
  - REML handles unbalanced missing cells automatically, falling back to `compound_symmetry` with a logged reason on collinear or degenerate timepoints. Nonparametric Friedman drops incomplete rows listwise.
- **References & verification:**
  - **References:** Zar (2010); Welch (1951); Levene (1960) & Brown & Forsythe (1974a,b); Steiger (2004); Mauchly (1940); Greenhouse & Geisser (1959); Huynh & Feldt (1976); Patterson & Thompson (1971); Pinheiro & Bates (1996); Kenward & Roger (1997); Fai & Cornelius (1996); Kruskal & Wallis (1952); Friedman (1937); Tukey (1949/1953) & Kramer (1956); Newman (1939) & Keuls (1952); Dunnett (1955, 1980); Games & Howell (1976); Tamhane (1979); Dunn (1964); Piepho (2004).
  - **Verified against:** NIST StRD ANOVA benchmarks and `R 4.5.2` (`stats`, `nlme`, `oneway.test`, `kruskal.test`, `friedman.test`) ([statistics evidence](../evidence/statistics.md)).

---

## Two-way, three-way, and multifactor ANOVA / REML {#analysis-factorial-anova}

Fits fixed-effects or repeated-measures factorial linear models and REML mixed models across two-way, three-way $2 \times 2 \times K$ (Grouped), and $N$-way multifactor (Multiple Variables) designs. Reports Type I or Type III sums of squares, interactions, and simple-effect or main-effect post-hoc tests. Compatible with Grouped and Multiple Variables tables (see [Analysis Registry: `factorial-anova`](registry/tables.html#analysis-kind-factorial-anova)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `two_way_anova` (canonical), `factorial_anova`, or `three_way_anova`.
- **Options & defaults:**
  - `three_way`: boolean (default `false`; requires 4 grouped columns encoding $2 \times 2$ factors by $K$ rows).
  - `multifactor_columns` and `response_column`: factor and outcome column indices on Multiple Variables tables.
  - `max_interaction_order`: integer interaction depth cap (e.g., `2` or `3`).
  - `ss_type`: `type_iii` (default marginal SS for unbalanced designs) or `type_i` (sequential).
  - `repeated_measures`: `none` (default), `column_factor_matched`, `row_factor_matched`, or `both_factors_matched`.
  - `mixed_effects_reml` (boolean), `df_method` (`kenward_roger` default or `satterthwaite`), `residual_covariance` (`compound_symmetry` default or `unstructured`), and `fit_reml_on_missing` (boolean, default `true` in REML paths).
  - `compute_effect_sizes`, `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, and `control_column`.
- **Reported outputs:**
  - **ANOVA Table** (`factorial-anova`): main effects, interactions, subject/residual terms, SS, df, MS, $F$ (or Kenward–Roger $F^*$), *P*, and partial $\eta^2$ / $\omega^2$.
  - **Diagnostics** (`rm-sphericity`): Greenhouse–Geisser and Huynh–Feldt $\varepsilon$ on repeated-measures factors.
  - **Parameters and CIs** & **Covariance Matrix**: REML variance components and unstructured covariance estimates.
  - **Multiple Comparisons:** simple-effect and main-effect pairwise comparisons (requiring a valid `control_column` for `dunnett`).
- **Diagnostic checks:** Reports cell/residual normality and variance checks, sphericity $\varepsilon$ thresholds, REML boundary or convergence warnings, dropped empty levels, and missing-cell notes.
- **Edge cases & limits:**
  - Strictly rejects pre-summarized subcolumns on Grouped tables because raw observations are required.
  - Compacts empty or fully excluded levels cleanly before Type III coding, falls back from `unstructured` to `compound_symmetry` with a logged reason when covariance is under-observed or non-positive-definite, and errors on missing RM cells when `fit_reml_on_missing: false`.
- **References & verification:**
  - **References:** Searle (1987); Maxwell & Delaney (2004); Zar (2010); Lindstrom & Bates (1988); Pinheiro & Bates (1996); Kenward & Roger (1997); Fai & Cornelius (1996).
  - **Verified against:** `R 4.5.2` (`aov`, `lm`, `nlme::lme`, `nlme::gls`) ([statistics evidence](../evidence/statistics.md)).

---

## Nested *t*-test and nested one-way ANOVA {#analysis-nested-comparison}

Fits a hierarchical mixed-effects model separating fixed treatment effects from random biological-unit (subcluster) variance and within-subcluster technical residual variance to prevent pseudoreplication. Runs a two-group nested *t*-test ($t = \sqrt{F}$ with sign) or $k \ge 3$ nested one-way ANOVA with post-hoc tests. Compatible exclusively with Nested tables (see [Analysis Registry: `nested-comparison`](registry/tables.html#analysis-kind-nested-comparison)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `nested_anova` (canonical), `nested_t`, or `nested`.
- **Options & defaults:**
  - `two_group_t_test`: boolean (default `false`; requires 2 selected treatment groups when `true`).
  - `df_method`: `kenward_roger` (default) or `satterthwaite`.
  - `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, and `control_column` (for $k \ge 3$).
- **Reported outputs:**
  - **ANOVA Table** (`nested-anova`): among-groups, among-subclusters within groups, and technical residual SS, df (KR or Satterthwaite), MS, $F$ (or signed $t$ and CI in `nested_t`), and *P* values.
  - **Parameters and CIs** (`variance-components`): REML variance components ($\hat{\sigma}^2_{\text{subcluster}}, \hat{\sigma}^2_{\text{error}}$), SD, percentage of total variance, $\text{ICC}_{\text{subclusters}}$, and both KR and Satterthwaite denominator df.
  - **Multiple Comparisons:** hierarchical group comparisons using $\hat{\sigma}_u^2 (n_{g1}^2 + \dots + n_{gm}^2) / N_g^2 + \hat{\sigma}_e^2 / N_g$.
- **Diagnostic checks:** Reports biological-vs-technical clustering ICC (passes when $\text{ICC} > 0.05$) and REML variance boundary status (zero boundary vs. search limit).
- **Edge cases & limits:** Models unequal technical replicates per subcluster and unequal subclusters per group directly via REML rather than unweighted pooling; non-numeric and excluded replicates are ignored cleanly.
- **References & verification:**
  - **References:** Zar (2010); Satterthwaite (1946); Kenward & Roger (1997).
  - **Verified against:** `R 4.5.2` (REML, Kenward–Roger, and hierarchical group-mean variance benchmarks) ([statistics evidence](../evidence/statistics.md)).

---

## Contingency, proportions, odds/risk ratios, and agreement {#analysis-contingency}

Evaluates categorical count tables for independence, exact $2 \times 2$ association, trend across ordered proportions, paired discordance, inter-rater agreement, and proportion goodness-of-fit. Compatibility depends on method: Contingency tables support all methods; Parts of Whole tables support `pearson_chi_square` and `exact_binomial`; Column tables support `exact_binomial` only (see [Analysis Registry: `contingency`](registry/tables.html#analysis-kind-contingency)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `contingency` (canonical), `chi_square`, `fisher_exact`, `barnard_exact`, `stats.barnard_exact`, `cohen_kappa`, `kappa`, `fleiss_kappa`, `exact_binomial`, or `binomial`.
- **Options & defaults:**
  - `method`: selects the test:
    - `pearson_chi_square` (with `yates_continuity` boolean for $2 \times 2$) or `likelihood_ratio_g2`.
    - `fisher_exact` ($2 \times 2$ exact or $R \times C$ Freeman–Halton network / deterministic Monte Carlo).
    - `barnard_exact` ($2 \times 2$ unconditional CSM/Boschloo supremum over $\pi \in (0, 1)$).
    - `cochran_armitage_trend` ($2 \times C$ or $R \times 2$) or `mcnemar_paired` ($2 \times 2$).
    - `cohen_kappa` (square $k \times k$, `weighting`: `unweighted`, `linear`, `quadratic`) or `fleiss_kappa` ($N \times k$).
    - `exact_binomial` (`null_proportion` $p_0 \in (0, 1)$, default `0.5`).
  - `compute_odds_and_risk_ratios`: Baptista–Pike OR CI, Koopman score RR CI, Newcombe Wilson-score RD CI, and Altman NNT.
  - `compute_sensitivity_specificity`: sensitivity, specificity, PPV, NPV, and LR+ with Wilson CIs.
  - `compute_phi_and_cramers_v`: $\phi$ and Cramér's $V$ with noncentral-$\chi^2$ CIs.
  - `expected_proportions`: optional non-negative weights for Parts of Whole goodness-of-fit (normalized internally; uniform $1/k$ when unset).
- **Reported outputs:**
  - **Summary** (`contingency-main`, `parts-gof`), **Parameters and CIs** (`contingency-ratios`, `contingency-mcnemar`, `kappa-agreement`, etc.), and **Diagnostics**: statistics ($\chi^2, G^2, z, \kappa, \phi, V$), exact or asymptotic *P* values, OR/RR/RD/NNT and accuracy CIs, plus Clopper–Pearson, Wilson, and Agresti–Coull binomial intervals.
- **Diagnostic checks:** Flags Cochran's expected-cell rule (expected counts $< 5$ or $< 1$), warns when all-zero margins are trimmed, checks degenerate kappa ($p_e = 1$), and logs sparse-cell notes.
- **Edge cases & limits:**
  - All-zero rows or columns are trimmed before association tests; tables reduced below $2 \times 2$ fail cleanly.
  - Non-integer or negative counts fail validation. Cochran–Armitage omits statistics outside $2 \times C$ or $R \times 2$ tables or at zero variance, and Koopman RR bounds report open bounds when bracketing fails.
- **References & verification:**
  - **References:** Pearson (1900); Yates (1934); Wilks (1938); Fisher (1935); Freeman & Halton (1951); Barnard (1945/1947) & Suissa & Shuster (1985); Cochran (1954) & Armitage (1955); McNemar (1947); Baptista & Pike (1977); Koopman (1984); Newcombe (1998); Altman (1998); Clopper & Pearson (1934) & Sterne (1954); Wilson (1927); Agresti & Coull (1998); Cohen (1960, 1968); Fleiss (1971); Fleiss, Cohen & Everitt (1969); Fleiss, Nee & Landis (1979).
  - **Verified against:** `R 4.5.2`, `statsmodels 0.15.0` (`cohens_kappa`), and published exact hypergeometric/binomial benchmarks ([statistics evidence](../evidence/statistics.md)).

---

## Survival analysis (Kaplan–Meier and Cox PH) {#analysis-survival}

Fits nonparametric Kaplan–Meier survival curves with cohort log-rank / Gehan–Breslow–Wilcoxon / trend comparisons and hazard ratios, or multivariable Cox Proportional Hazards regression $h(t | X) = h_0(t)\exp(X \beta)$ with Schoenfeld diagnostics. Compatible with Survival tables (`kaplan_meier`) and Multiple Variables tables (`kaplan_meier` via `mv_time_column`, `mv_status_column`, optional `mv_group_column`; `cox_ph` via `time_column`, `status_column`, `predictor_columns`; see [Analysis Registry: `survival`](registry/tables.html#analysis-kind-survival)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `kaplan_meier` (canonical), `survival`, `log_rank`, `cox_ph`, or `cox`.
- **Options & defaults:**
  - **Kaplan–Meier:** `log_rank_mantel_cox` (boolean), `gehan_breslow_wilcoxon` (boolean), `trend_test` (Tarone 1975 1-df trend when $\ge 3$ cohorts), `asymmetrical_greenwood_ci` (boolean, default Kalbfleisch–Prentice $\ln(-\ln S(t))$ bounds in $[0, 1]$ vs. symmetric linear Greenwood), `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, `control_column`, `mv_time_column`, `mv_status_column`, and `mv_group_column`.
  - **Cox PH:** `time_column`, `status_column`, `predictor_columns`, `reference_levels`, and `tie_method` (`efron` default, `breslow`, `exact`).
- **Reported outputs:**
  - **Kaplan–Meier:** **Summary** (`km-summary` with median survival and Brookmeyer–Crowley 1982 CIs, subjects, events, censored), **Number at Risk**, **Model Comparison** (log-rank, Gehan–Breslow–Wilcoxon, trend $\chi^2, \text{df}, P$), **Parameters and CIs** (Mantel–Haenszel and O/E hazard ratios), **Multiple Comparisons** (`survival-pairwise` for $\ge 3$ cohorts), and step-curve overlays.
  - **Cox PH:** **Parameters and CIs** (`cox-ph` with $\hat{\beta}_j$, SE, Wald $z, P$, hazard ratio $\exp(\hat{\beta}_j)$, Wald CI, likelihood-ratio omnibus $\chi^2$), **Covariance Matrix**, **Baseline Survival** ($H_0(t), S_0(t)$ at design means), **Log-Minus-Log** ($\ln(-\ln \hat{S}(t))$ diagnostics), Harrell's $C$-index, and Grambsch–Therneau Schoenfeld proportional-hazards tests.
- **Diagnostic checks:** Reports Schoenfeld proportional-hazards *P* values, Cox PH monotone-likelihood separation or collinearity warnings (suppressing unstable Wald SE/CIs while keeping omnibus LR *P*), low event counts, and Harrell's $C$-index.
- **Edge cases & limits:**
  - Negative times or invalid censoring indicators fail validation.
  - Zero events in either arm leave O/E or Mantel–Haenszel hazard ratios unestimable; Brookmeyer–Crowley upper median bounds stay open when $S(t)$ never drops below its threshold.
  - Monotone separation in Cox PH halts cleanly with non-converged status instead of diverging estimates.
- **References & verification:**
  - **References:** Kaplan & Meier (1958); Greenwood (1926); Kalbfleisch & Prentice (2002); Brookmeyer & Crowley (1982); Mantel (1966) & Mantel–Haenszel (1959); Greenland & Robins (1985); Peto & Peto (1972); Gehan (1965); Breslow (1970, 1974); Tarone (1975); Berry, Kitchin & Mock (1991); Cox (1972); Efron (1977); Schoenfeld (1982); Grambsch & Therneau (1994); Harrell et al. (1982).
  - **Verified against:** `R 4.5.2` (`survival 3.8.3`, `coxph`) ([statistics evidence](../evidence/statistics.md)).

---

## ROC curve analysis {#analysis-roc-curve}

Builds empirical Receiver Operating Characteristic (ROC) curves comparing a negative control column against a positive case column. Reports trapezoidal AUC, SE, CI, *P* value against $\text{AUC} = 0.5$, Youden's $J$ cutoff, and full threshold tables. Compatible with Column and Multiple Variables tables (see [Analysis Registry: `roc-curve`](registry/tables.html#analysis-kind-roc-curve)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `roc`.
- **Options & defaults:**
  - `ci_method`: `de_long` (default, exact DeLong et al. 1988 structural components) or `hanley_mc_neil` (Hanley & McNeil 1982 exponential approximation).
- **Reported outputs:**
  - **Summary** (`roc-summary`): AUC, SE, `conf%` AUC CI, *P* value testing $\text{AUC} = 0.5$, sample sizes ($n_{\text{controls}}, n_{\text{cases}}$), optimal cutoff, sensitivity, specificity, and maximum Youden's $J = \text{Sens} + \text{Spec} - 1$.
  - **Curve Coordinates** (`roc-coordinates`): threshold table (Cutoff, Sensitivity, Specificity, 1 - Specificity, Youden J, PPV, NPV, LR+) plus curve points for plotting.
- **Diagnostic checks:** Flags $\text{AUC} < 0.5$ (assuming higher values indicate cases). Warns when $n_{\text{controls}} < 2$ or $n_{\text{cases}} < 2$, switching from DeLong variance to Hanley–McNeil SE, Wilson score CI, and exact Mann–Whitney $U$ *P* value.
- **Edge cases & limits:** Uses midpoint ranks for ties at each threshold. Leaves LR+ blank when $\text{Specificity} = 1$, and requires at least one finite value in both controls and cases.
- **References & verification:**
  - **References:** DeLong, DeLong & Clarke-Pearson (1988); Hanley & McNeil (1982); Youden (1950).
  - **Verified against:** `R 4.5.2` and published DeLong / Hanley–McNeil benchmarks ([statistics evidence](../evidence/statistics.md)).

---

## Generalized linear models (linear, logistic, Poisson) {#analysis-generalized-linear-model}

Fits multivariable Generalized Linear Models for Gaussian identity (`linear`), Binomial logit (`logistic`), and Poisson log (`poisson`) families with dummy-encoded categorical predictors, optional VIF collinearity diagnostics, residuals, and goodness-of-fit statistics. Compatible with Multiple Variables, XY, and Column tables (see [Analysis Registry: `generalized-linear-model`](registry/tables.html#analysis-kind-generalized-linear-model)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `glm` (canonical), `linear_glm`, `multiple_linear`, `logistic_glm`, `multiple_logistic`, `poisson_glm`, or `multiple_poisson`.
- **Options & defaults:**
  - `family`: `linear` (default), `logistic`, or `poisson`.
  - `response_column`: 0-based dependent variable index.
  - `predictor_columns`: ordered list of covariate and categorical predictor indices.
  - `reference_levels`: list of `(categorical_col_idx, reference_level_label)` overrides for treatment coding.
  - `compute_vif`: boolean (default `true` on multivariable models).
- **Reported outputs:**
  - **`linear`:** coefficient estimates, SEs, *t*, *P*, `conf%` *t*-CIs, VIF, Type III partial SS/F/P, $R^2$, adjusted $R^2$, $S_{y.x}$, omnibus *F* and *P*, AIC, AICc, BIC, Durbin–Watson, studentized residuals, leverage, and Cook's distance across **Summary**, **Parameters and CIs**, and **Diagnostics**.
  - **`logistic`:** log-odds $\hat{\beta}_j$, SE, Wald $z, P$, odds ratios $\exp(\hat{\beta}_j)$ with Wald CIs, likelihood-ratio $\chi^2, P$, McFadden / Nagelkerke / Tjur pseudo-$R^2$, AIC, AICc, Hosmer–Lemeshow $\chi^2$ and *P*, ROC AUC, and classification cutoff table.
  - **`poisson`:** log-rate $\hat{\beta}_j$, SE, Wald $z, P$, incidence rate ratios $\exp(\hat{\beta}_j)$ with Wald CIs (dispersion $= 1$), residual deviance, Pearson dispersion $\chi^2 / \text{df}$, LR $\chi^2, P$, McFadden pseudo-$R^2$, AIC, AICc, and deviance residuals.
- **Diagnostic checks:** Flags rank-deficient or aliased collinear columns via SVD, high multicollinearity (`VIF > 5` and `VIF > 10`), Gaussian residual normality (Shapiro–Wilk), Albert–Anderson separation in logistic IRLS, Poisson overdispersion or convergence issues, and listwise complete-case row omissions.
- **Edge cases & limits:**
  - Outcomes outside $\{0, 1\}$ fail in `logistic`; negative or non-count outcomes fail in `poisson`.
  - Categorical predictors collapsing below 2 levels after complete-case filtering raise a zero-variance error.
  - Requires $n \ge p + 1$ complete rows (at least 3 rows on XY tables), rejects pre-summarized subcolumns, and stops cleanly with separation flagged instead of diverging estimates.
- **References & verification:**
  - **References:** Golub (1965) & Björck (1967) QR/SVD; McCullagh & Nelder (1989); Albert & Anderson (1984); Hosmer & Lemeshow (1980).
  - **Verified against:** NIST StRD Linear Least Squares and `R 4.5.2` (`stats::lm`, `stats::glm`) ([statistics evidence](../evidence/statistics.md)).

---

## Multivariate analysis (PCA, PCR, correlation, clustering, MICE) {#analysis-multivariate}

Combines exploratory multivariate analysis, correlation matrices, clustering, supervised principal-component regression (PCR), and multiple imputation. Compatible with Multiple Variables, XY, and Column tables (see [Analysis Registry: `multivariate`](registry/tables.html#analysis-kind-multivariate)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `pca` (canonical), `principal_components`, `correlation`, `correlation_matrix`, `clustering`, `kmeans`, `multiple_imputation`, `stats.multiple_imputation`, or `mice`.
- **Options & defaults:**
  - **PCA / PCR:** `standardize` (`true` for correlation PCA, `false` for covariance PCA), `parallel_analysis_seed` (optional seed for Horn 1965 retention cutoffs), and `pcr_outcome_column` (optional target enabling supervised PCR).
  - **Correlation matrix:** `spearman_rank` (`false` for Pearson $r$, `true` for Spearman $\rho$). Standalone Kendall $\tau_b$ is also supported.
  - **Clustering:** `distance` (`euclidean`, `manhattan`, `cosine`, `pearson_correlation_dissimilarity`), `linkage` (`single`, `complete`, `average` UPGMA, `weighted` WPGMA, `centroid` UPGMC, `median` WPGMC, `ward_d2` Murtagh–Legendre Ward criterion), `k_means_k` (optional $k \ge 2$ `k-means++`), and `k_sweep_max` (optional candidate-$k$ WCSS and silhouette sweep).
  - **Multiple imputation:** `imputations_m` (2 to 50), `max_iters` (1 to 100), `seed`, `response_column`, and `predictor_columns` (MICE Predictive Mean Matching pooled via Rubin 1987 and Barnard–Rubin 1999 degrees of freedom).
- **Reported outputs:**
  - **PCA / PCR:** **Summary** (`pca-eigenvalues`, `pcr-summary` with $R^2$, adjusted $R^2$, *F*, RMSE, AIC/BIC), **Parameters and CIs** (`pca-loadings`, `pcr-predictors`, `pcr-components`), scores/biplot coordinates, and **Diagnostics** (`pcr-diagnostics` leverage and Cook's $D$).
  - **Correlation matrix:** symmetric matrices of $r$, unadjusted *P*, Benjamini–Hochberg adjusted *q*-values across off-diagonal pairs, and pairwise sample counts $n_{ij}$.
  - **Clustering:** cluster centroids, assignments, WCSS, and mean silhouette (**Summary** / **Parameters and CIs**); linkage merge schedule, cut silhouettes, and cophenetic $r$ (**Diagnostics**); dense distance and cophenetic matrices when $n$ fits within display limits (**Covariance Matrix**).
  - **Multiple imputation:** pooled means and optional pooled OLS coefficients with pooled SE, *t*, *P*, `conf%` Barnard–Rubin CIs, and Fraction of Missing Information (FMI).
- **Diagnostic checks:** Notes dense matrix omission for large clustering tables (retaining cophenetic $r$ and the linkage tree), cluster separation quality (`silhouette >= 0.25`), Kaiser/parallel component retention, and imputation convergence.
- **Edge cases & limits:** Correlation matrices use pairwise complete cases, emitting `NaN` when pairwise $n < 3$ or variance is zero and omitting non-estimable pairs from the BH family. PCA, clustering, and MICE require complete finite cases and reject pre-summarized subcolumns.
- **References & verification:**
  - **References:** Hotelling (1933) & Jolliffe (2002); Horn (1965); Pearson (1896); Fisher (1921); Spearman (1904); Kendall (1938, 1945); Lloyd (1982); Arthur & Vassilvitskii (2007); Rousseeuw (1987); Lance & Williams (1967); Ward (1963); Murtagh & Legendre (2014); Rubin (1987); Barnard & Rubin (1999); van Buuren & Groothuis-Oudshoorn (2011).
  - **Verified against:** `R 4.5.2` (`prcomp`, `hclust`, `pt`, `qt` Rubin pooling) ([statistics evidence](../evidence/statistics.md)).

---

## Simple and Deming linear regression {#analysis-linear-regression}

Fits ordinary least-squares simple linear regression $Y = \beta_0 + \beta_1 X$ (including ANCOVA slope and intercept comparisons when two or more datasets are selected) or Deming Model II orthogonal regression accounting for error in both *X* and *Y*. Compatible with XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `linear-regression`](registry/tables.html#analysis-kind-linear-regression)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `fit.run` with `linear` (canonical), `simple_linear`, `linear_regression`, `deming`, or `deming_regression`.
- **Options & defaults:**
  - `deming_model_ii`: boolean (default `false`; uses orthogonal variance ratio $\lambda = 1.0$ with leave-one-out jackknife SEs).
  - `force_through_origin`: boolean (default `false`; fixes $\beta_0 = 0$, $\text{df} = n - 1$).
  - `confidence_bands` and `prediction_bands`: booleans enabling Working–Hotelling / Student-*t* curve bands.
  - `interpolate_unknowns`: boolean enabling forward ($X$ to $Y$) prediction and exact Fieller inverse calibration ($Y$ to $X$).
  - `x_range_filter`: optional inclusive `[x_min, x_max]` predictor filter (`plot_x_min`, `plot_x_max`, and `plot_segments`, default `100`, max `10,000`).
- **Reported outputs:**
  - **Summary:** Slope, Intercept, $1/\text{slope}$, $X$-intercept, SEs, `conf%` CIs, $R^2$, Pearson $r$, $S_{y.x}$, $\text{RSS}$, $F$, *P*, Wald–Wolfowitz runs test along $X$, and replicate lack-of-fit *F* test.
  - **Model Comparison:** ANCOVA slope homogeneity and elevation comparisons across multiple datasets.
  - **Interpolated Unknowns:** `Interpolated X` and `Interpolated Y` predictions, plus curve bands.
- **Diagnostic checks:** Notes orthogonal Deming $\lambda = 1.0$ assumptions, runs-test nonlinearity, replicate lack-of-fit, and range-filter exclusions.
- **Edge cases & limits:**
  - Fails with insufficient data when $n \le 2$ (or $n < 1$ when forced through the origin with nonzero $X$ variance).
  - Rejects non-finite points, zero $X$ variance, and pre-summarized inputs needing raw pairs.
  - When `slope == 0` or Deming jackknife variance collapses, $X$-intercept and $1/\text{slope}$ are omitted cleanly.
- **References & verification:**
  - **References:** Deming (1943); Linnet (1990); Fieller (1954); Zar (2010) ANCOVA.
  - **Verified against:** NIST StRD benchmarks and `R 4.5.2` (`stats::lm`) ([curve-fitting evidence](../evidence/curve-fitting.md)).

---

## Nonlinear regression and custom equations {#analysis-nonlinear-regression}

Fits built-in models ([Parametric Equations Registry](registry/equations.html)) or sandboxed custom algebraic, piecewise `<A>`/`<B>`, implicit, and first-order ODE equations using trust-region Levenberg–Marquardt, Poisson deviance IRLS, Cauchy robust regression, or ROUT outlier elimination. Supports shared parameters, hooks, profile-likelihood CIs, model comparison, and unknown interpolation. Compatible with XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `nonlinear-regression`](registry/tables.html#analysis-kind-nonlinear-regression) and [Curve fitting reference](fitting.md)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `fit.run` with `nonlinear` (canonical), `nonlinear_regression`, `curve_fit`, `fit`, `custom_equation`, `custom_dsl`, or any built-in equation ID directly as `kind` (for example `log_agonist_vs_response_4pl`, `michaelis_menten`).
- **Options & defaults:**
  - Model: `equation_id` and `custom_dsl`.
  - Objective & weights: `method` (`least_squares` default, `poisson_deviance`, `robust_cauchy`, `rout_outlier_elimination`), `fdr_q_percent` (default `1.0`, bounded to $(0, 50]$), and `weighting` (`none` default, `relative_square`, `relative`, `general_power_y` + `weighting_power`, `inverse_x`, `inverse_x2`, `observed_sd`).
  - Constraints & solver: `parameter_rules` (`scope`: `local`, `shared_global`, `constant`, `hooked_constant`, `per_dataset_constant`, `column_title_constant`, plus `initial_override`, `lower_bound`, `upper_bound`), `max_iters` (default `1000`, hard cap `50,000`), `x_range_filter`, `y_range_filter`, `plot_function_only`, `plot_x_min`, and `plot_x_max`.
  - Uncertainty & diagnostics: `profile_likelihood_ci` (`true` default), `identify_unstable_parameters`, `flag_r_squared_below`, `confidence_bands`, `prediction_bands`, `interpolate_unknowns`, `replicates_lack_of_fit_test`, `normality_of_residuals`, `homoscedasticity_test`, `emit_residuals_tab`, and `emit_covariance_tab`.
  - Model comparison: `comparison` (`extra_sum_of_squares_f`, `aicc`, `likelihood_ratio_test`), `alternative_equation_id`, and `shared_parameter_comparison`.
- **Reported outputs:**
  - **Parameters and CIs:** estimates, SEs, Venzon–Moolgavkar profile or Wald CIs (including open bounds), dependency $R^2_j$, Hougaard $|g_1|$, parameter notes, and derived scalars such as `EC50`, `t_half`, and `Span`.
  - **Summary:** $N$, $K_{\text{free}}$, df, unweighted $R^2$, `rss`, `weighted_ss`, $S_{y.x}$ or `RSDR`, $\text{AIC}_c$, $\text{BIC}$, runs test, replicates lack-of-fit, residual normality tests, and Spearman homoscedasticity.
  - **Additional tabs:** **Residuals**, **Covariance Matrix**, **Model Comparison**, **Interpolated Unknowns** (`Interpolated Mean X` and `Interpolated Replicates` with `Extrapolated` flags), and **Diagnostics** (`fit_auc`; see [Uncertainty and diagnostics](fitting.md#uncertainty-and-diagnostics)).
- **Diagnostic checks:** Under `robust_cauchy`, parametric covariance, CIs, and bands are omitted intentionally. Flags ambiguous or unstable fits when the Jacobian is rank-deficient, dependency exceeds `0.9999`, or profile bounds plateau; also flags `Dependency > 0.99`, $|g_1| > 1.0$, and $R^2$ below threshold.
- **Edge cases & limits:** Honors exact sufficient statistics on [pre-summarized XY subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N) and enforces strict custom equation limits (`4,096` AST nodes, `50,000` ODE steps per point; see [Custom DSL and ODE caps](fitting.md#custom-dsl-and-ode)).
- **References & verification:**
  - **References:** Bates & Watts (1988); Seber & Wild (1989); Motulsky & Christopoulos (2004); Venzon & Moolgavkar (1988); Motulsky & Brown (2006); Moré (1978); Hairer, Nørsett & Wanner (1993).
  - **Verified against:** NIST StRD Nonlinear Least Squares, `R 4.5.2` (`stats::nls`, `stats::lm`), and `mpmath 1.3.0` (80- and 120-digit references) ([curve-fitting evidence](../evidence/curve-fitting.md); see [Curve fitting verification](fitting.md#oracle-verification)).

---

## Analyze a stack of *P* values {#analysis-p-value-stack}

Treats one or more numeric columns of raw hypothesis-test *P* values $p \in [0.0, 1.0]$ as testing families and computes FWER-adjusted *P* values or FDR *q*-values, ranks, and discovery flags in original row order. Compatible with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `p-value-stack`](registry/tables.html#analysis-kind-p-value-stack)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.pvalueStack` with `pvalue_stack` (canonical), `stats.pvalue_stack`, `p_value_stack`, `pvalues`, `pvaluestack`, or `stats.pvaluestack`.
- **Options & defaults:**
  - `method`: `benjamini_hochberg_fdr` (default), `bonferroni`, `holm_bonferroni`, `sidak`, `holm_sidak`, `benjamini_krieger_yekutieli_fdr`, or `benjamini_yekutieli_fdr`.
  - `threshold`: strict error threshold ($\alpha$ for FWER, $Q$ for FDR) in $(0.0, 1.0)$ (default `0.05`).
  - `column`: optional 0-based column index override when `selected_data_sets` is omitted.
- **Reported outputs:**
  - **Summary** (`pvalue-stack-summary`): method, error-rate family (`FWER` vs. `FDR`), threshold, valid count $m$, discoveries, non-discoveries, discovery fraction, skipped cells, min raw *P*, and min adjusted *P*/*q*, plus scalars (`InputCount`, `Discoveries`, `MinRawP`, `MinAdjustedP`, `Threshold`, `TotalInputCount`, `TotalDiscoveries`).
  - **Multiple Comparisons** (`pvalue-stack-adjusted`): row label, 1-based row index, competition rank, raw *P*, adjusted *P* or *q*, and `Discovery?`.
- **Diagnostic checks:** Notes whether the chosen method assumes independence (`sidak`, `holm_sidak`, BH, BKY) or handles arbitrary dependence (`bonferroni`, `holm_bonferroni`, BY), and logs ignored text cells.
- **Edge cases & limits:** Rejects values outside $[0.0, 1.0]$ or non-finite numbers, skips blank and excluded cells cleanly, requires `threshold` strictly in $(0.0, 1.0)$, rejects pre-summarized subcolumns, and preserves exact ties in ranks and monotonicity.
- **References & verification:**
  - **References:** Bonferroni (1936) & Dunn (1961); Holm (1979); Šídák (1967); Holland & Copenhaver (1987); Benjamini & Hochberg (1995); Benjamini & Yekutieli (2001); Benjamini, Krieger & Yekutieli (2006).
  - **Verified against:** `R 4.5.2` (`p.adjust`) and `statsmodels 0.15.0` (`multipletests(method="fdr_tsbky")`) ([statistics evidence](../evidence/statistics.md)).

---

## Statistical power and sample size {#analysis-power-analysis}

Computes prospective or post-hoc sample sizes ($n_1, n_2, N$), achieved power ($1 - \beta$), minimum detectable effect sizes, sensitivity grids, and power-vs-$N$ curves across eight study designs. Compatible with all [table archetypes](tables.md#table-archetypes) and optionally hooks scalars from an info sheet (see [Analysis Registry: `power-analysis`](registry/tables.html#analysis-kind-power-analysis)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `power` (canonical), `power_analysis`, `stats.power`, or `power_sample_size`.
- **Options & defaults:**
  - `family`: `two_sample_unpaired_t` (default), `two_sample_paired_t`, `one_sample_t`, `one_way_anova`, `two_proportions`, `correlation`, `chi_square`, or `simple_linear_regression`.
  - `solve_target`: `sample_size` (default), `power`, or `detectable_effect`.
  - `tail_mode`: `two_sided`, `one_sided_greater`, or `one_sided_less` (one-sided tests are disallowed on non-directional `one_way_anova` and `chi_square`).
  - `alpha` in $(0, 1)$ (default `0.05`); `target_power` in $(0, 1)$ (for `sample_size` or `detectable_effect`); `sample_size`, `sample_size_group2`, and `allocation_ratio` ($k = n_2 / n_1 > 0$).
  - `effect` (standardized `d`, `d_z`, `f`, `h`, `w`, `r`, `r_squared`, `f_squared`, or raw means + SD, proportions $p_1, p_2$, or regression slope + $\sigma_x, \sigma_\varepsilon$), `num_groups` ($\ge 2$ for ANOVA), `chi_square_df`, `two_proportions_mode` (`pooled_null_unpooled_alt`, `unpooled`, `cohens_h` + `continuity_correction`), `correlation_method` (`fisher_z`, `pwr_hybrid`, `exact_conditional_t`), and optional `info_sheet_id`.
- **Reported outputs:**
  - **Summary** (`power-summary`): family, solve target, tail mode, $\alpha$, target power, standardized and raw effects, integer $n_1, n_2, N_{\text{total}}$, `achieved_power`, `continuous_n`, `power_at_continuous_n`, denominator df, critical cutoff, noncentrality $\lambda$, and implied metrics ($p_2$, Cohen's $h$, $R^2$).
  - **Multiple Comparisons** (`power-sensitivity`): $(\alpha, 1 - \beta, \text{effect})$ sensitivity grid.
  - **Curve Coordinates** (`power-curve`): power-vs-$N$ coordinates, curve overlay, and analyzable output table.
- **Diagnostic checks:** Notes discrete rounding uplift between `continuous_n` and integer `n_per_group`, exact vs. asymptotic distribution scope, and invalid effect boundaries.
- **Edge cases & limits:** Powered by tail-safe noncentral-$t$, noncentral-$F$, and noncentral-$\chi^2$ evaluators with Brent–Dekker root finding. Zero effects or $p_1 = p_2$ fail when solving for sample size; `r_squared` must lie in $[0, 1)$ and proportions in $[0, 1]$.
- **References & verification:**
  - **References:** Cohen (1988); Owen (1965) noncentral-$t$; Fleiss, Tytun & Ury (1980) & Fleiss, Levin & Paik (2003); Chow, Shao & Wang (2008); Fisher (1921).
  - **Verified against:** `R 4.5.2` (`pwr` package) and exact noncentral distributions ([statistics evidence](../evidence/statistics.md)).

---

## Simple logistic regression {#analysis-simple-logistic-regression}

Fits single-predictor binary logistic regression $\ln\!\left(\frac{P(Y=1 \mid X)}{1 - P(Y=1 \mid X)}\right) = \beta_0 + \beta_1 X$ on single-subcolumn XY tables with binary $Y_i \in \{0, 1\}$. See [Analysis Registry: `simple-logistic-regression`](registry/tables.html#analysis-kind-simple-logistic-regression).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `simple_logistic` (canonical), `simple_logistic_regression`, `stats.simple_logistic`, or `xy_logistic`.
- **Options & defaults:**
  - `emit_row_classification`: boolean (default `true`; includes observed $(X_i, Y_i)$, predicted $\hat{P}(X_i)$ with CIs, and automatic predictions for `(X_new, blank Y)` rows).
  - `confidence_bands`: boolean (default `true`; adds two-sided inverse-logit confidence bands on the sigmoid overlay).
  - `classification_cutoff`: threshold in $(0, 1)$ (default `0.5`).
  - `plot_x_min`, `plot_x_max`, and `plot_segments` (default `100`, max `10,000`).
- **Reported outputs:**
  - **Parameters and CIs** (`parameter-estimates`): $\hat{\beta}_0$, $\hat{\beta}_1$, odds ratios $e^{\hat{\beta}_0}, e^{\hat{\beta}_1}$, and $X_{50} = -\hat{\beta}_0 / \hat{\beta}_1$ with asymptotic SEs, asymmetrical profile-likelihood CIs, and Wald $Z$ tests.
  - **Summary** (`goodness-of-fit`): $N$, event/non-event counts, separation status, deviances ($D_0, D_1$), likelihood-ratio $G^2 = D_0 - D_1 \sim \chi^2_1$, $\text{AIC}$, $\text{AIC}_c$, $\text{BIC}$, Tjur/Cox–Snell/Nagelkerke/McFadden $R^2$, DeLong ROC AUC with SE/CI, and classification accuracy/sensitivity/specificity.
  - **Curve Coordinates** (`roc-curve`) and **Interpolated Unknowns** (`row-classification`, optional), plus the sigmoid curve overlay.
- **Diagnostic checks:** Detects collinearity (constant $X$), complete separation, and quasi-complete separation explicitly instead of emitting diverging estimates; warns when minority event count $< 10$.
- **Edge cases & limits:** Rejects non-binary $Y$, all-zero/all-one outcomes, non-finite inputs, $N < 3$, non-XY tables, and multi-subcolumn XY tables; returns open profile bounds for $X_{50}$ whenever $G^2 \le \chi^2_{1, 1-\alpha}$.
- **References & verification:**
  - **References:** Hosmer, Lemeshow & Sturdivant (2013); McCullagh & Nelder (1989); Tjur (2009); Albert & Anderson (1984); Heinze & Schemper (2002); Venzon & Moolgavkar (1988); DeLong, DeLong & Clarke-Pearson (1988).
  - **Verified against:** `R 4.5.2` (`stats::glm(..., family = binomial)` and `confint`) ([statistics evidence](../evidence/statistics.md)).

---

## Fit spline / LOWESS curve {#analysis-fit-spline-lowess}

Generates model-free smooth XY curve overlays, optional sampled `(X, Y)` coordinate tables, and nonparametric unknown interpolation (`Y` from `X` and `X` from `Y`) without assuming a parametric equation. Compatible exclusively with XY tables (see [Analysis Registry: `fit-spline-lowess`](registry/tables.html#analysis-kind-fit-spline-lowess)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `transform.run` with `fit_spline_lowess` (canonical), `spline_lowess`, `spline`, `cubic_spline`, `natural_cubic_spline`, `akima`, `akima_subspline`, `smoothing_spline`, `lowess`, `point_to_point`, or `fit.spline_lowess`.
- **Options & defaults:**
  - `method`: `natural_cubic_spline` (default), `akima_subspline`, `smoothing_spline`, `lowess`, or `point_to_point`.
  - `segment_count`: polyline vertices in `[2, 10000]` (default `200`; LOWESS automatically uses $\ge 4N$ segments).
  - `smoothing_knots` / `smoothing_df`: knot count ($\ge 2$) or effective degrees of freedom ($\text{df} = \text{tr}(S_\lambda) \in [2, G]$) for Reinsch smoothing splines.
  - `lowess_preset` (`coarse` = `0.75`, `medium` = `0.50` default, `fine` = `0.25`), optional `lowess_fraction` in `(0, 1]`, and `lowess_robustness_iters` (default `3`, max `20`).
  - `plot_x_min`, `plot_x_max`: optional explicit evaluation bounds.
  - `emit_curve_coordinates_tab`: boolean (default `false`; emits sampled `(X, Y)` vertices).
  - `interpolate_unknowns`: boolean (default `false`; predicts $Y$ from $X$ and solves $S(x) = y_{\text{unk}}$ for $X$ from $Y$).
- **Reported outputs:**
  - **Summary** (`summary`): method, active $N$, distinct $X$ knots $G$, vertex count, domain bounds, RSS, RMSE, plus effective $\text{df}$ and $\lambda$ (smoothing spline) or span $f$ and passes (LOWESS), with hookable scalars (`n_points`, `distinct_x`, `segments`, `rss`, `rmse`, `effective_df`, `smoothing_lambda`, `lowess_span`).
  - **Curve Coordinates** (`curve-coordinates`, optional): sampled `(X, Y)` curve vertices per dataset.
  - **Interpolated Unknowns** (`interpolated-unknowns`, optional): forward and inverse predictions with multi-crossing notes on non-monotonic curves.
- **Diagnostic checks:** Warns when `lowess` is run with $N < 20$ active points (noting spline alternatives for small samples).
- **Edge cases & limits:** Collapses tied $X$ values to `(x_g, mean y_g)` for splines/Akima/point-to-point and multiplicity-weighted means for Reinsch smoothing splines; rejects non-finite inputs, fewer than 2 distinct $X$ values, out-of-range spans/df, and non-XY tables.
- **References & verification:**
  - **References:** de Boor (1978); Akima (1970); Reinsch (1967); Hutchinson & de Hoog (1985); Cleveland (1979); Brent (1973).
  - **Verified against:** `R 4.5.2` (`stats::splinefun(..., method="natural")`, `stats::smooth.spline`, `stats::lowess`) and analytical Akima step-overshoot benchmarks ([curve-fitting evidence](../evidence/curve-fitting.md)).

---

## Levene / Brown–Forsythe variance test {#analysis-levene-test}

Tests the null hypothesis that selected groups have equal population variances. Each selected column of a raw-replicate Column table or numeric variable of a Multiple Variables table is treated as one group. Pre-summarized Column subcolumns are rejected because absolute deviations require individual replicates (see [Analysis Registry: `levene-test`](registry/tables.html#analysis-kind-levene-test)).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `stats.run` with `levene` (canonical), `levene_test`, `stats.levene`, `levene_median`, `brown_forsythe_variance`, `brown_forsythe_levene`, `variance_homogeneity`, `equal_variance_test` (all median-centered), or `levene_mean` (mean-centered).
- **Options & defaults:**
  - `center`: `median` (default; Brown–Forsythe) or `mean` (Levene's original test).
  - `selected_data_sets`: columns to compare (empty selects every column, or every numeric variable on Multiple Variables tables).
- **Reported outputs:**
  - **Summary** (`levene-summary`): method, center, group count $k$, total $N$, $F$, $\text{DFn} = k - 1$, $\text{DFd} = N - k$, *P* value, and decision at $\alpha$.
  - **Descriptive Summary** (`levene-groups`): per-group $n$, center, mean absolute deviation, and SD.
  - **Scalars:** `F`, `DFn`, `DFd`, `P value`, `Groups`, `Total N`, and per-group `N`, `Center`, and `Mean abs deviation`.
- **Diagnostic checks:** Reports the equal-variance decision, warns when any group has fewer than 5 values, notes skewness sensitivity for `mean` centering, and lists excluded or ignored cells.
- **Edge cases & limits:**
  - Fewer than 2 groups or any group with $n < 2$ fails with insufficient sample size.
  - Zero within-group deviation spread (such as constant groups or all groups of size 2) fails with zero variance instead of an undefined $F$.
  - Tables other than Column or Multiple Variables are rejected.
- **References & verification:**
  - **References:** Levene (1960); Brown & Forsythe (1974); Zar (2010, ch. 10).
  - **Verified against:** `SciPy 1.18.1` (`scipy.stats.levene`) and `R 4.5.2` (`anova(lm(|x - c| ~ g))`) ([statistics evidence](../evidence/statistics.md)).

---

## Area under the curve (AUC) {#analysis-area-under-curve}

Computes linear trapezoidal Area Under the Curve (Yeh & Kwan 1978) across continuous curves. Supports optional `[x_min, x_max]` partial-window interpolation, clipped peak detection above the baseline, and design-gated Bailer (1988) standard errors for signed net area. Compatible exclusively with XY tables (see [Analysis Registry: `area-under-curve`](registry/tables.html#analysis-kind-area-under-curve)).

Requires finite numeric `X` coordinates in the desired integration units (see the [Numeric-X contract](tables.md#numeric-x-contract): day-fraction serials yield AUC in day units unless converted first, and unparsed `hh:mm:ss` text cells are ignored).

- **Commands & shorthands:** Run via [`analysis.run`](../commands.md) or `transform.run` with `auc` (canonical), `area_under_curve`, or `transform.auc`.
- **Options & defaults:**
  - `baseline`: `{ "mode": "fixed", "value": 0.0 }` (or bare finite number, default $Y = 0$) or `{ "mode": "first_point" }` (or `"first_point"`, using $Y(\text{Start } X)$ on each series's clipped window).
  - `x_min`, `x_max`: optional inclusive bounds (`x_min < x_max` when both set); bounds between knots interpolate $(a, Y(a))$ and $(b, Y(b))$ linearly.
  - `independent_time_points`: boolean (default `false`), gating Bailer (1988) `NetAreaSe`.
  - `min_peak_height`: non-negative height threshold above baseline to retain a peak (default `0.0`).
  - `min_area_fraction`: fraction of positive area in $[0.0, 1.0]$ required to retain a peak (default `0.0`).
- **Partial-AUC interpolation, Bailer knot weights, and peak clipping:**
  - Explicit `x_min` and `x_max` bounds enforce coverage within `tol(bound) = 1e-12 * max(1, |bound|)`. Floating-point jitter snaps cleanly while truncated spans fail.
  - Each segment $[x_i, x_{i+1}]$ clipped to $[a, b]$ of width $L = b - a$ computes $t_a = (a - x_i) / (x_{i+1} - x_i)$, $t_b = (b - x_i) / (x_{i+1} - x_i)$, and midpoint $m = (t_a + t_b) / 2$. Linear trapezoidal integration accumulates knot weights `w_i += L * (1 - m)` and `w_{i+1} += L * m`.
  - Under `first_point` baseline, subtracting $W = \text{End } X - \text{Start } X$ times the baseline interpolation weights (`w_{k_0} -= W * (1 - t_0)`, `w_{k_0+1} -= W * t_0`) ensures $\sum_k w_k = 0$. This preserves exact Bailer variance $\text{Var}(\text{Net Area}) = \sum_k w_k^2 \text{SE}_k^2$ even when the baseline shares knots with the integral.
  - `NetAreaSe` applies strictly to signed `Net Area`. It is computed **only** when `independent_time_points` is `true` and every contributing knot has a finite point SE ($n \ge 2$ replicates or parametric summary subcolumns). When `false` or when any knot has $n = 1$, `NetAreaSe` stays blank with a diagnostic note explaining the assumption or naming the $n = 1$ knots.
  - Peak detection runs on the clipped curve over `[Start X, End X]` relative to the resolved baseline, bounding every peak inside `[Start X, End X]` and filtering by `min_peak_height` and `min_area_fraction`.
- **Reported outputs:**
  - **Summary** (`auc-summary`): Start X, End X, Span, N Points, Baseline, Positive Area, Negative Area, Net Area, Total Area, Net Area SE (Bailer, indep.), Peak Count, Peak X, Peak Y, and Status.
  - **Descriptive Summary** (`auc-peaks`, when peaks qualify): Series, Peak #, Peak X, Peak Y, Height Above Baseline, Start X, End X, Area, and Area Fraction.
  - **Scalars** (keyed by dataset index): `PositiveArea`, `NegativeArea`, `NetArea`, `TotalArea`, and `NetAreaSe` (when computed).
  - **Analyzable table:** Multiple Variables sheet (Series, Start X, End X, Span, N Points, Baseline, Positive Area, Negative Area, Net Area, Total Area, Net Area SE) ready for downstream analyses or `table.importPlateMap` (`source.value_column`).
- **Diagnostic checks:**
  - When either or both window bounds are omitted, CurveBook compares effective `Start X` and `End X` across all series and warns whenever series span different windows.
  - Interior missing or excluded rows between valid knots are bridged linearly (Yeh & Kwan 1978) and counted in an info note.
  - Unavailable series (truncated spans or fewer than 2 distinct finite `X` points) keep their Series row with blank numeric cells, a Status explanation, and a warning. The analysis fails only if zero series succeed.
  - Non-numeric text entries in `X` trigger a warning explaining the numeric-X requirement.
- **Edge cases & limits:** Tied `X` observations collapse to their arithmetic mean (pooling summary moments or replicate SEM). Invalid bounds (`x_min >= x_max`), non-finite baselines, negative `min_peak_height`, `min_area_fraction` outside $[0, 1]$, and non-XY tables fail validation cleanly.
- **References & verification:**
  - **References:** Yeh & Kwan (1978) (*J. Pharmacokinet. Biopharm.* 6(1):79–98); Bailer (1988) (*J. Pharmacokinet. Biopharm.* 16(3):303–309).
  - **Verified against:** `R 4.5.2`, `SciPy`, and exact closed-form partial-window and first-point knot-weight benchmarks ([statistics evidence](../evidence/statistics.md)).
