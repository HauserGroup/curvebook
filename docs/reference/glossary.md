# Glossary

Concise definitions of scientific, statistical, numerical, graphing, and product concepts used across CurveBook.

---

## A–C {#glossary-a-c}

- **$\text{AIC}_c$ (Corrected Akaike Information Criterion):** Bias-corrected information criterion $\text{AIC}_c = N\ln(\text{SS}/N) + 2K_{\text{eff}} + \frac{2K_{\text{eff}}(K_{\text{eff}}+1)}{N - K_{\text{eff}} - 1}$ ($K_{\text{eff}} = K + 1$) used by CurveBook (`Aicc`) to compare both nested and non-nested candidate models via $\Delta\text{AIC}_c$, Akaike weights, and evidence ratios (Akaike 1974; Hurvich & Tsai 1989; Burnham & Anderson 2002).
- **Alpha ($\alpha$):** Pre-specified Type I family/comparison error threshold (`0.05` default for tests and power planning); corresponds to confidence level $(1 - \alpha) \times 100\%$.
- **ANCOVA (Analysis of Covariance) Line Comparison:** In [`LinearRegression`](registry/tables.html#analysis-kind-linear-regression), $F$-tests comparing fitted slopes and (when slopes do not differ) intercepts/elevations across two or more datasets.
- **Archetype (`TableKind`):** One of CurveBook's eight structural table schemas (`Xy`, `Column`, `Grouped`, `Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`, `Nested`) governing subcolumn interpretation, statistical dispatch, and compatible graph families.
- **AUC (Area Under Curve):** Either the trapezoidal integral along $(X, Y)$ curves (`compute_sheet_auc`; Yeh & Kwan 1978) or the nonparametric area under an empirical Receiver Operating Characteristic curve ([`RocCurve`](registry/tables.html#analysis-kind-roc-curve)).
- **Barnard's Exact Test:** Unconditional exact $2\times 2$ association test maximizing the hypergeometric/binomial *P*-value over the nuisance success probability $\pi \in (0, 1)$ (Barnard 1945).
- **Bland–Altman Plot & Limits of Agreement:** Method-comparison difference plot of $(A - B)$ or $\%$-difference vs. $(A + B)/2$ displaying mean bias, bias CI, and limits of agreement $\bar{d} \pm z_{1-\alpha/2}\text{SD}_d$ (Bland & Altman 1986, 1999).
- **Broken Axis (`AxisSegment`):** Axis divided into `1` to `3` disjoint linear or nonlinear data ranges separated by configurable gap widths and break ornaments (`DoubleSlash`, `Zigzag`, `BlankGap`).
- **Brown–Forsythe Test:** Variance-homogeneity test running one-way ANOVA on absolute deviations from group **medians** (Brown & Forsythe 1974a), alongside the heteroscedastic Brown–Forsythe $F^*$ mean test (Brown & Forsythe 1974b).
- **Calibration (Inverse Interpolation):** Predicting unknown $X$ from observed $Y$ along a fitted curve or straight line; uses exact Fieller (1954) quadratic interval topology for OLS linear fits and root-finding + band intersections for nonlinear fits.
- **Checkpoint (`NamedCheckpoint`):** Full JSON snapshot of a `ProjectDocument` stored explicitly inside the archive (`project.createCheckpoint`), distinct from volatile session undo/redo entries.
- **Compact Letter Display (CLD):** Equivalence-letter assignment (`"a"`, `"ab"`, `"b"`, ...) summarizing pairwise statistical comparisons above plotted groups via Piepho's (2004) insert-and-absorb sweeping algorithm so two groups share a letter if and only if they do not differ significantly.
- **Concordance ($C$-index):** Harrell's discrimination statistic in Cox PH measuring the fraction of comparable subject pairs ordered consistently by predicted risk score (`Harrell et al. 1982`).
- **CVD (Colour-Vision Deficiency) Preview:** Non-destructive canvas viewport simulation (`CanvasPreviewFilter`: Deuteranopia, Protanopia, Tritanopia, Grayscale) computed with Machado (2009) matrices (`QOL-9`).

---

## D–L {#glossary-d-l}

- **DAG (Directed Acyclic Graph):** Acyclic dependency graph linking raw `DataSheet`s, `=` Calculated Variables, `LinkedDataRegion`s, `InfoSheet` constants, `AnalysisSpec` `ResultSheet`s, `HookSource` parameters, `GraphSheet`s, and `LayoutSheet`s (`crates/engine/src/dag.rs`).
- **DeLong Method:** Nonparametric structural-components U-statistic variance estimator for ROC AUC and correlated/uncorrelated two-curve AUC comparisons (DeLong, DeLong & Clarke-Pearson 1988).
- **Deming (Model II) Regression:** Errors-in-variables linear regression minimizing perpendicular (or variance-ratio-weighted) residuals accounting for measurement error in both $X$ and $Y$, reporting Leave-One-Out jackknife SEs (Deming 1943; Linnet 1993).
- **Dependency (Parameter Collinearity):** Multi-parameter redundancy index $R_j^2$ measuring how strongly parameter $\theta_j$ correlates with linear combinations of all other free parameters in nonlinear regression; $\text{Dependency}_j > 0.9999$ flags `FitStatus::Ambiguous`.
- **`EmbeddedImage` (`GraphAnnotation::EmbeddedImage` / `LayoutItem::EmbeddedImage`):** Content-addressed PNG, JPEG, or TIFF raster image (`ProjectImageAsset`, capped at `32 MiB`, `16,384 px` per side, and `67,108,864` decoded pixels) embedded on a `GraphSheet` or `LayoutSheet` with optional `CropInsetsPt`, rotation, aspect locking, and border styling.
- **`EqualizeScaleMode`:** Page-layout scaling rule (`MatchSmallest`, `MatchLargest`, `ExplicitPercent`) used by `layout.equalizeScaling` to normalize inner `plot_rect_pt` data-to-point ratios across multi-panel graphs within `<= 0.25 pt`.
- **`ErrorCode`:** Canonical `snake_case` diagnostic identifier emitted across document, engine, format, CLI, and MCP layers (`bad_input`, `path_escape`, `unsupported_version`, etc.).
- **Exclusion (`Cell.excluded` & `CellExclusionNote`):** Non-destructive observation flag omitting a cell from calculations and curves while preserving raw value and optional `(reason_tag, note, timestamp_utc)` audit provenance (`QOL-3`).
- **Extra-Sum-of-Squares *F*-Test:** Likelihood/variance-ratio *F*-test comparing a simpler nested model against a fuller model (`ExtraSumOfSquaresF`; Motulsky & Christopoulos 2004).
- **FDR (False Discovery Rate):** Expected proportion of false discoveries among rejected hypotheses, controlled via Benjamini–Hochberg (1995), adaptive two-stage Benjamini–Krieger–Yekutieli (2006), Benjamini–Yekutieli (2001), or `ROUT` outlier elimination.
- **`FitIdentifiability` (`NotAssessed`, `Identifiable`, `Ambiguous`):** Post-fit parameter identifiability classification (`crates/fit`) reporting structural/numerical unidentifiability diagnostics when extreme dependency, rank-deficient Jacobians, boundary constraints, or high nonlinearity leave parameters poorly determined.
- **FWER (Family-Wise Error Rate):** Probability of making at least one Type I error across a family of tests, controlled by Bonferroni, Holm, Šídák, Holm–Šídák, Tukey–Kramer, Dunnett, Games–Howell, Newman–Keuls, or Dunn procedures.
- **Geisser–Greenhouse $\hat{\varepsilon}$ / Huynh–Feldt $\tilde{\varepsilon}$:** Sphericity correction multipliers reducing repeated-measures ANOVA numerator and denominator degrees of freedom when compound symmetry fails (Greenhouse & Geisser 1959; Huynh & Feldt 1976; Lecoutre 1991).
- **GLM (Generalized Linear Model):** Exponential-dispersion multivariable regression (`Linear` identity link, `Logistic` logit link, `Poisson` log link) fitted by QR/SVD or IRLS (McCullagh & Nelder 1989).
- **Greenwood Standard Error:** Product-limit variance estimator for Kaplan–Meier survival $S(t)$ yielding symmetric linear or asymmetric complementary log-log `ln(-ln S(t))` confidence bounds (Greenwood 1926; Kalbfleisch & Prentice 2002).
- **Hedges' $g$:** Sample-size bias-corrected standardized mean difference $g = J(df)\,d$ accompanied by exact noncentral-*t* confidence intervals (Hedges 1981; Steiger 2004).
- **Hodges–Lehmann Shift Estimator:** Nonparametric median of all pairwise differences $x_i - y_j$ (Mann–Whitney) or Walsh averages $(d_i + d_j)/2$ (Wilcoxon signed-rank) with exact order-statistic CIs (Hodges & Lehmann 1963; Monahan 1984).
- **`HookSource`:** Reactive scalar reference binding an upstream `InfoSheet` constant or `ResultSheet` scalar dynamically to a curve-fit `ParameterScope::HookedConstant`, transform `NormalizeAnchor`, or graph annotation.
- **Hougaard Skewness ($|g_1|$):** Third-moment nonlinearity curvature measure diagnosing whether parameter sampling distributions deviate from Gaussian symmetry (`Hougaard 1985`; `Ratkowsky 1990`).
- **IRLS (Iteratively Reweighted Least Squares):** Maximum-likelihood / M-estimator optimization loop used by logistic regression, Poisson regression, Poisson nonlinear deviance, and Cauchy robust regression (Green 1984).
- **Kenward–Roger Approximation:** Small-sample denominator degrees-of-freedom and covariance adjustment for unbalanced REML mixed-effects models (Kenward & Roger 1997).
- **Levenberg–Marquardt (Trust-Region):** Damped nonlinear least-squares solver interpolating between Gauss–Newton and gradient-descent trust steps (Levenberg 1944; Marquardt 1963; Moré 1978).

---

## M–Z {#glossary-m-z}

- **Mauchly's $W$:** Likelihood-ratio test of sphericity ($\text{Cov}(\Delta Y) \propto I$) for repeated-measures ANOVA with $k \ge 3$ conditions (Mauchly 1940).
- **MCP (Model Context Protocol):** Open stdio JSON-RPC 2.0 tool/resource/prompt interface exposed by `curvebook-cli mcp` ([Automation: MCP](automation/mcp.md)).
- **MICE PMM (Multiple Imputation by Chained Equations — Predictive Mean Matching):** Iterative imputation of missing `MultipleVariables` entries across $m \ge 2$ completed datasets combined via Rubin (1987) and Barnard–Rubin (1999) pooling.
- **`MultipleVariables` Table:** Tidy observational table (`rows = subjects`, `columns = typed variables`) declaring explicit `Continuous`, `Categorical`, `Label`, or `Calculated` column roles.
- **NIST StRD (Statistical Reference Datasets):** Certified benchmark collections (`univ`, `anova`, `lls`, `nls`) from the U.S. National Institute of Standards and Technology used to certify floating-point and nonlinear solver accuracy (`LRE` digits).
- **$\text{NNTB}$ / $\text{NNTH}$ (Number Needed to Treat / Harm):** Reciprocal absolute risk reduction $1/|\text{RD}|$ with Altman (1998) inverted Newcombe Wilson-score confidence bounds (including exact rays through $\infty$ when RD spans `0`).
- **Passing–Bablok Regression:** Nonparametric median-slope method-comparison regression resistant to outliers with Kolmogorov CUSUM linearity diagnostics (Passing & Bablok 1983).
- **Piepho Algorithm:** See [Compact Letter Display](#glossary-a-c).
- **Profile-Likelihood CI (`Venzon–Moolgavkar`):** Asymmetric likelihood-contour confidence interval traced by holding one parameter fixed and re-optimizing all nuisance parameters until $\text{SS}(\theta_j)$ crosses the critical $F$ or deviance threshold; emits `CiBound::Open` when unbounded (Venzon & Moolgavkar 1988).
- **`QOL-7` Diagnostics-Only Checklist:** Non-blocking diagnostic strips and `ChecklistItem` rows on `ResultSheet`s reporting assumption metrics, statuses, and limits without test recommendations or automatic remediations.
- **Quick Statistics:** Live `n`, `excluded`, `mean`, `SD`, `SEM`, `median`, `min`, and `max` readout over active numeric cells in the current Data grid selection.
- **REML (Restricted Maximum Likelihood):** Unbiased variance-component estimation for linear mixed models supporting `CompoundSymmetry` and `Unstructured` within-subject covariance structures (Patterson & Thompson 1971).
- **ROUT Outlier Elimination:** Two-stage outlier detector pairing Cauchy robust regression, `RSDR` residual scaling, and Benjamini–Hochberg FDR over the furthest ~30% residuals before OLS/WLS inlier refitting (Motulsky & Brown 2006).
- **RSDR (Robust Standard Deviation of Residuals):** Scale estimate derived from the 68.27th percentile of sorted $|r_i|$ multiplied by $N/(N - K)$ for robust regression and ROUT.
- **Schoenfeld Residuals:** Covariate-specific time-indexed Cox PH residuals tested for zero slope vs. transformed time to diagnose proportional-hazards violations (Schoenfeld 1982; Grambsch & Therneau 1994).
- **Subcolumn Mode (`SubcolumnMode`):** Interpretation of subcolumns per dataset (`Single`, `RawReplicates`, `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`).
- **WAL (`CrashRecoveryJournal`):** Directory-backed Write-Ahead Log (`snapshot.curvebook`, CRC-32 `ops.journal`, `curvebook.lock`) preserving unsaved desktop mutations between explicit saves.
- **Welch–Satterthwaite `df`:** Fractional denominator degrees of freedom correcting two-sample *t*-tests and one-way ANOVA under unequal group variances (Welch 1947, 1951; Satterthwaite 1946).
- **Youden's $J$:** Optimal diagnostic cutoff criterion $J = \text{Sensitivity} + \text{Specificity} - 1$ maximized across empirical ROC thresholds (Youden 1950).
