# Biostatistics & Hypothesis Testing Evidence (`curvebook-stats`)

> Governing mathematical & algorithmic specification: [`statistics-implementation.md`](../implementation/statistics-implementation.md) · Roadmap: [`ROADMAP.md`](../history/roadmap/r3.md#r3-core-biostatistics-anovareml-survival-contingency-roc--multivariate) ([`G1`/`G3`/`G7`/`G8` mapping](../history/legacy-ids-and-defects.md))
>
> **Status:** `curvebook-math` (`L0`), `curvebook-stats` (`L2`), and `curvebook-engine` (`L5`) statistical workflows are implemented and verified (`cargo test -p curvebook-stats --offline`, including `d1_reference_oracles`, `d1_edge_matrix` and offline R `4.5.2` (`stats` 4.5.2, `survival` 3.8-3, `nlme` 3.1-168, `MASS`, `cluster`) D1 oracles in `crates/stats/tests/reference/r23_stats_oracles.{R,json}`, `d1_poisson_cox_result_sheets`, `r13_2_survival_group_exclusion`, `parity_g8_barnard_pb_mi`, `parity_g3_g7`, `nist_strd`, `engine_smoke`, `prop_stats`, and Wave 5 `G8` Koopman non-bracketing unit tests in `crates/stats/src/categorical/ratios_nnt.rs`; plus companion offline R `4.5.2` + 60-digit Python `decimal.Decimal` transform/special-function D1 oracles in `crates/transform/tests/reference/r23_transform_oracles.{R,py,json}` and CLI/MCP/engine E2E workflows in `curvebook-engine`, `curvebook-mcp`, and `curvebook-cli`, `0` Clippy warnings; current per-crate counts in [`STATUS.md`](../STATUS.md)). Dual Normal/Lognormal Q–Q tabs & plots, Relative Likelihood normality comparison, One-Sample Ratio & Lognormal Welch *t*-tests, Lognormal 1-Way Ordinary/Welch-BF/RM-REML ANOVA, Multiple Row Lognormal/Ratio *t*-tests + FDR/Volcano, 3-way RM + GG/HF sphericity + 4-mode REML variance policy + Spearman heteroscedasticity, 17-index consensus K-means + Gap + Hartigan–Wong/MacQueen + PCA retention, `LogRankSurvival` power (Schoenfeld/Freedman), full Multivariable GLM §5.4, Cox PH covariate-profile survival/Cox–Snell/nested LRT & reverse-KM follow-up, Newman–Keuls, Fisher LSD, `MultipleVariables` two-group *t*-tests and Kaplan–Meier, Bland–Altman, Barnard's exact 2×2 test, Passing–Bablok + CUSUM, MICE Multiple Imputation + Rubin pooling, and Koopman `Option<f64>` `None`-bound handling are all implemented and verified end-to-end.

## Descriptive, Normality & Outlier Identification
- [x] Column descriptive statistics (mean, SD, SEM, arbitrary $0.01\%\text{–}99.99\%$ CI, median + exact binomial CI reporting actual confidence level, IQR, symmetric custom $(100-P)\% / P\%$ percentiles, geometric mean & SD factor, harmonic mean, quadratic mean / RMS + CI, $\% \text{CV}$, skewness $G_1$, excess kurtosis $G_2$, and 3 subcolumn aggregation modes on `XY`/`Grouped` sheets)
- [x] D'Agostino–Pearson omnibus $K^2$, Shapiro–Wilk $W$, Anderson–Darling $A^{*2}$, and Lilliefors Kolmogorov–Smirnov $D$ normality & lognormality battery + Normal vs. Lognormal log-LR & Relative Likelihood (`normality_only` mode supported; verified against `r23_stats_oracles.json`)
- [x] Dual Normal & Lognormal Q–Q expected quantiles *(`qq_expected_quantiles`, `normal-qq` / `lognormal-qq` tabs, and `DiagnosticPlotKind::{NormalQqPlot, LognormalQqPlot}` verified end-to-end)*
- [x] One-sample *t*-test, One-sample ratio *t*-test (`OneSampleRatioT`), and Wilcoxon signed-rank test against hypothetical value $\mu_0$
- [x] Outlier identification: Grubbs' ESD test (single & iterative)
- [x] Outlier identification: ROUT method (Lorentzian/Cauchy robust baseline + $\text{RSDR}$ + Benjamini–Hochberg FDR $Q$)

## Two-Group Comparisons & Standardized Effect Sizes
- [x] Unpaired two-sample Student *t*-test (with *F*-test for equal variances)
- [x] Unpaired Welch *t*-test (unequal variances, Welch–Satterthwaite fractional df)
- [x] Paired *t*-test, Ratio paired *t*-test (geometric mean of ratios), Lognormal unpaired *t*-test, and Lognormal Welch *t*-test (`LognormalWelchT`)
- [x] Standardized *t*-test effect sizes with noncentral-$t$ CIs: Cohen's $d$, Hedges' $g$, and Glass's $\Delta$
- [x] Mann–Whitney $U$ test (exact small-sample & tie-corrected asymptotic *p*-values + Hodges–Lehmann median difference & CI)
- [x] Wilcoxon matched-pairs signed-rank test (exact & asymptotic + Pratt/Wilcoxon zero handling + Hodges–Lehmann pseudomedian)
- [x] Two-sample Kolmogorov–Smirnov test (exact Smirnov lattice & asymptotic)
- [x] Multiple unpaired/paired/lognormal/ratio/nonparametric tests (one per row) with Holm–Šídák or BH/BY/BKY FDR (Volcano plot source) *(`AnalysisKind::MultipleRowTTests` + `multiple_row_t_tests` verified end-to-end against `r23_stats_oracles.json`)*
- [x] Unified *t*-tests directly from `MultipleVariables` tables via typed/color-coded variable assignment *(`TwoGroupSpec` `mv_group_column` + `mv_response_column` and multi-column extraction verified end-to-end)*

## One-Way, Factorial, Multifactor ($N$-Way), Nested & Mixed-Effects Models
- [x] Ordinary one-way ANOVA + Brown–Forsythe and Bartlett variance-homogeneity tests, plus Lognormal one-way Ordinary, Welch/Brown–Forsythe, and RM/REML ANOVA with geometric-mean ratio post-hoc CIs (`LognormalOrdinary`, `LognormalWelchBf`, `LognormalRepeatedMeasures`)
- [x] Brown–Forsythe and Welch one-way ANOVA (unequal SDs)
- [x] Repeated-measures one-way ANOVA with Geisser–Greenhouse $\hat{\varepsilon}$ and Huynh–Feldt $\tilde{\varepsilon}$ sphericity corrections
- [x] Kruskal–Wallis nonparametric one-way test and Friedman nonparametric repeated-measures test
- [x] Ordinary & repeated-measures Two-Way factorial ANOVA (balanced & unbalanced Type I / Type III SS, GG/HF sphericity, 2-way 1-factor-diff post-hoc comparisons, Spearman heteroscedasticity tab & `HomoscedasticityPlot`)
- [x] Three-Way factorial ANOVA ($2 \times 2 \times K$, ordinary & 1-/2-/3-factor repeated measures / mixed REML with canonical `three_way_rm_factors = [factor1_rm, factor2_rm, row_rm]` ordering and GG/HF sphericity)
- [x] Multifactor ($N$-way) GLM ANOVA from `MultipleVariables` tables with arbitrary factor count & interactions
- [x] Standardized ANOVA effect sizes with noncentral-$F$ CIs: $\eta^2$, partial $\eta_p^2$, $\omega^2$, and Cohen's $f$
- [x] Nested *t*-test and Nested one-way ANOVA (hierarchical biological/technical subcolumns)
- [x] Linear Mixed-Effects Model (REML) with Kenward–Roger / Satterthwaite df and 4-mode `RemlZeroVariancePolicy` (`BoundAtZero`, `DropZeroRandomTerm`, `Unbounded`, `LegacyFallback`) for missing-replicate and complete repeated-measures designs

## Multiple Comparisons (Post-Hoc & FDR)
- [x] Tukey HSD / Tukey–Kramer (all pairwise comparisons, exact Studentized range $q$)
- [x] Dunnett's test (each group vs. control, exact multivariate Student *t*)
- [x] Šídák and Bonferroni adjusted comparisons & simultaneous CIs
- [x] Holm–Šídák step-down multiplicity-adjusted *p*-values
- [x] Newman–Keuls and Fisher's unprotected LSD *(`MultiplicityMethod::{NewmanKeuls, FisherLsd}` verified across `doc`, `stats`, and `engine`)*
- [x] Dunnett's T3, Games–Howell, and Tamhane's T2 (unequal variances)
- [x] Dunn's multiple-comparison rank-sum test
- [x] Post-hoc linear trend test across ordered columns
- [x] Benjamini–Hochberg, Benjamini–Yekutieli, and two-stage Benjamini–Krieger–Yekutieli FDR ($q$-value discovery flagging)

## Contingency, Survival, Diagnostic & Multivariate
- [x] Contingency $2 \times 2$ and $R \times C$: Pearson $\chi^2$, Yates correction, likelihood-ratio $G^2$, Fisher's exact test, Barnard's exact $2 \times 2$ test (`ContingencyMethod::BarnardExact`), Cochran–Armitage trend, McNemar's test
- [x] Contingency CIs & effect sizes: Relative Risk (Koopman score CI with Wave 5 `G8` `Option<f64>` `None`/open-bound reporting when bracketing or finiteness fails), Odds Ratio (Baptista–Pike CI), sensitivity/specificity/PPV/NPV (Wilson & Clopper–Pearson CIs), NNT, and $\phi$ / Cramér's $V$ with CIs
- [x] Kaplan–Meier survival (from `Survival` and `MultipleVariables` tables, with custom event/censor codes) with asymmetrical $\log(-\log)$ Greenwood CIs, median survival + ratio of median survival, reverse-KM median follow-up, and number-at-risk table *(verified on both `Survival` and `MultipleVariables` via `mv_time_column` / `mv_status_column` / `mv_group_column` and `r23_stats_oracles.json`)*
- [x] Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon, trend log-rank, multiplicity-adjusted pairwise curve comparisons, and Mantel–Haenszel / log-rank hazard ratios
- [x] Cox proportional hazards regression (Breslow, Efron & Exact ties, formula interactions & transforms, Wald & profile-likelihood CIs, delta-method HR SE, parameter correlation matrix, nested Cox LRT comparison, user covariate-profile survival curves `cox-profile-survival` / `cox_profile:{idx}`, Schoenfeld / Martingale / deviance / covariate / Cox–Snell residuals, `ResultTabKind::BaselineSurvival` & `ResultTabKind::LogMinusLog` tabs verified end-to-end against `r23_stats_oracles.json`)
- [x] Pearson $r$ and Spearman $\rho$ correlation matrices (fast $O(N \log N)$ tie-safe Spearman on large `MultipleVariables` tables)
- [x] Bland–Altman method-comparison bias & $95\%$ limits of agreement *(`AnalysisKind::BlandAltman` `ResultSheet` + `DiagnosticPlotKind::BlandAltman` verified end-to-end)*
- [x] Passing–Bablok method-comparison regression (`AnalysisKind::PassingBablok`) with intercept CI and CUSUM linearity check
- [x] Receiver Operating Characteristic (ROC) curve, AUC, Hanley–McNeil / DeLong SE & CIs, Youden index cutoff table
- [x] Multiple linear, logistic, and Poisson GLMs (full §5.4 suite: 2-/3-way interactions, polynomial/transform terms, `include_intercept`, nested *F* / deviance LRT / $\Delta\text{AIC}_c$, WLS weights, quasi-Poisson dispersion, Wald & profile-likelihood CIs, parameter correlation matrix, leverage / Cook's $D$ / DFBETAs, out-of-sample row prediction, configurable logistic cutoffs, Hosmer–Lemeshow calibration bins, and ROC) plus Multiple Imputation (`MultivariateSpec::MultipleImputation`, MICE chained equations + Rubin pooling)
- [x] Principal Component Analysis (scree, parallel analysis, Kaiser / cumulative-variance / fixed retention rules, loadings, biplots) plus K-means (Lloyd, Hartigan–Wong, MacQueen, 17-index consensus optimal-$k$ voting, Tibshirani Gap statistic, optional cluster ANOVA tab) & hierarchical clustering
- [x] Prospective Power & Sample Size across 9 study-design families including `LogRankSurvival` (Schoenfeld 1983 & Freedman 1982)
- [x] Plain-language clean-room Analysis Checklists across all test families
