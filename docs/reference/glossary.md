# Glossary

<!-- Audience: Tier 2 Technical Reference — concise technical definitions of CurveBook statistical, numerical, graphing, and software terms belong here; link to Tier 1 guides (get-started.md or desktop-gui.md) for scientist walkthroughs. -->

This glossary defines the statistical, numerical, graphing, and software terms used throughout CurveBook's reference manual. If you are new to CurveBook, start with [Get started and installation](get-started.md) or the [Desktop GUI guide](desktop-gui.md).

---

## A–C {#glossary-a-c}

- **$\text{AIC}_c$ (Corrected Akaike Information Criterion):** A bias-corrected model selection score, $\text{AIC}_c = N\ln(\text{SS}/N) + 2K_{\text{eff}} + \frac{2K_{\text{eff}}(K_{\text{eff}}+1)}{N - K_{\text{eff}} - 1}$, where $K_{\text{eff}} = K + 1$. CurveBook reports $\Delta\text{AIC}_c$, Akaike weights, and evidence ratios for both nested and non-nested models (Akaike 1974; Hurvich & Tsai 1989; Burnham & Anderson 2002).
- **Alpha ($\alpha$):** The pre-specified Type I error threshold, `0.05` by default for tests and power planning. It corresponds to a confidence level of $(1 - \alpha) \times 100\%$.
- **ANCOVA line comparison:** In [linear regression](registry/tables.html#analysis-kind-linear-regression), a set of $F$-tests that compare fitted slopes across two or more datasets. When the slopes match, a second $F$-test compares the intercepts.
- **Archetype:** One of CurveBook's eight structural table schemas (`xy`, `column`, `grouped`, `contingency`, `survival`, `parts_of_whole`, `multiple_variables`, `nested`). The archetype determines a table's subcolumn layout, eligible analyses, and graph families.
- **AUC (Area Under Curve):** Either the trapezoidal integral under an $(X, Y)$ curve (Yeh & Kwan 1978) or the nonparametric area under an empirical [ROC curve](registry/tables.html#analysis-kind-roc-curve).
- **Barnard's exact test:** An unconditional exact test of association in a $2\times 2$ table. It maximizes the hypergeometric or binomial *P*-value over the nuisance probability $\pi \in (0, 1)$ (Barnard 1945).
- **Bland–Altman plot and limits of agreement:** A method-comparison plot of the difference $(A - B)$, or the percentage difference, against the mean $(A + B)/2$. It shows the mean bias, the bias CI, and the limits of agreement $\bar{d} \pm z_{1-\alpha/2}\text{SD}_d$ (Bland & Altman 1986, 1999).
- **Broken axis:** An axis split into 1 to 3 disjoint data ranges, each of which can be linear or nonlinear. Configurable gaps and break marks (double slash, zigzag, or blank gap) separate the segments.
- **Brown–Forsythe test:** A test of equal variances that runs a one-way ANOVA on the absolute deviations from the group medians (Brown & Forsythe 1974a). The name also refers to the Brown–Forsythe $F^*$ test of group means under unequal variances (Brown & Forsythe 1974b).
- **Calibration (inverse interpolation):** Predicting an unknown $X$ from an observed $Y$ with a fitted line or curve. Linear fits use exact Fieller (1954) intervals, and nonlinear fits use root finding with band intersections.
- **Checkpoint:** A full, validated JSON snapshot of a project, stored inside the project archive (`project.createCheckpoint`). Checkpoints persist across sessions and are kept separate from the volatile undo/redo history.
- **Compact Letter Display (CLD):** Letter groupings (`a`, `ab`, `b`, …) shown above plotted groups to summarize pairwise comparisons, computed with Piepho's (2004) algorithm. Two groups share a letter when they do not differ significantly.
- **Concordance ($C$-index):** Harrell's discrimination statistic for Cox proportional hazards models. It measures the fraction of comparable subject pairs that the predicted risk orders consistently (Harrell et al. 1982).
- **CVD (Colour-Vision Deficiency) preview:** A non-destructive simulation of colour-vision deficiency (deuteranopia, protanopia, tritanopia, or grayscale) in the canvas viewport, based on the Machado (2009) matrices. Stored figure colours and exports stay unchanged.

---

## D–L {#glossary-d-l}

- **DAG (Directed Acyclic Graph):** The dependency graph that links data tables, `=` Calculated Variables, linked data regions, info constants, results sheets, hooked parameters, graphs, and page layouts.
- **DeLong method:** A nonparametric U-statistic estimator of the variance of a ROC AUC. It supports comparisons of two AUCs from either correlated or uncorrelated curves (DeLong, DeLong & Clarke-Pearson 1988).
- **Deming (Model II) regression:** Errors-in-variables linear regression for data with error in both $X$ and $Y$. It minimizes residuals weighted by the error-variance ratio and reports leave-one-out jackknife SEs (Deming 1943; Linnet 1990).
- **Dependency (parameter collinearity):** An index of multi-parameter redundancy, $R_j^2$, that measures how strongly $\theta_j$ correlates with linear combinations of the other free parameters. Values above 0.9999 flag an ambiguous fit.
- **Diagnostics-only checklist:** Non-blocking diagnostic rows on results sheets that report assumption checks, statuses, and methodological limits. CurveBook never recommends replacement tests or makes automatic edits.
- **Embedded image:** A content-addressed PNG, JPEG, or TIFF raster image of up to 32 MiB, 16,384 px per side, and 67,108,864 decoded pixels. You can place images on graphs or page layouts with crop, rotation, aspect-lock, and border options.
- **Equalize-scale mode:** A page-layout rule (`match_smallest`, `match_largest`, `explicit_percent`) used by `layout.equalizeScaling`. It aligns the data-to-point ratios of the inner plot areas across multi-panel graphs to within 0.25 pt.
- **Error code:** A standard `snake_case` diagnostic identifier shared by the document, engine, format, CLI, and MCP layers, such as `bad_input`, `path_escape`, or `unsupported_version`.
- **Exclusion:** A non-destructive cell mask that omits an observation from calculations and plotted curves. The raw value stays intact, together with optional reason tags, notes, and UTC timestamps.
- **Extra-sum-of-squares *F*-test:** A variance-ratio *F*-test that compares a simpler nested model with a fuller model (Motulsky & Christopoulos 2004).
- **FDR (False Discovery Rate):** The expected proportion of false discoveries among the rejected hypotheses. CurveBook controls it with Benjamini–Hochberg (1995), two-stage Benjamini–Krieger–Yekutieli (2006), Benjamini–Yekutieli (2001), or ROUT outlier elimination.
- **Fit identifiability:** A post-fit status (`NotAssessed`, `Identifiable`, or `Ambiguous`). It flags poorly determined parameters caused by extreme dependency, rank-deficient Jacobians, boundary hits, or high nonlinearity.
- **FWER (Family-Wise Error Rate):** The probability of making at least one Type I error across a family of tests. It is controlled by the Bonferroni, Holm, Šídák, Holm–Šídák, Tukey–Kramer, Dunnett, Games–Howell, Newman–Keuls, or Dunn procedures.
- **Geisser–Greenhouse $\hat{\varepsilon}$ / Huynh–Feldt $\tilde{\varepsilon}$:** Sphericity multipliers that adjust the degrees of freedom of a repeated-measures ANOVA when compound symmetry fails (Greenhouse & Geisser 1959; Huynh & Feldt 1976; Lecoutre 1991).
- **GLM (Generalized Linear Model):** Multivariable regression for exponential-dispersion families (`linear`, `logistic`, `poisson`), fitted by QR/SVD or IRLS (McCullagh & Nelder 1989).
- **Greenwood standard error:** The product-limit variance estimator for the Kaplan–Meier survival estimate $S(t)$. It yields either linear or asymmetric complementary log-log (`ln(-ln S(t))`) confidence bounds (Greenwood 1926; Kalbfleisch & Prentice 2002).
- **Hedges' $g$:** A standardized mean difference with a small-sample bias correction, $g = J(df)\,d$. CurveBook pairs it with exact noncentral-*t* confidence intervals (Hedges 1981; Steiger 2004).
- **Hodges–Lehmann shift estimator:** A nonparametric shift estimate: the median of the pairwise differences $x_i - y_j$ (Mann–Whitney) or of the Walsh averages $(d_i + d_j)/2$ (Wilcoxon signed-rank). CurveBook reports exact order-statistic CIs for it (Hodges & Lehmann 1963; Monahan 1984).
- **Hook source:** A reactive link that feeds an upstream info-sheet constant or result scalar into a curve-fit constant, a normalization anchor, or a graph annotation.
- **Hougaard skewness ($|g_1|$):** A third-moment measure of nonlinearity that shows whether a parameter's sampling distribution departs from Gaussian symmetry (Hougaard 1985; Ratkowsky 1990).
- **IRLS (Iteratively Reweighted Least Squares):** The optimization loop used by logistic regression, Poisson regression, nonlinear Poisson deviance fitting, and robust Cauchy regression (Green 1984).
- **Kenward–Roger approximation:** A small-sample adjustment to the denominator degrees of freedom and the covariance for unbalanced REML mixed-effects models (Kenward & Roger 1997).
- **Levenberg–Marquardt (trust-region):** A damped nonlinear least-squares solver whose trust-region steps interpolate between Gauss–Newton and gradient-descent steps (Levenberg 1944; Marquardt 1963; Moré 1978).

---

## M–Z {#glossary-m-z}

- **Mauchly's $W$:** A likelihood-ratio test of sphericity ($\text{Cov}(\Delta Y) \propto I$) for repeated-measures ANOVA with $k \ge 3$ conditions (Mauchly 1940).
- **MCP (Model Context Protocol):** An open JSON-RPC 2.0 interface over stdio for tools, resources, and prompts. CurveBook provides it through `curvebook-cli mcp` ([Automation: MCP](automation/mcp.md)).
- **MICE PMM (Multiple Imputation by Chained Equations — Predictive Mean Matching):** Iterative imputation of missing entries in a Multiple Variables table, producing $m \ge 2$ completed datasets. The fits to the completed datasets are pooled with the Rubin (1987) and Barnard–Rubin (1999) rules.
- **Multiple Variables table:** A tidy observational table with one row per subject and one column per typed variable (`rows = subjects`, `columns = typed variables`). Each variable takes a continuous, categorical, label, or calculated role.
- **NIST StRD (Statistical Reference Datasets):** Certified benchmark datasets (`univ`, `anova`, `lls`, `nls`) from the U.S. National Institute of Standards and Technology. CurveBook verifies its floating-point and solver accuracy against them.
- **$\text{NNTB}$ / $\text{NNTH}$ (Number Needed to Treat / Harm):** The reciprocal of the absolute risk reduction, $1/|\text{RD}|$. Its confidence bounds are inverted Newcombe Wilson-score bounds (Altman 1998), which include rays through $\infty$ when RD spans 0.
- **Passing–Bablok regression:** A nonparametric, outlier-resistant method-comparison regression based on median slopes. CurveBook pairs it with Kolmogorov CUSUM linearity diagnostics (Passing & Bablok 1983).
- **Piepho algorithm:** See [Compact Letter Display](#glossary-a-c).
- **Profile-likelihood CI (Venzon–Moolgavkar):** An asymmetric contour confidence interval, traced by varying one parameter and re-optimizing the nuisance parameters until $\text{SS}(\theta_j)$ crosses the critical $F$ or deviance threshold. When an endpoint is unbounded, CurveBook reports an open bound (Venzon & Moolgavkar 1988).
- **Quick statistics:** A live readout of the active count, excluded count, sum, mean, SD, SEM, median, minimum, and maximum of the numeric cells in the current table selection.
- **REML (Restricted Maximum Likelihood):** An unbiased variance-component estimator for mixed models. CurveBook supports compound-symmetry and unstructured within-subject covariance (Patterson & Thompson 1971).
- **ROUT outlier elimination:** A three-stage outlier method. It fits a robust Cauchy regression, applies Benjamini–Hochberg FDR control to the ~30% of RSDR-scaled residuals furthest from the curve, and then refits the remaining inliers by least squares (Motulsky & Brown 2006).
- **RSDR (Robust Standard Deviation of Residuals):** A robust residual scale equal to the 68.27th percentile of the sorted $|r_i|$, multiplied by $N/(N - K)$. Robust regression and ROUT use it to scale residuals.
- **Schoenfeld residuals:** Covariate-specific residuals from a Cox proportional hazards model, tested for zero slope over transformed time. The test checks the proportional-hazards assumption (Schoenfeld 1982; Grambsch & Therneau 1994).
- **Subcolumn mode:** The way replicate subcolumns are stored for each dataset: `Single`, `RawReplicates`, `MeanSd`, `MeanSem`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr`.
- **WAL (Write-Ahead Log):** A directory-based crash-recovery journal (`snapshot.curvebook`, CRC-32 `ops.journal`, `curvebook.lock`). It preserves unsaved desktop edits between explicit saves.
- **Welch–Satterthwaite `df`:** Fractional denominator degrees of freedom that correct two-sample *t*-tests and one-way ANOVA for unequal variances (Welch 1947, 1951; Satterthwaite 1946).
- **Youden's $J$:** The ROC cutoff criterion $J = \text{Sensitivity} + \text{Specificity} - 1$; the optimal cutoff maximizes $J$ across the empirical thresholds (Youden 1950).
