# Glossary

Concise definitions of scientific, statistical, numerical, graphing, and product concepts used across CurveBook.

---

## A–C {#glossary-a-c}

- **$\text{AIC}_c$ (Corrected Akaike Information Criterion):** Bias-corrected criterion $\text{AIC}_c = N\ln(\text{SS}/N) + 2K_{\text{eff}} + \frac{2K_{\text{eff}}(K_{\text{eff}}+1)}{N - K_{\text{eff}} - 1}$ ($K_{\text{eff}} = K + 1$). CurveBook uses $\Delta\text{AIC}_c$, Akaike weights, and evidence ratios to compare nested and non-nested models (Akaike 1974; Hurvich & Tsai 1989; Burnham & Anderson 2002).
- **Alpha ($\alpha$):** Pre-specified Type I error threshold (`0.05` default for tests and power planning). Corresponds to confidence level $(1 - \alpha) \times 100\%$.
- **ANCOVA line comparison:** In [linear regression](registry/tables.html#analysis-kind-linear-regression), $F$-tests comparing fitted slopes and (when slopes match) intercepts across two or more datasets.
- **Archetype:** One of CurveBook's eight structural table schemas (`xy`, `column`, `grouped`, `contingency`, `survival`, `parts_of_whole`, `multiple_variables`, `nested`) governing subcolumns, analyses, and graphs.
- **AUC (Area Under Curve):** Either the trapezoidal integral along $(X, Y)$ curves (Yeh & Kwan 1978) or the nonparametric area under an empirical [ROC curve](registry/tables.html#analysis-kind-roc-curve).
- **Barnard's exact test:** Unconditional exact $2\times 2$ association test maximizing the hypergeometric/binomial *P*-value over nuisance probability $\pi \in (0, 1)$ (Barnard 1945).
- **Bland–Altman plot and limits of agreement:** Method-comparison plot of $(A - B)$ or percentage difference vs. $(A + B)/2$. Displays mean bias, bias CI, and limits of agreement $\bar{d} \pm z_{1-\alpha/2}\text{SD}_d$ (Bland & Altman 1986, 1999).
- **Broken axis:** Axis split into 1 to 3 disjoint linear or nonlinear data ranges separated by configurable gaps and break marks (double slash, zigzag, or blank gap).
- **Brown–Forsythe test:** Variance-homogeneity test running one-way ANOVA on absolute deviations from group medians (Brown & Forsythe 1974a), alongside the heteroscedastic Brown–Forsythe $F^*$ test of means (Brown & Forsythe 1974b).
- **Calibration (inverse interpolation):** Predicting unknown $X$ from observed $Y$ along a fitted line or curve. Linear fits use exact Fieller (1954) intervals; nonlinear fits use root-finding with band intersections.
- **Checkpoint:** Full validated JSON snapshot of a project stored explicitly inside the archive (`project.createCheckpoint`), separate from volatile session undo/redo history.
- **Compact Letter Display (CLD):** Letter groupings (`a`, `ab`, `b`, …) summarizing pairwise comparisons above plotted groups via Piepho's (2004) algorithm. Two groups share a letter when they do not differ significantly.
- **Concordance ($C$-index):** Harrell's discrimination statistic in Cox proportional hazards measuring the fraction of comparable subject pairs ordered consistently by predicted risk (Harrell et al. 1982).
- **CVD (Colour-Vision Deficiency) preview:** Non-destructive canvas viewport simulation (deuteranopia, protanopia, tritanopia, grayscale) using Machado (2009) colour matrices without altering stored exports.

---

## D–L {#glossary-d-l}

- **DAG (Directed Acyclic Graph):** Dependency graph linking raw data tables, `=` Calculated Variables, linked data regions, info constants, results sheets, hooked parameters, graphs, and page layouts.
- **DeLong method:** Nonparametric U-statistic variance estimator for ROC AUC and correlated or uncorrelated two-curve AUC comparisons (DeLong, DeLong & Clarke-Pearson 1988).
- **Deming (Model II) regression:** Errors-in-variables linear regression minimizing variance-ratio-weighted residuals for error in both $X$ and $Y$, reporting leave-one-out jackknife SEs (Deming 1943; Linnet 1993).
- **Dependency (parameter collinearity):** Multi-parameter redundancy index $R_j^2$ measuring how strongly $\theta_j$ correlates with linear combinations of other free parameters; values above 0.9999 flag an ambiguous fit.
- **Diagnostics-only checklist:** Non-blocking diagnostic rows on results sheets that report assumption checks, statuses, and methodological limits without recommending alternative tests or automatic edits.
- **Embedded image:** Content-addressed PNG, JPEG, or TIFF raster image (up to 32 MiB, 16,384 px per side, and 67,108,864 decoded pixels). Images can be placed on a graph or page layout with crop, rotation, aspect lock, and border options.
- **Equalize-scale mode:** Page-layout rule (`match_smallest`, `match_largest`, `explicit_percent`) used by `layout.equalizeScaling` to align inner plot-area data-to-point ratios across multi-panel graphs within 0.25 pt.
- **Error code:** Canonical `snake_case` diagnostic identifier shared across document, engine, format, CLI, and MCP layers (`bad_input`, `path_escape`, `unsupported_version`, etc.).
- **Exclusion:** Non-destructive cell mask omitting an observation from calculations and plotted curves while keeping its raw value and optional reason tag, note, and UTC timestamp.
- **Extra-sum-of-squares *F*-test:** Variance-ratio *F*-test comparing a simpler nested model against a fuller model (Motulsky & Christopoulos 2004).
- **FDR (False Discovery Rate):** Expected proportion of false discoveries among rejected hypotheses, controlled via Benjamini–Hochberg (1995), two-stage Benjamini–Krieger–Yekutieli (2006), Benjamini–Yekutieli (2001), or ROUT outlier elimination.
- **Fit identifiability:** Post-fit classification (`NotAssessed`, `Identifiable`, `Ambiguous`) reporting when extreme dependency, rank-deficient Jacobians, boundary hits, or high nonlinearity leave parameters poorly determined.
- **FWER (Family-Wise Error Rate):** Probability of making at least one Type I error across a family of tests, controlled by Bonferroni, Holm, Šídák, Holm–Šídák, Tukey–Kramer, Dunnett, Games–Howell, Newman–Keuls, or Dunn procedures.
- **Geisser–Greenhouse $\hat{\varepsilon}$ / Huynh–Feldt $\tilde{\varepsilon}$:** Sphericity multipliers adjusting repeated-measures ANOVA degrees of freedom when compound symmetry fails (Greenhouse & Geisser 1959; Huynh & Feldt 1976; Lecoutre 1991).
- **GLM (Generalized Linear Model):** Exponential-dispersion multivariable regression (`linear`, `logistic`, `poisson`) fitted by QR/SVD or IRLS (McCullagh & Nelder 1989).
- **Greenwood standard error:** Product-limit variance estimator for Kaplan–Meier survival $S(t)$ yielding linear or asymmetric complementary log-log `ln(-ln S(t))` confidence bounds (Greenwood 1926; Kalbfleisch & Prentice 2002).
- **Hedges' $g$:** Small-sample bias-corrected standardized mean difference $g = J(df)\,d$ with exact noncentral-*t* confidence intervals (Hedges 1981; Steiger 2004).
- **Hodges–Lehmann shift estimator:** Nonparametric median of pairwise differences $x_i - y_j$ (Mann–Whitney) or Walsh averages $(d_i + d_j)/2$ (Wilcoxon signed-rank) with exact order-statistic CIs (Hodges & Lehmann 1963; Monahan 1984).
- **Hook source:** Reactive scalar link binding an upstream info-sheet constant or result scalar dynamically to a curve-fit constant, normalization anchor, or graph annotation.
- **Hougaard skewness ($|g_1|$):** Third-moment nonlinearity measure diagnosing whether parameter sampling distributions depart from Gaussian symmetry (Hougaard 1985; Ratkowsky 1990).
- **IRLS (Iteratively Reweighted Least Squares):** Optimization loop used by logistic regression, Poisson regression, Poisson nonlinear deviance, and Cauchy robust regression (Green 1984).
- **Kenward–Roger approximation:** Small-sample denominator degrees-of-freedom and covariance adjustment for unbalanced REML mixed-effects models (Kenward & Roger 1997).
- **Levenberg–Marquardt (trust-region):** Damped nonlinear least-squares solver interpolating between Gauss–Newton and gradient-descent trust steps (Levenberg 1944; Marquardt 1963; Moré 1978).

---

## M–Z {#glossary-m-z}

- **Mauchly's $W$:** Likelihood-ratio sphericity test ($\text{Cov}(\Delta Y) \propto I$) for repeated-measures ANOVA with $k \ge 3$ conditions (Mauchly 1940).
- **MCP (Model Context Protocol):** Open stdio JSON-RPC 2.0 tool, resource, and prompt interface exposed by `curvebook-cli mcp` ([Automation: MCP](automation/mcp.md)).
- **MICE PMM (Multiple Imputation by Chained Equations — Predictive Mean Matching):** Iterative imputation of missing Multiple Variables entries across $m \ge 2$ completed datasets. Completed fits are pooled via Rubin (1987) and Barnard–Rubin (1999) rules.
- **Multiple Variables table:** Tidy observational table (`rows = subjects`, `columns = typed variables`) assigning continuous, categorical, label, or calculated variable roles.
- **NIST StRD (Statistical Reference Datasets):** Certified benchmark datasets (`univ`, `anova`, `lls`, `nls`) from the U.S. National Institute of Standards and Technology used to verify floating-point and solver accuracy.
- **$\text{NNTB}$ / $\text{NNTH}$ (Number Needed to Treat / Harm):** Reciprocal absolute risk reduction $1/|\text{RD}|$ with Altman (1998) inverted Newcombe Wilson-score confidence bounds (including rays through $\infty$ when RD spans 0).
- **Passing–Bablok regression:** Nonparametric median-slope method-comparison regression resistant to outliers, paired with Kolmogorov CUSUM linearity diagnostics (Passing & Bablok 1983).
- **Piepho algorithm:** See [Compact Letter Display](#glossary-a-c).
- **Profile-likelihood CI (Venzon–Moolgavkar):** Asymmetric contour confidence interval traced by varying one parameter while re-optimizing nuisance parameters until $\text{SS}(\theta_j)$ crosses the critical $F$ or deviance threshold. Reports an open bound when unbounded (Venzon & Moolgavkar 1988).
- **Quick statistics:** Live readout of active count, excluded count, sum, mean, SD, SEM, median, min, and max over numeric cells in the current table selection.
- **REML (Restricted Maximum Likelihood):** Unbiased variance-component estimator for mixed models supporting compound-symmetry and unstructured within-subject covariance (Patterson & Thompson 1971).
- **ROUT outlier elimination:** Two-stage outlier detector pairing Cauchy robust regression, RSDR residual scaling, and Benjamini–Hochberg FDR across the furthest ~30% of residuals before refitting clean inliers (Motulsky & Brown 2006).
- **RSDR (Robust Standard Deviation of Residuals):** Robust residual scale derived from the 68.27th percentile of sorted $|r_i|$ multiplied by $N/(N - K)$ for robust regression and ROUT.
- **Schoenfeld residuals:** Covariate-specific Cox PH residuals tested for zero slope over transformed time to check the proportional-hazards assumption (Schoenfeld 1982; Grambsch & Therneau 1994).
- **Subcolumn mode:** How replicate subcolumns per dataset are stored (`Single`, `RawReplicates`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`).
- **WAL (Write-Ahead Log):** Directory-backed crash recovery journal (`snapshot.curvebook`, CRC-32 `ops.journal`, `curvebook.lock`) preserving unsaved desktop edits between explicit saves.
- **Welch–Satterthwaite `df`:** Fractional denominator degrees of freedom correcting two-sample *t*-tests and one-way ANOVA under unequal variances (Welch 1947, 1951; Satterthwaite 1946).
- **Youden's $J$:** Optimal ROC cutoff criterion $J = \text{Sensitivity} + \text{Specificity} - 1$ maximized across empirical thresholds (Youden 1950).
