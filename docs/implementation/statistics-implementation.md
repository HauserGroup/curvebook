# Biostatistics & Numerical Foundations — Implementation Guidelines

Governing specification for `crates/math` (`L0`) and `crates/stats` (`L2`). Parity checklist: [`evidence/statistics.md`](../evidence/statistics.md).

Primary literature & numerical references (exact equations, polynomial approximations & test vectors):
- **Oracle benchmarks & R cross-validation:** NIST Statistical Reference Datasets (StRD), Statistical Methods (`crates/stats/tests/nist_strd.rs`), and R `4.5.2` independent reference oracles (`crates/stats/tests/d1_edge_matrix.rs`).
- **Special functions & Studentized range:** Abramowitz & Stegun; NIST DLMF; Copenhaver & Holland (1988) *J. Stat. Comput. Simul.* 30:1–15 (`10.1080/00949658808811082`).
- **Normality & outliers:** Dallal & Wilkinson (1986) *Am. Stat.* 40:294–296 (`10.1080/00031305.1986.10475419`); D'Agostino, Belanger & D'Agostino (1990) *Am. Stat.* 44:316–321 (`10.1080/00031305.1990.10475751`); Royston (1992) *Stat. Comput.* 2:117–119 (`10.1007/BF01891203`) & Royston (1995) *Appl. Stat.* 44:547–551 (`10.2307/2986146`); Motulsky & Brown (2006) *BMC Bioinformatics* 7:123 (`10.1186/1471-2105-7-123`).
- **Multiplicity, REML mixed models & ROC:** Benjamini, Krieger & Yekutieli (2006) *Biometrika* 93:491–507 (`10.1093/biomet/93.3.491`); Kenward & Roger (1997) *Biometrics* 53:983–997 (`10.2307/2533558`); DeLong, DeLong & Clarke-Pearson (1988) *Biometrics* 44:837–845 (`10.2307/2531595`).
- **Contingency CIs & survival:** Baptista & Pike (1977) *Appl. Stat.* 26:214–220 (`10.2307/2347041`); Koopman (1984) *Biometrics* 40:513–517 (`10.2307/2531405`); Newcombe (1998) *Stat. Med.* 17:873–890 (`10.1002/(SICI)1097-0258(19980430)17:8<873::AID-SIM779>3.0.CO;2-I`); Klein & Moeschberger, *Survival Analysis*.
- **Further sources cited in `crates/math` / `crates/stats` doc comments:** Wilson (1927) and Clopper & Pearson (1934) binomial intervals; Boschloo (1970) and Suissa & Shuster (1985) unconditional `2x2` tests; Brookmeyer & Crowley (1982) median-survival CI; Grambsch & Therneau (1994) scaled Schoenfeld test; Passing & Bablok (1983); Rubin (1987) and Barnard & Rubin (1999) imputation pooling; Lenth (1989, AS 243) and Benton & Krishnamoorthy (2003) noncentral distributions; Dunnett (1955, 1964).

---

## 1. Numerical Foundations (`crates/math`)

### 1.1 Tail-Safe Distribution Primitives

Never compute small upper-tail probabilities via `1.0 - cdf(x)` (which loses all significant bits below $10^{-16}$ and collapses to `0.0`). Every continuous distribution in `curvebook-math` exposes direct lower-tail `cdf`, upper-tail `sf`, and log-survival `ln_sf` evaluators backed by continued fractions and asymptotic expansions:

- **Two-sided Student $t$ probability** ($\nu > 0$ real, supporting Welch–Satterthwaite fractional degrees of freedom):
  $$p_{2}(t, \nu) = I_{\frac{\nu}{\nu + t^2}}\!\left(\frac{\nu}{2}, \frac{1}{2}\right)$$
  where $I_x(a,b)$ is the regularized incomplete beta function evaluated via Lentz's modified continued fraction with symmetry reflection when $x > \frac{a+1}{a+b+2}$.
- **Upper-tail Snedecor $F$ probability** ($\nu_1 > 0, \nu_2 > 0$):
  $$p(F, \nu_1, \nu_2) = I_{\frac{\nu_2}{\nu_2 + \nu_1 F}}\!\left(\frac{\nu_2}{2}, \frac{\nu_1}{2}\right)$$
- **Upper-tail $\chi^2$ probability** ($\nu > 0$):
  $$p(\chi^2, \nu) = Q\!\left(\frac{\nu}{2}, \frac{\chi^2}{2}\right)$$
  where $Q(a, x) = \Gamma(a, x)/\Gamma(a)$ is the upper regularized incomplete gamma function (series expansion for $x < a + 1$, Legendre continued fraction for $x \ge a + 1$, Temme uniform asymptotic expansion for large $a$).
- **Standard normal two-sided probability:** $p_2(z) = \operatorname{erfc}\!\left(|z| / \sqrt{2}\right)$.
- **Studentized range distribution $q(k, \nu)$** (for Tukey HSD and Newman–Keuls): Gauss–Legendre quadrature over the inner normal integral + outer $\chi$ density (Copenhaver & Holland 1988), accurate to $\ge 10$ digits for $2 \le k \le 200$.
- **Multivariate Student $t$ integral** (for Dunnett's many-to-one test with unequal group sizes): 1D Gauss–Legendre quadrature (`curvebook_math::quad::gauss_legendre_32`, `crates/math/src/dist/dunnett.rs`) conditioning over the shared control group normal variate + common $\chi_\nu$ scale factor, yielding exact Dunnett *p*-values and critical values without table-interpolation caps.

### 1.2 Arbitrary Confidence Levels & P-Value Formatting

- **Confidence levels:** All confidence-interval routines accept any `ConfidenceLevel` built via `ConfidenceLevel::from_percent` in `[0.01, 99.99]` (`alpha = (100 - percent) / 100`, `crates/math/src/pvalue.rs`), supporting $\alpha = 0.001$ ($99.9\%$ CI) and arbitrary decimal levels.
- **P-value display presets (`PValueFormat`):**
  - `FourDecimal` (default): leading zero, 4 decimal places (`0.1234`), floor `< 0.0001`, up to 4 stars (`ns` $> 0.05$, `*` $\le 0.05$, `**` $\le 0.01$, `***` $\le 0.001$, `****` $\le 0.0001$).
  - `Apa`: no leading zero, 3 decimal places (`.123`), floor `< .001`, ceiling of 3 stars (`***` for all $p \le 0.001$).
  - `Nejm`: leading zero, 3 decimal places (`0.123`), floor `< 0.001`, ceiling of 3 stars (`***` for all $p \le 0.001$).
  - `Custom { decimals: 1..=15, scientific_below: Option<f64> }`: full precision or scientific notation (`2.41e-9`) with 4-star thresholds.
- Raw `f64` *p*-values are always stored unrounded in `ResultSheet` so downstream FDR procedures and live graph brackets never suffer rounding quantization.

### 1.3 Compensated Summation & Rank-Revealing Linear Algebra

- Sample variance, covariance, and ANOVA sums of squares use two-pass mean-centered Neumaier compensated summation (`curvebook_math::sum`) so NIST StRD stiffness benchmarks (`NumAcc1`–`NumAcc4`) match an exact-arithmetic `f64` oracle to LRE $\ge 14.5$ even when $\bar{x} / s > 10^8$; against the certified decimal values the SD LRE floor is relaxed to $\ge 9.2$ (`NumAcc3`) and $\ge 8.0$ (`NumAcc4`) because those inputs are not exactly representable in `f64` (tolerances documented in `crates/stats/tests/nist_strd.rs`).
- Design matrices $X \in \mathbb{R}^{n \times p}$ use column-pivoted Householder QR (`qr_col_pivoted`) with explicit rank-deficiency detection (`|R_{kk}| \le \epsilon \cdot \max(n, p) \cdot |R_{00}|`), falling back cleanly to SVD pseudoinverse or reporting aliased/collinear predictors rather than dividing by zero.

---

## 2. Descriptive Statistics, Normality & Outlier Detection

### 2.1 Column Descriptive Suite & Subcolumn Aggregation Modes
- **Summary metrics:** Computes $n$ (non-excluded), missing/excluded counts, arithmetic mean, sample SD ($n-1$), SEM, $(1-\alpha)$ CI of mean, minimum, 25th percentile ($Q_1$), median, 75th percentile ($Q_3$), maximum, **symmetric custom percentile pairs** (selecting custom $P \in (50, 100)$ emits both $(100 - P)\%$ and $P\%$ Hyndman–Fan Type-7 tails), interquartile range (Hyndman–Fan Type-6 hinge definition matching standard biostatistics conventions), range, coefficient of variation ($\%\text{CV}$), geometric mean & geometric SD factor + CI (when all $x_i > 0$), harmonic mean + CI, **quadratic mean (root-mean-square, $\text{RMS} = \sqrt{\frac{1}{n}\sum_{i=1}^n x_i^2}$) + CI**, sum, skewness $G_1$, excess kurtosis $G_2$, and one-sample *t*-test / Wilcoxon signed-rank test against a user-specified hypothetical value $\mu_0$.
- **Exact nonparametric binomial Median CI:** Selects order-statistic endpoints $[x_{(L)}, x_{(U)}]$ using exact $\operatorname{Binomial}(n, 0.5)$ cumulative tail probabilities without normal approximation, reporting the discrete **`Actual confidence level`** achieved alongside the requested target level (returning `StatsError::InsufficientSampleSize` cleanly when $n < 3$).
- **Three replicate-subcolumn aggregation modes** when invoked on `XY` or `Grouped` sheets with $N > 1$ subcolumns:
  1. *Row-means summary:* average the non-excluded replicates within each row first, then compute column descriptive statistics across those row means;
  2. *Per-subcolumn summary:* compute an independent descriptive column for each subcolumn $1 \dots N$;
  3. *Pooled grand summary:* flatten all non-excluded cells across rows and subcolumns of a data set into a single pooled sample.

### 2.2 Normality & Lognormality Battery
Evaluated on raw values $x_i$ (normality) and $\ln(x_i)$ when $x_i > 0$ (lognormality), returning the likelihood ratio between normal and lognormal generative models:
1. **D'Agostino–Pearson omnibus $K^2$** ($n \ge 8$): D'Agostino skewness $Z(\sqrt{b_1})$ + Anscombe–Glynn kurtosis $Z(b_2)$ (D'Agostino, Belanger & D'Agostino 1990), $K^2 = Z(\sqrt{b_1})^2 + Z(b_2)^2 \sim \chi^2_2$.
2. **Shapiro–Wilk $W$** ($3 \le n \le 5000$): Royston (1992) polynomial approximations for $a_i$ weights and normalizing transformation of $W$, verified against the Royston (1995, AS R94) $n = 25$ lognormal benchmark ($W = 0.83467, p = 0.000914$).
3. **Anderson–Darling $A^2$** ($n \ge 8$): Stephens small-sample modified statistic $A^{*2} = A^2 (1 + 0.75/n + 2.25/n^2)$.
4. **Kolmogorov–Smirnov with Lilliefors $p$-value** ($n \ge 5$): Dallal & Wilkinson (1986) analytical polynomial approximation + Monte Carlo refinement in boundary regions.
5. **Normal & lognormal Q–Q expected quantiles** (`qq_expected_quantiles`, `ResultTabKind::NormalQq`, and `DiagnosticPlotKind::NormalQqPlot`): Blom plotting positions $p_i = (i - 0.375)/(n + 0.25)$ mapped through $\Phi^{-1}(p_i)$ against observed order statistics.

### 2.3 Outlier Detection (`Grubbs` & `ROUT`)
- **Grubbs' ESD test** ($n \ge 3$, $\alpha$): $G = \max_i |x_i - \bar{x}| / s$, compared against critical value $G_{\text{crit}}(\alpha, n) = \frac{n-1}{\sqrt{n}} \sqrt{\frac{t^2_{\alpha/(2n), n-2}}{n - 2 + t^2_{\alpha/(2n), n-2}}}$. Single-outlier or iterative mode.
- **ROUT method** (Motulsky & Brown 2006, $n \ge 3$, maximum desired FDR $Q \in (0, 1]$, default $Q = 1\%$):
  1. Fit constant location (for a column stack, $Y = M$) or nonlinear curve via Lorentzian/Cauchy robust regression.
  2. Compute the 68.27th percentile $P_{68.27}$ of absolute residuals $|r_i|$ and convert to the Robust Standard Deviation of Residuals:
     $$\text{RSDR} = P_{68.27} \cdot \frac{N}{N - K}$$
     where $K$ is the number of fitted parameters ($K = 1$ for a column stack).
  3. Rank absolute residuals $|r_{(1)}| \le \dots \le |r_{(N)}|$. Scanning from $i = \lfloor 0.70 N \rfloor + 1$ up to $N$ (allowing up to $30\%$ outliers), compute two-tailed Student $t$ probabilities $p_{(i)}$ from $t_i = |r_{(i)}| / \text{RSDR}$ with $\nu = N - K$, and compare against Benjamini–Hochberg step-up thresholds $\alpha_i = Q \cdot \frac{N - (i - 1)}{N}$. Once the smallest $i^*$ with $p_{(i^*)} \le \alpha_{i^*}$ is found, flag observations $(i^* \dots N)$ as outliers.

---

## 3. Two-Group, One-Way, Factorial & Mixed-Effects Comparisons

### 3.1 Two-Group Comparisons & Standardized Effect Sizes
- **Unpaired *t*-test** (pooled SD + *F*-test of variances), **Welch's *t*-test** (Welch–Satterthwaite $\nu$), **Paired *t*-test**, **Ratio paired *t*-test** (paired *t* on $\ln x_i$, reporting geometric mean of ratios), and **Unpaired lognormal *t*-test**.
- **Standardized effect sizes:**
  - Unpaired: **Cohen's $d$** $= (\bar{x}_1 - \bar{x}_2)/s_{\text{pooled}}$, **Hedges' $g$** $= J(\nu)\,d$ with exact gamma bias factor $J(\nu) = \frac{\Gamma(\nu/2)}{\sqrt{\nu/2}\,\Gamma((\nu-1)/2)}$, and **Glass's $\Delta$** $= (\bar{x}_1 - \bar{x}_2)/s_{\text{control}}$, with noncentral-$t$ exact confidence intervals (`curvebook_math::noncentral_t_ci`).
  - Paired: $d_z = \bar{d} / s_d$ and Hedges' corrected $g_z$ with noncentral-$t$ CIs.
- **Nonparametric two-group:**
  - **Mann–Whitney $U$:** Exact dynamic-programming distribution for small/medium samples (including exact tie permutation up to bounded state-space limit), normal approximation with tie variance correction beyond threshold, plus exact Hodges–Lehmann median difference and CI.
  - **Wilcoxon matched-pairs signed-rank:** Exact signed-rank distribution + Pratt vs. Wilcoxon zero-difference handling + Hodges–Lehmann pseudomedian.
  - **Kolmogorov–Smirnov two-sample:** Exact Smirnov lattice path count for small $n_1, n_2$ and Kolmogorov asymptotic series for large samples.
- **Multiple Row *t*-tests (`MultipleRowTTests`):** Row-by-row unpaired, Welch, paired, or Mann–Whitney / Wilcoxon comparisons across `Grouped` or `MultipleVariables` tables with Holm–Šídák FWER or BH/BY/BKY FDR $q$-values and $(-\log_{10} p, -\log_{10} q, \Delta)$ Volcano plot coordinates.

### 3.2 One-Way, Two-Way, Three-Way & Multifactor ($N$-Way) ANOVA
- **One-way designs:** Ordinary ANOVA (with Bartlett and Brown–Forsythe variance homogeneity tests), Brown–Forsythe & Welch unequal-variance one-way ANOVA, RM one-way ANOVA with Geisser–Greenhouse $\hat{\varepsilon}$ and Huynh–Feldt $\tilde{\varepsilon}$ sphericity adjustments, Kruskal–Wallis, and Friedman test.
- **Factorial & Multifactor designs:**
  - Classic `Grouped` Two-Way and Three-Way ANOVA supporting Type I (sequential) and Type III (marginal orthogonal-contrast) sums of squares for unbalanced cells, plus repeated measures on one or both factors.
  - **Multifactor ($N$-way) GLM (`MultipleVariables`):** Accepts arbitrary numbers of categorical grouping factors from a `MultipleVariables` table with configurable main effects, two-way through $k$-way interactions, reference/sum contrasts, residual diagnostics, and post-hoc main/simple/cell comparisons.
  - **ANOVA effect sizes:** $\eta^2 = \text{SS}_{\text{effect}} / \text{SS}_{\text{total}}$, partial $\eta_p^2 = \text{SS}_{\text{effect}} / (\text{SS}_{\text{effect}} + \text{SS}_{\text{error}})$, omega-squared $\omega^2$, and Cohen's $f = \sqrt{\eta_p^2 / (1 - \eta_p^2)}$, with noncentral-$F$ confidence intervals.
  - When a post-hoc comparison family contains only $m = 1$ comparison, emit an explicit informational note on the `ResultSheet` clarifying why multiplicity adjustment leaves the unadjusted *p*-value unchanged.

### 3.3 Linear Mixed-Effects Models (REML) & Nested Designs
- When repeated-measures `Grouped` tables have missing values (or for `Nested` tables with unequal subcolumn sizes), fit the random-intercept linear mixed model $y = X\beta + Z u + \varepsilon$, $u \sim N(0, \sigma_u^2 I)$ (`fit_random_intercept_reml` in `crates/stats/src/anova/reml.rs`) by **Restricted Maximum Likelihood (REML)**, profiling the exact log-REML over the variance ratio $\tau = \sigma_u^2 / \sigma_e^2$ (grid + golden-section search, Cholesky solves):
  - Compound-symmetry covariance across repeated timepoints (implied by the random intercept); unstructured residual covariance is not implemented (planned in [`R3.4`](../history/roadmap/r3.md#r34-unstructured-residual-covariance-for-reml-mixed-models), todo),
  - Fixed-effect Wald *F*-tests with **Kenward–Roger** or **Satterthwaite** denominator degrees of freedom,
  - Variance-component estimates, intraclass correlation (ICC), and likelihood-ratio $\chi^2$ test for random subject effects.

---

## 4. Multiple Comparisons & False Discovery Rate

All multiplicity engines operate cleanly on contrast vectors $c_j^\top \hat{\beta}$ with covariance $V_{\hat{\beta}}$:
- **Familywise Error Rate (FWER, $\alpha$):**
  - **Tukey HSD** (all $\binom{k}{2}$ pairs, equal variance; Tukey–Kramer for unequal $n_i$), **Student–Newman–Keuls (`NewmanKeuls`)** step-down Studentized range comparisons, and **Unprotected Fisher's LSD (`FisherLsd`)** pairwise *t* comparisons using pooled ANOVA $\text{MS}_{\text{error}}$,
  - **Dunnett** (each treatment vs. designated control, exact multivariate-$t$),
  - **Šídák** ($p_{\text{adj}} = 1 - (1 - p)^m$, simultaneous CIs at $1 - (1-\alpha)^{1/m}$) and **Bonferroni** ($p_{\text{adj}} = \min(1, m p)$),
  - **Holm–Šídák** step-down multiplicity-adjusted *p*-values (sequentially monotonic; does not produce fixed-width simultaneous CIs),
  - **Dunnett's T3** ($n < 50$ per group), **Games–Howell** ($n \ge 50$), and **Tamhane's T2** following Welch/Brown–Forsythe unequal-variance ANOVA,
  - **Dunn's rank-sum test** following Kruskal–Wallis or Friedman,
  - **Linear trend test** across ordered columns.
- **False Discovery Rate (FDR, $Q$):**
  - **Benjamini–Hochberg (BH)** one-stage step-up,
  - **Benjamini–Yekutieli (BY)** under arbitrary dependency,
  - **Two-stage linear step-up Benjamini–Krieger–Yekutieli (BKY)** estimating $\hat{\pi}_0$ in stage 1 with $Q' = Q/(1+Q)$. Discoveries are always flagged strictly from adjusted $q$-values ($q_i \le Q$), never raw $p_i$.
- **Planned (`R10.10`, todo):** a standalone "Analyze a stack of *P* values" column analysis reusing these multiplicity engines (Bonferroni, Holm, Šídák, Holm–Šídák, BH, BKY, BY) on a user-supplied column of *P* values; not yet available.

---

## 5. Contingency, Survival, ROC & Multivariate Analysis

### 5.1 Contingency Tables (`crates/stats/src/categorical.rs`)
- $2 \times 2$ and $R \times C$ Pearson $\chi^2$ (with optional Yates continuity correction on $2 \times 2$), likelihood-ratio $G^2$, Fisher's exact test ($2 \times 2$ hypergeometric tail sum via log-factorials; Freeman–Halton network/Monte Carlo extension for $R \times C$), **Barnard's unconditional exact test** (`BarnardExact`: $2 \times 2$ nuisance-parameter grid supremum over $p \in [0, 1]$), Cochran–Armitage linear trend test on $2 \times C$, and McNemar's paired test.
- **Exact & score confidence intervals:**
  - **Odds Ratio:** **Baptista–Pike** mid-*p* / exact hypergeometric interval (preferable to Woolf logit when cells are small or zero; Haldane–Anscombe $+0.5$ fallback noted when needed).
  - **Relative Risk:** **Koopman (1984)** asymptotic score interval via constrained maximum likelihood root-finding.
  - **Difference of proportions:** Newcombe hybrid Wilson score interval.
  - **Sensitivity, Specificity, PPV, NPV:** Wilson score (default) and Clopper–Pearson exact binomial intervals.
  - **Standardized effect sizes:** $\phi$ coefficient ($2 \times 2$) and Cramér's $V = \sqrt{\chi^2 / (N \cdot \min(R-1, C-1))}$ with noncentral-$\chi^2$ confidence intervals (detecting perfect separation / zero-margin edge cases explicitly).

### 5.2 Survival Analysis (`crates/stats/src/survival.rs`)
Supports both classic `Survival` sheets and `MultipleVariables` variable assignment (time, event/censoring indicator, grouping factor(s)):
- **Kaplan–Meier** product-limit estimator $\hat{S}(t) = \prod_{t_i \le t} (1 - d_i / n_i)$ with **asymmetrical $\log(-\log \hat{S}(t))$ Greenwood confidence intervals** (guaranteed inside $[0, 1]$), median survival time + Brookmeyer–Crowley CI, and number-at-risk table at axis tick marks.
- **Curve comparisons:** Log-rank (Mantel–Cox) $\chi^2$, Gehan–Breslow–Wilcoxon (early-event weighted), log-rank test for linear trend across $\ge 3$ ordered groups, pairwise post-hoc curve comparisons with multiplicity-adjusted *p*-values, and both Mantel–Haenszel and log-rank hazard ratios with $95\%$ CIs.
- **Cox proportional hazards regression:** Partial likelihood maximization (Newton–Raphson) with **Breslow** and **Efron** tie approximations, Wald hazard-ratio CIs ($\exp(\hat\beta \pm z_{1-\alpha/2}\,\mathrm{SE})$), omnibus likelihood-ratio / Wald / score tests, Schoenfeld proportional-hazards diagnostics, Martingale/deviance residuals, and baseline survival curves.

### 5.3 ROC Curves, Method Comparison, Multiple Regression, Imputation, PCA & Clustering
- **ROC:** Empirical step curve across all distinct cutoffs, trapezoidal AUC, **Hanley–McNeil** and **DeLong** non-parametric SE & CIs, sensitivity/specificity/likelihood-ratio table with Youden's $J = \text{Sens} + \text{Spec} - 1$.
- **Method Comparison (`BlandAltman` & `PassingBablok`):** Bland–Altman difference / ratio / percent-difference bias and $95\%$ limits of agreement ($\bar{d} \pm 1.96 s_d$) with exact CIs; Passing–Bablok nonparametric median shifted slope and intercept regression with rank-order CIs and CUSUM linearity test.
- **Multiple Linear, Logistic & Poisson GLMs:** IRLS solver with configurable categorical **Reference-level contrast coding** (selecting reference level $r$ for a $K$-level predictor emits $K - 1$ named indicator columns `Var[Level vs. Ref]` in parameter and VIF tables), full Type I / Type III omnibus ANOVA / Deviance tables (including categorical $\times$ categorical interaction blocks), VIF multicollinearity diagnostics, AICc, pseudo-$R^2$ (Tjur, McFadden, Nagelkerke), Hosmer–Lemeshow goodness-of-fit, classification confusion matrices, and ROC curves. *(Planned XY `SimpleLogisticRegression` and multivariable `GlmSpec` extensions — interactions, polynomial terms, no-intercept models, nested GLM comparison, linear weights, Cook's distance/leverage/DFBETAs, out-of-sample row prediction, and configurable classification cutoffs — are specified in [`curve-fitting-implementation.md`](curve-fitting-implementation.md#53-planned-standalone-xy-simple-logistic-regression-target-specification--hosmer-lemeshow--sturdivant-2013-tjur-2009) §§5.3–5.4.)*
- **Multiple Imputation (`MultipleImputation`):** Multivariate Imputation by Chained Equations (MICE) using Type-1 Predictive Mean Matching (PMM) seeded via `DeterministicRng`, pooled across $M$ completed datasets with Rubin's rules and Barnard–Rubin small-sample degrees of freedom.
- **PCA & Clustering:** Mean-centered / standardized correlation or covariance SVD, Kaiser criterion ($\lambda > 1$), Horn's parallel analysis (Monte Carlo random eigenvalues via `curvebook_math::rng`), scree/loading/biplot coordinates, plus K-means++ and agglomerative hierarchical clustering (alphabetical cluster label normalization for deterministic palette mapping).

---

## 6. Plain-Language Analysis Checklists & Verification Standard

Every `ResultSheet` produced by `curvebook-stats` includes a structured `checklist: Vec<ChecklistItem>` (`crates/doc/src/result.rs`: `assumption`, `status: ChecklistStatus` = `Pass | Warn | Violated | Info`, `diagnostic_metric`, `explanation`, optional `remediation`) written in original clean-room scientific prose (checking independence, normality/lognormality test results, SD ratio / variance homogeneity, sample size adequacy, sphericity $\hat{\varepsilon}$, expected contingency counts $< 5$, proportional hazards).

**Mandatory test gate:** Every statistical module in `crates/stats` must carry:
1. Analytical / textbook exact-value unit tests,
2. NIST StRD accuracy tests (`Pontius`, `Norris`, `AtmWtAg`, `SiRstv`, `SmLs01`–`SmLs09`) in `crates/stats/tests/`,
3. Property tests (`proptest`) verifying invariance under row permutation, scale invariance of dimensionless statistics (*t*, *F*, *r*, *p*), and zero panics on degenerate inputs (`n = 0`, `n = 1`, all identical values, `NaN`, `±Inf`).
