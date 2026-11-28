# Statistical analyses and transforms reference

<!-- Audience: Tier 2 Technical Reference — complete statistical formulas, parameters, assumptions, edge cases, and citations belong here; keep desktop-gui.md and get-started.md concise for beginners. -->

This reference describes, for every CurveBook analysis and transform, the statistical method, its option keys, the outputs it reports, the assumption checks it runs, how it handles edge cases, and the peer-reviewed sources it follows. For an interactive walkthrough of running analyses and viewing live graphs, start with the [Desktop GUI overview](desktop-gui.md).

Use the sidebar to jump to a specific test. Compact summaries of table compatibility and parameters are in the [Analysis Kinds Registry](registry/tables.html#analysis-kinds) and [Table Archetypes](tables.md#table-archetypes).

> [!NOTE]
> CurveBook's diagnostic checklists report assumption metrics, sample-size limits, convergence status, and missing-data notes as facts. They never rank tests, recommend a different procedure, or transform your data automatically (see [Quality-of-life capabilities](qol.md#active-qol)).

- **Settings shared by all analyses:** Every analysis stores `confidence_level`, `p_value_format`, and `selected_data_sets`.
  - For `confidence_level`, commands take `confidence_percent` in $(1.0, 99.99]$, with a default of `95.0`. Values of 1.0 or less are rejected so that a fraction such as `0.95` is not mistaken for a percentage. Saved analysis specs accept the wider range $[0.01, 99.99]$.
  - An empty `selected_data_sets` selects every compatible column, unless the analysis requires explicit indices.
  - Excluded cells are always removed before any computation ([Cell exclusions](concepts.md#cell-exclusions)).
- **Running analyses:** Run an analysis with [`analysis.run`](../commands.md) (or `transform.run`, `stats.run`, `fit.run`), change it with [`analysis.updateParams`](../commands.md), or point it at different data with [`analysis.retargetData`](../commands.md). [`analyze.run`](../commands.md) recalculates every pending sheet in the project. An unknown option key returns `bad_input` ([Engine error codes](registry/tables.html#error-codes)).
- **Multiple-comparison methods (`multiplicity`):** The accepted values are `none` (the default; unadjusted or omnibus results only), `tukey_hsd`, `dunnett` (with `control_column`), `sidak`, `bonferroni`, `holm_bonferroni` (alias `holm`), `holm_sidak`, `dunnett_t3`, `games_howell`, `tamhane_t2`, `dunn_rank_sum`, `linear_trend`, `benjamini_hochberg_fdr`, `benjamini_yekutieli_fdr`, `two_stage_bky_fdr`, `newman_keuls`, `duncan` (alias `duncan_mrt`), and `fisher_lsd`. Supplying `control_column` restricts comparisons to the $k - 1$ treatment-versus-control family for `dunnett`, `bonferroni`, `holm_bonferroni`, `sidak`, `holm_sidak`, `dunn_rank_sum`, `fisher_lsd`, `dunnett_t3`, `tamhane_t2`, and the three FDR procedures. Supplying `selected_pairs` (`[[a, b], ...]`, mutually exclusive with `control_column` and `dunnett`) restricts comparisons to $m$ planned pairs for `bonferroni`, `holm_bonferroni`, `sidak`, `holm_sidak`, `dunn_rank_sum`, `fisher_lsd`, `dunnett_t3`, `tamhane_t2`, and the three FDR procedures. FDR methods read `threshold` as the false discovery rate $Q$. FWER methods read it as the family-wise $\alpha$ (default `0.05`; see [Number formatting](concepts.md#number-and-pvalue-formatting)).

---

## Data transforms, reshaping, and simulation {#analysis-transform}

Transforms apply math functions, normalization, baseline removal, transposition, row pruning, formula filters, synthetic XY simulation, or row sampling without changing the source data. Each run produces a linked derived data table and a summary sheet. Transforms work with every [table archetype](tables.md#table-archetypes) (see [Analysis Registry: `transform`](registry/tables.html#analysis-kind-transform)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `transform.run` with `kind` set to `math` (the preferred name), `transform`, `normalize`, `remove_baseline`, `baseline`, `transpose`, `prune`, `prune_rows`, `select`, `select_and_transform`, `simulate`, or `random_sample`.
- **Options and defaults:**
  - `math`: `x_transform` and `y_transform` take `none` or a preset such as `log10`, `ln`, `log2`, `exp10`, `exp`, `reciprocal`, `square`, `sqrt`, `logit`, `probit`, or `arcsin_sqrt`. `pharmacology_preset` applies classic enzyme-kinetics and radioligand transforms including `lineweaver_burk` ($1/X$ vs. $1/Y$ with zero-value guarding, double-reciprocal OLS diagnostics for apparent $V_{\max}$ and $K_m$, and an explicit reciprocal error-distortion checklist warning recommending nonlinear Michaelis–Menten regression for parameter estimation), `eadie_hofstee`, `hanes_woolf`, and `scatchard`. `error_propagation` is `replicate_endpoints` (the default) or `first_order_delta` / `delta_method`.
  - `normalize`: `zero_anchor` and `hundred_anchor` each take `column_min`, `column_max`, `column_sum`, `first_row`, `last_row`, a fixed constant, a control-column mean, or a scalar linked from another result. `output_percent` and `per_data_set` are booleans.
  - `remove_baseline`: `baseline_columns`; `baseline_kind` (`ColumnsRowWise`, `ColumnsMean`, `Constant`, `RowRangeMean`, `FirstAndLastLinear`, `OlsLinearWithX`, or `PairedEveryOtherColumn`); and `operation` (`difference`/`subtract`, `ratio`/`divide`, `percent_change`, or `fractional_change`).
  - `transpose`: `swap_row_and_column_titles` (boolean).
  - `prune_rows`: `every_k_rows` ($\ge 1$) and `average_window` (boolean).
  - `select_and_transform`: `filter_formula`, `target_archetype`, and `derived_columns`.
  - `simulate`: `seed`, `simulation_mode` (`XyCurve`, `Column`, or `Contingency2x2`), `row_count`, `replicates`, `x_min`, `x_max`, `equation_id`, `parameters`, and `noise`.
  - `random_sample`: `seed`, `sample_size`, `stratify_column`, and `proportional_allocation`.
- **Reported outputs:**
  - Each run produces a linked derived data table and a **Summary** tab (`summary`). The tab records the transform settings and, where relevant, the anchor scalars (`ZeroAnchor`, `HundredAnchor`) or the sampling fields (`sample_size`, `population_size`, `seed`).
  - Curve utilities (AUC, Savitzky–Golay and LOWESS smoothing, numerical derivatives, and cumulative integrals) are described under [Transforms and reshaping](tables.md#transforms-and-reshaping).
- **Diagnostic checks:** When a domain guard blanks an endpoint or derivative, the transform still completes and adds a warning and a checklist note. Typical causes are `ln(u)` at $u \le 0$ and a transformed mean of zero in a Mean/%CV/N summary.
- **Edge cases and limits:**
  - On [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N, Mean/Low/High, Median/IQR, Upper/Lower Limits), `first_order_delta` scales the spread by $|g'(\text{mean})|$. When $g' < 0$, it swaps the asymmetric `-Error` and `+Error` bounds.
  - `replicate_endpoints` maps $[\text{mean} - \text{SD}, \text{mean} + \text{SD}]$ through $g$ and halves the resulting width. Exclusion notes are kept.
  - Equal anchors (`zero_anchor == hundred_anchor`) fail with an invalid-input error.
  - `random_sample` fills stratified quotas reproducibly from the recorded seed.
- **References and verification:**
  - References: JCGM 100:2008 / GUM §5.1.2 (delta-method propagation); Zar (2010) (logit, probit, angular $\arcsin\sqrt{u}$); Cleveland (1979); Savitzky & Golay (1964); Gorry (1990).
  - Verified against: `mpmath 1.3.0` (80- and 120-digit rational and transcendental benchmarks) and `R 4.5.2` ([statistics evidence](../evidence/statistics.md)).

---

## Column descriptive statistics, normality, and one-sample tests {#analysis-descriptive-stats}

This analysis summarizes the distribution of each column, tests for Gaussian or lognormal distributions, produces Q–Q plot coordinates, and can run one-sample tests against a hypothetical value. It works with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `descriptive-stats`](registry/tables.html#analysis-kind-descriptive-stats)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `descriptive_stats` (the preferred name), `descriptive`, or `column_stats`.
- **Options and defaults:**
  - `aggregation` controls how replicate subcolumns are handled:
    - `RowMeans` (default) first collapses replicate subcolumns to their row means.
    - `PerSubcolumn` summarizes each replicate subcolumn separately.
    - `PooledGrandSample` pools all active replicates into one sample.
  - `custom_upper_percentile` is an optional $p_{\text{hi}} \in [50.0, 100.0)$. When set, the analysis reports the Hyndman–Fan Type-6 quantiles $(100 - p_{\text{hi}}, p_{\text{hi}})$ (`p(n + 1)` linear interpolation, matching standard quartiles). It is unset by default.
  - `run_normality_battery` is a boolean (default `true`). Set `normality_only` to skip the descriptive tables and report only the normality and lognormality tests. Set `emit_qq_tabs` to add both the `normal-qq` and `lognormal-qq` coordinate tabs.
  - `hypothetical_value` is an optional finite $\mu_0$ (unset by default). When set, the analysis also runs a one-sample Student *t* test and a Wilcoxon signed-rank test.
  - `visible_metrics` toggles which of the 12 descriptive statistic groups (`min_max_range`, `quartiles`, `mean_sd_sem`, `ci_of_mean`, `ci_of_sd`, `ci_of_median`, `cv_percent`, `geometric_mean`, `harmonic_mean`, `quadratic_mean`, `skewness_kurtosis`, `sum`) appear in the summary table (all `true` by default; `Number of values` and `Excluded values` always remain visible).
  - `significant_digits` is an optional integer in `1..=15` (default `None`, automatic) controlling displayed numeric precision on descriptive statistics tables and embedded result tables while keeping stored values full-precision `f64`.
  - `save_as_default` is a boolean (default `false`); when enabled in the desktop UI, saves the current descriptive statistics configuration into user preferences to seed future Descriptive Statistics analyses.
- **Reported outputs:**
  - **Descriptive Summary** (`desc-summary` or `desc-pooled`):
    - $N$, Minimum, 25% Percentile, Median, 75% Percentile, Maximum, Interquartile Range (IQR), Range, Sum, Mean, Std. Deviation, Std. Error of Mean, and %CV.
    - The `conf%` CI of the mean, the `conf%` CI of the SD ($\chi^2$ normal-theory interval), and an exact or interpolated binomial CI of the median.
    - Geometric Mean, Geometric SD Factor, and the CI of the geometric mean (when all values are $> 0$); Harmonic Mean and its CI (when valid); Quadratic Mean (RMS) and its CI.
    - Skewness ($G_1$), excess kurtosis ($G_2$), and lag-1 autocorrelation ($r_1$).
  - **Diagnostics** (`normality`): The Shapiro–Wilk $W$ ($3 \le n \le 5000$), D'Agostino–Pearson $K^2$ ($n \ge 8$), Anderson–Darling $A^{2*}$ ($n \ge 8$), and Lilliefors Kolmogorov–Smirnov $D$ ($n \ge 5$) tests, applied to the raw data and, when all values are strictly positive, to $\ln(Y)$. The tab also reports the normal-versus-lognormal log-likelihood ratio and the relative likelihood.
  - **Normal & Lognormal Q–Q** (`normal-qq`, `lognormal-qq`): Blom (1958) plotting positions $p_i = (i - 3/8)/(n + 1/4)$ and the expected normal or lognormal $z$ quantiles used by `NormalQqPlot` and `LognormalQqPlot`.
  - **Summary** (`one-sample`): When `hypothetical_value` is set, this tab reports the one-sample *t* test ($t, \text{df}, P$) and the Wilcoxon signed-rank test ($W, P$), with Pratt (1959) handling of zero differences and the Hodges–Lehmann pseudomedian and its CI.
  - **Scalars:** Per-data-set `N`, plus `Mean`, `Median`, `SD`, and `SEM` (omitted when `normality_only` is set). When the normality battery runs, CurveBook emits per-data-set `DAgostinoPearsonK2`, `DAgostinoPearsonP`, `ShapiroWilkW`, `ShapiroWilkP`, `AndersonDarlingA2` (unadjusted $A^2$), `AndersonDarlingA2Star` (Stephens-modified $A^{2*}$), `AndersonDarlingP`, `KolmogorovSmirnovD`, and `KolmogorovSmirnovP`. Strictly positive columns also emit matching `Lognormal*` keys, along with `ImpossibleNonPositiveCount`, `NormalVsLognormalLogLR`, `PNormal`, and `PLognormal`.
- **Diagnostic checks:** For each column, the checklist reports the Shapiro–Wilk result (pass if $P > \alpha$, warning if $P \le \alpha$) and lists any non-numeric text cells that were ignored.
- **Edge cases and limits:**
  - On [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N), CurveBook reconstructs the exact pooled mean, SD, SEM, $N$, and CI of the mean. Nonparametric quantiles, medians, Q–Q plots, and the normality and Wilcoxon tests need raw values, so they are omitted.
  - For a zero-variance column, the analysis reports the constant summaries and leaves out the *t* statistics. A column with no finite numeric values returns an empty-input error.
- **References and verification:**
  - References: Hyndman & Fan (1996) Type-6 quantiles; Royston (1995) AS R94; D'Agostino, Belanger & D'Agostino (1990); Anscombe & Glynn (1983); Stephens (1986); Lilliefors (1967) & Dallal & Wilkinson (1986); Pratt (1959) & Hollander, Wolfe & Chicken (2014); Kirkwood (1979).
  - Verified against: NIST StRD Univariate benchmarks, `SciPy 1.18.1` (`normaltest`, `anderson`), and `statsmodels 0.15.0` (`normal_ad`, `lilliefors`) ([statistics evidence](../evidence/statistics.md)).

---

## Outlier detection (Grubbs and ROUT) {#analysis-outlier-detection}

This analysis screens each column on its own for one or more univariate outliers. It offers the normal-theory Grubbs extreme studentized deviate (ESD) test and the robust ROUT method, which controls the false discovery rate. It works with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `outlier-detection`](registry/tables.html#analysis-kind-outlier-detection)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `outliers` (the preferred name), `outlier`, `rout`, or `grubbs`.
- **Options and defaults:**
  - ROUT (`method: "rout"`, the default): `fdr_q_percent` lies in $(0.0, 100.0]$, with a default of `1.0` ($Q = 1\%$). Column ROUT accepts $Q \in (0, 100]$, whereas ROUT in curve fitting is limited to $Q \in (0, 50]$.
  - Grubbs (`method: "grubbs"`): `alpha` lies in $(0.0, 1.0)$ (default `0.05`). The boolean `iterative` is `false` by default, which tests for a single outlier; `true` runs the sequential ESD procedure.
- **Reported outputs:**
  - **Summary** (`outliers`, titled "Identified Outliers"): for each flagged value, the column, the 1-based row index, the observed value, the test statistic ($G$, or $t_i = |y_i - M| / \text{RSDR}$ for ROUT), and the threshold or *P* value (the Grubbs $G_{\text{crit}}$ or the Benjamini–Hochberg $q$ value).
  - Scalars: `OutlierCount` for each column, plus `RobustCenter` and `RSDR` for ROUT.
- **Diagnostic checks:** The checklist notes the normality assumption behind Grubbs and describes the ROUT screen, which uses the median and the 68.27th-percentile RSDR and examines the furthest 30% of observations. It also reports how many outliers were found. Whether a flagged cell should be excluded is a decision for the experimenter, based on how the data were collected.
- **Edge cases and limits:**
  - Both methods need at least $n \ge 3$ active values. Grubbs reports zero outliers when $\text{SD} = 0$.
  - ROUT resists masking when several extreme outliers are present. When the clean core of the data has zero spread, ROUT avoids division by zero and still flags points that lie off the constant value. Non-numeric cells are ignored and noted in the checklist.
- **References and verification:**
  - References: Grubbs (1969); Rosner (1983) generalized ESD; Motulsky & Brown (2006) ROUT; Benjamini & Hochberg (1995).
  - Verified against: the NIST/SEMATECH e-Handbook Grubbs benchmark and the Motulsky & Brown (2006) ROUT reference cases ([statistics evidence](../evidence/statistics.md)).

---

## Two-group and one-sample comparisons {#analysis-two-group-comparison}

This analysis compares two independent groups, two matched sets of values, or one group against a hypothetical location. It offers parametric tests, lognormal and ratio tests, and nonparametric rank and distribution tests. It works with Column and Multiple Variables tables. In a long-format Multiple Variables table, `mv_group_column` and `mv_response_column` split the response by a two-level factor (see [Analysis Registry: `two-group-comparison`](registry/tables.html#analysis-kind-two-group-comparison)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `unpaired_t` (the preferred name), `t_test`, `unpaired_t_test`, `welch_t`, `paired_t`, `ratio_paired_t`, `one_sample_ratio_t`, `lognormal_unpaired_t`, `lognormal_welch_t`, `mann_whitney`, `wilcoxon`, `wilcoxon_matched_pairs`, `kolmogorov_smirnov`, `ks_test`, `paired_sign_test`, `sign_test`, `one_sample_t`, or `one_sample_wilcoxon`.
- **Options and defaults:**
  - `method`: `unpaired_t` (default), `welch_t`, `paired_t`, `ratio_paired_t`, `one_sample_ratio_t`, `lognormal_unpaired_t`, `lognormal_welch_t`, `mann_whitney`, `wilcoxon_signed_rank`, `kolmogorov_smirnov`, `paired_sign_test`, `one_sample_t`, or `one_sample_wilcoxon`.
  - `two_tailed`: boolean (default `true`). A one-tailed rank test always evaluates the tail in the direction the sample points.
  - `compute_effect_sizes`: boolean (default `true`). The analysis reports Cohen's $d$, Hedges' $g$, and Glass's $\Delta$ with noncentral-*t* CIs, and $R^2$ ($\eta^2$). For `mann_whitney` and `wilcoxon_signed_rank` it reports the rank-biserial correlation and Cliff's $\delta$.
  - `mv_group_column` and `mv_response_column` (Multiple Variables tables), and `hypothetical` (one-sample tests).
- **Reported outputs:**
  - **Summary** (`two-group`, `paired-t`, `mann-whitney`, and similar): the group means or medians; the difference or ratio of means, or the Hodges–Lehmann shift, with its CI; the test statistic ($t, U, W, D_{\text{KS}}, S$); $\text{df}$; and the *P* value.
  - Additional diagnostics: the *F* test for equal variances and the median-centered Levene (Brown–Forsythe) test for unpaired tests; the pairing correlation $r$ and its *P* value for `paired_t` and `ratio_paired_t`; and any requested effect sizes.
  - For `ratio_paired_t`, `one_sample_ratio_t`, `lognormal_unpaired_t`, and `lognormal_welch_t`, CurveBook back-transforms the results automatically and reports geometric mean ratios $\exp(\Delta \bar{y}_{\ln})$ with their CIs.
- **Diagnostic checks:** The checklist reports Shapiro–Wilk normality for each group, the *F* and Levene tests of equal variance, and whether pairing was effective (with a warning if $r \le 0$ or $|r| = 1$). For small $n$ it also reports the coverage actually achieved by the exact Hodges–Lehmann CI. In ratio and lognormal tests it flags values that are not positive.
- **Edge cases and limits:**
  - `unpaired_t`, `welch_t`, and `one_sample_t` accept [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N). With such input the exact *F* test is kept, but the Levene test, which needs raw values, is omitted.
  - Exact Mann–Whitney permutation *P* values run for both untied and tied samples up to $N = n_a + n_b \le 250$ within bounded DP memory (Mann & Whitney 1947; Streitberg & Röhmel 1986 shift algorithm with two-sided symmetric deviation $P(|S - E[S]| \ge |S_{\text{obs}} - E[S]|)$). Above that budget, CurveBook switches to the tie- and 0.5-continuity-corrected normal approximation.
  - Exact Wilcoxon signed-rank distributions run for small zero- and tie-free samples. When $m \cdot n > 40000$, Hodges–Lehmann selection switches to $O((m+n)\log(mn))$. Incomplete paired rows are dropped listwise, and the omission is recorded in the audit notes.
- **References and verification:**
  - References: Student (1908); Welch (1947); Satterthwaite (1946); Mann & Whitney (1947); Streitberg & Röhmel (1986); Wilcoxon (1945); Pratt (1959); Hodges & Lehmann (1963); Smirnov (1948) & Hodges (1958); Dixon & Mood (1946); Cohen (1988), Hedges (1981) & Steiger & Fouladi (1997); Cliff (1993) & Wendt (1972) / Kerby (2014); Levene (1960) & Brown & Forsythe (1974a).
  - Verified against: `R 4.5.2` (`stats::ks.test`, `pt`, `pf`), `SciPy 1.18.1` (`scipy.stats.nct`), and published exact rank benchmarks ([statistics evidence](../evidence/statistics.md)).

---

## Bland–Altman method comparison {#analysis-bland-altman}

This analysis measures agreement between two quantitative assays applied to the same subjects. It plots each difference ($A - B$, or the percentage difference $100 \cdot (A - B) / ((A + B)/2)$) against the pair average $(A + B) / 2$ and reports the mean bias and the limits of agreement, $\text{LoA} = \text{bias} \pm z_{1-\alpha/2} \cdot \text{SD}_{\text{diff}}$. It works with Column, XY, and Multiple Variables tables (see [Analysis Registry: `bland-altman`](registry/tables.html#analysis-kind-bland-altman)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `bland_altman` (the preferred name) or `stats.bland_altman`.
- **Options and defaults:**
  - `column_a`: 0-based index of method *A* (default `0`).
  - `column_b`: 0-based index of method *B* (default `1`).
  - `percent_difference`: boolean. `false` (the default) plots `A - B`; `true` plots the percentage difference.
  - `xy_compare_x_vs_y`: optional boolean for XY tables. `true` compares $X$ with $Y_0$; `false` compares `Y[column_a]` with `Y[column_b]`. When it is unset, CurveBook compares $X$ with $Y_0$ if the table has one $Y$ dataset, and compares two $Y$ datasets otherwise.
- **Reported outputs:**
  - **Summary** (`bland-altman`): the number of pairs $n$; the mean bias with its SE and `conf%` *t*-based CI; and the SD of the differences. It also reports the lower and upper limits of agreement with their Bland–Altman (1999) standard errors $\text{SD}\sqrt{1/n + z^2 / (2(n - 1))}$ and *t*-based CIs, and an OLS slope, intercept, and *P* value that test for proportional bias.
  - **Residuals:** the (average, difference) scatter coordinates for each pair.
- **Diagnostic checks:** The checklist tests the differences for normality (Shapiro–Wilk, for $3 \le n \le 5000$), checks for proportional bias using the *P* value of the slope of difference against mean, and records how many incomplete pairs were filtered out.
- **Edge cases and limits:** With `percent_difference: true`, the analysis fails if any pair has an average $(A + B)/2 = 0$. Incomplete rows are skipped. Pre-summarized tables are rejected, because the method needs matched pairs from individual subjects.
- **References and verification:**
  - References: Bland & Altman (1986, 1999).
  - Verified against: `R 4.5.2` and Bland & Altman (1986) Table 1 ([statistics evidence](../evidence/statistics.md)).

---

## Multiple row-wise *t*-tests and nonparametric tests {#analysis-multiple-row-t-tests}

This analysis runs a separate independent or paired two-group comparison on every row of a table. Typical uses are screening experiments, dose or time tables, and volcano plots. Multiplicity is controlled across all testable rows by an FWER or FDR method. The analysis works with Grouped, XY, Column (as paired blocks of replicate columns), and Multiple Variables tables (see [Analysis Registry: `multiple-row-t-tests`](registry/tables.html#analysis-kind-multiple-row-t-tests)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `multiple_row_t_tests` (the preferred name), `stats.multiple_row_t_tests`, `multiple_row_tests`, or `multiple_t_tests`.
- **Options and defaults:**
  - `paired`: boolean (default `false`). A paired analysis needs matched subcolumns on Grouped and XY tables, or an even number ($\ge 4$) of selected columns on Column tables.
  - `nonparametric`: boolean (default `false`). When `true`, each row is tested with the Mann–Whitney $U$ test or the Wilcoxon signed-rank test.
  - `lognormal_mode`: `none` (default), `lognormal` (unpaired or Welch *t* tests on log-transformed values in each row), or `ratio_paired` (paired *t* tests on log ratios in each row). The lognormal modes require strictly positive data and report back-transformed geometric mean ratios.
  - `assume_equal_sd`: boolean. `true` (the default) uses Student's pooled variance; `false` uses Welch–Satterthwaite. It applies only to unpaired parametric tests.
  - `multiplicity` (`benjamini_hochberg_fdr`, `two_stage_bky_fdr`, `benjamini_yekutieli_fdr`, `holm_sidak`, `bonferroni`, `sidak`, or `none`), `threshold` (default `0.05`), `report_confidence_intervals`, and `control_column`.
- **Reported outputs:**
  - **Summary** and **Multiple Comparisons** (`multiple-row-comparisons`): for each row, $n_A$ and $n_B$, the mean or median difference, $\text{SE}$, the test statistic, the raw *P* value, the adjusted *P* value or FDR *q* value, the volcano coordinate $-\log_{10}(P)$, a `Discovery?` flag, and optionally the CI of the difference.
- **Diagnostic checks:** The checklist reports how many rows were tested and how many were skipped (too few replicates, or zero variance). It also states which error-rate family the chosen method controls (`FWER` or `FDR`).
- **Edge cases and limits:** [Pre-summarized subcolumns](tables.md#subcolumn-modes) are always rejected with `insufficient_data`, because each row needs raw replicates. When a Column table is split into groups, at least 4 columns must be selected, and the count must be even when `paired: true`.
- **References and verification:**
  - References: Benjamini & Hochberg (1995); Benjamini, Krieger & Yekutieli (2006); Holm (1979); Šídák (1967).
  - Verified against: `R 4.5.2` and `statsmodels 0.15.0` (`multipletests(method="fdr_tsbky")`) ([statistics evidence](../evidence/statistics.md)).

---

## Passing–Bablok regression {#analysis-passing-bablok}

This analysis fits the Passing & Bablok (1983) nonparametric errors-in-variables regression line $Y = \alpha + \beta X$. The fit is robust to outliers and allows for measurement error in both *X* and *Y*. It works with Column, XY, and Multiple Variables tables (see [Analysis Registry: `passing-bablok`](registry/tables.html#analysis-kind-passing-bablok)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `passing_bablok` (the preferred name) or `stats.passing_bablok`.
- **Options and defaults:**
  - `x_column`: 0-based index of the reference assay *X* (default `0`).
  - `y_column`: 0-based index of the candidate assay *Y* (default `1`).
- **Reported outputs:**
  - **Summary** and **Parameters and CIs**: The slope $\hat{\beta}$ is the shifted median of the valid pairwise slopes $S_{ij} = (y_i - y_j)/(x_i - x_j) \ne -1$, using the offset $K = \text{count}(S_{ij} < -1)$, with exact rank-based CIs. The intercept is $\hat{\alpha} = \text{median}(y_i - \hat{\beta}x_i)$, also with CIs.
  - Scalars: `Slope`, `Slope CI lower/upper`, `Intercept`, `Intercept CI lower/upper`, `CUSUM H`, `CUSUM statistic`, `CUSUM P value`, `N`, `Valid slopes`, and `K offset`.
  - **Residuals:** fitted values and perpendicular and vertical residuals.
- **Diagnostic checks:** The analysis runs the Passing & Bablok Kolmogorov-type CUSUM test for linearity ($H$, the statistic, and the *P* value). The checklist notes whether the 95% CI of the slope excludes 1 (proportional bias) and whether the CI of the intercept excludes 0 (systematic bias).
- **Edge cases and limits:** At least $n \ge 3$ complete pairs are required. Exact duplicate points $(x_i, y_i) = (x_j, y_j)$ and slopes $S_{ij} = -1$ are dropped symmetrically. The analysis fails if no valid slopes remain or if only vertical ties remain. Pre-summarized XY subcolumns are rejected.
- **References and verification:**
  - References: Passing & Bablok (1983).
  - Verified against: exact rank-order and CUSUM reference values from `Python 3.13` and `R 4.5.2` ([statistics evidence](../evidence/statistics.md)).

---

## One-way ANOVA and nonparametric equivalents {#analysis-one-way-anova}

This analysis compares $k \ge 2$ groups. Groups in independent columns are compared with ordinary ANOVA, Brown–Forsythe and Welch ANOVA, or Kruskal–Wallis. Groups matched by subject rows are compared with repeated-measures ANOVA, a REML mixed model, or Friedman's test. The analysis also offers post-hoc tests, trend tests, and a compact letter display (CLD). It works with Column and Multiple Variables tables (see [Analysis Registry: `one-way-anova`](registry/tables.html#analysis-kind-one-way-anova)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `one_way_anova` (the preferred name), `anova`, `welch_anova`, `brown_forsythe`, `rm_one_way_anova`, `repeated_measures_anova`, `lognormal_one_way_anova`, `lognormal_welch_anova`, `lognormal_rm_anova`, `kruskal_wallis`, or `friedman`.
- **Options and defaults:**
  - `method`: `ordinary_anova` (default), `brown_forsythe_and_welch`, `repeated_measures`, `lognormal_ordinary`, `lognormal_welch_bf` (alias `lognormal_welch_anova`), `lognormal_repeated_measures`, `kruskal_wallis`, or `friedman`.
  - `geisser_greenhouse`: boolean; applies to `repeated_measures` and `lognormal_repeated_measures`.
  - `mixed_effects_reml`: boolean (default `false`). REML is used automatically when repeated-measures subjects have missing cells; setting this option also requests it for complete tables.
  - `df_method`: `kenward_roger` (default) or `satterthwaite`.
  - `residual_covariance`: `compound_symmetry` (default) or `unstructured`. The unstructured model uses the Pinheiro–Bates log-Cholesky REML parameterization and falls back automatically to compound symmetry when it is under-identified.
  - `compute_effect_sizes`: boolean. Reports $\eta^2$, $\omega^2$, and Cohen's $f$ with noncentral-*F* CIs, or $\varepsilon^2$ and Kendall's $W$ for rank tests.
  - `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, `control_column`, and `selected_pairs`.
- **Reported outputs:**
  - **ANOVA Table** (`anova-table`, `unequal-var-anova`, `rm-one-way-anova`, `kruskal-wallis`, `friedman`): treatment, residual, and total SS, df, and MS; the test statistic ($F$, Welch's $W$, Brown–Forsythe $F^*$, Kruskal–Wallis $H$, or Friedman's $\chi^2_F$); and the *P* value.
  - **Diagnostics:** for unpaired ANOVA, Bartlett's test and the Brown–Forsythe (median-centered Levene) test of equal variances; for repeated-measures ANOVA, Mauchly's $W$ and the Greenhouse–Geisser $\hat{\varepsilon}$ and Huynh–Feldt $\tilde{\varepsilon}$ sphericity estimates.
  - **Parameters and CIs** and **Covariance Matrix**: the REML variance components or the estimated unstructured covariance matrix.
  - **Multiple Comparisons:** pairwise differences of means or of ranks, SEs, raw and adjusted *P* or *q* values, simultaneous CIs (omitted for step-down procedures such as `holm_bonferroni`, `holm_sidak`, `newman_keuls`, and `duncan`, for which simultaneous CIs are not defined; Duncan's multiple range test uses protection levels $\alpha_p = 1 - (1 - \alpha)^{p - 1}$ and does not guarantee strong FWER control for $k > 2$), and Piepho (2004) CLD letter groups.
- **Diagnostic checks:** The checklist reports Shapiro–Wilk normality for each group, homogeneity of variance, and sphericity (with a warning when $\hat{\varepsilon} < 0.75$). It also flags REML variance estimates at the zero boundary or the upper search limit, and rows dropped listwise in repeated-measures analyses without REML and in Friedman's test.
- **Edge cases and limits:**
  - `ordinary_anova` and `brown_forsythe_and_welch` accept [pre-summarized subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N). The normality and Levene checks, which need raw values, are then omitted.
  - REML handles unbalanced designs with missing cells automatically. When timepoints are collinear or degenerate, it falls back to `compound_symmetry` and records the reason. Friedman's test drops incomplete rows listwise.
  - Kruskal–Wallis evaluates exact conditional permutation *P* values for small samples ($k = 2$ up to $N \le 250$; $k = 3$ up to $N \le 25$; $k \ge 4$ within bounded meet-in-the-middle splits, covering designs such as $(5,5,5,5)$ and $(3,3,3,3,3)$ with or without ties). Above that budget, it falls back to the tie-corrected $\chi^2(k - 1)$ approximation.
- **References and verification:**
  - References: Zar (2010); Welch (1951); Levene (1960) & Brown & Forsythe (1974a,b); Steiger (2004); Mauchly (1940); Greenhouse & Geisser (1959); Huynh & Feldt (1976); Patterson & Thompson (1971); Pinheiro & Bates (1996); Kenward & Roger (1997); Fai & Cornelius (1996); Kruskal & Wallis (1952); Iman, Quade & Alexander (1975); Streitberg & Röhmel (1986); Baglivo, Olivier & Pagano (1992); Meyer & Seaman (2013); Friedman (1937); Holm (1979); Duncan (1955); Tukey (1949/1953) & Kramer (1956); Newman (1939) & Keuls (1952); Dunnett (1955, 1980); Games & Howell (1976); Tamhane (1979); Dunn (1964); Piepho (2004).
  - Verified against: NIST StRD ANOVA benchmarks and `R 4.5.2` (`stats`, `nlme`, `oneway.test`, `kruskal.test`, `friedman.test`, `p.adjust`, `ptukey`) ([statistics evidence](../evidence/statistics.md)).

---

## Analysis of covariance (ANCOVA) {#analysis-ancova}

One-way / *k*-group analysis of covariance (ANCOVA) compares group means on a continuous outcome while linearly adjusting for one or more continuous covariates. It reports Type III (marginal) or Type I (sequential) sums of squares, grand-mean-centered adjusted group means (estimated marginal means, EMMs) with standard errors and confidence intervals, common covariate slopes, an optional group-by-covariate interaction *F*-test for homogeneity of regression slopes, partial $\eta^2$ and $\omega^2$ effect sizes, and optional covariate-adjusted pairwise group comparisons. It works with Multiple Variables tables (explicit response, categorical grouping factor, and one or more covariate columns) and XY tables (using *X* as the covariate and each *Y* dataset as a group) (see [Analysis Registry: `ancova`](registry/tables.html#analysis-kind-ancova)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `ancova` (the preferred name), `general_ancova`, `one_way_ancova`, `analysis_of_covariance`, or `stats.ancova`.
- **Options and defaults:**
  - `response_column`, `group_column`, and `covariate_columns`: zero-based column indices on Multiple Variables tables.
  - `ss_type`: `type_iii` (default; marginal SS for unbalanced designs) or `type_i` (sequential SS with covariates entered prior to group).
  - `test_homogeneity_of_slopes`: boolean (default `true`). Fits the separate-slope interaction model ($Y \sim \text{Group} + \mathbf{X} + \text{Group}:\mathbf{X}$) and evaluates the omnibus (and per-covariate, when $q > 1$) extra-sum-of-squares *F*-test against the common-slope additive model.
  - `compute_effect_sizes`: boolean (default `true`). Reports partial $\eta^2$ and partial $\omega^2$ for the group factor and every covariate.
  - `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, `control_column`, and `selected_pairs`: covariate-adjusted pairwise contrasts between estimated marginal means using exact covariance $\text{SE}(\hat{\mu}_a^* - \hat{\mu}_b^*) = \sqrt{\text{MS}_{\text{error}} (\mathbf{c}^\top (Z^\top Z)^{-1} \mathbf{c})}$.
- **Reported outputs:**
  - **ANCOVA Table** (`ancova-table`): group, covariate, residual, and total SS, df, MS, *F*, *P*, partial $\eta^2$, and partial $\omega^2$, plus model $R^2$ and adjusted $R^2$.
  - **Adjusted Group Means** (`ancova-adjusted-means`): unadjusted mean, grand-mean-centered adjusted mean (EMM), SE, and confidence interval for each group level.
  - **Covariate Slopes** (`ancova-covariate-slopes`): common slope estimate, SE, *t*, *P*, and confidence interval for each covariate.
  - **Homogeneity of Slopes** (`ancova-homogeneity`): omnibus and per-covariate group-by-covariate interaction *F*-tests.
  - **Multiple Comparisons** (`ancova-posthoc`): covariate-adjusted pairwise contrasts with multiplicity-adjusted *P*/*q* values and CIs (where defined).
  - **Residuals** (`ancova-residuals`): observed *Y*, additive fitted $\hat{Y}$, and residual values for diagnostic plotting.
- **Diagnostic checks:** The assumption checklist warns when the homogeneity-of-slopes interaction test is significant ($p < \alpha$), reports residual Shapiro–Wilk normality, and flags omitted incomplete rows.
- **Edge cases and limits:** Requires at least two non-empty groups, at least one continuous covariate, full rank of the additive design matrix, and residual degrees of freedom $n - k - q \ge 1$. Collinear covariates or zero residual degrees of freedom return a structured error rather than invalid estimates. Pre-summarized XY subcolumns are rejected because ANCOVA requires raw observations.
- **References and verification:**
  - References: Maxwell & Delaney (2004); Kutner, Nachtsheim, Neter, & Li (2005); Searle, Speed, & Milliken (1980).
  - Verified against: `R 4.5.2` (`lm`, `drop1(..., test = "F")`, `anova`, `predict(..., se.fit = TRUE)`, `p.adjust`) for unbalanced 1-covariate and 2-covariate designs ([statistics evidence](../evidence/statistics.md)).

---

## Two-way, three-way, and multifactor ANOVA / REML {#analysis-factorial-anova}

This analysis fits fixed-effects or repeated-measures factorial linear models, and REML mixed models, for two-way designs, three-way $2 \times 2 \times K$ designs (Grouped tables), and $N$-way multifactor designs (Multiple Variables tables). It reports Type I or Type III sums of squares, tests of interactions, and post-hoc tests of simple or main effects. It works with Grouped and Multiple Variables tables (see [Analysis Registry: `factorial-anova`](registry/tables.html#analysis-kind-factorial-anova)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `two_way_anova` (the preferred name), `factorial_anova`, or `three_way_anova`.
- **Options and defaults:**
  - `three_way`: boolean (default `false`). A three-way analysis needs 4 grouped columns that encode the $2 \times 2$ factors, with $K$ rows. The optional `three_way_rm_factors` (`[factor1_rm, factor2_rm, row_rm]`) marks which factors are repeated measures.
  - `multifactor_columns` and `response_column`: the indices of the factor and outcome columns in a Multiple Variables table.
  - `max_interaction_order`: the highest order of interaction to include (for example, `2` or `3`).
  - `ss_type`: `type_iii` (default; marginal SS, suited to unbalanced designs) or `type_i` (sequential SS).
  - `repeated_measures`: `none` (default), `column_factor_matched`, `row_factor_matched`, or `both_factors_matched`. Repeated-measures factors receive Greenhouse–Geisser and Huynh–Feldt sphericity corrections.
  - Mixed effects and diagnostics:
    - Set `mixed_effects_reml` to fit a REML model. In REML fits, `fit_reml_on_missing` defaults to `true`.
    - `reml_zero_variance_policy`: `bound_at_zero` (default), `drop_zero_random_term`, `unbounded`, or `legacy_fallback`.
    - `df_method`: `kenward_roger` (default) or `satterthwaite`.
    - `residual_covariance`: `compound_symmetry` (default) or `unstructured`.
    - The optional `heteroscedasticity_test` adds a tab of Spearman $|r|$ against predicted values for `HomoscedasticityPlot`. `emit_consolidated_data_tab` exports the reshaped data.
  - `compute_effect_sizes`, `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, `control_column`, `selected_pairs`, `control_factor` (`0` = row factor, `1` = column factor for two-way control-versus-all or selected-pairs families), and `comparison_family_mode` (`standard`, `per_slice_families`, `two_factor_families`, `cells_differing_by_one_factor`, `single_grand_family`, or `preselected_pairs`).
- **Reported outputs:**
  - **ANOVA Table** (`factorial-anova`): main effects, interactions, and subject and residual terms, with SS, df, MS, $F$ (or the Kenward–Roger $F^*$), *P*, and partial $\eta^2$ or $\omega^2$.
  - **Diagnostics** (`rm-sphericity`): the Greenhouse–Geisser and Huynh–Feldt $\varepsilon$ for each repeated-measures factor.
  - **Parameters and CIs** and **Covariance Matrix**: REML variance components and the estimated unstructured covariance.
  - **Multiple Comparisons:** simple-effect and main-effect pairwise, control-versus-all, or selected-pairs comparisons (targeting `control_factor` with `control_column` or `selected_pairs` when specified).
- **Diagnostic checks:** The checklist reports normality and variance checks on cells and residuals, sphericity $\varepsilon$ against its thresholds, REML boundary and convergence warnings, empty levels that were dropped, and notes on missing cells.
- **Edge cases and limits:**
  - Pre-summarized subcolumns on Grouped tables are always rejected, because the analysis needs raw observations.
  - Empty or fully excluded levels are removed before Type III coding.
  - Split-plot two-way repeated-measures designs (`row_factor_matched` or `column_factor_matched`) with unequal between-subject group sizes and complete within-subject observations use classical split-plot RM ANOVA. They run separate between- and within-subject error strata with pooled within-group sphericity, upgrading to mixed-effects REML only when within-subject observations are missing (`fit_reml_on_missing: true`) or when REML or unstructured covariance is requested.
  - When the covariance is under-observed or not positive definite, the `unstructured` model falls back to `compound_symmetry` and records the reason.
  - With `fit_reml_on_missing: false`, missing repeated-measures cells cause an error.
- **References and verification:**
  - References: Searle (1987); Maxwell, Delaney, & Kelley (2017); Greenhouse & Geisser (1959); Huynh & Feldt (1976); Lecoutre (1991); Zar (2010); Lindstrom & Bates (1988); Pinheiro & Bates (1996); Kenward & Roger (1997); Fai & Cornelius (1996).
  - Verified against: `R 4.5.2` (`aov`, `lm`, `nlme::lme`, `nlme::gls`) ([statistics evidence](../evidence/statistics.md)).

---

## Nested *t*-test and nested one-way ANOVA {#analysis-nested-comparison}

This analysis fits a hierarchical mixed-effects model that separates the fixed treatment effect from the random variance between biological units (subclusters) and the technical residual variance within them. Modeling the hierarchy in this way prevents pseudoreplication. For two groups it runs a nested *t* test ($t = \sqrt{F}$, carrying the sign of the difference); for $k \ge 3$ groups it runs a nested one-way ANOVA with post-hoc tests. It works only with Nested tables (see [Analysis Registry: `nested-comparison`](registry/tables.html#analysis-kind-nested-comparison)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `nested_anova` (the preferred name), `nested_t`, or `nested`.
- **Options and defaults:**
  - `two_group_t_test`: boolean (default `false`). When `true`, exactly 2 treatment groups must be selected.
  - `df_method`: `kenward_roger` (default) or `satterthwaite`.
  - `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, `control_column`, and `selected_pairs` (for $k \ge 3$).
- **Reported outputs:**
  - **ANOVA Table** (`nested-anova`): SS for groups, for subclusters within groups, and for the technical residual, with df (Kenward–Roger or Satterthwaite), MS, $F$ (or, for `nested_t`, the signed $t$ and its CI), and *P* values.
  - **Parameters and CIs** (`variance-components`): the REML variance components $\hat{\sigma}^2_{\text{subcluster}}$ and $\hat{\sigma}^2_{\text{error}}$, their SDs, each as a percentage of the total variance, $\text{ICC}_{\text{subclusters}}$, and the denominator df by both the Kenward–Roger and Satterthwaite methods.
  - **Multiple Comparisons:** comparisons between groups that account for the hierarchy, using the variance $\hat{\sigma}_u^2 (n_{g1}^2 + \dots + n_{gm}^2) / N_g^2 + \hat{\sigma}_e^2 / N_g$ for each group mean.
- **Diagnostic checks:** The checklist reports the ICC that measures clustering of biological relative to technical variation (it passes when $\text{ICC} > 0.05$) and whether a REML variance estimate reached a boundary (zero or the search limit).
- **Edge cases and limits:** REML models unequal numbers of technical replicates per subcluster, and unequal numbers of subclusters per group, directly rather than pooling them without weights. Non-numeric and excluded replicates are ignored.
- **References and verification:**
  - References: Zar (2010); Satterthwaite (1946); Kenward & Roger (1997).
  - Verified against: `R 4.5.2` (REML, Kenward–Roger, and hierarchical group-mean variance benchmarks) ([statistics evidence](../evidence/statistics.md)).

---

## Contingency, proportions, odds/risk ratios, and agreement {#analysis-contingency}

This analysis evaluates tables of categorical counts. It tests independence, exact association in $2 \times 2$ tables, trends across ordered proportions, and discordance between paired observations. It also measures agreement between raters and tests proportions for goodness of fit. Which methods are available depends on the table type:

- Contingency tables support every method.
- Parts of Whole tables support `pearson_chi_square` and `exact_binomial`.
- Column tables support `exact_binomial` only.

See [Analysis Registry: `contingency`](registry/tables.html#analysis-kind-contingency).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `contingency` (the preferred name), `chi_square`, `fisher_exact`, `barnard_exact`, `stats.barnard_exact`, `cohen_kappa`, `kappa`, `fleiss_kappa`, `exact_binomial`, or `binomial`.
- **Options and defaults:**
  - `method` selects the test:
    - `pearson_chi_square` (with the boolean `yates_continuity` for $2 \times 2$ tables) or `likelihood_ratio_g2`.
    - `fisher_exact`: exact for $2 \times 2$ tables; for $R \times C$ tables, the Freeman–Halton test by network algorithm or by reproducible Monte Carlo.
    - `barnard_exact`: the unconditional $2 \times 2$ test, taking the CSM/Boschloo supremum over $\pi \in (0, 1)$.
    - `cochran_armitage_trend` ($2 \times C$ or $R \times 2$) or `mcnemar_paired` ($2 \times 2$).
    - `cohen_kappa` (square $k \times k$ tables; `weighting` is `unweighted`, `linear`, or `quadratic`) or `fleiss_kappa` ($N \times k$).
    - `exact_binomial` (`null_proportion` $p_0 \in (0, 1)$, default `0.5`).
  - `compute_odds_and_risk_ratios`: the Baptista–Pike CI of the odds ratio, the Koopman score CI of the relative risk, the Newcombe Wilson-score CI of the risk difference, and Altman's NNT.
  - `compute_sensitivity_specificity`: sensitivity, specificity, PPV, NPV, and LR+ with Wilson CIs.
  - `compute_phi_and_cramers_v`: $\phi$ and Cramér's $V$ with noncentral-$\chi^2$ CIs.
  - `expected_proportions`: optional non-negative weights for a goodness-of-fit test on a Parts of Whole table. The weights are normalized internally; when unset, each category is expected with proportion $1/k$.
- **Reported outputs:**
  - **Summary** (`contingency-main`, `parts-gof`), **Parameters and CIs** (`contingency-ratios`, `contingency-mcnemar`, `kappa-agreement`, and others), and **Diagnostics**. Together these report the test statistics ($\chi^2, G^2, z, \kappa, \phi, V$); exact or asymptotic *P* values; the OR, RR, RD, and NNT and the accuracy measures with their CIs; and the Clopper–Pearson, Wilson, and Agresti–Coull binomial intervals.
- **Diagnostic checks:** The checklist applies Cochran's rule for expected counts (flagging expected counts $< 5$ or $< 1$), warns when rows or columns with all-zero margins are removed, checks for a degenerate kappa ($p_e = 1$), and notes sparse cells.
- **Edge cases and limits:**
  - Rows and columns that are entirely zero are removed before the association tests. If the remaining table is smaller than $2 \times 2$, the analysis fails with an error.
  - Non-integer or negative counts fail validation.
  - The Cochran–Armitage test reports no statistic for tables that are not $2 \times C$ or $R \times 2$, or when the variance is zero.
  - When the Koopman search cannot bracket a bound of the RR interval, that bound is reported as open.
- **References and verification:**
  - References: Pearson (1900); Yates (1934); Wilks (1938); Fisher (1935); Freeman & Halton (1951); Barnard (1945/1947) & Suissa & Shuster (1985); Cochran (1954) & Armitage (1955); McNemar (1947); Baptista & Pike (1977); Koopman (1984); Newcombe (1998); Altman (1998); Clopper & Pearson (1934) & Sterne (1954); Wilson (1927); Agresti & Coull (1998); Cohen (1960, 1968); Fleiss (1971); Fleiss, Cohen & Everitt (1969); Fleiss, Nee & Landis (1979).
  - Verified against: `R 4.5.2`, `statsmodels 0.15.0` (`cohens_kappa`), and published exact hypergeometric and binomial benchmarks ([statistics evidence](../evidence/statistics.md)).

---

## Survival analysis (Kaplan–Meier and Cox PH) {#analysis-survival}

Kaplan–Meier estimates time-to-event survival curves and compares cohorts with the log-rank, Gehan–Breslow–Wilcoxon, or trend tests and with hazard ratios. Cox proportional hazards regression, $h(t | X) = h_0(t)\exp(X \beta)$, models several predictors at once and checks the proportional-hazards assumption with Schoenfeld residuals. Table support depends on the method:

- `kaplan_meier` works with Survival tables, and with Multiple Variables tables through `mv_time_column`, `mv_status_column`, and the optional `mv_group_column`.
- `cox_ph` works with Multiple Variables tables through `time_column`, `status_column`, and `predictor_columns`.

See [Analysis Registry: `survival`](registry/tables.html#analysis-kind-survival).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `kaplan_meier` (the preferred name), `survival`, `log_rank`, `cox_ph`, or `cox`.
- **Options and defaults:**
  - Kaplan–Meier:
    - Tests: turn on `log_rank_mantel_cox`, `gehan_breslow_wilcoxon`, or `trend_test` (the Tarone 1975 1-df trend test, for $\ge 3$ cohorts).
    - Confidence bands: `asymmetrical_greenwood_ci` chooses between Kalbfleisch–Prentice $\ln(-\ln S(t))$ bounds, which stay within $[0, 1]$ (the default), and symmetric linear Greenwood bounds.
    - Coding and comparisons: `event_code`, `censored_code`, `treat_other_as`, `multiplicity`, `threshold` (default `0.05`), `report_confidence_intervals`, `control_column`, and `selected_pairs`.
    - On Multiple Variables tables, set `mv_time_column`, `mv_status_column`, and `mv_group_column`.
  - Cox PH:
    - Model: `time_column`, `status_column`, `predictor_columns`, `reference_levels`, and `formula` (interaction and transform terms).
    - Fitting: `tie_method` (`efron` by default, `breslow`, or `exact`) and `ci_method` (Wald or profile-likelihood CIs).
    - Extras: the optional `nested_comparison` (a likelihood-ratio test against a nested Cox model), `covariate_profiles` (survival curves for user-defined covariate profiles), and `emit_cox_snell_residuals`.
    - Coding: `event_code`, `censored_code`, and `treat_other_as`.
- **Reported outputs:**
  - Kaplan–Meier:
    - **Summary** (`km-summary`): median survival with Brookmeyer–Crowley (1982) CIs, the ratio of median survival times, the median follow-up time by reverse Kaplan–Meier, and the numbers of subjects, events, and censored observations.
    - **Number at Risk**.
    - **Model Comparison**: $\chi^2$, $\text{df}$, and $P$ for the log-rank, Gehan–Breslow–Wilcoxon, and trend tests.
    - **Parameters and CIs**: Mantel–Haenszel and O/E hazard ratios.
    - **Multiple Comparisons** (`survival-pairwise`, for $\ge 3$ cohorts), and step-curve overlays.
  - Cox PH:
    - **Parameters and CIs** (`cox-ph`): $\hat{\beta}_j$, SE, Wald $z$ and $P$, the hazard ratio $\exp(\hat{\beta}_j)$ with its delta-method SE and its Wald or profile-likelihood CI, and the likelihood-ratio omnibus $\chi^2$.
    - **Covariance Matrix** (`cox-covariance`, with the parameter correlation matrix).
    - **Baseline Survival**: $H_0(t)$ and $S_0(t)$ evaluated at the covariate means.
    - **Covariate Profile Survival** (`cox-profile-survival`, with `cox_profile:{idx}` overlays).
    - **Model Comparison** (the nested likelihood-ratio test) and **Log-Minus-Log** ($\ln(-\ln \hat{S}(t))$ diagnostic curves).
    - **Residuals**: Schoenfeld, martingale, and deviance residuals, `cox-covariate-residuals`, and `cox-snell-residuals`, together with Harrell's $C$-index and the Grambsch–Therneau tests of proportional hazards based on Schoenfeld residuals.
- **Diagnostic checks:** The checklist reports the Schoenfeld proportional-hazards *P* values, low event counts, and Harrell's $C$-index. For Cox PH it warns about monotone likelihood (separation) and collinearity. In those cases the unstable Wald SEs and CIs are suppressed, but the omnibus likelihood-ratio *P* value is kept.
- **Edge cases and limits:**
  - Negative times and invalid censoring codes fail validation.
  - When either arm has no events, the O/E and Mantel–Haenszel hazard ratios cannot be estimated. When $S(t)$ never falls below the required level, the upper Brookmeyer–Crowley bound of the median stays open.
  - Under monotone separation, the Cox fit stops and reports a non-converged status instead of returning diverging estimates.
- **References and verification:**
  - References: Kaplan & Meier (1958); Greenwood (1926); Kalbfleisch & Prentice (2002); Brookmeyer & Crowley (1982); Mantel (1966) & Mantel–Haenszel (1959); Greenland & Robins (1985); Peto & Peto (1972); Gehan (1965); Breslow (1970, 1974); Tarone (1975); Berry, Kitchin & Mock (1991); Cox (1972); Efron (1977); Schoenfeld (1982); Grambsch & Therneau (1994); Harrell et al. (1982).
  - Verified against: `R 4.5.2` (`survival 3.8.3`, `coxph`) ([statistics evidence](../evidence/statistics.md)).

---

## ROC curve analysis {#analysis-roc-curve}

This analysis builds an empirical receiver operating characteristic (ROC) curve from a column of negative controls and a column of positive cases. It reports the trapezoidal AUC with its SE, CI, and *P* value against $\text{AUC} = 0.5$, the cutoff that maximizes Youden's $J$, and a full table of thresholds. It works with Column and Multiple Variables tables (see [Analysis Registry: `roc-curve`](registry/tables.html#analysis-kind-roc-curve)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `roc`.
- **Options and defaults:**
  - `ci_method`: `de_long` (default; the nonparametric structural-components variance of DeLong et al. 1988) or `hanley_mc_neil` (the exponential approximation of Hanley & McNeil 1982).
- **Reported outputs:**
  - **Summary** (`roc-summary`): AUC, SE, the `conf%` CI of the AUC, the *P* value for $\text{AUC} = 0.5$, the sample sizes $n_{\text{controls}}$ and $n_{\text{cases}}$, and the optimal cutoff with its sensitivity, specificity, and maximum Youden's $J = \text{Sens} + \text{Spec} - 1$.
  - **Curve Coordinates** (`roc-coordinates`): a threshold table (Cutoff, Sensitivity, Specificity, 1 - Specificity, Youden J, PPV, NPV, LR+) and the curve points for plotting.
- **Diagnostic checks:** The checklist flags $\text{AUC} < 0.5$; the analysis assumes that higher values indicate cases. It warns when $n_{\text{controls}} < 2$ or $n_{\text{cases}} < 2$. In that situation CurveBook replaces the DeLong variance with the Hanley–McNeil SE, a Wilson score CI, and an exact Mann–Whitney $U$ *P* value.
- **Edge cases and limits:** Ties at a threshold are handled with midpoint ranks. LR+ is left blank when $\text{Specificity} = 1$. Both the controls and the cases need at least one finite value.
- **References and verification:**
  - References: DeLong, DeLong & Clarke-Pearson (1988); Hanley & McNeil (1982); Youden (1950).
  - Verified against: `R 4.5.2` and published DeLong and Hanley–McNeil benchmarks ([statistics evidence](../evidence/statistics.md)).

---

## Generalized linear models (linear, logistic, Poisson) {#analysis-generalized-linear-model}

This analysis fits multivariable generalized linear models from three families: Gaussian with identity link (`linear`), binomial with logit link (`logistic`), and Poisson with log link (`poisson`). Categorical predictors are dummy-coded. The analysis can also report VIF collinearity diagnostics, residuals, and goodness-of-fit statistics. It works with Multiple Variables, XY, and Column tables (see [Analysis Registry: `generalized-linear-model`](registry/tables.html#analysis-kind-generalized-linear-model)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `glm` (the preferred name), `linear_glm`, `multiple_linear`, `logistic_glm`, `multiple_logistic`, `poisson_glm`, or `multiple_poisson`.
- **Options and defaults:**
  - `family`: `linear` (default), `logistic`, or `poisson`.
  - `response_column`: 0-based index of the dependent variable.
  - `predictor_columns`: ordered list of the indices of covariates and categorical predictors.
  - `reference_levels`: list of `(categorical_col_idx, reference_level_label)` pairs that override the reference level used in treatment coding.
  - `formula`: optional two- and three-way interactions, polynomial terms (`X^2`, `X^3`), and transformed terms (`sqrt(X)`, `ln(X)`). `include_intercept` defaults to `true`.
  - `nested_comparison`: an optional formula for a reduced model. CurveBook compares it with the full model by an extra-sum-of-squares *F* test (linear) or a deviance likelihood-ratio test (logistic and Poisson), and by $\Delta\text{AIC}_c$.
  - Weighting, covariance, and diagnostics:
    - `weights`: `None` (default), an analytical `Scheme`, or an explicit `Column` index for weighted least squares in linear models.
    - `dispersion_mode`: `canonical` (default), `quasi_poisson`, or `robust_sandwich`.
    - `ci_method`: Wald or profile-likelihood CIs.
    - `emit_influence_diagnostics` (leverage, Cook's $D$, DFBETAs), `compute_vif` (default `true` for multivariable models), and `classification_cutoff` (the probability cutoff for logistic models, default `0.5`).
- **Reported outputs:**
  - `linear`:
    - Across **Summary**, **Parameters and CIs**, and **Diagnostics**: coefficient estimates, SEs, *t*, *P*, `conf%` *t*-based CIs, VIF, and Type III partial SS, *F*, and *P*.
    - Fit statistics: $R^2$, adjusted $R^2$, $S_{y.x}$, the omnibus *F* and *P*, AIC, AICc, BIC, the Durbin–Watson statistic, and the parameter correlation matrix (`glm-linear-correlation`).
    - Studentized residuals, leverage, Cook's distance, and DFBETAs (`glm-linear-diagnostics`).
    - Predictions with CIs and PIs, computed automatically for rows whose $Y$ is blank.
  - `logistic`:
    - Log-odds coefficients $\hat{\beta}_j$, SE, Wald $z$ and $P$, and odds ratios $\exp(\hat{\beta}_j)$ with delta-method SEs and Wald or profile-likelihood CIs.
    - Fit statistics: the likelihood-ratio $\chi^2$ and $P$, McFadden, Nagelkerke, and Tjur pseudo-$R^2$, AIC, AICc, and the Hosmer–Lemeshow $\chi^2$, *P*, and calibration bins (`glm-logistic-calibration`).
    - The ROC curve (`glm-logistic-roc`) and AUC, the parameter correlation matrix (`glm-logistic-correlation`), influence diagnostics (`glm-logistic-diagnostics`), predicted probabilities for new rows, and a classification table at `classification_cutoff`.
  - `poisson`:
    - Log-rate coefficients $\hat{\beta}_j$, SE, Wald $z$ and $P$, and incidence rate ratios $\exp(\hat{\beta}_j)$ with delta-method SEs and Wald or profile-likelihood CIs (optionally scaled for quasi-Poisson dispersion).
    - Fit statistics: residual deviance, the Pearson dispersion $\chi^2 / \text{df}$, the likelihood-ratio $\chi^2$ and $P$, McFadden pseudo-$R^2$, AIC, and AICc.
    - The parameter correlation matrix (`glm-poisson-correlation`), influence and deviance diagnostics (`glm-poisson-diagnostics`), and predicted rates for new rows.
- **Diagnostic checks:** The checklist flags rank-deficient or aliased (collinear) columns found by SVD, high multicollinearity (`VIF > 5` and `VIF > 10`), and non-normal residuals in Gaussian models (Shapiro–Wilk). It also reports Albert–Anderson separation during logistic IRLS, overdispersion or convergence problems in Poisson models, and rows omitted because they were incomplete.
- **Edge cases and limits:**
  - `logistic` fails if an outcome is not 0 or 1; `poisson` fails if an outcome is negative or not a count.
  - If a categorical predictor has fewer than 2 levels left after incomplete rows are removed, the analysis raises a zero-variance error.
  - The model needs at least $n \ge p + 1$ complete rows (and at least 3 rows on XY tables). Pre-summarized subcolumns are rejected. Under separation, the fit stops and reports it instead of returning diverging estimates.
- **References and verification:**
  - References: Golub (1965) & Björck (1967) QR/SVD; McCullagh & Nelder (1989); Albert & Anderson (1984); Hosmer & Lemeshow (1980).
  - Verified against: NIST StRD Linear Least Squares and `R 4.5.2` (`stats::lm`, `stats::glm`) ([statistics evidence](../evidence/statistics.md)).

---

## Multivariate analysis (PCA, PCR, correlation, clustering, MICE) {#analysis-multivariate}

This analysis groups several multivariate methods: exploratory principal component analysis (PCA), correlation matrices, clustering, supervised principal component regression (PCR), and multiple imputation. It works with Multiple Variables, XY, and Column tables (see [Analysis Registry: `multivariate`](registry/tables.html#analysis-kind-multivariate)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `pca` (the preferred name), `principal_components`, `correlation`, `correlation_matrix`, `kendall`, `kendall_tau_b`, `kendall_correlation`, `clustering`, `kmeans`, `multiple_imputation`, `stats.multiple_imputation`, or `mice`.
- **Options and defaults:**
  - PCA and PCR:
    - `standardize`: `true` for PCA on the correlation matrix, `false` for PCA on the covariance matrix.
    - `retention_rule`: the Kaiser criterion ($\lambda > 1$), Horn (1965) parallel analysis (seeded by `parallel_analysis_seed`), a cumulative-variance threshold, or a fixed number of components.
    - Provide `pcr_outcome_column` to run supervised PCR.
  - Correlation matrix: `method` selects `pearson` (default; Pearson $r$), `spearman` (Spearman rank $\rho$), or `kendall_tau_b` (alias `kendall`; tie-adjusted Kendall's $\tau_b$ with tie-corrected normal-approximation *P* values). The legacy boolean `spearman_rank` remains supported for backward compatibility.
  - Clustering:
    - `distance`: `euclidean`, `manhattan`, `cosine`, or `pearson_correlation_dissimilarity`.
    - `linkage`: `single`, `complete`, `average` (UPGMA), `weighted` (WPGMA), `centroid` (UPGMC), `median` (WPGMC), or `ward_d2` (the Murtagh–Legendre Ward criterion).
    - `cluster_direction` (rows or columns), `cut_height`, and `emit_anova_tab`.
    - For partitioning, set `k_means_k` ($k \ge 2$, initialized with `k-means++`) and `kmeans_algorithm` (`lloyd`, `hartigan_wong`, or `macqueen`).
    - `k_sweep_max` and `consensus_optimal_k` choose $k$ by majority consensus over 17 indices, in the style of NbClust, and by the Tibshirani–Walther–Hastie gap statistic (`gap-statistic`).
  - Multiple imputation: `imputations_m` (2 to 50), `max_iters` (1 to 100), `seed`, `response_column`, and `predictor_columns`. CurveBook imputes by MICE with predictive mean matching and pools the estimates using Rubin's (1987) rules with Barnard–Rubin (1999) degrees of freedom.
- **Reported outputs:**
  - PCA and PCR: **Summary** (`pca-eigenvalues`; `pcr-summary` with $R^2$, adjusted $R^2$, *F*, RMSE, and AIC/BIC), **Parameters and CIs** (`pca-loadings`, `pcr-predictors`, `pcr-components`), score and biplot coordinates, and **Diagnostics** (`pcr-diagnostics`, with leverage and Cook's $D$).
  - Correlation matrix: symmetric matrices of $r$, unadjusted *P* values, Benjamini–Hochberg *q* values adjusted across the off-diagonal pairs, and the pairwise sample counts $n_{ij}$.
  - Clustering: **Summary** and **Parameters and CIs** report cluster centroids, cluster assignments, the within-cluster sum of squares (WCSS), and the mean silhouette. **Diagnostics** reports the linkage merge schedule, the silhouettes at each cut, and the cophenetic $r$. **Covariance Matrix** holds the full distance and cophenetic matrices when $n$ is small enough to display.
  - Multiple imputation: pooled means and, optionally, pooled OLS coefficients with pooled SE, *t*, *P*, `conf%` Barnard–Rubin CIs, and the fraction of missing information (FMI).
- **Diagnostic checks:** The checklist notes when the full matrices are omitted for a large clustering table (the cophenetic $r$ and the linkage tree are kept). It also reports cluster separation quality (`silhouette >= 0.25`), how many components the Kaiser or parallel-analysis rule retained, and whether the imputation converged.
- **Edge cases and limits:** Correlation matrices use pairwise complete cases. A pair with $n < 3$ or zero variance gets `NaN` and is left out of the Benjamini–Hochberg family. PCA, clustering, and MICE need complete, finite cases and reject pre-summarized subcolumns.
- **References and verification:**
  - References: Hotelling (1933) & Jolliffe (2002); Horn (1965); Pearson (1896); Fisher (1921); Spearman (1904); Kendall (1938, 1945); Lloyd (1982); Arthur & Vassilvitskii (2007); Rousseeuw (1987); Lance & Williams (1967); Ward (1963); Murtagh & Legendre (2014); Rubin (1987); Barnard & Rubin (1999); van Buuren & Groothuis-Oudshoorn (2011).
  - Verified against: `R 4.5.2` (`prcomp`, `hclust`, and `pt`/`qt` for Rubin pooling) ([statistics evidence](../evidence/statistics.md)).

---

## Simple and Deming linear regression {#analysis-linear-regression}

Simple linear regression fits the ordinary least-squares line $Y = \beta_0 + \beta_1 X$. With two or more datasets, it compares slopes and elevations by ANCOVA. When both *X* and *Y* carry measurement error, switch to Deming (Model II) regression. The analysis works with XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `linear-regression`](registry/tables.html#analysis-kind-linear-regression)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `fit.run` with `linear` (the preferred name), `simple_linear`, `linear_regression`, `deming`, or `deming_regression`.
- **Options and defaults:**
  - `deming_model_ii`: boolean (default `false`). `deming_error` sets the Model II error-variance model: `EqualError` (orthogonal regression, $\lambda = 1.0$), `{"StandardDeviations": {"sd_x": ..., "sd_y": ...}}`, or `{"VarianceRatio": {"lambda": ...}}`. Deming SEs come from the leave-one-out jackknife.
  - `force_through_origin`: boolean (default `false`). When `true`, $\beta_0 = 0$ and $\text{df} = n - 1$. The more general `fixed_point` takes an optional `[x0, y0]` and constrains the line to $Y - y_0 = \beta_1 (X - x_0)$.
  - `standard_addition`: boolean (default `false`). When `true`, the analysis reports the unknown concentration $-\hat{X}_{\text{int}}$ with negated Fieller limits.
  - `extend_line_to_x_intercept`: boolean (default `false`). When `true`, the plotted regression line is extended to $Y = 0$.
  - `confidence_bands` and `prediction_bands`: booleans that add Working–Hotelling or Student-*t* bands to the curve.
  - `interpolate_unknowns`: boolean that turns on forward prediction ($X$ to $Y$) and exact Fieller inverse calibration ($Y$ to $X$).
  - `x_range_filter`: an optional inclusive `[x_min, x_max]` filter on the predictor. Related plot settings are `plot_x_min`, `plot_x_max`, and `plot_segments` (default `100`, maximum `10,000`).
- **Reported outputs:**
  - **Summary:** slope, intercept, $1/\text{slope}$, $X$-intercept, their SEs and `conf%` CIs, $R^2$, Pearson $r$, $S_{y.x}$, $\text{RSS}$, $F$, *P*, a Wald–Wolfowitz runs test along $X$, and a lack-of-fit *F* test based on replicates.
  - **Model Comparison:** ANCOVA tests of equal slopes and of equal elevations across datasets.
  - **Interpolated Unknowns:** `Interpolated X` and `Interpolated Y` predictions, and the curve bands.
- **Diagnostic checks:** The checklist notes the error-variance ratio ($\lambda$) assumed by Deming regression, nonlinearity detected by the runs test, the replicate lack-of-fit result, and points excluded by the range filter.
- **Edge cases and limits:**
  - The analysis fails with insufficient data when $n \le 2$, or when $n \le 1$ if the line is forced through the origin.
  - It rejects non-finite points, zero variance in $X$, and pre-summarized inputs, because it needs raw pairs.
  - When `slope == 0` or the Deming jackknife variance collapses, the $X$-intercept and $1/\text{slope}$ are omitted.
- **References and verification:**
  - References: Deming (1943); Linnet (1990); Fieller (1954); Zar (2010) ANCOVA.
  - Verified against: NIST StRD benchmarks and `R 4.5.2` (`stats::lm`) ([curve-fitting evidence](../evidence/curve-fitting.md)).

---

## Nonlinear regression and custom equations {#analysis-nonlinear-regression}

This analysis fits built-in models ([Parametric Equations Registry](registry/equations.html)) or custom equations, which run in a sandbox. Custom equations can be algebraic, piecewise (`<A>`/`<B>`), implicit, or first-order ODEs. The fitting method can be trust-region Levenberg–Marquardt, IRLS on the Poisson deviance, robust Cauchy regression, or ROUT outlier elimination. The analysis supports shared parameters, parameters linked from other results, profile-likelihood CIs, model comparison, and interpolation of unknowns. It works with XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `nonlinear-regression`](registry/tables.html#analysis-kind-nonlinear-regression) and the [Curve fitting reference](fitting.md)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `fit.run` with `nonlinear` (the preferred name), `nonlinear_regression`, `curve_fit`, `fit`, `custom_equation`, or `custom_dsl`. Any built-in equation ID can also be passed directly as `kind` (for example `log_agonist_vs_response_4pl` or `michaelis_menten`).
- **Options and defaults:**
  - Model:
    - `equation_id` names a built-in model, a user equation saved with `fit.saveUserEquation` or `fit.cloneBuiltinEquation`, or `"plot_distribution"`. With `"plot_distribution"`, the `plot_distribution` option selects a theoretical probability distribution to plot.
    - Alternatively, `custom_dsl` supplies an equation with `[Equation]`, `[InitialValues]`, `[Constraints]`, and `[Transforms]` sections.
  - Objective and weights:
    - `method`: `least_squares` (default), `poisson_deviance`, `robust_cauchy`, or `rout_outlier_elimination`.
    - `outlier_handling` (`None`, `IdentifyAndCountOnly`, or `Eliminate`), `emit_cleaned_data_tab`, and `fdr_q_percent` (default `1.0`, limited to $(0, 50]$).
    - `weighting`: `none` (default), `relative_square`, `relative`, `general_power_y` (with `weighting_power`), `inverse_x`, `inverse_x2`, or `observed_sd`.
    - `weighting_source`: `ObservedY`, or `PredictedY`, which weights by the height of the fitted curve using IRLS.
  - Constraints and solver:
    - `parameter_rules` sets each parameter's `scope` (`local`, `shared_global`, `constant`, `hooked_constant`, `per_dataset_constant`, or `column_title_constant`) together with `initial_override`, `lower_bound`, and `upper_bound`.
    - Relational inequality constraints go in the `[Constraints]` section (for example `KFast > KSlow`).
    - `replicate_fit_mode`: `IndividualReplicates` or `RowMeansOnly`.
    - `convergence_strictness`: `Default`, `Quick`, `Medium`, or `Strict`.
    - `max_iters`: default `1000`, with a hard cap of `50,000`.
    - `mv_y_columns` fits several responses from a Multiple Variables table.
    - Range and plotting: `x_range_filter`, `y_range_filter`, `plot_function_only`, `plot_x_min`, and `plot_x_max`.
  - Uncertainty, output, and diagnostics:
    - `profile_likelihood_ci` (default `true`), `show_standard_errors`, `identify_unstable_parameters`, and `identifiability_mode` (`IdentifyUnstableParameters`, `IdentifyAmbiguousFits`, or `ReportAll`).
    - Quality alerts and summaries: `flag_r_squared_below`, `fit_flag_rules`, `summary_parameter`, `create_summary_graph`, and `output_significant_digits`.
    - Optional outputs and tests: `confidence_bands`, `prediction_bands`, `interpolate_unknowns`, `replicates_lack_of_fit_test`, `normality_of_residuals`, `homoscedasticity_test`, `emit_residuals_tab`, and `emit_covariance_tab`.
  - Model comparison: `comparison` (`extra_sum_of_squares_f`, `aicc`, or `likelihood_ratio_test`), `comparison_question` (`AlternativeEquation`, `ParameterInvarianceAcrossDatasets`, `SingleGlobalCurveVsSeparate`, or `ParameterEqualsTheoretical`), `comparison_method`, `prefer_well_defined_fit`, `alternative_equation_id`, and `shared_parameter_comparison`.
- **Reported outputs:**
  - **Parameters and CIs:** estimates, SEs, Venzon–Moolgavkar profile-likelihood or Wald CIs (including open bounds), the dependency $R^2_j$, Hougaard's $|g_1|$, and notes on each parameter. Derived quantities (`EC50`, `KB`, `t_half`, `Span`, and analytical inflection points) are reported with SEs and CIs propagated by monotone transformation or by the delta method.
  - **Summary:** $N$, $K_{\text{free}}$, df, unweighted $R^2$, adjusted $R^2$, the Willett–Singer weighted $R^2$, `rss`, `weighted_ss`, `RMSE`, and $S_{y.x}$ (or `RSDR`). Under `poisson_deviance` it adds the Poisson negative log-likelihood, deviance, pseudo-$R^2$, and dispersion. It also reports $\text{AIC}_c$, $\text{BIC}$, the runs test, the replicate lack-of-fit test, residual normality tests, and the Spearman test of homoscedasticity.
  - Additional tabs: **Residuals**, **Covariance Matrix**, **Model Comparison**, **Interpolated Unknowns** (`Interpolated Mean X` and `Interpolated Replicates`, with `Extrapolated` flags), and **Diagnostics** (`fit_auc`; see [Uncertainty and diagnostics](fitting.md#uncertainty-and-diagnostics)).
- **Diagnostic checks:** Under `robust_cauchy`, the parametric covariance, CIs, and bands are deliberately omitted. A fit is flagged as ambiguous or unstable when the Jacobian is rank-deficient, when a dependency exceeds `0.9999`, or when a profile-likelihood bound reaches a plateau. The checklist also warns when `Dependency > 0.99`, when $|g_1| > 1.0$, and when $R^2$ falls below the chosen threshold.
- **Edge cases and limits:** On [pre-summarized XY subcolumns](tables.md#subcolumn-modes) (Mean/SD/N, Mean/SEM/N, Mean/%CV/N), the fit uses the exact sufficient statistics. Custom equations have strict limits: `4,096` AST nodes and `50,000` ODE steps per point (see [Custom DSL and ODE caps](fitting.md#custom-dsl-and-ode)).
- **References and verification:**
  - References: Bates & Watts (1988); Seber & Wild (1989); Motulsky & Christopoulos (2004); Venzon & Moolgavkar (1988); Motulsky & Brown (2006); Moré (1978); Hairer, Nørsett & Wanner (1993).
  - Verified against: NIST StRD Nonlinear Least Squares, `R 4.5.2` (`stats::nls`, `stats::lm`), and `mpmath 1.3.0` (80- and 120-digit references) ([curve-fitting evidence](../evidence/curve-fitting.md); see [Curve fitting verification](fitting.md#oracle-verification)).

---

## Analyze a stack of *P* values {#analysis-p-value-stack}

This analysis treats one or more numeric columns of raw *P* values $p \in [0.0, 1.0]$ from earlier hypothesis tests as families of tests. For each family it computes FWER-adjusted *P* values or FDR *q* values, ranks, and discovery flags, and it reports them in the original row order. It works with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `p-value-stack`](registry/tables.html#analysis-kind-p-value-stack)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.pvalueStack` with `pvalue_stack` (the preferred name), `stats.pvalue_stack`, `p_value_stack`, `pvalues`, `pvaluestack`, or `stats.pvaluestack`.
- **Options and defaults:**
  - `method`: `benjamini_hochberg_fdr` (default), `bonferroni`, `holm_bonferroni`, `sidak`, `holm_sidak`, `benjamini_krieger_yekutieli_fdr`, or `benjamini_yekutieli_fdr`.
  - `threshold`: the error-rate threshold ($\alpha$ for FWER methods, $Q$ for FDR methods), strictly inside $(0.0, 1.0)$ (default `0.05`).
  - `column`: an optional 0-based column index, used when `selected_data_sets` is omitted.
- **Reported outputs:**
  - **Summary** (`pvalue-stack-summary`): the method, the error-rate family (`FWER` or `FDR`), the threshold, the number of valid *P* values $m$, the numbers of discoveries and non-discoveries, the fraction of discoveries, the number of skipped cells, the smallest raw *P* value, and the smallest adjusted *P* or *q* value. Scalars that other analyses can link to are `InputCount`, `Discoveries`, `MinRawP`, `MinAdjustedP`, `Threshold`, `TotalInputCount`, and `TotalDiscoveries`.
  - **Multiple Comparisons** (`pvalue-stack-adjusted`): for each value, the row label, the 1-based row index, the competition rank, the raw *P* value, the adjusted *P* or *q* value, and `Discovery?`.
- **Diagnostic checks:** The checklist states whether the chosen method assumes independent tests (`sidak`, `holm_sidak`, BH, BKY) or allows arbitrary dependence (`bonferroni`, `holm_bonferroni`, BY). It also lists text cells that were ignored.
- **Edge cases and limits:** The analysis rejects values outside $[0.0, 1.0]$, non-finite numbers, and pre-summarized subcolumns, and it requires `threshold` to lie strictly inside $(0.0, 1.0)$. Blank and excluded cells are skipped. Exact ties keep equal ranks, and the adjusted values stay monotone.
- **References and verification:**
  - References: Bonferroni (1936) & Dunn (1961); Holm (1979); Šídák (1967); Holland & Copenhaver (1987); Benjamini & Hochberg (1995); Benjamini & Yekutieli (2001); Benjamini, Krieger & Yekutieli (2006).
  - Verified against: `R 4.5.2` (`p.adjust`) and `statsmodels 0.15.0` (`multipletests(method="fdr_tsbky")`) ([statistics evidence](../evidence/statistics.md)).

---

## Statistical power and sample size {#analysis-power-analysis}

Power analysis plans sample sizes ($n_1$, $n_2$, $N$) before a study, computes the power ($1 - \beta$) of a given design, or finds the smallest detectable effect. It also produces sensitivity grids and curves of power against $N$ for each supported design. It works with every [table archetype](tables.md#table-archetypes), and it can take scalar inputs linked from an info sheet (see [Analysis Registry: `power-analysis`](registry/tables.html#analysis-kind-power-analysis)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `power` (the preferred name), `power_analysis`, `stats.power`, or `power_sample_size`.
- **Options and defaults:**
  - `family`: `two_sample_unpaired_t` (default), `two_sample_paired_t`, `one_sample_t`, `one_way_anova`, `two_proportions`, `correlation`, `chi_square`, `simple_linear_regression`, or `log_rank_survival` (which uses the Schoenfeld 1983 and Freedman 1982 equations for survival sample size and power).
  - `solve_target`: `sample_size` (default), `power`, or `detectable_effect`.
  - `tail_mode`: `two_sided`, `one_sided_greater`, or `one_sided_less`. One-sided tests are not allowed for `one_way_anova` and `chi_square`, which have no direction.
  - `alpha` lies in $(0, 1)$ (default `0.05`). `target_power` lies in $(0, 1)$ and is used when solving for `sample_size` or `detectable_effect`. Further options are `sample_size`, `sample_size_group2`, and `allocation_ratio` ($k = n_2 / n_1 > 0$).
  - `effect`: either a standardized effect (`d`, `d_z`, `f`, `h`, `w`, `r`, `r_squared`, or `f_squared`) or raw inputs (means and SD; proportions $p_1$ and $p_2$; or a regression slope with $\sigma_x$ and $\sigma_\varepsilon$).
  - Options for particular families:
    - `num_groups` ($\ge 2$, for ANOVA) and `chi_square_df`.
    - `two_proportions_mode`: `PooledNullUnpooledAlt`, `Unpooled`, or `CohensH`, with `continuity_correction`.
    - `correlation_method`: `FisherZ`, `FisherZWithTCrit`, `ExactTTransform`, or `ConditionalSlopeT`.
    - The optional `info_sheet_id`.
- **Reported outputs:**
  - **Summary** (`power-summary`): the family, solve target, tail mode, $\alpha$, target power, the standardized and raw effects, the integer sample sizes $n_1$, $n_2$, and $N_{\text{total}}$, and `achieved_power`. It also reports `continuous_n`, `power_at_continuous_n`, the denominator df, the critical value, the noncentrality $\lambda$, and implied quantities ($p_2$, Cohen's $h$, $R^2$).
  - **Multiple Comparisons** (`power-sensitivity`): a sensitivity grid over $(\alpha, 1 - \beta, \text{effect})$.
  - **Curve Coordinates** (`power-curve`): coordinates of power against $N$, a curve overlay, and a table that other analyses can use as input.
- **Diagnostic checks:** The checklist notes how much rounding `continuous_n` up to the integer `n_per_group` raises the power, whether the result rests on an exact or an asymptotic distribution, and any effect size outside its valid range.
- **Edge cases and limits:** The calculations use noncentral $t$, $F$, and $\chi^2$ distribution functions that stay accurate in the tails, with Brent–Dekker root finding. Solving for sample size fails when the effect is zero or when $p_1 = p_2$. `r_squared` must lie in $[0, 1)$, and proportions must lie in $[0, 1]$.
- **References and verification:**
  - References: Cohen (1988); Owen (1965) noncentral-$t$; Fleiss, Tytun & Ury (1980) & Fleiss, Levin & Paik (2003); Chow, Shao & Wang (2008); Fisher (1921).
  - Verified against: `R 4.5.2` (`pwr` package) and exact noncentral distributions ([statistics evidence](../evidence/statistics.md)).

---

## Simple logistic regression {#analysis-simple-logistic-regression}

This analysis fits a binary logistic regression with one predictor, $\ln\!\left(\frac{P(Y=1 | X)}{1 - P(Y=1 | X)}\right) = \beta_0 + \beta_1 X$, to an XY table with a single $Y$ subcolumn and binary outcomes $Y_i \in \{0, 1\}$. See [Analysis Registry: `simple-logistic-regression`](registry/tables.html#analysis-kind-simple-logistic-regression).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `simple_logistic` (the preferred name), `simple_logistic_regression`, `stats.simple_logistic`, or `xy_logistic`.
- **Options and defaults:**
  - `emit_row_classification`: boolean (default `true`). Adds a table of the observed $(X_i, Y_i)$ and the predicted $\hat{P}(X_i)$ with CIs. Rows that have an $X$ value but a blank $Y$ (`(X_new, blank Y)`) receive predictions automatically.
  - `confidence_bands`: boolean (default `true`). Adds two-sided confidence bands, back-transformed through the inverse logit, to the sigmoid curve.
  - `classification_cutoff`: a threshold in $(0, 1)$ (default `0.5`).
  - `plot_x_min`, `plot_x_max`, and `plot_segments` (default `100`, maximum `10,000`).
- **Reported outputs:**
  - **Parameters and CIs** (`parameter-estimates`): $\hat{\beta}_0$, $\hat{\beta}_1$, the odds ratios $e^{\hat{\beta}_0}$ and $e^{\hat{\beta}_1}$, and $X_{50} = -\hat{\beta}_0 / \hat{\beta}_1$, each with an asymptotic SE, an asymmetric profile-likelihood CI, and a Wald $Z$ test.
  - **Summary** (`goodness-of-fit`): $N$; the numbers of events and non-events; the separation status; the deviances $D_0$ and $D_1$; the likelihood-ratio statistic $G^2 = D_0 - D_1 \sim \chi^2_1$; $\text{AIC}$, $\text{AIC}_c$, and $\text{BIC}$; the Tjur, Cox–Snell, Nagelkerke, and McFadden $R^2$; the ROC AUC with its DeLong SE and CI; and the classification accuracy, sensitivity, and specificity.
  - **Curve Coordinates** (`roc-curve`), the optional **Interpolated Unknowns** tab (`row-classification`), and the sigmoid curve overlay.
- **Diagnostic checks:** The analysis detects collinearity (constant $X$), complete separation, and quasi-complete separation and reports them explicitly instead of returning diverging estimates. It warns when the less frequent outcome has fewer than 10 events.
- **Edge cases and limits:** The analysis rejects non-binary $Y$, outcomes that are all 0 or all 1, non-finite inputs, $N < 3$, tables that are not XY, and XY tables with more than one subcolumn. Whenever $G^2 \le \chi^2_{1, 1-\alpha}$, the profile-likelihood bounds of $X_{50}$ are reported as open.
- **References and verification:**
  - References: Hosmer, Lemeshow & Sturdivant (2013); McCullagh & Nelder (1989); Tjur (2009); Albert & Anderson (1984); Heinze & Schemper (2002); Venzon & Moolgavkar (1988); DeLong, DeLong & Clarke-Pearson (1988).
  - Verified against: `R 4.5.2` (`stats::glm(..., family = binomial)` and `confint`) ([statistics evidence](../evidence/statistics.md)).

---

## Fit spline / LOWESS curve {#analysis-fit-spline-lowess}

This analysis draws a smooth curve through XY data without assuming a parametric equation. It can also produce a table of sampled `(X, Y)` coordinates and interpolate unknowns nonparametrically (`Y` from `X`, and `X` from `Y`). It works only with XY tables (see [Analysis Registry: `fit-spline-lowess`](registry/tables.html#analysis-kind-fit-spline-lowess)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `transform.run` with `fit_spline_lowess` (the preferred name), `spline_lowess`, `spline`, `cubic_spline`, `natural_cubic_spline`, `akima`, `akima_subspline`, `smoothing_spline`, `lowess`, `point_to_point`, or `fit.spline_lowess`.
- **Options and defaults:**
  - `method`: `natural_cubic_spline` (default), `akima_subspline`, `smoothing_spline`, `lowess`, or `point_to_point`.
  - `segment_count`: the number of polyline vertices, in `[2, 10000]` (default `200`). LOWESS automatically uses at least $4N$ segments.
  - `smoothing_knots` or `smoothing_df`: for Reinsch smoothing splines, either the number of knots ($\ge 2$) or the effective degrees of freedom ($\text{df} = \text{tr}(S_\lambda) \in [2, G]$).
  - `lowess_preset` (`coarse` = `0.75`, `medium` = `0.50` (the default), or `fine` = `0.25`), the optional `lowess_fraction` in `(0, 1]`, and `lowess_robustness_iters` (default `3`, maximum `20`).
  - `plot_x_min` and `plot_x_max`: optional explicit bounds for evaluating the curve.
  - `emit_curve_coordinates_tab`: boolean (default `false`). Adds a tab of the sampled `(X, Y)` vertices.
  - `interpolate_unknowns`: boolean (default `false`). Predicts $Y$ from $X$, and finds $X$ from $Y$ by solving $S(x) = y_{\text{unk}}$.
- **Reported outputs:**
  - **Summary** (`summary`): the method, the number of active points $N$, the number of distinct $X$ knots $G$, the vertex count, the domain bounds, RSS, and RMSE. For a smoothing spline it adds the effective $\text{df}$ and $\lambda$; for LOWESS, the span $f$ and the number of robustness passes. Scalars that other analyses can link to are `n_points`, `distinct_x`, `segments`, `rss`, `rmse`, `effective_df`, `smoothing_lambda`, and `lowess_span`.
  - **Curve Coordinates** (`curve-coordinates`, optional): the sampled `(X, Y)` vertices of the curve for each dataset.
  - **Interpolated Unknowns** (`interpolated-unknowns`, optional): forward and inverse predictions, with a note when a non-monotonic curve crosses the target value more than once.
- **Diagnostic checks:** The checklist warns when `lowess` runs on fewer than 20 active points and mentions the spline methods as alternatives for small samples.
- **Edge cases and limits:** For splines, Akima curves, and point-to-point curves, tied $X$ values are collapsed to `(x_g, mean y_g)`. For Reinsch smoothing splines, they are collapsed to means weighted by multiplicity. The analysis rejects non-finite inputs, fewer than 2 distinct $X$ values, spans or df outside their valid ranges, and tables that are not XY.
- **References and verification:**
  - References: de Boor (1978); Akima (1970); Reinsch (1967); Hutchinson & de Hoog (1985); Cleveland (1979); Brent (1973).
  - Verified against: `R 4.5.2` (`stats::splinefun(..., method="natural")`, `stats::smooth.spline`, `stats::lowess`) and analytical benchmarks for Akima overshoot at a step ([curve-fitting evidence](../evidence/curve-fitting.md)).

---

## Levene / Brown–Forsythe variance test {#analysis-levene-test}

This analysis tests the null hypothesis that the selected groups have equal population variances. Each selected column of a Column table with raw replicates, or each numeric variable of a Multiple Variables table, is one group. Pre-summarized Column subcolumns are rejected, because the absolute deviations must be computed from individual replicates (see [Analysis Registry: `levene-test`](registry/tables.html#analysis-kind-levene-test)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `levene` (the preferred name), `levene_test`, `stats.levene`, `levene_median`, `brown_forsythe_variance`, `brown_forsythe_levene`, `variance_homogeneity`, or `equal_variance_test`, all of which center on the median. `levene_mean` centers on the mean.
- **Options and defaults:**
  - `center`: `median` (default; the Brown–Forsythe test) or `mean` (Levene's original test).
  - `selected_data_sets`: the columns to compare. An empty list selects every column, or every numeric variable on a Multiple Variables table.
- **Reported outputs:**
  - **Summary** (`levene-summary`): the method, the center, the number of groups $k$, the total $N$, $F$, $\text{DFn} = k - 1$, $\text{DFd} = N - k$, the *P* value, and the decision at $\alpha$.
  - **Descriptive Summary** (`levene-groups`): for each group, $n$, the center, the mean absolute deviation, and the SD.
  - Scalars: `F`, `DFn`, `DFd`, `P value`, `Groups`, `Total N`, and, for each group, `N`, `Center`, and `Mean abs deviation`.
- **Diagnostic checks:** The checklist reports the equal-variance decision, warns when any group has fewer than 5 values, notes that mean centering is sensitive to skewness, and lists excluded or ignored cells.
- **Edge cases and limits:**
  - Fewer than 2 groups, or any group with $n < 2$, fails with an insufficient-sample-size error.
  - If the absolute deviations have no spread within groups (for example, constant groups, or groups that all have size 2), the analysis fails with a zero-variance error instead of returning an undefined $F$.
  - Tables other than Column and Multiple Variables are rejected.
- **References and verification:**
  - References: Levene (1960); Brown & Forsythe (1974a); Zar (2010, ch. 10).
  - Verified against: `SciPy 1.18.1` (`scipy.stats.levene`) and `R 4.5.2` (`anova(lm(|x - c| ~ g))`) ([statistics evidence](../evidence/statistics.md)).

---

## Area under the curve (AUC) {#analysis-area-under-curve}

This analysis computes the area under a curve by the linear trapezoidal rule (Yeh & Kwan 1978). It can restrict the area to a partial window `[x_min, x_max]`, interpolating at the window edges, and it can detect peaks above the baseline within that window. When the design allows, it reports Bailer (1988) standard errors for the signed net area. It works only with XY tables (see [Analysis Registry: `area-under-curve`](registry/tables.html#analysis-kind-area-under-curve)).

The `X` values must be finite numbers in the units you want the area expressed in (see the [Numeric-X contract](tables.md#numeric-x-contract)). For example, `X` values stored as day-fraction serial numbers give an AUC in days unless you convert them first, and `hh:mm:ss` text that was not parsed is ignored.

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `transform.run` with `auc` (the preferred name), `area_under_curve`, or `transform.auc`.
- **Options and defaults:**
  - `baseline`: either `{ "mode": "fixed", "value": 0.0 }` (or a bare finite number; the default is $Y = 0$), or `{ "mode": "first_point" }` (or `"first_point"`), which uses $Y(\text{Start } X)$ within each series' clipped window.
  - `x_min` and `x_max`: optional inclusive bounds. When both are set, `x_min < x_max` is required. A bound that falls between data points is placed on the curve by linear interpolation, giving $(a, Y(a))$ and $(b, Y(b))$.
  - `independent_time_points`: boolean (default `false`). It must be `true` for the Bailer (1988) `NetAreaSe` to be computed.
  - `min_peak_height`: the non-negative height above the baseline that a peak must reach to be kept (default `0.0`).
  - `min_area_fraction`: the fraction of the positive area, in $[0.0, 1.0]$, that a peak must contain to be kept (default `0.0`).
- **Partial-window interpolation, Bailer knot weights, and peak clipping:**
  - When `x_min` and `x_max` are given, the data must cover them to within `tol(bound) = 1e-12 * max(1, |bound|)`. Floating-point jitter within this tolerance is snapped to the bound; a window that extends beyond the data fails.
  - Each segment $[x_i, x_{i+1}]$ is clipped to $[a, b]$; let $L = b - a$ be the clipped width. The analysis computes $t_a = (a - x_i) / (x_{i+1} - x_i)$, $t_b = (b - x_i) / (x_{i+1} - x_i)$, and the midpoint $m = (t_a + t_b) / 2$. The trapezoidal integral then accumulates the knot weights `w_i += L * (1 - m)` and `w_{i+1} += L * m`.
  - With the `first_point` baseline, the analysis subtracts $W = \text{End } X - \text{Start } X$ times the baseline's interpolation weights (`w_{k_0} -= W * (1 - t_0)`, `w_{k_0+1} -= W * t_0`). This makes $\sum_k w_k = 0$ and keeps the Bailer variance $\text{Var}(\text{Net Area}) = \sum_k w_k^2 \text{SE}_k^2$ exact even when the baseline shares knots with the integral.
  - `NetAreaSe` applies only to the signed `Net Area`. It is computed only when `independent_time_points` is `true` and every contributing knot has a finite SE (from $n \ge 2$ replicates or from parametric summary subcolumns). Otherwise `NetAreaSe` stays blank, and a diagnostic note either explains the independence assumption or names the knots with $n = 1$.
  - Peak detection runs on the clipped curve over `[Start X, End X]`, relative to the resolved baseline. Every peak therefore lies inside `[Start X, End X]`, and peaks are filtered by `min_peak_height` and `min_area_fraction`.
- **Reported outputs:**
  - **Summary** (`auc-summary`): Start X, End X, Span, N Points, Baseline, Positive Area, Negative Area, Net Area, Total Area, Net Area SE (Bailer, indep.), Peak Count, Peak X, Peak Y, and Status.
  - **Descriptive Summary** (`auc-peaks`, when any peak qualifies): Series, Peak #, Peak X, Peak Y, Height Above Baseline, Start X, End X, Area, and Area Fraction.
  - Scalars (keyed by dataset index): `PositiveArea`, `NegativeArea`, `NetArea`, `TotalArea`, and `NetAreaSe` (when computed).
  - Analyzable table: a Multiple Variables sheet (Series, Start X, End X, Span, N Points, Baseline, Positive Area, Negative Area, Net Area, Total Area, Net Area SE) that can feed further analyses or `table.importPlateMap` (`source.value_column`).
- **Diagnostic checks:**
  - When one or both window bounds are omitted, CurveBook compares the effective `Start X` and `End X` of all series and warns if the series cover different windows.
  - Missing or excluded interior rows between valid knots are bridged linearly (Yeh & Kwan 1978) and counted in an information note.
  - A series that cannot be computed (because its span is truncated or it has fewer than 2 distinct finite `X` values) keeps its row in the Series table, with blank numeric cells, an explanation in Status, and a warning. The analysis fails only if no series succeeds.
  - Text entries in `X` trigger a warning that explains the numeric-X requirement.
- **Edge cases and limits:** Tied `X` observations are collapsed to their arithmetic mean, pooling the summary moments or the replicate SEM. Validation fails for invalid bounds (`x_min >= x_max`), non-finite baselines, a negative `min_peak_height`, a `min_area_fraction` outside $[0, 1]$, and tables that are not XY.
- **References and verification:**
  - References: Yeh & Kwan (1978) (*J. Pharmacokinet. Biopharm.* 6(1):79–98); Bailer (1988) (*J. Pharmacokinet. Biopharm.* 16(3):303–309).
  - Verified against: `R 4.5.2`, `SciPy`, and exact closed-form benchmarks for partial windows and first-point knot weights ([statistics evidence](../evidence/statistics.md)).

---

## Row statistics (replicate summaries) {#analysis-row-statistics}

Row statistics collapse the active replicates in each row into a total, a mean (with SD, SEM, %CV, a confidence interval, or the minimum and maximum), a median (with quartiles or custom percentiles), or a geometric mean (with the geometric SD factor or a CI). The result is a derived table that further analyses and graphs can use. The analysis works with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `row-statistics`](registry/tables.html#analysis-kind-row-statistics)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `transform.run` with `row_statistics` (the preferred name), `row_stats`, or `transform.row_statistics`.
- **Options and defaults:**
  - `summary`: `mean_sd`, `mean_sem`, `mean_cv`, `mean_ci`, `mean_min_max`, `median_quartiles`, `median_min_max`, `median_percentiles`, `geometric_mean_sd`, `geometric_mean_ci`, or `total`.
  - `upper_percentile`: with `summary: "median_percentiles"`, a custom upper quantile $p_{\text{hi}} \in (50.0, 100.0)$, paired with the symmetric lower quantile $100 - p_{\text{hi}}$.
- **Reported outputs:**
  - **Summary** and an **Analyzable Data** table that other analyses can use. The table keeps the row labels and $X$ coordinates and adds the row summaries and the number of active replicates $N$.
- **Diagnostic checks:** Excluded and blank replicate cells are skipped. In rows that contain non-positive values, the geometric summaries are left blank and a diagnostic note explains why.
- **References and verification:**
  - References: Hyndman & Fan (1996) Type-6 and Type-7 quantile estimators; Kirkwood (1979) geometric SD factor.
  - Verified against: `R 4.5.2` offline transform oracles ([statistics evidence](../evidence/statistics.md)).

---

## Fraction of total {#analysis-fraction-of-total}

This analysis expresses each active cell as a fraction or percentage of its row total, its column total, the grand total, or all three. It can add Wilson score or exact Clopper–Pearson binomial confidence intervals. The output table can feed further analyses. The analysis works with Column, XY, Grouped, Contingency, Parts of Whole, and Multiple Variables tables (see [Analysis Registry: `fraction-of-total`](registry/tables.html#analysis-kind-fraction-of-total)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `transform.run` with `fraction_of_total` (the preferred name) or `transform.fraction_of_total`.
- **Options and defaults:**
  - `denominator`: `column_total` (default), `row_total`, `grand_total`, or `all_three`.
  - `output_percent`: boolean. `true` (the default) reports percentages from $0$ to $100\%$; `false` reports fractions in $[0, 1]$.
  - `ci_method`: `none` (default), `wilson`, or `clopper_pearson`.
- **Reported outputs:**
  - A **Summary** tab for each selected denominator (with columns for the lower and upper binomial CI limits when CIs are turned on), and an **Analyzable Data** table that can feed further statistical analyses or graphs.
- **Diagnostic checks:** A denominator that sums to zero is handled without producing `NaN`. When binomial CIs are requested, all counts must be non-negative.
- **References and verification:**
  - References: Wilson (1927); Clopper & Pearson (1934).
  - Verified against: `R 4.5.2` offline transform oracles ([statistics evidence](../evidence/statistics.md)).

---

## Curve calculus (differentiate, integrate, smooth) {#analysis-curve-calculus}

This analysis computes the numerical first or second derivative of a curve, its cumulative trapezoidal integral, or a smoothed version of it (Savitzky–Golay, moving average, or Cleveland LOWESS). It runs as a full XY analysis linked to its source data, and it publishes an `analyzable_data` table that further analyses can use. It works only with XY tables (see [Analysis Registry: `curve-calculus`](registry/tables.html#analysis-kind-curve-calculus)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `transform.run` with `curve_calculus` (the preferred name), `smooth_differentiate_integrate`, `differentiate`, `smooth`, or `transform.curve_calculus`.
- **Options and defaults:**
  - `operation` takes one of these values:
    - `first_derivative` or `second_derivative`, computed by shifted 3-point Lagrange quadratic differentiation on uniform or non-uniform grids.
    - `{"CumulativeIntegral": {"baseline": 0.0}}`.
    - `{"SmoothSavitzkyGolay": {"window": 5, "order": 2}}`.
    - `{"SmoothMovingAverage": {"window": 3}}`.
    - `{"SmoothLowess": {"span": 0.5}}`.
- **Reported outputs:**
  - **Summary** and an **Analyzable Data** XY sheet that holds the differentiated, integrated, or smoothed curves. You can graph the sheet directly or pass it on to a regression or an AUC analysis.
- **Diagnostic checks:** Numerical differentiation rejects duplicate $X$ values. The analysis also checks that a Savitzky–Golay window is odd and larger than the polynomial order ($w > \text{order}$), and that a LOWESS span lies in $(0, 1]$.
- **References and verification:**
  - References: Savitzky & Golay (1964); Gorry (1990); Cleveland (1979); Burden & Faires (2011) shifted Lagrange differentiation.
  - Verified against: `R 4.5.2` offline transform oracles ([statistics evidence](../evidence/statistics.md)).

---

## Monte Carlo simulation {#analysis-monte-carlo-simulation}

Monte Carlo simulation estimates empirical power, the coverage of confidence intervals, and the distributions of parameters by repeating a synthetic experiment many times. Each run is a bounded, reproducible loop of up to `100,000` iterations. In each iteration CurveBook:

1. re-simulates an upstream synthetic table,
2. runs a downstream transform, test, or nonlinear regression that is chained to it,
3. records the chosen scalar outputs, and
4. classifies the iteration with user-defined `"Hit"` rules.

Fits that are flagged or did not converge can be excluded. The results are summarized on separate `Hits`, `Not Hits`, and `All` tabs. The analysis works with every [table archetype](tables.md#table-archetypes) (see [Analysis Registry: `monte-carlo-simulation`](registry/tables.html#analysis-kind-monte-carlo-simulation)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `monte_carlo` (the preferred name), `monte_carlo_simulation`, or `stats.monte_carlo`.
- **Options and defaults:**
  - `seed`: a 64-bit seed that makes the synthetic table in each iteration reproducible.
  - `iterations`: a positive integer, at most `100,000`.
  - `downstream_result_sheet_id`: the downstream result sheet that is recomputed in each iteration. If `tracked_scalars` is omitted, up to 8 finite scalars from this sheet are tracked automatically.
  - `tracked_scalars`: a list of scalar references, `{ label, result_sheet_id, parameter_key, data_set_index }`, recorded in each iteration.
  - `hit_rules`: threshold rules that classify each retained iteration as a hit or not a hit (for example, $P < 0.05$ for empirical power, or a rule that checks CI coverage).
  - `exclude_flagged_fits`: boolean. When set, CurveBook applies `fit_flag_rules.exclude_flagged_from_summary` and leaves ambiguous or non-converged nonlinear fits out of the summary distributions.
- **Reported outputs:**
  - Summary tabs (`mc-summary-all`, `mc-summary-hits`, `mc-summary-not-hits`): the numbers of retained, flagged, hit, and non-hit iterations; the hit rate as a percentage; and, for each tracked scalar, the mean, SD, median, IQR, minimum, and maximum.
  - Per-iteration table and **Analyzable Data**: the tracked scalar values and hit status of every iteration, available for histograms, normality checks, or distribution plots.
- **Diagnostic checks:** The analysis checks that `downstream_result_sheet_id` and `tracked_scalars` are actually linked to the upstream simulated table. It stops promptly when you cancel it, and it records iterations that were excluded as flagged or that failed, without stopping the study.
- **References and verification:**
  - References: Robert & Casella (2004) *Monte Carlo Statistical Methods*; Motulsky & Christopoulos (2004).
  - Verified against: engine integration tests that confirm reproducible results, correct partitioning into `Hits`, `Not Hits`, and `All`, and correct filtering by `fit_flag_rules.exclude_flagged_from_summary` ([statistics evidence](../evidence/statistics.md)).

---

## Frequency distribution {#analysis-frequency-distribution}

This analysis partitions numeric observations into uniform right-closed intervals $[e_0, e_1], (e_1, e_2], \dots, (e_{k-1}, e_k]$ and tabulates bin counts, relative frequencies, percentages, probability densities, and cumulative distributions. It can overlay a fitted Gaussian density or cumulative curve parameterized by each series' sample mean and standard deviation, and it can automatically create a linked histogram graph. It works with Column, XY, Grouped, and Multiple Variables tables (see [Analysis Registry: `frequency-distribution`](registry/tables.html#analysis-kind-frequency-distribution)).

- **Commands and aliases:** Run [`analysis.run`](../commands.md) or `stats.run` with `frequency_distribution` (the preferred name), `frequency`, `histogram`, `freq_dist`, or `stats.frequency_distribution`.
- **Options and defaults:**
  - `binning_rule` (or shorthand `binning` / `bin_width`):
    - `AutoFreedmanDiaconis` (default): sets $h = 2\,\text{IQR}\,n^{-1/3}$ using Hyndman–Fan Type-7 quartiles, with $k = \lceil (\max - \min) / h \rceil$.
    - `AutoScott`: sets $h = 3.5\,s\,n^{-1/3}$ using the sample standard deviation $s$.
    - `AutoSturges`: sets $k = \lceil \log_2 n \rceil + 1$.
    - `ExplicitWidth` (`width > 0`, or shorthand `bin_width`): uses a fixed positive bin width.
  - `first_bin_center` and `last_bin_center`: optional finite bin centers that anchor or bound the uniform bin grid (unset by default so the grid spans the observed data range).
  - `output_mode` (or shorthand `output`): `Counts` (default), `RelativeFrequency` ($c_j / N$), `Percentages` ($100\,c_j / N$), `Density` ($c_j / (N\,w_j)$), `CumulativeCounts`, `CumulativeFrequency` ($\text{ECDF}(e_{j+1})$), or `CumulativePercentages`.
  - `overlay_gaussian`: boolean (default `false`). Evaluates a smooth 201-point Gaussian density or cumulative distribution curve across the bin range from each series' sample mean and SD.
  - `aggregation`: `PooledGrandSample` (default), `RowMeans`, or `PerSubcolumn` for tables with replicate subcolumns.
  - `create_histogram_graph`: boolean (default `true`). Automatically creates a linked histogram graph sheet when run through engine commands.
- **Reported outputs:**
  - **Frequency distribution** (`freq-distribution`): lower edge, upper edge, bin center, and the selected output value for every series.
  - **Bin counts, densities & cumulative distribution** (`freq-detail`): complete table of counts, relative frequencies, percentages, probability densities, cumulative counts, cumulative frequencies, and cumulative percentages per bin.
  - **Distribution summary** (`freq-summary`): per-series $N$, excluded cell count, below-first-bin and above-last-bin counts, bin count, bin width, sample mean, SD, and IQR.
  - **Analyzable Data:** an XY table with bin centers on $X$ and the selected frequency scale on $Y$, ready for downstream nonlinear curve fitting or histogram plotting.
  - **Scalars:** `Bin count`, `Bin width`, `First bin center`, `Last bin center`, `First bin edge`, `Last bin edge`, plus per-series `N`, `Mean`, `SD`, `IQR`, `Underflow`, and `Overflow`.
- **Diagnostic checks:** The checklist records the active binning rule and bin width, warns when constant data or zero IQR triggers an automatic fallback, warns when explicit bin boundaries leave active observations in underflow or overflow tails, warns if a Gaussian overlay cannot be drawn ($n < 2$ or zero variance), and lists any ignored non-numeric cells.
- **Edge cases and limits:**
  - Pre-summarized subcolumn modes require individual raw observations and are refused with a typed error.
  - Constant data ($\min = \max = c$) or a single observation ($n = 1$) uses one unit-width bin $[c - 0.5, c + 0.5]$ centered on $c$.
  - Bin configurations exceeding `10,000` bins or non-positive widths return a typed invalid-parameter error.
  - When explicit bounds omit lower-tail values, cumulative counts still include observations strictly below the first bin edge so cumulative frequency matches the exact empirical CDF at every upper bin edge.
- **References and verification:**
  - References: Sturges (1926); Scott (1979); Freedman & Diaconis (1981); Hyndman & Fan (1996).
  - Verified against: `R 4.5.2` (`hist`, `nclass.Sturges`, `nclass.scott`, `nclass.FD`, `ecdf`, `dnorm`, `pnorm`) ([statistics evidence](../evidence/statistics.md)).
