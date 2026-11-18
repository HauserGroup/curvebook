# Statistical & Analytical Methods Reference

This page provides an interface-neutral reference for every `AnalysisKind` variant in CurveBook (`crates/doc/src/analysis.rs`, dispatched via [`analyze.run`](../commands.md), `stats.run`, `fit.run`, `transform.run`, `stats.pvalueStack`, and `analysis.run`).

All diagnostic checklists surfaced in `ResultSheet::checklist` obey [`QOL-7` diagnostics-only semantics](qol.md#active-qol): they report executed assumption diagnostics, statistics, statuses (`Pass`, `Warn`, `Fail`, `Inconclusive`, `NotApplicable`), and sample-size/power limits without recommending alternative procedures, ranking methods, or mutating document state. For complete programmatic invocation contracts, see [Command Reference](../commands.md), [Automation: MCP](automation/mcp.md), and [Automation: CLI](automation/cli.md).

---

## Transform (`AnalysisKind::Transform`) {#analysis-transform}

- **Purpose:** Derives a clean-room transformed, normalized, baseline-adjusted, transposed, pruned, filtered, curve-calculus, Monte Carlo simulated, or without-replacement randomly sub-sampled `DataSheet` (`analyzable_data`) inside a `ResultSheet`.
- **Accepted Table Archetypes:** `Xy`, `Column`, `Grouped`, `MultipleVariables`, `Contingency`, `PartsOfWhole`, `Survival`, or `Nested` depending on the `TransformAnalysisSpec` variant (e.g., `Normalize`, `RemoveBaseline`, and `Math` require numeric value columns; `SelectAndTransform` targets arbitrary `TableKind`; `Simulate` creates a new `Xy` dataset; `RandomSample` draws from active rows).
- **Commands & Automation:** `transform.run`, `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`TransformAnalysisSpec`):**
  - `Math { x_transform, y_transform, error_propagation }`: optional `MathTransformPreset` (`Ln`, `Log10`, `Log2`, `Exp10`, `Exp`, `Reciprocal`, `Sqrt`, `Square`, `Logit`, `Probit`, `ArcsinSqrt`, `LinearScale`, `ConcentrationToNegativeLogM`); `ErrorPropagationMode` for `MeanSdN` / `MeanSemN` / `MeanCvN` spread propagation.
  - `Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set }`: `NormalizeAnchor` (`ColumnMin`, `ColumnMax`, `ColumnSum`, `FirstRow`, `LastRow`, `FixedValue`, `ControlColumnMean`, `Hooked`).
  - `RemoveBaseline { baseline_columns, operation }`: `BaselineOperation` (`Difference`, `Ratio`, `PercentChange`, `FractionalChange`).
  - `Transpose { swap_row_and_column_titles }`: swaps row and column dimensions.
  - `PruneRows { every_k_rows, average_window }`: stride $k \ge 1$, averaging within non-overlapping $k$-row windows or sampling every $k$-th row.
  - `SelectAndTransform { filter_formula, target_archetype, derived_columns }`: formula filter predicate, target `TableKind`, and `=` formula expressions.
  - `Simulate { seed, row_count, replicates, x_min, x_max, equation_id, parameters, noise }`: seeded `DeterministicRng` simulation with `SimulationNoise` (`None`, `Gaussian`, `RelativePercent`, `Poisson`).
  - `RandomSample { seed, sample_size, stratify_column, proportional_allocation }` (`proportional_allocation` defaults to `true`).
- **Reported Values & Semantics:** Emits `ResultSheet::analyzable_data` (preserving non-destructive cell exclusions) plus `TransformedData` tab and simulation/sampling metadata scalars.
- **Assumptions & `QOL-7` Diagnostics:** Records non-destructive warnings when domain-invalid cells (e.g., $\ln(y)$ on $y \le 0$) become blank.
- **Edge Cases & Limits:** Enforces `MAX_ROWS`, `MAX_DATA_SETS`, `MAX_SUBCOLUMNS`, and `MAX_IMPORT_CELLS = 10,000,000`; `Normalize` rejects `zero_anchor == hundred_anchor` (zero span); `RandomSample` rejects $k > N$ or missing stratification column. See [Data Tables & Transforms](tables.md#transforms-and-reshaping).
- **Literature & Oracle Verification:** Deterministic across native and `wasm32` targets using SplitMix64/Xoshiro256** (`DeterministicRng`) and `curvebook_transform` roundtrip tests (`<= 1e-12`).

---

## Descriptive Statistics (`AnalysisKind::DescriptiveStats`) {#analysis-descriptive-stats}

- **Purpose:** Summarizes column distributions (location, dispersion, robust quantiles, confidence intervals, ratios), evaluates normality/lognormality batteries, and optionally runs one-sample hypothesis tests against a hypothetical location.
- **Accepted Table Archetypes:** `Column`, `Xy` (Y datasets), `Grouped`, `MultipleVariables` (numeric continuous columns), and pre-summarized `MeanSdN` / `MeanSemN` subcolumns for parametric moments.
- **Commands & Automation:** `stats.run` (`kind: "descriptive_stats"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`DescriptiveSpec`):**
  - `aggregation: SubcolumnAggregation` — default `RowMeans` (`RowMeans`, `PerSubcolumn`, `PooledGrandSample`).
  - `custom_upper_percentile: Option<f64>` — default `None` (when set in `[50.0, 100.0)`, reports paired lower $100 - p$ and upper $p$ percentiles).
  - `run_normality_battery: bool` — default `true`.
  - `hypothetical_value: Option<f64>` — default `None` (triggers `one_sample_t_test` and `one_sample_wilcoxon` against $\mu_0$ when supplied).
  - Parent `confidence_level` defaults to `95%`.
- **Reported Values & Semantics (`DescriptiveSummary`):**
  - `n`, `min`, `max`, `range`, `sum`, `mean`, `sd` ($n - 1$ denominator via Chan–Golub–LeVeque variance), `sem` ($\text{SD}/\sqrt{n}$), `cv_percent` ($100\cdot\text{SD}/\text{mean}$), `quadratic_mean` (RMS).
  - **Parametric CIs at `confidence_level`:** Two-sided Student-*t* confidence interval for the mean (`df = n - 1`), exact $\chi^2$ confidence interval `sd_ci` (`[s*sqrt((n-1)/chi2_{1-alpha/2,n-1}), s*sqrt((n-1)/chi2_{alpha/2,n-1})]`) assuming Gaussian normality (`R9.12`).
  - **Order & Robust Quantiles:** `median`, Hyndman & Fan (1996) Type-6 hinges `q1`, `q3`, `iqr`, plus Hyndman–Fan Type-7 custom percentiles; **exact binomial nonparametric `MedianCi`** achieving $\ge$ target coverage with achieved coverage percentage (`n >= 3`).
  - **Geometric & Harmonic Summaries (positive observations $x > 0$):** `geometric_mean`, geometric SD factor `geometric_sd_factor` (Kirkwood 1979), log-back-transformed geometric mean CI, `harmonic_mean`, skewness `g1`, excess kurtosis `g2`.
  - **Optional One-Sample Tests (when `hypothetical_value` is `Some`):** `one_sample_t_test` (`t`, `df = n - 1`, two-sided *P*, discrepancy CI, Hedges' $g$) and `one_sample_wilcoxon` ($W^+$, exact small-$n$ or Pratt tie-adjusted asymptotic *P*, Walsh-average Hodges–Lehmann pseudomedian & CI).
- **Assumptions & `QOL-7` Diagnostics:** When `run_normality_battery == true` on raw replicates ($n \ge 3$), executes Shapiro–Wilk (`3 <= n <= 5000`), D'Agostino–Pearson $K^2$ (`n >= 8`), Anderson–Darling $A^{2*}$, and Lilliefors KS on raw values and (when strictly positive) log-transformed values; reports `Pass`/`Fail`/`Inconclusive` without suggesting test switches. Ignores text cells explicitly via checklist + footnotes (`append_ignored_text_checklist_and_footnote`).
- **Edge Cases & Known Limits:** Excluded and blank cells are omitted; zero-variance samples mark normality/one-sample `t` unavailable cleanly (`ZeroVariance`); $n < 3$ omits Shapiro–Wilk / binomial median CI; `MeanSdN`/`MeanSemN` inputs skip raw-replicate normality and rank summaries (`RequiresRawReplicates`). Non-positive entries omit geometric/lognormality summaries (`None`).
- **Literature & Oracle Verification:** Chan, Golub & LeVeque (1983); Student (1908); Kirkwood (1979); Hyndman & Fan (1996); Shapiro & Wilk (1965) / Royston (1995); D'Agostino, Belanger & D'Agostino (1990); Anderson & Darling (1954); Lilliefors (1967) / Dallal & Wilkinson (1986). Verified vs. NIST StRD `univ` (`mean LRE >= 12.0`–`13.0`, `sd LRE >= 8.0`–`10.0` offline stiff benchmarks, double-double `LRE >= 14.0`) and R 4.5.2 (`1e-10`–`1e-15` on quantiles/moments/`t`, `<= 1e-10` on Shapiro–Wilk `W`).

---

## Outlier Detection (`AnalysisKind::OutlierDetection`) {#analysis-outlier-detection}

- **Purpose:** Identifies candidate statistical outliers within univariate columns via Grubbs' ESD test or univariate ROUT FDR detection.
- **Accepted Table Archetypes:** `Column`, `Xy` (Y columns), `MultipleVariables` continuous columns (raw replicates required).
- **Commands & Automation:** `stats.run` (`kind: "outlier_detection"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`OutlierSpec`, default `Rout { fdr_q_percent: 1.0 }`):**
  - `Grubbs { alpha, iterative }`: two-sided significance level `alpha` in `(0, 1)` (`0.05` default if unspecified); `iterative: bool` switches single extreme (`Grubbs 1969`) vs. sequential Rosner ESD masking removal.
  - `Rout { fdr_q_percent }`: Motulsky & Brown (2006) adapted to univariate location using 68.27th-percentile `RSDR` scale + Benjamini–Hochberg step-up FDR across the furthest `~30%` points at `Q`% in `(0, 100]` (default `1.0`%).
- **Reported Values & Semantics:** Per-outlier flags (`row_index`, `value`, `statistic` $G$ or $|r_i|/\text{RSDR}$, *P*-value or BH FDR *q*-value, `is_outlier` boolean); cleaned descriptive summaries excluding flagged outliers (reported read-only without mutating the source table).
- **Assumptions & `QOL-7` Diagnostics:** Grubbs assumes approximate univariate normality of inliers; ROUT resists multiple outliers / swamping up to ~30% contamination. Checklist reports Gaussian diagnostics when $n \ge 3$ and notes outlier counts strictly descriptively.
- **Edge Cases & Known Limits:** Rejects pre-summarized modes (`RequiresRawReplicates`); requires $n \ge 3$ for Grubbs and $n \ge 4$ for ROUT; zero variance (`SD = 0` or `RSDR = 0`) cleanly reports zero outliers rather than dividing by zero.
- **Literature & Oracle Verification:** Grubbs (1969); Rosner (1983); Motulsky & Brown (2006); Benjamini & Hochberg (1995). Verified in `d1_reference_oracles.rs` against exact analytic Grubbs ESD cutoffs (`1e-12`) and R/ROUT outlier fixtures.

---

## Two-Group Comparison (`AnalysisKind::TwoGroupComparison`) {#analysis-two-group-comparison}

- **Purpose:** Tests location and variance differences between two independent or matched groups, or one sample against a reference constant.
- **Accepted Table Archetypes:** `Column` (2 selected columns, or 1+ columns for one-sample tests; supports `MeanSdN` / `MeanSemN` for `UnpairedT`, `WelchT`, `OneSampleT`) and `MultipleVariables` (specifying `mv_group_column` + `mv_response_column`).
- **Commands & Automation:** `stats.run` (`kind: "two_group"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`TwoGroupSpec`):**
  - `method: TwoGroupMethod` — default `UnpairedT`. Supported variants: `UnpairedT`, `WelchT`, `PairedT`, `RatioPairedT`, `LognormalUnpairedT`, `MannWhitneyU`, `WilcoxonSignedRank`, `KolmogorovSmirnov`, `OneSampleT { hypothetical }`, `OneSampleWilcoxon { hypothetical }`, `PairedSignTest`.
  - `two_tailed: bool` — default `true` (`false` evaluates directional one-sided *P*-value in the observed contrast direction).
  - `compute_effect_sizes: bool` — default `true`.
  - `mv_group_column: Option<usize>`, `mv_response_column: Option<usize>` — default `None` (auto-detects first binary categorical + continuous variable on `MultipleVariables`).
- **Reported Values & Semantics:**
  - Test statistic (`t`, Mann–Whitney `U_a`/`U_min`, Wilcoxon `W+`/`W-`, KS `D`, exact binomial sign count), degrees of freedom (`n_a + n_b - 2` pooled, Welch–Satterthwaite fractional `df`, or `n - 1` paired), *P*-value (exact permutation for small untied rank tests, Pratt zero handling for Wilcoxon, lattice exact for KS, exact Binomial for Sign test), difference / geometric ratio / Hodges–Lehmann shift estimate and CI at `confidence_level`, plus variance equality *F*-test (`f_test_variances`) / Levene / Brown–Forsythe (`levene_variance_test`) and pairing effectiveness correlation $r$.
  - **Standardized effect sizes:** Cohen's $d$, bias-corrected Hedges' $g$, Glass's $\Delta$ with Steiger (2004) exact noncentral-*t* CIs, $\eta^2 / R^2$, Cliff's $\delta$, and rank-biserial $r_{rb}$.
- **Assumptions & `QOL-7` Diagnostics:** Evaluates per-group (or paired difference / log-ratio) Shapiro–Wilk normality, *F*-test equality of variances, and minimum sample sizes in `checklist` (diagnostics-only).
- **Edge Cases & Known Limits:** Paired tests enforce complete-case pair filtering; `df = 1` two-pair paired *t*-tests yield valid primary *t*, *P*, `df`, and mean-difference CI while omitting undefined gamma-pole Hedges' $g$ cleanly; `RatioPairedT` and `LognormalUnpairedT` require strictly positive values ($> 0$); `ZeroVariance` when both groups are constant with zero within-group SD; `MannWhitneyU`/`WilcoxonSignedRank` reject pre-summarized modes (`RequiresRawReplicates`).
- **Literature & Oracle Verification:** Student (1908); Welch (1947); Satterthwaite (1946); Zar (2010); Levene (1960); Brown & Forsythe (1974); Cohen (1988); Hedges (1981); Glass (1976); Steiger & Fouladi (1997); Steiger (2004); Bland & Altman (1996); Mann & Whitney (1947); Wilcoxon (1945); Pratt (1959); Hodges & Lehmann (1963) / Monahan (1984); Dixon & Mood (1946); Smirnov (1948) / Hodges (1958); Cliff (1993). Verified vs. R 4.5.2 within `1e-10`–`1e-12` (`1e-5` noncentral-*t* CIs).

---

## Bland–Altman Method Comparison (`AnalysisKind::BlandAltman`) {#analysis-bland-altman}

- **Purpose:** Evaluates agreement between two quantitative measurement methods $A$ and $B$ via bias, standard deviation of differences, and limits of agreement (LoA).
- **Accepted Table Archetypes:** `Column` (two columns `column_a`, `column_b`), `Xy` (X vs. Y or two specified Y columns), and `MultipleVariables` (two continuous columns).
- **Commands & Automation:** `stats.run` (`kind: "bland_altman"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`BlandAltmanSpec`):**
  - `column_a: usize` — default `0`.
  - `column_b: usize` — default `1`.
  - `percent_difference: bool` — default `false` (computes $A - B$ vs. $(A + B)/2$; when `true`, computes $100(A - B)/((A + B)/2)$).
- **Reported Values & Semantics (`BlandAltmanResult`):** `n` (paired sample count), `bias` ($\bar{d}$), `sd_bias` ($\text{SD}_d$), `bias_ci` ($\bar{d} \pm t_{1-\alpha/2,\,n-1}\text{SD}_d/\sqrt{n}$), `loa_lower` and `loa_upper` ($\bar{d} \pm z_{1-\alpha/2}\text{SD}_d$ at `AnalysisSpec::confidence_level`) with normal-approximation SE CIs for each LoA bound, proportional-bias slope test, and scatter coordinates feeding `DiagnosticPlotKind::BlandAltman`.
- **Assumptions & `QOL-7` Diagnostics:** Assesses normality of paired differences (`shapiro_wilk`) and proportional bias correlation between difference and average.
- **Edge Cases & Known Limits:** Rejects explicit out-of-range column indices with `MissingColumn`; omits pairs with zero pair mean when `percent_difference == true`; requires $n \ge 2$ complete finite pairs.
- **Literature & Oracle Verification:** Bland & Altman (1986, 1999). Verified vs. R 4.5.2 within `1e-9`–`1e-12` (`bias`, `sd_bias`, `bias_ci`, `loa_lower`, `loa_upper`).

---

## Multiple Row-Wise Comparisons (`AnalysisKind::MultipleRowTTests`) {#analysis-multiple-row-t-tests}

- **Purpose:** Executes independent or paired two-group *t*-tests or rank tests row-by-row across high-throughput matrices (e.g., differential expression / screening assays) with multiplicity control and Volcano plot outputs.
- **Accepted Table Archetypes:** `Grouped`, `Xy`, `Column`, and `MultipleVariables` (`AnalysisKind::is_compatible_with`).
- **Commands & Automation:** `stats.run` (`kind: "multiple_row_t_tests"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`MultipleRowTestsSpec`):**
  - `paired: bool` — default `false`.
  - `nonparametric: bool` — default `false` (`MannWhitneyU` unpaired / `WilcoxonSignedRank` paired).
  - `assume_equal_sd: bool` — default `true` (Student pooled *t* vs. Welch–Satterthwaite *df* when `!paired && !nonparametric`).
  - `multiplicity: MultiplicitySpec` — default `MultiplicityMethod::None`, `threshold: 0.05`, `report_confidence_intervals: true`.
- **Reported Values & Semantics:** Per-row `MultipleRowTestEntry`: mean difference / shift, SE, test statistic, `df`, raw two-sided *P*-value, adjusted *P*-value / FDR *q*-value (`Bonferroni`, `Sidak`, `HolmSidak`, `BenjaminiHochbergFdr`, `TwoStageBkyFdr`, `BenjaminiYekutieliFdr`), discovery boolean, and $-\log_{10} P$ / effect coordinates for `Volcano` diagnostic graphs.
- **Assumptions & `QOL-7` Diagnostics:** Reports tested row count, discovery count, multiplicity rule, and zero-variance / under-replicated skipped rows without auto-filtering rows destructively.
- **Edge Cases & Known Limits:** Rejects odd column selection when `paired == true`; handles per-row zero variance or $n < 2$ cleanly per entry; two-pair (`df = 1`) paired rows still yield valid *t*/*P* while omitting unavailable Hedges' $g$.
- **Literature & Oracle Verification:** Student (1908); Welch (1947); Benjamini & Hochberg (1995); Benjamini, Krieger & Yekutieli (2006); Benjamini & Yekutieli (2001); Holm (1979); Šidák (1967). Verified against R 4.5.2 in `d1_reference_oracles.rs` (`1e-10`–`1e-15`).

---

## Passing–Bablok Regression (`AnalysisKind::PassingBablok`) {#analysis-passing-bablok}

- **Purpose:** Performs robust, nonparametric Model II linear method comparison resistant to outliers and measurement error in both reference ($X$) and candidate ($Y$) methods.
- **Accepted Table Archetypes:** `Column` (`x_column` vs. `y_column`), `Xy` (X vs. Y dataset), and `MultipleVariables` (two continuous variables).
- **Commands & Automation:** `stats.run` (`kind: "passing_bablok"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`PassingBablokSpec`):**
  - `x_column: usize` — default `0`.
  - `y_column: usize` — default `1`.
  - Parent `confidence_level` — default `95%` (symmetry-shifted order-statistic confidence intervals).
- **Reported Values & Semantics (`PassingBablokResult`):** Shifted median slope $\hat{b}$ over non-excluded pairwise slopes $S_{ij} = (Y_j - Y_i)/(X_j - X_i)$ excluding identical pairs and exact $-1$ antiparallel slopes, median intercept $\hat{a} = \text{median}(Y_i - \hat{b} X_i)$, exact Passing–Bablok order-statistic confidence intervals `(slope_ci, intercept_ci)`, CUSUM linearity test (`cusum_h`, `cusum_statistic`, `cusum_p_value`, `linearity_passed`), and residuals. Also exposes `FittedCurveOverlay` and `ModelEquation` box support.
- **Assumptions & `QOL-7` Diagnostics:** Evaluates Kolmogorov–Smirnov CUSUM linearity assumption ($H_0$: linear relationship) and positive slope convention ($b > 0$); notes when CI quantiles clamp at endpoint order statistics (`slope_ci_clamped`).
- **Edge Cases & Known Limits:** Bounded to `PASSING_BABLOK_MAX_N = 10,000` observations due to $O(N^2)$ pairwise slope construction; rejects fewer than `3` valid non-coincident pairs or zero valid non-antiparallel slopes.
- **Literature & Oracle Verification:** Passing & Bablok (1983, *J. Clin. Chem. Clin. Biochem.* 21:709–720). Verified against `R` / analytic reference suites (`parity_g8_barnard_pb_mi.rs`).

---

## One-Way ANOVA & Kruskal–Wallis (`AnalysisKind::OneWayAnova`) {#analysis-one-way-anova}

- **Purpose:** Tests differences across $k \ge 2$ groups under independent homoscedastic ANOVA, heteroscedastic Welch/Brown–Forsythe ANOVA, repeated-measures / mixed-effects REML, or nonparametric rank-sum / block-rank models with post-hoc multiple comparisons.
- **Accepted Table Archetypes:** `Column` (`k >= 2` columns; `MeanSdN`/`MeanSemN` supported for `OrdinaryAnova` and `BrownForsytheAndWelch`, raw replicates required for `RepeatedMeasures` and `KruskalWallis` / `Friedman`), and `MultipleVariables` (one categorical factor + one continuous response).
- **Commands & Automation:** `stats.run` (`kind: "one_way_anova"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`OneWayAnovaSpec`):**
  - `method: OneWayMethod` — default `OrdinaryAnova` (`OrdinaryAnova`, `BrownForsytheAndWelch`, `RepeatedMeasures { geisser_greenhouse }`, `KruskalWallis`, `Friedman`).
  - `mixed_effects_reml: bool` — default `false` (engaged automatically when RM tables contain missing cells).
  - `df_method: MixedModelDfMethod` — default `KenwardRoger` (`KenwardRoger`, `Satterthwaite`).
  - `residual_covariance: ResidualCovariance` — default `CompoundSymmetry` (`CompoundSymmetry`, `Unstructured`).
  - `compute_effect_sizes: bool` — default `true`.
  - `multiplicity: MultiplicitySpec` — `TukeyHsd`, `GamesHowell`, `Dunnett`, `DunnettT3`, `TamhaneT2`, `Bonferroni`, `Sidak`, `HolmSidak`, `NewmanKeuls`, `FisherLsd`, `DunnRankSum`, `LinearTrend`, `BenjaminiHochbergFdr`, `BenjaminiYekutieliFdr`, or `TwoStageBkyFdr` (`None` by default).
- **Reported Values & Semantics:** ANOVA partition (`SS`, `df`, `MS`, `F`, *P*-value), variance homogeneity tests (Bartlett $\chi^2$, Brown–Forsythe median Levene), Welch `W` & Brown–Forsythe $F^*$ with Satterthwaite denominator `df2`, Mauchly's $W$ sphericity test + Greenhouse–Geisser $\hat{\varepsilon}$ and Huynh–Feldt $\tilde{\varepsilon}$ (Lecoutre 1991), Kruskal–Wallis tie-corrected $H$ / Friedman $Q$ + Kendall's $W$, omnibus effect sizes ($\eta^2$, $\omega^2$, Cohen's $f$ with noncentral-*F* CIs, $\varepsilon^2$), REML variance components ($\sigma^2_{\text{subj}}, \sigma^2_{\text{err}}$), and `ResultTabKind::MultipleComparisons` adjusted *P*-values + simultaneous confidence intervals (`PairwiseComparisonRecord` feeding CLD/brackets).
- **Assumptions & `QOL-7` Diagnostics:** Reports per-group Shapiro–Wilk normality, Bartlett/Brown–Forsythe homoscedasticity, Mauchly sphericity (`k >= 3`), and REML missing-cell handling strictly as diagnostics.
- **Edge Cases & Known Limits:** Unbalanced REML falls back deterministically from unstructured to compound-symmetry only with explicit `UnstructuredFallbackReason` reporting when covariance estimation is singular; `MeanSdN`/`MeanSemN` reject raw-only RM/Kruskal/Friedman; post-hoc `Dunnett` requires specifying control index (defaults to column `0`).
- **Literature & Oracle Verification:** Zar (2010); Maxwell & Delaney (2004); Searle (1987); Bartlett (1937); Brown & Forsythe (1974a, 1974b); Welch (1951); Mauchly (1940); Greenhouse & Geisser (1959); Huynh & Feldt (1976); Lecoutre (1991); Kruskal & Wallis (1952); Friedman (1937); Tukey/Kramer (1956); Games & Howell (1976); Dunnett (1955, 1980); Dunn (1964); Steiger (2004); Patterson & Thompson (1971); Kenward & Roger (1997); Satterthwaite (1946); Pinheiro & Bates (1996). Verified vs. NIST StRD `anova` (`LRE >= 8.0`–`10.0`), R `stats`/`nlme` (`1e-9`–`1e-12` classical; `5e-4`–`1e-3` REML variance components/Kenward–Roger *F*).

---

## Factorial & Repeated-Measures ANOVA / REML (`AnalysisKind::FactorialAnova`) {#analysis-factorial-anova}

- **Purpose:** Fits balanced or unbalanced two-way, three-way, or $N$-way crossed factorial ANOVA, repeated-measures factorial models (one or both factors matched), and mixed-effects REML longitudinal models with main-effect and simple-effect multiple comparisons.
- **Accepted Table Archetypes:** `Grouped` (2-way or 3-way subcolumn-grouped; raw replicates or `MeanSdN`/`MeanSemN` for fixed non-RM designs) and `MultipleVariables` ($N$-way categorical factors + continuous response).
- **Commands & Automation:** `stats.run` (`kind: "factorial_anova"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`FactorialAnovaSpec`):**
  - `three_way: bool` — default `false`.
  - `multifactor_columns: Vec<usize>`, `response_column: Option<usize>`, `max_interaction_order: usize` — default `2` (for `MultipleVariables`).
  - `ss_type: SumOfSquaresType` — default `TypeIII` (`TypeI`, `TypeII`, `TypeIII`).
  - `repeated_measures: RepeatedMeasuresDesign` — default `None` (`None`, `RowFactorMatched`, `ColumnFactorMatched`, `BothFactorsMatched`).
  - `mixed_effects_reml: bool` — default `false` (`fit_reml_on_missing: true` switches automatically when missing cells exist under RM).
  - `df_method: MixedModelDfMethod` — default `KenwardRoger`.
  - `residual_covariance: ResidualCovariance` — default `CompoundSymmetry` (`CompoundSymmetry`, `Unstructured`).
  - `compute_effect_sizes: bool` — default `true`.
  - `multiplicity: MultiplicitySpec` — default `MultiplicityMethod::None` (`R9.12` post-hoc tabs over marginal/simple effects).
- **Reported Values & Semantics:** Per-term ANOVA table (`SS`, `df`, `MS`, `F`, *P*-value), partial $\eta^2_p$ / $\omega^2_p$ with noncentral-*F* CIs, sphericity estimates (`Geisser–Greenhouse`, `Huynh–Feldt`) per within-subject stratum, REML variance components and Wald/Kenward–Roger *F* statistics, and `ResultTabKind::MultipleComparisons` tabs across main effects and simple cell contrasts.
- **Assumptions & `QOL-7` Diagnostics:** Reports residual/cell normality, cell balance, Type III weighted-contrast coding (`effect`/`sum` coding), and sphericity warnings without test switching.
- **Edge Cases & Known Limits:** Unbalanced empty cells reject invalid Type III contrasts or switch cleanly to identifiable REML variance-component parameterizations; missing RM cells trigger REML when `fit_reml_on_missing == true`; raw replicates required for repeated-measures covariance estimation.
- **Literature & Oracle Verification:** Searle (1987); Maxwell & Delaney (2004); Patterson & Thompson (1971); Gilmour, Thompson & Cullis (1995); Kenward & Roger (1997); Fai & Cornelius (1996); Verbeke & Molenberghs (2000). Verified vs. R 4.5.2 & `nlme 3.1.168` in `d1_reference_oracles.rs`, `d1_edge_matrix.rs`, `r34_unstructured_reml_oracles.rs`, `r35_anova_reml_oracles.rs`, `r36_rm_reml_oracles.rs` (`1e-9` fixed SS, `1e-4`–`1e-5` RM/REML *F*, `5e-4` REML variance components).

---

## Nested Comparison (`AnalysisKind::NestedComparison`) {#analysis-nested-comparison}

- **Purpose:** Analyzes hierarchical designs where technical measurements sit inside random biological subclusters (e.g., litters, animals, wells) nested under fixed treatment groups, preventing technical pseudoreplication.
- **Accepted Table Archetypes:** `Nested`.
- **Commands & Automation:** `stats.run` (`kind: "nested_comparison"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`NestedAnalysisSpec`):**
  - `two_group_t_test: bool` — default `false` (nested *t*-test when `true` on 2 groups, nested one-way ANOVA otherwise).
  - `df_method: MixedModelDfMethod` — default `KenwardRoger` (`KenwardRoger`, `Satterthwaite`).
  - `multiplicity: MultiplicitySpec` — post-hoc comparisons when $k \ge 3$ groups.
- **Reported Values & Semantics (`NestedAnovaResult`):** Among-group (`MS_groups`, `F_among_groups`, `p_among_groups`), nested biological-unit (`SS_subclusters`, `variance_subclusters`), residual error (`SS_error`, `variance_error`), intra-class correlation (ICC), nested *t*-statistic (`t^2 == F`) & mean-difference CI when `two_group_t_test == true`, and adjusted post-hoc comparisons for $k \ge 3$.
- **Assumptions & `QOL-7` Diagnostics:** Diagnoses pseudoreplication boundary ($\hat{\sigma}^2_{\text{sub}} = 0$ boundary clamping per Self & Liang 1987), cluster size imbalance, and normality of subcluster residuals.
- **Edge Cases & Known Limits:** Uses exact balanced variance-component ANOVA when equal subclusters/replicates exist and automatically fits REML with Kenward–Roger / Satterthwaite denominator *df* when unequal/sparse clusters occur. Requires at least 2 treatment groups and 2 subclusters across the table.
- **Literature & Oracle Verification:** Zar (2010); Patterson & Thompson (1971); Self & Liang (1987); Kenward & Roger (1997); Satterthwaite (1946). Verified in `d1_reference_oracles.rs` (`1e-9` balanced SS, `1e-3` balanced REML variances, `1e-2` unbalanced REML `F`).

---

## Contingency, Agreement & Categorical Tests (`AnalysisKind::Contingency`) {#analysis-contingency}

- **Purpose:** Evaluates association, trend, exact small-sample inference, diagnostic accuracy, matched-pair discordance, inter-rater agreement, and multinomial/binomial goodness of fit on count tables.
- **Accepted Table Archetypes:** `Contingency` ($2 \times 2$, $R \times C$, square $k \times k$ for Cohen's $\kappa$, subjects $\times$ categories for Fleiss' $\kappa$, paired $2 \times 2$ for McNemar), `PartsOfWhole` (`PearsonChiSquare` goodness-of-fit and `ExactBinomial`), and `Column` (`ExactBinomial`).
- **Commands & Automation:** `stats.run` (`kind: "contingency"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`ContingencySpec`):**
  - `method: ContingencyMethod` — default `PearsonChiSquare { yates_continuity: false }`; variants: `PearsonChiSquare { yates_continuity }`, `LikelihoodRatioG2`, `FisherExact`, `BarnardExact`, `CochranArmitageTrend`, `McNemarPaired`, `CohenKappa { weighting: KappaWeighting::{Unweighted, Linear, Quadratic} }`, `FleissKappa`, `ExactBinomial`.
  - `compute_odds_and_risk_ratios: bool`, `compute_sensitivity_specificity: bool`, `compute_phi_and_cramers_v: bool` — default `false`.
  - `expected_proportions: Option<Vec<f64>>` — optional expected proportions/weights for `PartsOfWhole` GOF (`None` tests uniform $1/k$).
  - `null_proportion: Option<f64>` — null proportion $p_0 \in (0, 1)$ for `ExactBinomial` (defaults to `0.5`).
- **Reported Values & Semantics:**
  - $\chi^2$, Yates-corrected $\chi^2$, Wilks $G^2$, exact Fisher two-sided *P* ($2\times 2$ hypergeometric; $R\times C$ Freeman–Halton or bounded deterministic Monte Carlo), Barnard exact supremum *P* (CSM/Boschloo ordering), Cochran–Armitage $z$ & 1-df $\chi^2$.
  - $2\times 2$ effect measures at `confidence_level`: Odds Ratio (Baptista–Pike exact mid-*P* CI & Woolf logit CI), Relative Risk (Koopman score CI), Risk Difference (Newcombe Wilson score CI), Altman `NNTB`/`NNTH` CIs (including exact continuous union across $\infty$ when RD CI crosses $0$), Sensitivity/Specificity/PPV/NPV with Wilson score CIs, $\text{LR}^+ / \text{LR}^-$, $\phi$ and Cramér's $V$ with exact noncentral-$\chi^2$ CIs.
  - McNemar continuity-corrected $\chi^2$, exact binomial *P*, mid-*P*, and discordant-pair OR $(b/c)$ with exact Clopper–Pearson transformed bounds; Cohen's / Fleiss' $\kappa$, $p_o$, $p_e$, asymptotic SE, CI, and $z$; `ExactBinomial` Sterne/doubled two-sided *P* + Clopper–Pearson, Wilson, and Agresti–Coull CIs.
- **Assumptions & `QOL-7` Diagnostics:** Reports Cochran's minimum expected cell count and fraction of cells with $E_{ij} < 5$ (`cells_lt5_fraction`), guiding inspection toward exact methods when counts are sparse.
- **Edge Cases & Known Limits:** Rejects negative, fractional, or non-finite cell counts; handles zero marginal rows/columns safely (`0/0` OR/RR bounds return `None`/open rather than `NaN`); large $R \times C$ tables switch deterministically from exact network enumeration to seeded Monte Carlo (`1e-3` tolerance).
- **Literature & Oracle Verification:** Pearson (1900); Yates (1934); Wilks (1938); Fisher (1935); Freeman & Halton (1951); Barnard (1945); Cochran (1954); Armitage (1955); McNemar (1947); Edwards (1948); Baptista & Pike (1977); Koopman (1984); Newcombe (1998); Altman (1998); Wilson (1927); Clopper & Pearson (1934); Agresti & Coull (1998); Sterne (1954); Cohen (1960, 1968); Fleiss, Cohen & Everitt (1969); Fleiss (1971, 1979). Verified vs. R 4.5.2 within `1e-10` (`chi2`, Fisher $2\times 2$, Cochran–Armitage, McNemar, Fleiss/Cohen $\kappa$), `1e-6` (Koopman RR, Baptista–Pike OR), and `0.003` (Monte Carlo $R\times C$ Fisher).

---

## Survival Analysis (`AnalysisKind::Survival`) {#analysis-survival}

- **Purpose:** Estimates right-censored Kaplan–Meier survival curves with cohort comparison tests, or multivariable Cox proportional hazards regression with Schoenfeld PH diagnostics.
- **Accepted Table Archetypes:** `Survival` (Kaplan–Meier) and `MultipleVariables` (both Kaplan–Meier via `mv_time_column`/`mv_status_column`/`mv_group_column` and Cox PH via `time_column`/`status_column`/`predictor_columns`).
- **Commands & Automation:** `stats.run` (`kind: "survival"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`SurvivalAnalysisSpec`):**
  - `KaplanMeier { log_rank_mantel_cox, gehan_breslow_wilcoxon, trend_test, asymmetrical_greenwood_ci, multiplicity, mv_time_column, mv_status_column, mv_group_column }`: `asymmetrical_greenwood_ci: true` computes complementary log-log bounds (Kalbfleisch & Prentice 2002); `false` emits linear Greenwood bounds.
  - `CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method }`: `CoxTieMethod` defaults to `Efron` (`Breslow`, `Efron`, `Exact`).
- **Reported Values & Semantics:**
  - **Kaplan–Meier:** Step table (`time`, `n_at_risk`, `n_events`, `n_censored`, $S(t)$, Greenwood SE, CI at `confidence_level`), Number at Risk grid, Brookmeyer–Crowley median survival & CI, Mantel–Cox log-rank $\chi^2$, Gehan–Breslow–Wilcoxon $\chi^2$, Tarone $1\text{ df}$ trend test ($k \ge 3$), Mantel–Haenszel & log-rank Hazard Ratios with CI, pairwise multiplicity tabs, and `FittedCurveOverlay` step coordinates.
  - **Cox PH (`CoxPhResult`):** Covariate $\hat{\beta}_j$, SE, Wald $z$, *P*-value, Hazard Ratio $\exp(\hat{\beta}_j)$ with CI at `confidence_level`, model LR/Score/Wald $\chi^2$ & global *P*, Harrell's concordance $C$-index, per-covariate & global Grambsch–Therneau weighted Schoenfeld proportional-hazards tests, and Breslow baseline survival curve.
- **Assumptions & `QOL-7` Diagnostics:** Reports proportional-hazards Schoenfeld global/covariate *P*-values, monotone-likelihood / infinite-HR divergence checks, censoring rates, and zero comparable pairs in `checklist`.
- **Edge Cases & Known Limits:** Negative follow-up times or invalid non-binary event indicators are rejected; all-censored cohorts yield undefined median survival (`None` / `"Undefined"`); when `comparable_pairs == 0` in Cox PH, concordance is treated as unavailable rather than meaningful discrimination; Cox `Exact` ties gracefully handle moderate tie counts via exact elementary symmetric polynomials.
- **Literature & Oracle Verification:** Kaplan & Meier (1958); Greenwood (1926); Kalbfleisch & Prentice (2002); Brookmeyer & Crowley (1982); Mantel & Haenszel (1959); Mantel (1966); Peto & Peto (1972); Gehan (1965); Breslow (1970, 1974); Tarone (1975); Berry, Kitchin & Mock (1991); Cox (1972); Efron (1977); Schoenfeld (1982); Therneau, Grambsch & Fleming (1990); Grambsch & Therneau (1994); Harrell et al. (1982). Verified vs. R `survival 3.8.3` within `1e-9`–`1e-12` (KM steps, Greenwood CI, log-rank, Gehan–Breslow, Cox Breslow/Efron/Exact logLik, `1e-6`–`1e-7` Cox betas/SEs/Wald/Score).

---

## ROC Curve Analysis (`AnalysisKind::RocCurve`) {#analysis-roc-curve}

- **Purpose:** Evaluates binary diagnostic classifier discrimination across all empirical thresholds, trapezoidal AUC, exact or asymptotic confidence intervals, optimal Youden's $J$ cutoff, and paired/unpaired two-ROC curve AUC comparisons.
- **Accepted Table Archetypes:** `Column` (controls in first selected column, cases in second column) and `MultipleVariables` (binary outcome + continuous score).
- **Commands & Automation:** `stats.run` (`kind: "roc_curve"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`RocSpec`):** `ci_method: RocCiMethod` — default `DeLong` (`DeLong` or `HanleyMcNeil`). Parent `confidence_level` defaults to `95%`.
- **Reported Values & Semantics (`RocResult`, `RocComparisonResult`):** `auc`, `se`, two-sided confidence interval clamped to `[0, 1]`, two-sided $z$-test *P*-value vs. $0.5$, optimal cutoff maximizing Youden's $J = \text{Sensitivity} + \text{Specificity} - 1$, full operating-point threshold table (`cutoff`, `sensitivity`, `specificity`, `1 - specificity`, `youden_j`, `ppv`, `npv`, `LR+`, `LR-`), structural placement vectors (`delong_v10`, `delong_v01`), and `FittedCurveOverlay` points for `DiagnosticPlotKind::RocCurve`.
- **Assumptions & `QOL-7` Diagnostics:** Notes higher-values-indicate-cases directional convention and sample counts (`n_controls`, `n_cases`).
- **Edge Cases & Known Limits:** Requires at least `1` control and `1` case observation; perfect separation ($\text{AUC} = 1.0$) produces `DeLong` structural variance $0.0$ (SE = `0.0`), while division-by-zero likelihood ratios (`LR+` when specificity = `1.0`, `LR-` when sensitivity = `0.0`) cleanly emit `None`.
- **Literature & Oracle Verification:** DeLong, DeLong & Clarke-Pearson (1988); Hanley & McNeil (1982); Youden (1950). Verified vs. R 4.5.2 within `1e-12` (AUC), `1e-11` (SE), `1e-10` (CI bounds & paired DeLong $z$), and `1e-9` (paired *P*-value).

---

## Generalized Linear Models (`AnalysisKind::GeneralizedLinearModel`) {#analysis-generalized-linear-model}

- **Purpose:** Fits multivariable Gaussian OLS linear regression, Binomial logistic regression (logit link), or Poisson log-linear count regression (log link) with diagnostics, VIF multicollinearity checks, and residual/leverage tables.
- **Accepted Table Archetypes:** `MultipleVariables` (continuous/categorical predictors with reference-level dummy coding via `reference_levels`), `Xy`, and `Column` (`AnalysisKind::is_compatible_with`).
- **Commands & Automation:** `stats.run` (`kind: "generalized_linear_model"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`GlmSpec`):**
  - `family: GlmFamily` — default `Linear` (`Linear`, `Logistic`, `Poisson`).
  - `response_column: usize`, `predictor_columns: Vec<usize>`, `reference_levels: Vec<(usize, String)>`, `compute_vif: bool`.
- **Reported Values & Semantics:**
  - **`Linear` (`MultipleLinearResult`):** Term $\hat{\beta}_j$, SE, $t$, *P*-value, CI at `confidence_level`, optional `VIF`, Type III SS / $F$ / *P*, $R^2$, adjusted $R^2$, residual SD, overall model $F$ & *P*, `AIC`, `AICc`, `BIC`, Durbin–Watson autocorrelation, covariance matrix, plus row-wise `fitted`, `residuals`, hat `leverage`, and `cooks_distance`.
  - **`Logistic` (`MultipleLogisticResult`):** MLE $\hat{\beta}_j$, SE, Wald $z$, *P*-value, Odds Ratio $\exp(\hat{\beta}_j)$ & CI, log-likelihood, null/residual deviance, `AIC`, `BIC`, McFadden & Tjur pseudo-$R^2$, Hosmer–Lemeshow decile goodness-of-fit $\chi^2$, ROC AUC of predicted probabilities, and separation flag.
  - **`Poisson` (`MultiplePoissonResult`):** MLE $\hat{\beta}_j$, SE, Wald $z$, *P*-value, Rate Ratio $\exp(\hat{\beta}_j)$ & CI, null/residual deviance, Pearson $\chi^2$ & dispersion parameter $\hat{\phi}$, `AIC`, `BIC`, fitted counts, and residuals.
- **Assumptions & `QOL-7` Diagnostics:** Reports complete-case listwise filtering counts (retaining true 1-based source row indices in diagnostic tables), residual Shapiro–Wilk normality (Linear), multicollinearity `VIF` thresholds (`> 5` / `> 10`), complete/quasi-complete separation (Logistic; Albert & Anderson 1984), and overdispersion $\hat{\phi}$ (Poisson).
- **Edge Cases & Known Limits:** Rejects out-of-range `response_column` / `predictor_columns`; `Linear` detects collinear aliased predictors cleanly via column-pivoted QR (`aliased_columns`); `Logistic` requires binary `{0, 1}` outcomes; `Poisson` requires finite non-negative counts; singular design matrices or non-positive variance return structured `StatsError`.
- **Literature & Oracle Verification:** Golub (1965); Björck (1967); McCullagh & Nelder (1989); Albert & Anderson (1984); Hosmer & Lemeshow (1980). Verified vs. NIST StRD `lls` (`LRE >= 9.0`–`12.0`), and R 4.5.2 `glm()` (`1e-8` on logistic/poisson betas, deviance, AIC, BIC).

---

## Multivariate Analysis & Multiple Imputation (`AnalysisKind::Multivariate`) {#analysis-multivariate}

- **Purpose:** Performs Principal Component Analysis (PCA), Pearson/Spearman pairwise correlation matrices, agglomerative hierarchical clustering & $k$-means++ partitioning, or MICE predictive mean matching (PMM) with Rubin pooling.
- **Accepted Table Archetypes:** `MultipleVariables`, `Xy`, and `Column` (`AnalysisKind::is_compatible_with`).
- **Commands & Automation:** `stats.run` (`kind: "multivariate"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`MultivariateSpec`):**
  - `Pca { standardize, parallel_analysis_seed }`: SVD of centered covariance (`standardize: false`) or standardized correlation (`standardize: true`) matrix; optional deterministic Horn (1965) Monte Carlo parallel analysis.
  - `CorrelationMatrix { spearman_rank }`: pairwise Pearson $r$ (`false`) or Spearman $\rho$ (`true`) with two-sided *P*-values, Fisher-$z$ CIs, and BH FDR *q*-values.
  - `Clustering { k_means_k, distance, linkage, k_sweep_max }`: `ClusterDistance` (`Euclidean` default, `Manhattan`, `Cosine`, `Pearson`) and `ClusterLinkage` (`Ward` default, `Complete`, `Single`, `Upgma`, `Wpgma`, `Centroid`, `Median`); optional $k$-means++ (Arthur & Vassilvitskii 2007) with silhouette & $k$-sweep.
  - `MultipleImputation { imputations_m, max_iters, seed, response_column, predictor_columns }`: MICE PMM across $m \ge 2$ imputed datasets pooled by Rubin (1987) / Barnard–Rubin (1999) rules.
- **Reported Values & Semantics:** PCA eigenvalues, variance proportions, loadings, observation scores, Kaiser & parallel-analysis counts; full correlation/$P$/FDR matrices; hierarchical merge heights, dendrogram cuts, cophenetic correlation, cluster assignments, centroids & mean silhouette; MICE pooled parameter estimates, within/between variance, fraction of missing information (FMI), Barnard–Rubin `df`, and pooled CIs.
- **Assumptions & `QOL-7` Diagnostics:** Reports complete-case listwise deletion (PCA/Clustering), pairwise complete observations (CorrelationMatrix), standardization scale notes, and zero-variance constant-column guards.
- **Edge Cases & Known Limits:** Constant zero-variance columns fail correlation PCA standardization cleanly (`ZeroVariance`); `k >= n` in clustering is bounded/rejected cleanly; cophenetic correlation returns `None` when distance variance is zero.
- **Literature & Oracle Verification:** Pearson (1896); Fisher (1921); Spearman (1904); Kendall (1938, 1945); Jolliffe (2002); Horn (1965); Lloyd (1982); Arthur & Vassilvitskii (2007); Rousseeuw (1987); Sokal & Michener (1958); Gower (1967); Rubin (1987); Barnard & Rubin (1999). Verified vs. R `cor.test`/`prcomp`/`hclust` (`1e-10`–`1e-11`) and `parity_g8_barnard_pb_mi.rs`.

---

## Linear & Deming Regression (`AnalysisKind::LinearRegression`) {#analysis-linear-regression}

- **Purpose:** Fits simple OLS regression (with ANCOVA slope/elevation comparison across multiple datasets) or Model II orthogonal/Deming errors-in-variables regression, including inverse calibration and AUC reporting.
- **Accepted Table Archetypes:** `Xy`, `Grouped`, and `MultipleVariables` (`MeanSdN`/`MeanSemN`/`MeanCvN` supported on `Xy`/`Grouped`; `AnalysisKind::is_compatible_with`).
- **Commands & Automation:** `fit.run` (`kind: "linear"` / `"deming"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`LinearRegressionSpec`):** `deming_model_ii: false`, `force_through_origin: false`, `confidence_bands: true`, `prediction_bands: false`, `interpolate_unknowns: false`.
- **Reported Values & Semantics:**
  - **OLS (`SimpleLinearResult`):** Slope, Y-intercept, X-intercept ($-a/b$), SEs, symmetric *t* CIs at `confidence_level`, $R^2$, $S_{y.x}$, overall $F$ / *t* vs. zero slope, runs test, replicates lack-of-fit test, ANCOVA equality-of-slopes and equality-of-intercepts $F$-tests when $\ge 2$ datasets are fitted, band overlay (`band_grid_points = 101`), trapezoidal AUC, and interpolated unknowns (`LinearInterpolation` with exact Fieller 1954 topological interval classification).
  - **Deming Model II (`DemingResult`):** Slope and intercept under equal X/Y error variance ratio $\lambda = 1.0$, Leave-One-Out jackknife SEs and *t* CIs (`df = n - 2`), and $S_{y.x}$.
- **Assumptions & `QOL-7` Diagnostics:** Reports normality/runs/lack-of-fit diagnostics, identical-$X$ degeneracy (`ZeroVariance` in $S_{xx}$), and unbounded Fieller calibration ($g \ge 1$).
- **Edge Cases & Known Limits:** Requires $n \ge 3$ for Deming jackknife SEs and $n \ge 2$ (free intercept) / $n \ge 1$ (origin-forced with nonzero $X$) for OLS; `force_through_origin` uses `df = n - 1` and uncentered $R^2$.
- **Literature & Oracle Verification:** Draper & Smith (1998); Zar (2010); Fieller (1954); Seber & Lee (2003); NIST/SEMATECH §4.5.1.1; Deming (1943); Linnet (1990, 1993); Efron & Tibshirani (1993). Verified in `curvebook-fit` tests (`1e-10`–`1e-12`).

---

## Nonlinear Regression (`AnalysisKind::NonlinearRegression`) {#analysis-nonlinear-regression}

- **Purpose:** Fits built-in parametric or custom DSL (algebraic, implicit, piecewise, `<A>`/`<B>` shared, first-order RK45 ODE) models using OLS, WLS, Poisson deviance IRLS, Cauchy robust regression, or ROUT outlier elimination, with profile-likelihood CIs, bands, unknown interpolation, and nested/AICc model comparison.
- **Accepted Table Archetypes:** `Xy`, `Grouped`, and `MultipleVariables` (`grouping_columns` partitioning multiple curve datasets on `MultipleVariables`; `MeanSdN`/`MeanSemN`/`MeanCvN` parametric moment modes supported where applicable, while grouped/replicate extractors reject `MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr`).
- **Commands & Automation:** `fit.run`, `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`NonlinearFitSpec`):**
  - `equation_id: String`, `custom_dsl: Option<String>` (mutually resolved; custom DSL compiles via `CompiledDslModel` with `MAX_AST_NODES = 4,096`).
  - `method: FitMethod` — default `LeastSquares` (`LeastSquares`, `PoissonDeviance`, `RobustCauchy`, `RoutOutlierElimination { fdr_q_percent: 1.0 }`).
  - `weighting: WeightingScheme` — default `None` (`None`, `RelativeSquare`, `Relative`, `GeneralPowerY(k)`, `InverseX`, `InverseX2`, `ObservedSd`).
  - `parameter_rules: Vec<ParameterRule>` — `Local`, `SharedGlobal`, `Constant`, `HookedConstant`, `PerDatasetConstant`, `ColumnTitleConstant`, initial guesses, inclusive box bounds.
  - `profile_likelihood_ci: bool`, `confidence_bands: bool`, `prediction_bands: bool`, `interpolate_unknowns: bool`, `replicates_lack_of_fit_test: bool`, `normality_of_residuals: bool`, `homoscedasticity_test: bool`, `emit_residuals_tab: bool`, `emit_covariance_tab: bool`, `comparison: Option<ModelComparisonSpec>`, `max_iters: usize` (default `1,000`), `x_range_filter`, `y_range_filter`, `plot_function_only`.
- **Reported Values & Semantics:** Best-fit parameter table (`estimate`, SE, Wald CI & Venzon–Moolgavkar profile-likelihood `CiBound::Finite`/`Open`, parameter dependency, Hougaard skewness, `ParameterNote`), derived transforms (`EC50`, `IC50`, `Km`, `t_half`, `Span`), goodness-of-fit (`df`, $R^2$, adjusted $R^2$, weighted/unweighted `RSS`, `Sy.x`, `RSDR`, `AIC`, `AICc`, `BIC`, runs/lack-of-fit/normality/homoscedasticity), `Comparison` (`ExtraSumOfSquaresF` / `Aicc` / `LikelihoodRatioTest`), `RoutDetectionResult`, covariance/correlation matrices, `FittedCurveOverlay` (`curve_overlay_segments = 256`), and `InterpolatedUnknown`s. See [Curve Fitting & Custom Equation Reference](fitting.md) for equations, solver details, and DSL limits.
- **Assumptions & `QOL-7` Diagnostics:** Reports convergence status (`Converged`, `Ambiguous`, `HitMaxIters`, etc.), collinearity `Dependency > 0.9999`, active box bounds, unbounded profile-likelihood intervals, runs/normality/homoscedasticity checks, and Hougaard curvature metrics.
- **Edge Cases & Known Limits:** `WeightingScheme` domain zeros return `ImpossibleWeights`; `PoissonDeviance` rejects negative $Y$ or non-positive fitted $\hat{\mu} \le 0$; `RobustCauchy`/`ROUT` omit asymptotic SEs/bands on robust fits and reject pre-summarized inputs; RK45 ODE integration obeys `MAX_ODE_STEPS = 50,000` per curve and `DEFAULT_MAX_ODE_STEPS_PER_FIT = 300,000`.
- **Literature & Oracle Verification:** See full citations and NIST StRD (`LRE >= 6.5` betas, `>= 5.0` SEs) / `D1` R 4.5.2 verification tolerances in [Curve Fitting Reference](fitting.md#verification-and-citations).

---

## *P*-Value Stack Multiplicity Adjustment (`AnalysisKind::PValueStack`) {#analysis-p-value-stack}

- **Purpose:** Computes adjusted *P*-values (FWER) or FDR *q*-values, rank orderings, and discovery indicators across a standalone vector or table column of pre-computed raw *P*-values (`R10.10`).
- **Accepted Table Archetypes:** `Column` (raw *P* column(s)), `Xy` (Y column(s)), `Grouped`, or `MultipleVariables` continuous columns containing valid probabilities $p \in [0.0, 1.0]$.
- **Commands & Automation:** `stats.pvalueStack`, `stats.run`, `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`PValueStackSpec`):**
  - `method: PValueStackMethod` — default `BenjaminiHochberg`; supports `Bonferroni`, `HolmBonferroni`, `Sidak`, `HolmSidak`, `BenjaminiHochberg`, `TwoStageBky`, and `BenjaminiYekutieli`.
  - `threshold: f64` — family-wise $\alpha$ or desired FDR $Q$ strictly in `(0.0, 1.0)` (default `0.05`).
  - `column: Option<usize>` — optional 0-based column override (defaults to `selected_data_sets`).
- **Reported Values & Semantics (`PValueStackColumnResult`, `PValueStackRowResult`):** Preserves exact original row order while emitting `sorted_order`, competition `rank`, validated `raw_p`, `adjusted_p_or_q` clamped to `[0.0, 1.0]`, and `discovery` (`adjusted_p_or_q <= threshold`), plus summary scalars (`count`, `discovery_count`, `min_raw_p`, `min_adjusted`, `skipped_excluded_or_empty`).
- **Assumptions & `QOL-7` Diagnostics:** Notes whether FWER or FDR control applies (and two-stage adaptive $\hat{m}_0$ behavior for BKY, arbitrary dependence via harmonic factor $H_m$ for Benjamini–Yekutieli).
- **Edge Cases & Known Limits:** Strictly validates every active raw probability is finite in `[0.0, 1.0]` (`NaN`, `±Inf`, `< 0.0`, `> 1.0` return `MathError::DomainError`; never coerces out-of-range inputs); rejects `threshold <= 0.0` or `>= 1.0`; handles exact ties deterministically without altering step-up/step-down monotonicity.
- **Literature & Oracle Verification:** Dunn (1961) / Bonferroni (1936); Holm (1979); Šidák (1967); Holland & Copenhaver (1987); Benjamini & Hochberg (1995); Benjamini, Krieger & Yekutieli (2006); Benjamini & Yekutieli (2001). Verified in `r1010_pvalue_stack_oracles.rs` against independent textbook formulas (`<= 1e-12`).

---

## Statistical Power & Sample Size (`AnalysisKind::PowerAnalysis`) {#analysis-power-analysis}

- **Purpose:** Computes prospective sample size ($N$ / $n_1, n_2$), achieved statistical power ($1 - \beta$), or minimum detectable effect size across eight classical hypothesis-test families, accompanied by power curves and sensitivity grids (`R8.7`).
- **Accepted Table Archetypes:** Standalone calculation (`DataSheet` input optional; may link an upstream `InfoSheet` via `info_sheet_id` for constant hooks).
- **Commands & Automation:** `stats.run` (`kind: "power_analysis"`), `analyze.run`, `analysis.add` ([Command Reference](../commands.md), [MCP](automation/mcp.md), [CLI](automation/cli.md)).
- **Options & Defaults (`PowerAnalysisSpec`):**
  - `family: PowerTestFamily` — default `TwoSampleUnpairedT`; supports `TwoSampleUnpairedT`, `TwoSamplePairedT`, `OneSampleT`, `OneWayAnova`, `TwoProportions`, `Correlation`, `ChiSquare`, `SimpleLinearRegression`.
  - `solve_target: PowerSolveTarget` — default `SampleSize` (`SampleSize`, `Power`, `DetectableEffect`).
  - `tail_mode: PowerTailMode` — default `TwoSided` (`TwoSided` or directional `OneSided`).
  - `alpha: f64` (`0.05`), `target_power: Option<f64>` (`Some(0.80)`), `allocation_ratio: f64` (`1.0`, $k = n_2 / n_1$), `sample_size`, `sample_size_group2`, `effect: PowerEffectSpec` (`Standardized`, `MeanDiffAndSd`, `PairedDiff`, `GroupMeansAndSd`, `TwoProportions`, `BaselineProportion`, `RSquared`, `RegressionSlope`), `num_groups`, `chi_square_df` (`1.0`), `two_proportions_mode: TwoProportionsMethod` (`PooledNullUnpooledAlt`, `Unpooled`, `CohensH`), `continuity_correction: bool` (`false`), `correlation_method: CorrelationPowerMethod` (`FisherZ`, `FisherZWithTCrit`, `ExactTTransform`, `ConditionalSlopeT`).
- **Reported Values & Semantics (`PowerAnalysisResult`):** Continuous root `continuous_n`, ceiling integer `n_per_group`, `n_group2`, `total_n`, `actual_df`, `power_at_continuous_n`, integer `achieved_power`, standardized `effect_size` and optional `raw_effect_size`, noncentrality parameter $\lambda / \delta$, critical cutoff, sampled `PowerCurvePoint` table, and sensitivity matrix (`DEFAULT_SENSITIVITY_ALPHAS = [0.10, 0.05, 0.01, 0.001]` $\times$ `DEFAULT_SENSITIVITY_POWERS = [0.70, 0.80, 0.90, 0.95]`).
- **Assumptions & `QOL-7` Diagnostics:** Documents exact vs. asymptotic distributional assumptions (e.g., exact noncentral-*t*/*F*/$\chi^2$ vs. normal proportion/Fisher-$z$ approximations) and warns when `alpha >= target_power` or sample size approaches boundary regimes.
- **Edge Cases & Known Limits:** `allocation_ratio` clamped/validated in `[1e-6, 1e6]`; `OneWayAnova` bounded to `MAX_ANOVA_GROUPS = 10,000`; `PowerEffectSpec::Unspecified` allowed only for `DetectableEffect` solves; Brent root finder converges to `rel_tol = 1e-12, abs_tol = 1e-12` up to `MAX_CONTINUOUS_N = 1e12`.
- **Literature & Oracle Verification:** Cohen (1988); Owen (1965); Chow, Shao & Wang (2008); Fleiss, Levin & Paik (2003); Fleiss, Tytun & Ury (1980); Fisher (1921). Verified in `power_oracle.rs` against R `pwr` conventions and analytical benchmarks (`1e-4` continuous `n`, `1e-5`–`1e-12` power identities).
