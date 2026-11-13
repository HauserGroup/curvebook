# Biostatistics & Hypothesis Testing Parity (`lumencraft-stats`)

> Governing mathematical & algorithmic specification: [`statistics-implementation.md`](statistics-implementation.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G1`, `G3`, `G7`, `G8`)
>
> **Status:** `lumencraft-math` (`L0`), `lumencraft-stats` (`L2`), and `lumencraft-engine` (`L5`) statistical workflows are implemented and verified (`cargo test -p lumencraft-stats --offline`: `36 passed, 0 failed`, including `20` D1/textbook reference tests, `4` NIST StRD suites, `4` engine smoke tests, and `6` proptest suites; plus CLI/engine E2E workflows in `e2e_workflows` and `lumencraft-cli`, 0 Clippy warnings). Checked boxes (`[x]`) require end-to-end `AnalysisSpec` → `engine` → `ResultSheet` coverage; library-only helpers or partially wired items remain `[ ]` with notes below. Note: Barnard's exact test, Passing–Bablok regression, and Multiple Imputation are unimplemented (outside current Prism 11 milestone scope).

## Descriptive, Normality & Outlier Identification
- [x] Column descriptive statistics (mean, SD, SEM, arbitrary $0.01\%\text{–}99.99\%$ CI, median + exact binomial CI reporting actual confidence level, IQR, symmetric custom $(100-P)\% / P\%$ percentiles, geometric mean & SD factor, harmonic mean, quadratic mean / RMS + CI, $\% \text{CV}$, skewness $G_1$, excess kurtosis $G_2$, and 3 subcolumn aggregation modes on `XY`/`Grouped` sheets)
- [x] D'Agostino–Pearson omnibus $K^2$ normality & lognormality test (+ normal vs. lognormal likelihood ratio; `normality_battery` computes both Gaussian & lognormal scores, `ResultSheet` Diagnostics tab emits normal scores + log-LR)
- [x] Shapiro–Wilk $W$ normality & lognormality test
- [x] Anderson–Darling $A^{*2}$ and Kolmogorov–Smirnov (Lilliefors) normality tests
- [ ] Q–Q plot expected normal/lognormal quantiles *(library function `qq_expected_quantiles` implemented & tested in `crates/stats/src/column.rs`; `ResultSheet` Q–Q coordinates tab / dedicated Q–Q plot kind pending)*
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
- [ ] Multiple unpaired/paired/nonparametric tests (one per row) with Holm–Šídák or BH/BY/BKY FDR (Volcano plot source) *(`multiple_row_t_tests` implemented in `crates/stats/src/two_group.rs`; `AnalysisKind` / engine wiring pending)*
- [ ] Unified *t*-tests directly from `MultipleVariables` tables via typed/color-coded variable assignment (Prism 11.0/11.1) *(multi-column numeric extraction works on `MultipleVariables`; `TwoGroupSpec` does not yet take a separate categorical grouping column + response column selector)*

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
- [ ] Newman–Keuls and Fisher's unprotected LSD *(`MultiplicityProcedure::{NewmanKeuls, FisherLsd}` implemented in `crates/stats/src/multiplicity.rs`; not yet exposed on `doc::MultiplicityMethod` / engine commands)*
- [x] Dunnett's T3, Games–Howell, and Tamhane's T2 (unequal variances)
- [x] Dunn's multiple-comparison rank-sum test
- [x] Post-hoc linear trend test across ordered columns
- [x] Benjamini–Hochberg, Benjamini–Yekutieli, and two-stage Benjamini–Krieger–Yekutieli FDR ($q$-value discovery flagging)

## Contingency, Survival, Diagnostic & Multivariate
- [x] Contingency $2 \times 2$ and $R \times C$: Pearson $\chi^2$, Yates correction, likelihood-ratio $G^2$, Fisher's exact test, Cochran–Armitage trend, McNemar's test
- [x] Contingency CIs & effect sizes: Relative Risk (Koopman score CI), Odds Ratio (Baptista–Pike CI), sensitivity/specificity/PPV/NPV (Wilson & Clopper–Pearson CIs), NNT, and $\phi$ / Cramér's $V$ with CIs (Prism 11.0+)
- [ ] Kaplan–Meier survival (from `Survival` and `MultipleVariables` tables) with asymmetrical $\log(-\log)$ Greenwood CIs, median survival, and number-at-risk table *(verified end-to-end on `Survival` tables; `MultipleVariables` input currently routes `CoxProportionalHazards` and returns `IncompatibleTable` for `KaplanMeier`)*
- [x] Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon, trend log-rank, multiplicity-adjusted pairwise curve comparisons, and Mantel–Haenszel / log-rank hazard ratios
- [x] Cox proportional hazards regression (Breslow & Efron ties, Schoenfeld residuals, baseline survival & log-minus-log plots) *(Cox PH Breslow/Efron + Schoenfeld diagnostics verified end-to-end; dedicated `ResultSheet` baseline-survival / log-minus-log plot tabs pending)*
- [x] Pearson $r$ and Spearman $\rho$ correlation matrices (fast $O(N \log N)$ tie-safe Spearman on large `MultipleVariables` tables)
- [ ] Bland–Altman method-comparison bias & $95\%$ limits of agreement *(`two_group::bland_altman` library function + `DiagnosticPlotKind::BlandAltman` chart verified; standalone `AnalysisKind` `ResultSheet` command not yet wired)*
- [x] Receiver Operating Characteristic (ROC) curve, AUC, Hanley–McNeil / DeLong SE & CIs, Youden index cutoff table
- [x] Multiple linear (full categorical interaction ANOVA tables), multivariable logistic, and Poisson regression
- [x] Principal Component Analysis (scree, parallel analysis, loadings, biplots) plus K-means & hierarchical clustering
- [x] Plain-language clean-room Analysis Checklists across all test families
