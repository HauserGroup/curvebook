# Biostatistics & Hypothesis Testing Parity (`lumencraft-stats`)

> Governing mathematical & algorithmic specification: [`statistics-implementation.md`](statistics-implementation.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G1`, `G3`, `G7`, `G8`)
>
> **`L0` numerical foundations (`G1`):** `crates/math` and `crates/testkit` now provide the underlying cancellation-free incomplete beta/gamma functions, fractional-df `t`/`F`/$\chi^2$/normal distributions, noncentral `t`/`F`/$\chi^2$ confidence-interval solvers, Hartley/Copenhaver–Holland Studentized range $q(k,\nu)$ & $q(r,c,\nu)$ ($2 \le k \le 200$), exact equal/unequal-$n$ multivariate-*t* Dunnett integrals, compensated `SummaryMoments`, rank-revealing Householder QR & SVD, seeded `DeterministicRng`, unified `ConfidenceLevel` / `PValueFormat` types, and the NIST StRD `.dat` oracle. Checkboxes below remain unchecked until end-to-end `crates/stats` analyses, engine commands, and result sheets land (`G3`, `G7`, `G8`).

## Descriptive, Normality & Outlier Identification
- [ ] Column descriptive statistics (mean, SD, SEM, arbitrary $0.01\%\text{–}99.99\%$ CI, median + exact binomial CI reporting actual confidence level, IQR, symmetric custom $(100-P)\% / P\%$ percentiles, geometric mean & SD factor, harmonic mean, quadratic mean / RMS + CI, $\% \text{CV}$, skewness $G_1$, excess kurtosis $G_2$, and 3 subcolumn aggregation modes on `XY`/`Grouped` sheets)
- [ ] D'Agostino–Pearson omnibus $K^2$ normality & lognormality test (+ normal vs. lognormal likelihood ratio)
- [ ] Shapiro–Wilk $W$ normality & lognormality test
- [ ] Anderson–Darling $A^{*2}$ and Kolmogorov–Smirnov (Lilliefors) normality tests
- [ ] Q–Q plot expected normal/lognormal quantiles
- [ ] One-sample *t*-test and Wilcoxon signed-rank test against hypothetical value $\mu_0$
- [ ] Outlier identification: Grubbs' ESD test (single & iterative)
- [ ] Outlier identification: ROUT method (Lorentzian/Cauchy robust baseline + $\text{RSDR}$ + Benjamini–Hochberg FDR $Q$)

## Two-Group Comparisons & Standardized Effect Sizes
- [ ] Unpaired two-sample Student *t*-test (with *F*-test for equal variances)
- [ ] Unpaired Welch *t*-test (unequal variances, Welch–Satterthwaite fractional df)
- [ ] Paired *t*-test, Ratio paired *t*-test (geometric mean of ratios), and Lognormal unpaired *t*-test
- [ ] Standardized *t*-test effect sizes with noncentral-$t$ CIs: Cohen's $d$, Hedges' $g$, and Glass's $\Delta$ (Prism 11.0+)
- [ ] Mann–Whitney $U$ test (exact small-sample & tie-corrected asymptotic *p*-values + Hodges–Lehmann median difference & CI)
- [ ] Wilcoxon matched-pairs signed-rank test (exact & asymptotic + Pratt/Wilcoxon zero handling + Hodges–Lehmann pseudomedian)
- [ ] Two-sample Kolmogorov–Smirnov test (exact Smirnov lattice & asymptotic)
- [ ] Multiple unpaired/paired/nonparametric tests (one per row) with Holm–Šídák or BH/BY/BKY FDR (Volcano plot source)
- [ ] Unified *t*-tests directly from `MultipleVariables` tables via typed/color-coded variable assignment (Prism 11.0/11.1)

## One-Way, Factorial, Multifactor ($N$-Way), Nested & Mixed-Effects Models
- [ ] Ordinary one-way ANOVA + Brown–Forsythe and Bartlett variance-homogeneity tests
- [ ] Brown–Forsythe and Welch one-way ANOVA (unequal SDs, including lognormal mode)
- [ ] Repeated-measures one-way ANOVA with Geisser–Greenhouse $\hat{\varepsilon}$ and Huynh–Feldt $\tilde{\varepsilon}$ sphericity corrections
- [ ] Kruskal–Wallis nonparametric one-way test and Friedman nonparametric repeated-measures test
- [ ] Ordinary & repeated-measures Two-Way factorial ANOVA (balanced & unbalanced Type I / Type III SS)
- [ ] Three-Way factorial ANOVA ($2 \times 2 \times K$, ordinary & mixed)
- [ ] Multifactor ($N$-way) GLM ANOVA from `MultipleVariables` tables with arbitrary factor count & interactions (Prism 11.0/11.1)
- [ ] Standardized ANOVA effect sizes with noncentral-$F$ CIs: $\eta^2$, partial $\eta_p^2$, $\omega^2$, and Cohen's $f$ (Prism 11.0+)
- [ ] Nested *t*-test and Nested one-way ANOVA (hierarchical biological/technical subcolumns)
- [ ] Linear Mixed-Effects Model (REML) with Kenward–Roger / Satterthwaite df for missing-replicate repeated-measures designs

## Multiple Comparisons (Post-Hoc & FDR)
- [ ] Tukey HSD / Tukey–Kramer (all pairwise comparisons, exact Studentized range $q$)
- [ ] Dunnett's test (each group vs. control, exact multivariate Student *t*)
- [ ] Šídák and Bonferroni adjusted comparisons & simultaneous CIs
- [ ] Holm–Šídák step-down multiplicity-adjusted *p*-values
- [ ] Newman–Keuls and Fisher's unprotected LSD
- [ ] Dunnett's T3, Games–Howell, and Tamhane's T2 (unequal variances)
- [ ] Dunn's multiple-comparison rank-sum test
- [ ] Post-hoc linear trend test across ordered columns
- [ ] Benjamini–Hochberg, Benjamini–Yekutieli, and two-stage Benjamini–Krieger–Yekutieli FDR ($q$-value discovery flagging)

## Contingency, Survival, Diagnostic & Multivariate
- [ ] Contingency $2 \times 2$ and $R \times C$: Pearson $\chi^2$, Yates correction, likelihood-ratio $G^2$, Fisher's exact test, Cochran–Armitage trend, McNemar's test
- [ ] Contingency CIs & effect sizes: Relative Risk (Koopman score CI), Odds Ratio (Baptista–Pike CI), sensitivity/specificity/PPV/NPV (Wilson & Clopper–Pearson CIs), NNT, and $\phi$ / Cramér's $V$ with CIs (Prism 11.0+)
- [ ] Kaplan–Meier survival (from `Survival` and `MultipleVariables` tables) with asymmetrical $\log(-\log)$ Greenwood CIs, median survival, and number-at-risk table
- [ ] Log-rank (Mantel–Cox), Gehan–Breslow–Wilcoxon, trend log-rank, multiplicity-adjusted pairwise curve comparisons, and Mantel–Haenszel / log-rank hazard ratios
- [ ] Cox proportional hazards regression (Breslow & Efron ties, Schoenfeld residuals, baseline survival & log-minus-log plots)
- [ ] Pearson $r$ and Spearman $\rho$ correlation matrices (fast $O(N \log N)$ tie-safe Spearman on large `MultipleVariables` tables)
- [ ] Bland–Altman method-comparison bias & $95\%$ limits of agreement
- [ ] Receiver Operating Characteristic (ROC) curve, AUC, Hanley–McNeil / DeLong SE & CIs, Youden index cutoff table
- [ ] Multiple linear (full categorical interaction ANOVA tables), multivariable logistic, and Poisson regression
- [ ] Principal Component Analysis (scree, parallel analysis, loadings, biplots) plus K-means & hierarchical clustering
- [ ] Plain-language clean-room Analysis Checklists across all test families
