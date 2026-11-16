# Biostatistics & Hypothesis Testing Parity (`curvebook-stats`)

> Governing mathematical & algorithmic specification: [`statistics-implementation.md`](statistics-implementation.md) · Roadmap: [`ROADMAP.md`](ROADMAP.md#r3-core-biostatistics-anovareml-survival-contingency-roc--multivariate) ([`G1`/`G3`/`G7`/`G8` mapping](gaps.md))
>
> **Status:** `curvebook-math` (`L0`), `curvebook-stats` (`L2`), and `curvebook-engine` (`L5`) statistical workflows are implemented and verified (`cargo test -p curvebook-stats --offline`, including `d1_reference_oracles`, `d1_edge_matrix` verified against R 4.5.2, `parity_g8_barnard_pb_mi`, `parity_g3_g7`, `nist_strd`, `engine_smoke`, `prop_stats`, and Wave 5 `G8` Koopman non-bracketing unit tests in `crates/stats/src/categorical.rs`; plus CLI/engine E2E workflows in `curvebook-engine` and `curvebook-cli`, `0` Clippy warnings; current per-crate counts in [`STATUS.md`](STATUS.md)). Normal Q–Q tab/plot, Newman–Keuls, Fisher LSD, Multiple Row *t*-tests + FDR/Volcano, `MultipleVariables` two-group *t*-tests and Kaplan–Meier, Cox `BaselineSurvival`/`LogMinusLog` tabs, Bland–Altman analysis tab, Barnard's exact 2×2 test, Passing–Bablok + CUSUM, MICE Multiple Imputation + Rubin pooling, and Wave 5 `G8` Koopman `Option<f64>` `None`-bound handling are all implemented and verified end-to-end.

## Descriptive, Normality & Outlier Identification
- [x] Column descriptive statistics (mean, SD, SEM, arbitrary $0.01\%\text{–}99.99\%$ CI, median + exact binomial CI reporting actual confidence level, IQR, symmetric custom $(100-P)\% / P\%$ percentiles, geometric mean & SD factor, harmonic mean, quadratic mean / RMS + CI, $\% \text{CV}$, skewness $G_1$, excess kurtosis $G_2$, and 3 subcolumn aggregation modes on `XY`/`Grouped` sheets)
- [x] D'Agostino–Pearson omnibus $K^2$ normality & lognormality test (+ normal vs. lognormal likelihood ratio; `normality_battery` computes both Gaussian & lognormal scores, `ResultSheet` Diagnostics tab emits normal scores + log-LR)
- [x] Shapiro–Wilk $W$ normality & lognormality test
- [x] Anderson–Darling $A^{*2}$ and Kolmogorov–Smirnov (Lilliefors) normality tests
- [x] Q–Q plot expected normal/lognormal quantiles *(`qq_expected_quantiles`, `ResultTabKind::NormalQq`, and `DiagnosticPlotKind::NormalQqPlot` verified end-to-end)*
- [x] One-sample *t*-test and Wilcoxon signed-rank test against hypothetical value $\mu_0$
- [x] Outlier identification: Grubbs' ESD test (single & iterative)
- [x] Outlier identification: ROUT method (Lorentzian/Cauchy robust baseline + $\text{RSDR}$ + Benjamini–Hochberg FDR $Q$)

## Two-Group Comparisons & Standardized Effect Sizes
- [x] Unpaired two-sample Student *t*-test (with *F*-test for equal variances)
- [x] Unpaired Welch *t*-test (unequal variances, Welch–Satterthwaite fractional df)
- [x] Paired *t*-test, Ratio paired *t*-test (geometric mean of ratios), and Lognormal unpaired *t*-test
- [x] Standardized *t*-test effect sizes with noncentral-$t$ CIs: Cohen's $d$, Hedges' $g$, and Glass's $\Delta$ (Prism 11.0+)
- [x] Mann–Whitney $U$ test (exact small-sample & tie-corrected asymptotic *p*-values + Hodges–Lehmann median difference & CI)
- [x] Wilcoxon matched-pairs signed-rank test (exact & asymptotic + Pratt/Wilcoxon zero handling + Hodges–Lehmann pseudomedian)
- [x] Two-sample Kolmogorov–Smirnov test (exact Smirnov lattice & asymptotic)
- [x] Multiple unpaired/paired/nonparametric tests (one per row) with Holm–Šídák or BH/BY/BKY FDR (Volcano plot source) *(`AnalysisKind::MultipleRowTTests` + `multiple_row_t_tests` verified end-to-end)*
- [x] Unified *t*-tests directly from `MultipleVariables` tables via typed/color-coded variable assignment (Prism 11.0/11.1) *(`TwoGroupSpec` `mv_group_column` + `mv_response_column` and multi-column extraction verified end-to-end)*

## One-Way, Factorial, Multifactor ($N$-Way), Nested & Mixed-Effects Models
- [x] Ordinary one-way ANOVA + Brown–Forsythe and Bartlett variance-homogeneity tests
- [x] Brown–Forsythe and Welch one-way ANOVA (unequal SDs, including lognormal mode)
- [x] Repeated-measures one-way ANOVA with Geisser–Greenhouse $\hat{\varepsilon}$ and Huynh–Feldt $\tilde{\varepsilon}$ sphericity corrections
- [x] Kruskal–Wallis nonparametric one-way test and Friedman nonparametric repeated-measures test
- [x] Ordinary & repeated-measures Two-Way factorial ANOVA (balanced & unbalanced Type I / Type III SS)
- [x] Three-Way factorial ANOVA ($2 \times 2 \times K$, ordinary & mixed)
- [x] Multifactor ($N$-way) GLM ANOVA from `MultipleVariables` tables with arbitrary factor count & interactions (Prism 11.0/11.1)
- [x] Standardized ANOVA effect sizes with noncentral-$F$ CIs: $\eta^2$, partial $\eta_p^2$, $\omega^2$, and Cohen's $f$ (Prism 11.0+)
- [x] Nested *t*-test and Nested one-way ANOVA (hierarchical biological/technical subcolumns)
- [x] Linear Mixed-Effects Model (REML) with Kenward–Roger / Satterthwaite df for missing-replicate repeated-measures designs

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
- [x] Contingency CIs & effect sizes: Relative Risk (Koopman score CI with Wave 5 `G8` `Option<f64>` `None`/open-bound reporting when bracketing or finiteness fails), Odds Ratio (Baptista–Pike CI), sensitivity/specificity/PPV/NPV (Wilson & Clopper–Pearson CIs), NNT, and $\phi$ / Cramér's $V$ with CIs (Prism 11.0+)
- [x] Kaplan–Meier survival (from `Survival` and `MultipleVariables` tables) with asymmetrical $\log(-\log)$ Greenwood CIs, median survival, and number-at-risk table *(verified on both `Survival` and `MultipleVariables` via `mv_time_column` / `mv_status_column` / `mv_group_column`)*
- [x] Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon, trend log-rank, multiplicity-adjusted pairwise curve comparisons, and Mantel–Haenszel / log-rank hazard ratios
- [x] Cox proportional hazards regression (Breslow & Efron ties, Schoenfeld residuals, `ResultTabKind::BaselineSurvival` & `ResultTabKind::LogMinusLog` tabs verified end-to-end)
- [x] Pearson $r$ and Spearman $\rho$ correlation matrices (fast $O(N \log N)$ tie-safe Spearman on large `MultipleVariables` tables)
- [x] Bland–Altman method-comparison bias & $95\%$ limits of agreement *(`AnalysisKind::BlandAltman` `ResultSheet` + `DiagnosticPlotKind::BlandAltman` verified end-to-end)*
- [x] Passing–Bablok method-comparison regression (`AnalysisKind::PassingBablok`) with intercept CI and CUSUM linearity check
- [x] Receiver Operating Characteristic (ROC) curve, AUC, Hanley–McNeil / DeLong SE & CIs, Youden index cutoff table
- [x] Multiple linear (full categorical interaction ANOVA tables), multivariable logistic, Poisson regression, and Multiple Imputation (`MultivariateSpec::MultipleImputation`, MICE chained equations + Rubin pooling)
- [x] Principal Component Analysis (scree, parallel analysis, loadings, biplots) plus K-means & hierarchical clustering
- [x] Plain-language clean-room Analysis Checklists across all test families
