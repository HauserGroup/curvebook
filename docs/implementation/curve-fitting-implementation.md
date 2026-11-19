# Curve Fitting & Nonlinear Regression — Implementation Guidelines

Governing specification for `crates/fit` (`L2`). Parity checklist: [`evidence/curve-fitting.md`](../evidence/curve-fitting.md).

Primary literature & numerical references (exact optimization, profile CIs, outliers, mechanistic pharmacology/kinetics, GLMs & splines):
- **Oracle benchmarks:** NIST Statistical Reference Datasets (StRD), Nonlinear Regression ([`crates/fit/tests/d1_nist_and_pharma_oracles.rs`](../../crates/fit/tests/d1_nist_and_pharma_oracles.rs)).
- **Solver, IRLS weighting, model comparison & profile-likelihood CIs:** Moré (1978) *Lect. Notes Math.* 630:105–116 (`10.1007/BFb0067700`); Venzon & Moolgavkar (1988) *Appl. Stat.* 37:87–94 (`10.2307/2347496`); Bates & Watts (1988) *Nonlinear Regression Analysis and Its Applications*; Seber & Wild (1989) *Nonlinear Regression*; Carroll & Ruppert (1988) *Transformation and Weighting in Regression*; Burnham & Anderson (2002) *Model Selection and Multimodel Inference*; Motulsky & Christopoulos (2004) *Fitting Models to Biological Data Using Linear and Nonlinear Regression*.
- **Robust regression & ROUT:** Motulsky & Brown (2006) *BMC Bioinformatics* 7:123 (`10.1186/1471-2105-7-123`).
- **Dose–response, operational models, allostery & binding/enzyme kinetics:** Gaddum (1937) *J. Physiol.* 89:7P–9P; Arunlakshana & Schild (1959) *Br. J. Pharmacol.* 14:48–58 (`10.1111/j.1476-5381.1959.tb00928.x`); Colquhoun (2007) *Trends Pharmacol. Sci.* 28:608–614 (`10.1016/j.tips.2007.09.011`); Black & Leff (1983) *Proc. R. Soc. Lond. B* 220:141–162 (`10.1098/rspb.1983.0093`); Motulsky & Mahan (1984) *Mol. Pharmacol.* 25:1–9; Swillens (1995) *Mol. Pharmacol.* 47:1197–1203; Christopoulos & Kenakin (2002) *Pharmacol. Rev.* 54:323–374 (`10.1124/pr.54.2.323`); Gottschalk & Dunn (2005) *Anal. Biochem.* 343:54–65 (`10.1016/j.ab.2005.04.035`); Liao & Liu (2009) *J. Chemom.* 23:248–253 (`10.1002/cem.1218`); Sebaugh (2011) *Pharm. Stat.* 10:128–134 (`10.1002/pst.426`); Morrison (1969) *Biochim. Biophys. Acta* 185:269–286 (`10.1016/0005-2744(69)90420-3`); Cheng & Prusoff (1973) *Biochem. Pharmacol.* 22:3099–3108 (`10.1016/0006-2952(73)90196-2`).
- **Radiobiology, electrophysiology, linear/Deming/logistic GLMs & splines:** Hodgkin & Huxley (1952) *J. Physiol.* 117:500–544 (`10.1113/jphysiol.1952.sp004764`); Chadwick & Leenhouts (1973) *Phys. Med. Biol.* 18:78–87 (`10.1088/0031-9155/18/1/007`); Brenner (2008) *Semin. Radiat. Oncol.* 18:234–239 (`10.1016/j.semradonc.2008.04.004`); Fieller (1940) *J. R. Stat. Soc. Suppl.* 7:1–64 (`10.2307/2983630`); Deming (1943) *Statistical Adjustment of Data*; Linnet (1990) *Clin. Chem.* 36:1093–1101 & Linnet (1993) *Clin. Chem.* 39:424–432; McCullagh & Nelder (1989) *Generalized Linear Models*; Cook (1977) *Technometrics* 19:15–18 (`10.2307/1268249`); Heinze & Schemper (2002) *Stat. Med.* 21:2409–2419 (`10.1002/sim.1047`); Tjur (2009) *Am. Stat.* 63:366–372 (`10.1198/tast.2009.08210`); Hosmer, Lemeshow & Sturdivant (2013) *Applied Logistic Regression*; Reinsch (1967) *Numer. Math.* 10:177–183 (`10.1007/BF02162161`); Akima (1970) *J. ACM* 17:589–602 (`10.1145/321607.321609`); de Boor (1978) *A Practical Guide to Splines*; Cleveland (1979) *J. Am. Stat. Assoc.* 74:829–836 (`10.2307/2286407`).

---

## 1. Nonlinear Solver Architecture (`crates/fit`)

### 1.1 Objective Functions & Weighting Schemes

Given observations $(x_i, y_i)$ for $i = 1, \dots, N$ across $D \ge 1$ data sets (or expanded from `(Mean_i, SD_i, n_i)` summary subcolumns) and parameter vector $\theta \in \mathbb{R}^K$:

1. **Weighted Least Squares (Gaussian likelihood):**
   $$\text{SS}(\theta) = \sum_{i=1}^N w_i \big(y_i - f(x_i; \theta)\big)^2$$
   Supported `WeightingScheme` variants:
   - `None`: $w_i = 1$ (homoscedastic Gaussian scatter),
   - `RelativeSquare` ($1/Y^2$): $w_i = 1 / y_i^2$ (constant $\% \text{CV}$ proportional error; returns `FitStatus::ImpossibleWeights` if any included $y_i = 0$),
   - `Relative` ($1/Y$): $w_i = 1 / |y_i|$ (Poisson-like variance using observed $Y$),
   - `GeneralPowerY(k)`: $w_i = 1 / |y_i|^k$,
   - `InverseX` / `InverseX2`: $w_i = 1 / |x_i|$ or $1 / x_i^2$,
   - `ObservedSd` ($1/\text{SD}^2$): weights replicate group $g$ by $1 / s_g^2$ when $n_g \ge 2$.
2. **Poisson Regression (IRLS / Deviance minimization for count data $y_i \ge 0$):**
   Minimizes Poisson deviance $G^2(\theta) = 2 \sum_{i=1}^N \big[ y_i \ln(y_i / \hat{y}_i) - (y_i - \hat{y}_i) \big]$ (with $0 \ln 0 = 0$) where $\hat{y}_i = f(x_i; \theta) > 0$.
3. **Robust Nonlinear Regression (Lorentzian / Cauchy $t_1$ distribution):**
   Targets the heavy-tailed Cauchy objective (Motulsky & Brown 2006), implemented as iteratively reweighted least squares (`fit_cauchy_irls` in `crates/fit/src/solver/irls.rs`, Cauchy weights $1/(1 + z_i^2)$ with $z_i = |r_i| / \text{RSDR}$, up to 15 reweighting passes):
   $$\mathcal{L}_{\text{Cauchy}}(\theta, s) = \sum_{i=1}^N \ln\!\left(1 + \frac{w_i (y_i - f(x_i; \theta))^2}{s^2}\right)$$
   where scale $s$ is updated iteratively from the $68.27\text{th}$ percentile of weighted absolute residuals ($\text{RSDR}$). Pure Robust mode (`FitMethod::RobustCauchy`) requires raw replicates (`FitError::RequiresRawReplicates` on pre-summarized data) and suppresses least-squares inference (`InferenceAvailability::NotApplicableInRobustMode`, [`R4.5`](../history/roadmap/r4.md#r45-suppress-least-squares-inference-in-pure-robust-fit-mode): omits parameter SEs/CIs, covariance matrices, confidence/prediction bands, and *F*/LRT/$\text{AIC}_c$ comparisons while preserving best-fit estimates, `RSDR`, residuals, and point interpolation). Robust mode serves both for outlier-resistant curve estimation and as Stage 1 of **ROUT** automatic outlier elimination ($Q$ default $1\%$).

### 1.2 Damped Levenberg–Marquardt with Trust-Region Scaling & Simplex Warm-Start

1. **Initial parameter seeding:** Every built-in equation provides deterministic data-driven initial-value heuristics (`ParametricModel::initial_guess` in `crates/fit/src/library/mod.rs`, e.g. Y min/max/mid, X at Y-mid via `estimate_x_at_ymid`, linear regression on transformed endpoints). If the initial point evaluates to non-finite $f(x_i; \theta_0)$, a bounded Nelder–Mead rescue (`nelder_mead_rescue`, 240–600 evaluations depending on stage, when `SolverConfig::use_simplex_warm_start`) searches for a finite basin, otherwise the fit returns `FitStatus::BadInitialValues` / `FitError::BadInitialValues`.
2. **Moré trust-region Levenberg–Marquardt loop:**
   - Assemble weighted Jacobian $J_{ik} = \sqrt{w_i}\,\frac{\partial f(x_i; \theta)}{\partial \theta_k}$ (analytical derivatives where a model implements `ParametricModel::grad` — a subset of built-ins; otherwise central differences with step $h_k = 10^{-6}\max(1, |\theta_k|)$ and a one-sided $1.5 \times 10^{-8}$ fallback near domain/box edges, as for custom user equations).
   - Solve the damped normal equations via QR decomposition of the augmented system (avoiding explicit $J^\top J$ squaring of the condition number $\kappa(J)^2$!):
     $$\begin{bmatrix} J \\ \sqrt{\lambda}\,D \end{bmatrix} \delta = \begin{bmatrix} r \\ 0 \end{bmatrix}$$
     where $D = \operatorname{diag}(\|J_{:,k}\|_2)$ provides scale invariance across parameters spanning orders of magnitude (e.g., $B_{\max} \sim 10^4$ alongside $k_{\text{off}} \sim 10^{-3}$).
   - Projection onto the box (`project_theta`) with active-bound gradient masking handles box constraints ($a_k \le \theta_k \le b_k$); parameters that converge onto a bound are flagged with `ParameterNote::HitLowerBound` / `ParameterNote::HitUpperBound`.
   - **Convergence criterion (MINPACK-style, `run_trust_region_lm`):** actual and predicted relative SS reduction both $\le \text{ftol} = 10^{-15}$, **or** scaled step $\|\delta\|_D \le \text{xtol} \cdot \max(1, \|\theta\|_D)$ with $\text{xtol} = 10^{-14}$ (gradient tolerance $10^{-13}$), capped at `SolverConfig::max_iters` (default `1_000`; analysis specs clamp to `HARD_MAX_ITERS = 50_000` in `crates/doc/src/analysis.rs`).

### 1.3 Global Nonlinear Regression & Parameter Sharing

Across $D$ data sets (`A`, `B`, …), each model parameter $p \in \{1, \dots, P\}$ has a `ParameterScope`:
- `Local`: independent $\theta_{d, p}$ per data set $d$,
- `SharedGlobal`: one shared $\theta_{\text{global}, p}$ across all included data sets,
- `Constant(f64)`, `PerDatasetConstant(Vec<(usize, f64)>)`, `ColumnTitleConstant`, or `HookedConstant(HookSource)`: fixed per data set or globally (e.g., inhibitor concentration $[I]_d$ read from column titles or an Info sheet constant), contributing $0$ free parameters.

The global Jacobian stacks dataset blocks into one dense matrix whose shared columns are filled for every data set and whose local columns are block-diagonal, minimizing $\text{SS}_{\text{global}} = \sum_{d=1}^D \text{SS}_d$ with $\text{df} = N_{\text{total}} - K_{\text{free}}$.

### 1.4 Planned Solver, Weighting & Method Extensions *(target specification — not yet implemented)*

1. **Curve-predicted IRLS weighting (`PredictedY`, Carroll & Ruppert 1988; Motulsky & Christopoulos 2004):**
   - Current `RelativeSquare`, `Relative`, and `GeneralPowerY(k)` weight by observed $y_i$. Target default for response-dependent weighting computes weights from the **fitted curve height** $\hat{y}_i = f(x_i; \theta)$ via Iteratively Reweighted Least Squares (IRLS), eliminating downward bias on noisy low observations and avoiding `ImpossibleWeights` when a noisy observation equals $y_i = 0$ while $\hat{y}_i > 0$:
     1. Run one unweighted ($w_i = 1$) Levenberg–Marquardt fit from $\theta_0$ so inaccurate starting values do not blow up initial weights.
     2. Evaluate $\hat{y}_i^{(m)} = f(x_i; \hat{\theta}^{(m)})$ across all active observations and form fixed weights $w_i^{(m)} = 1 / |\hat{y}_i^{(m)}|^k$ ($k = 2$ for $1/Y^2$, $k = 1$ for $1/|Y|$, or arbitrary $k$; return `FitStatus::ImpossibleWeights` if $\hat{y}_i^{(m)} = 0$).
     3. Re-optimize $\theta^{(m+1)} = \arg\min_\theta \sum_i w_i^{(m)} (y_i - f(x_i; \theta))^2$ holding $w_i^{(m)}$ fixed during the inner LM loop, then refresh $w_i^{(m+1)}$ from $\hat{y}_i^{(m+1)}$ until relative change in weighted $\text{SS}$ stabilizes within convergence tolerance.
   - Pre-summarized `(Mean_i, SD_i, n_i)` rows expand sufficient statistics ($n_i \bar{y}_i$, $(n_i - 1) s_i^2$) identically under curve-based weights so results match raw replicate entry exactly. When pure `RobustCauchy` or Stage 1 of `ROUT` is combined with unequal weights, optimization runs unweighted (preventing extreme outliers from receiving inflated weights) and applies the chosen weights to Stage 2 outlier testing, Stage 2 WLS refits, residual tables, and outlier counts.
2. **Outlier count-only mode & analyzable `"Cleaned data"` tab (Motulsky & Brown 2006):**
   - Extend outlier configuration beyond binary elimination with `OutlierHandling::{None, IdentifyAndCountOnly, Eliminate}` ($Q = 0\%$ disables ROUT) and `emit_cleaned_data_tab: bool`:
     - `IdentifyAndCountOnly`: runs Stage 1 Cauchy IRLS + BH top-$30\%$ FDR detection at threshold $Q$, emits the `Outliers` result tab and per-dataset outlier count without removing flagged points from the least-squares fit.
     - `emit_cleaned_data_tab`: emits an analyzable `ResultTab` copying the input table with ROUT-identified outliers excluded so downstream graphs or secondary analyses can bind to cleaned observations directly.
     - **Model comparison under ROUT elimination:** fit $M_1$ and $M_2$ separately to detect each model's outlier set $O_1, O_2$, remove the **union** $O_1 \cup O_2$ from both datasets, and refit $M_1$ and $M_2$ on identical retained observations $I \setminus (O_1 \cup O_2)$ before computing Extra-SS *F* or $\text{AIC}_c$.
3. **Replicate independence modes (`ReplicateFitMode`):**
   - Support choosing between `IndividualReplicates` (default: each replicate subcell contributes an observation, or `(Mean, SD, n)` contributes $n_g$ degrees of freedom and within-group $\sum (n_g - 1) s_g^2$) and `RowMeansOnly` (collapses each row to its unweighted sample mean $\bar{y}_r$ with $1$ observation per row when technical replicates within a dose share a single biological subject and are non-independent).
4. **User-selectable convergence strictness presets (`ConvergenceStrictness`):**
   - Alongside MINPACK machine-precision defaults, expose `Quick` ($2$ consecutive iterations changing objective $\text{SS}$ by $< 0.01\%$, i.e. $10^{-4}$), `Medium` ($5$ consecutive iterations changing $\text{SS}$ by $< 0.0001\%$, i.e. $10^{-6}$), and `Strict` ($5$ consecutive iterations changing $\text{SS}$ by $< 10^{-8}\%$, i.e. $10^{-10}$) for high-throughput batch/simulation trade-offs.
5. **Multi-response `MultipleVariables` assignment (`mv_y_columns`):**
   - Generalize singular `NonlinearFitSpec::mv_y_column: Option<usize>` to `mv_y_columns: Vec<usize>`, fitting separate curves across the Cartesian product of $M$ assigned continuous response columns $\times G$ categorical grouping combinations ($M \times G$ curve columns).

---

## 2. Uncertainty, Diagnostics & Model Comparison

### 2.1 Asymptotic Standard Errors, Dependency & Identifiability

From $R$ of the converged QR factorization $J = Q R$ (with $\hat{\sigma}^2 = \text{SS} / \text{df}$):
- Covariance matrix: $\operatorname{Cov}(\hat{\theta}) = \hat{\sigma}^2 (R^\top R)^{-1}$, asymptotic $\text{SE}_k = \sqrt{\operatorname{Cov}_{kk}}$, symmetrical CI $= \hat{\theta}_k \pm t_{\alpha/2, \text{df}} \cdot \text{SE}_k$.
- **Parameter Dependency:**
  $$\text{Dependency}_k = 1 - \frac{1}{(R^\top R)_{kk} \cdot [(R^\top R)^{-1}]_{kk}} \in [0, 1)$$
  Values $> 0.99$ indicate severe multicollinearity; values $> 0.9999$ (or rank-deficient $R$) mark the fit as **Ambiguous / Unstable** (`~` prefix on tangled parameter estimates, open/missing CIs on unidentifiable parameters rather than garbage `1e18` SEs).
- **Hougaard's skewness** $g_{1,k}$ quantifies parameter nonlinearity ($|g_{1,k}| > 0.25$ recommends asymmetrical profile-likelihood intervals and logarithmic parameterization such as $\log \text{EC}_{50}$ instead of linear $\text{EC}_{50}$).

### 2.2 Asymmetrical Profile-Likelihood Confidence Intervals (Default)

Following Venzon & Moolgavkar (1988) and Bates & Watts (1988), the $100(1-\alpha)\%$ profile-likelihood CI for $\theta_k$ is the set of values $\theta_k^*$ for which fixing $\theta_k = \theta_k^*$ and re-optimizing nuisance parameters $\theta_{-k}$ satisfies the 1-df extra-sum-of-squares *F* threshold:
$$\text{SS}_{\text{crit}} = \text{SS}(\hat{\theta}) \left(1 + \frac{F_{1, \text{df}}(\alpha)}{\text{df}}\right)$$
`curvebook-fit` traces both lower and upper roots using a Venzon–Moolgavkar tangent warm-start along the covariance ratio, re-optimizing nuisance parameters at each candidate, then refining the bracketed crossing with Brent's method (`compute_profile_likelihood_cis` in `crates/fit/src/diagnostics.rs`). If $\text{SS}(\theta_k^*)$ asymptotes horizontally below $\text{SS}_{\text{crit}}$ (e.g., an incomplete dose–response plateau), that endpoint is returned cleanly as `CiBound::Open` (`???` / unbounded) rather than failing the fit.

### 2.3 Confidence/Prediction Bands & Inverse Interpolation

- **Confidence band** at grid point $x$: $\hat{y}(x) \pm t_{\alpha/2, \text{df}} \sqrt{g(x)^\top \operatorname{Cov}(\hat{\theta})\, g(x)}$ where $g(x) = \nabla_\theta f(x; \hat{\theta})$ (or propagated from profile bounds when exact).
- **Prediction band:** $\hat{y}(x) \pm t_{\alpha/2, \text{df}} \sqrt{\hat{\sigma}^2 / w(x) + g(x)^\top \operatorname{Cov}(\hat{\theta})\, g(x)}$.
- **Standard-curve interpolation (`Interpolate unknowns`):** Given rows with $Y$ entered and $X$ blank, solve $f(x; \hat{\theta}) = y_{\text{unk}}$ by a bracketing scan plus root refinement over the fitted $X$ range extended by $\pm 50\%$ of its span (`interpolate_x_from_y` in `crates/fit/src/solver/curves.rs`), reporting $\hat{x}_{\text{unk}}$ in `InterpolatedUnknown`, mean-response and single-observation calibration intervals (`R4.6`), and an out-of-range `note` ("Target Y falls outside fitted curve range") instead of an estimate when no root exists; unpaired `(X, blank Y)` rows evaluate forward predictions $\hat{y}(x_{\text{unk}})$ with CIs/PIs.

### 2.4 Comparing Models (`Extra-Sum-of-Squares F`, `LikelihoodRatioTest` & `AICc`)

- **Extra-sum-of-squares *F*-test** (for nested Gaussian/WLS models $M_1 \subset M_2$, $\text{df}_1 > \text{df}_2 > 0$):
  $$F = \frac{(\text{SS}_1 - \text{SS}_2) / (\text{df}_1 - \text{df}_2)}{\text{SS}_2 / \text{df}_2} \sim F_{\text{df}_1 - \text{df}_2,\, \text{df}_2}$$
- **Poisson deviance likelihood-ratio test** (for nested `PoissonDeviance` models): $\Delta G^2 = G_1^2 - G_2^2 \sim \chi^2_{\text{df}_1 - \text{df}_2}$.
- **Corrected Akaike Information Criterion ($\text{AIC}_c$)** (valid for nested and non-nested models on identical included data points, $N \ge K + 3$, where $K = p_{\text{free}} + 1$ includes $\sigma^2$ under Gaussian LS and $K = p_{\text{free}}$ under Poisson deviance):
  $$\text{AIC}_c = N \ln\!\left(\frac{\text{SS}}{N}\right) + 2K + \frac{2K(K+1)}{N - K - 1}, \qquad w_{\text{Akaike}, m} = \frac{e^{-\frac{1}{2}\Delta_m}}{\sum_j e^{-\frac{1}{2}\Delta_j}}$$
- **Residual diagnostics:** Replicates test for lack of fit, runs test, Spearman homoscedasticity test, D'Agostino–Pearson / Shapiro–Wilk / Anderson–Darling / Lilliefors KS normality of residuals, and residual plots.

### 2.5 Planned Identifiability, Model-Comparison, Output & Quality-Flag Extensions *(target specification — [`R13.1`](../history/roadmap/r13.md#r131-domain-restricted-regression-fit-identifiability-replicate-unknown-interpolation--outlier-overlays) items done)*

1. **Three-way parameter identifiability & greedy Hessian reduction (`IdentifiabilityMode`):**
   - Support three user-selectable modes on the `Confidence` tab:
     1. `IdentifyUnstableParameters` (default): assemble the converged objective Hessian $H \in \mathbb{R}^{K \times K}$ ($2 J^\top W J$ under WLS or exact Fisher/observed information under Poisson deviance) and compute its spectral/2-norm condition number $\kappa(H)$. Whenever $\kappa(H) > \frac{10}{\sqrt{\varepsilon_{\text{mach}}}} \approx 6.71 \times 10^8$ (with IEEE-754 `f64` $\varepsilon_{\text{mach}} = 2^{-52} \approx 2.22 \times 10^{-16}$), greedily identify index $k^* = \arg\min_k \kappa(H_{-k, -k})$ whose row/column deletion yields the largest reduction in condition number, mark $\theta_{k^*}$ as `"Unstable"` (displaying `"Unstable"` in place of its estimate, SE, and CI), replace $H \leftarrow H_{-k^*, -k^*}$, and repeat until $\kappa(H) \le 10 / \sqrt{\varepsilon_{\text{mach}}}$. Remaining well-conditioned parameters retain valid SEs and profile-likelihood CIs.
     2. `IdentifyAmbiguousFits`: legacy dependency criterion ($\max_k \text{Dependency}_k > 0.9999$), labeling the column `"Ambiguous"`, prefixing tangled estimates with `~`, and omitting CIs on unidentifiable parameters.
     3. `ReportAll` (`Neither`): report raw converged parameter values unconditionally.
   - Standard error column visibility: omit asymptotic SEs by default when asymmetrical profile-likelihood CIs are selected, offering SE output only when symmetrical Wald CIs are chosen (or explicitly requested).
   - Midpoint extrapolation warning (`R13.1`): emit a factual diagnostics-only notice whenever a fitted `EC50`/`IC50`/`LogEC50`/`LogIC50` lies outside the dataset's analyzed $[X_{\min}, X_{\max}]$ span.
2. **All four scientific model-comparison questions (`ModelComparisonQuestion`):**
   - Extend `ModelComparisonSpec` beyond two-equation comparison (`AlternativeEquation { alternative_equation_id }`) to directly encode:
     - `ParameterInvarianceAcrossDatasets { shared_params: Vec<String> }`: tests whether selected parameters share a single global value across datasets ($H_0$) vs. differ per dataset ($H_1$, e.g., parallel Hill slopes),
     - `SingleGlobalCurveVsSeparate`: tests whether one global curve (all parameters `SharedGlobal`, $H_0$) fits all datasets as well as separate per-dataset curves ($H_1$),
     - `ParameterEqualsTheoretical { param: String, theoretical_value: f64 }`: tests whether constraining $\theta_p = c$ ($H_0$, e.g., `HillSlope = 1.0` or `EC50Ratio = 1.0`) degrades fit relative to free $\theta_p$ ($H_1$),
     - Plus optional **`prefer_well_defined_fit: bool`**: prior to computing Extra-SS *F*, LRT, or $\text{AIC}_c$, if one candidate fit is `Ambiguous`, `Unstable`, or `Flagged` while the other converges cleanly, select the clean model immediately without statistical test calculation.
3. **Parameter summary table, secondary graph & output precision:**
   - Optional `summary_parameter: Option<String>` emitting an analyzable parameter summary `ResultTab` (and linked bar or XY `GraphSheet`, e.g., plotting per-column rate constant $k_{\text{obs}}$ against numeric column-title concentrations $[L]$ for secondary regression), plus `output_significant_digits: Option<u8>` controlling displayed precision on parameter and Embedded Result tables.
4. **Automated batch fit-quality flags (`FitFlagRules`):**
   - Configurable flag criteria labeling dubious fits `"Flagged"` (and optionally excluding flagged replicates from Monte Carlo parameter tabulations): (a) $R^2 < r^2_{\min}$ (`R13.1`), (b) analyzed observations $N < N_{\min}$, (c) $\max_k \text{Dependency}_k > d_{\max}$ (enabled when dependency diagnostics run), (d) residual normality $p < \alpha_{\text{norm}}$, (e) runs test or replicates lack-of-fit $p < \alpha_{\text{lof}}$, and (f) ROUT outlier count $> m_{\max}$.
5. **Multi-replicate standard-curve tabs & log-$X$ antilog chaining (`R13.1`):**
   - When `interpolate_unknowns == true` on `RawReplicates(k)` ($k \ge 2$), emit both **`Interpolated Mean X`** (from row mean $\bar{y}$) and **`Interpolated Replicates`** (per subcell $y_{r, s}$), and expose interpolated tabs as `analyzable_data` so `Transform` ($X = 10^X$ or $e^X$) converts interpolated $\log(\text{concentration})$ estimates and CI endpoints directly back to concentration units.

---

## 3. Canonical Built-In Equation Library (`crates/fit/src/library/`)

Every built-in model implements `ParametricModel` (`crates/fit/src/library/mod.rs`) with an immutable string ID (`id()`, listed in `PHARMA_EQUATION_IDS` / `CURVE_EQUATION_IDS` / `NIST_EQUATION_IDS`; the live model count is in [docs/STATUS.md](../STATUS.md)), human title, default bounds, initial-guess rules, reported derived transforms (`derived_transforms`, e.g. $\text{EC}_{50} = 10^{\log \text{EC}_{50}}$, half-life $\ln(2)/K$, $\tau = 1/K$, span, inflection point), and optional analytical partial derivatives (`grad`):

### 3.1 Dose–Response (Stimulation, Inhibition & Special)
- **Stimulation / Inhibition (`X = log10[C]` unnormalized & `[0, 100]` normalized; linear `X = [C]` unnormalized):**
  - 3PL (`HillSlope = ±1`): $Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log \text{EC}_{50} - X)}}$ (inhibition: $10^{(X - \log \text{IC}_{50})}$; linear `X = [C]`: $\text{EC}_{50}/X$ and $X/\text{IC}_{50}$).
  - 4PL Variable Slope: $Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log \text{EC}_{50} - X) \cdot \text{HillSlope}}}$.
- **Special Dose–Response (`X = log10[C]` built-ins in `pharma.rs`; linear `X = [C]` counterparts & additional mechanistic forms in §3.5):**
  - **Asymmetrical 5PL (`log_agonist_vs_response_5pl`):** $Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{\left(1 + 10^{(\log X_b - X) \cdot \text{HillSlope}}\right)^S}$ where $\log X_b = \log \text{EC}_{50} + \frac{1}{\text{HillSlope}}\log_{10}(2^{1/S} - 1)$ ($S > 0$).
  - **Biphasic (`biphasic_dose_response`)** (two saturating phases with fraction $F \in (0,1)$) and **Bell-shaped (`bell_shaped_dose_response`)** ($\text{Plateau}_1 \to \text{Dip} \to \text{Plateau}_2$).
  - **Black–Leff Operational Model — Partial Agonist (`operational_model_agonist`):** fits `Basal`, `Emax`, `LogKA`, `LogTau`, `n` across full + partial agonists via global fitting while avoiding overflow on $\log \tau$.
  - **Gaddum/Schild $\text{EC}_{50}$ shift (`ec50_shift_gaddum_schild`)** (fitting $\text{p}A_2$ and `SchildSlope`) and **Allosteric $\text{EC}_{50}$ shift (`allosteric_ec50_shift`)** (ternary complex cooperativity $\alpha$).
  - **$\text{EC}_{\text{anything}}$ (`log_agonist_vs_response_ecf`):** Parameterizes $\log \text{EC}_F$ directly for user constant $F \in (0, 100)$ via $\log \text{EC}_{50} = \log \text{EC}_F - \frac{1}{\text{HillSlope}}\log_{10}\!\left(\frac{F}{100 - F}\right)$.
  - **Absolute $\text{IC}_{50}$ (`absolute_ic50_4pl`):** Fits concentration where $Y = \text{Fifty}$ ($50\%$ between baseline constant and control; Sebaugh 2011) even when $\text{Bottom} \ne 0$.

### 3.2 Receptor Binding (Saturation, Competition & Kinetics)
- **Saturation:** Built-in in `crates/fit/src/library/pharma.rs`: One-site specific ($Y = \frac{B_{\max} X}{K_d + X}$), One-site total ($+ \text{NS}\cdot X + \text{Background}$), Global total & nonspecific (`one_site_total_and_ns_global`, `<A>` total, `<B>` nonspecific), One-site specific with ligand depletion (`one_site_specific_ligand_depletion`, quadratic mass-action root), Specific binding with Hill slope, Two-site specific, Allosteric modulator (`allosteric_modulator_saturation`), and Two-site total & nonspecific (`two_site_total_and_ns_global`).
- **Competition:** Built-in in `crates/fit/src/library/pharma.rs`: One-site & Two-site Fit $K_i$ (Cheng–Prusoff $\log \text{IC}_{50} = \log_{10}\!\big(10^{\log K_i}(1 + [\text{Hot}]/K_{d,\text{hot}})\big)$ with hooked constants `HotNM` and `HotKdNM`), One-site Fit $\log \text{IC}_{50}$ (`one_site_fit_logic50`), Homologous one-site competition (`homologous_one_site_competition`), and One-site competition with ligand depletion (`one_site_competition_ligand_depletion`).
- **Kinetics:** Dissociation (`dissociation_one_phase`), Association (one or two+ ligand concentrations globally fitting $k_{\text{on}}$ and $k_{\text{off}}$), Association-then-dissociation (`X < X0` vs. `X >= X0`), and **Motulsky–Mahan kinetics of competitive binding** (`motulsky_mahan_kinetics`).

### 3.3 Enzyme Kinetics
- **Substrate–Velocity:** Michaelis–Menten ($Y = \frac{V_{\max} X}{K_m + X}$), $k_{\text{cat}}$ ($Y = \frac{E_t k_{\text{cat}} X}{K_m + X}$, constant $E_t$), Allosteric sigmoidal ($Y = \frac{V_{\max} X^h}{K_{\text{half}}^h + X^h}$), Substrate inhibition ($Y = \frac{V_{\max} X}{K_m + X(1 + X/K_i)}$).
- **Reversible & Tight-Binding Inhibition (global over inhibitor `[I]`):** Competitive, Noncompetitive, Uncompetitive, Mixed-model ($\alpha K_i$), and **Morrison tight-binding $K_i$**:
  $$Y = v_0 \left(1 - \frac{(E_t + I + K_i^{\text{app}}) - \sqrt{(E_t + I + K_i^{\text{app}})^2 - 4 E_t I}}{2 E_t}\right), \qquad K_i^{\text{app}} = K_i \left(1 + \frac{S}{K_m}\right)$$

### 3.4 Exponentials, Lines, Polynomials, Peaks, Waves, Growth & Radiobiology
- **Exponential:** One-/two-/three-phase decay, one-phase association, plateau followed by one-phase decay/association, and exponential growth ($Y = Y_0 e^{kX}$, doubling time $\ln 2 / k$).
- **Lines & Polynomials:** Straight line (plus Deming Model II regression in `crates/fit/src/linear.rs`), line through origin, segmental linear ($X_0$ breakpoint), smooth hyperbolic hinge, semi-log/log-log/probit lines, power law ($Y = a X^b$), logarithmic curve, and centered & ordinary polynomials (`centered_poly_1..6`, `polynomial_2..6`).
- **Gaussian, Sine & Growth:** Gaussian (`gaussian`, `gaussian_baseline`), Log-Gaussian (`log_gaussian`), Cumulative Gaussian, sum of two Gaussians, Lorentzian, Weibull peak, sine and damped sine waves (each parameterized with `Baseline`) and sinc wave, Exponential plateau, Logistic growth (reporting inflection point $\frac{1}{k}\ln\!\left(\frac{Y_M - Y_0}{Y_0}\right)$), Gompertz growth, Beta growth-and-decline, and Linear–Quadratic surviving-fraction cell-survival ($S(D) = e^{-(\alpha D + \beta D^2)}$, `linear_quadratic_survival`).

### 3.5 Planned Built-In Equation Library Additions *(target specification — not yet registered in `PHARMA_EQUATION_IDS` / `CURVE_EQUATION_IDS`)*

1. **Linear-Concentration (`X = [C] > 0`) Normalized Dose–Response:**
   - `agonist_vs_response_normalized`: $Y = \frac{100 X}{\text{EC}_{50} + X}$ (reports derived $\log_{10}\text{EC}_{50}$),
   - `agonist_vs_response_normalized_var`: $Y = \frac{100}{1 + (\text{EC}_{50} / X)^{\text{HillSlope}}}$,
   - `inhibitor_vs_response_normalized`: $Y = \frac{100}{1 + X / \text{IC}_{50}}$ (reports derived $\log_{10}\text{IC}_{50}$),
   - `inhibitor_vs_response_normalized_var`: $Y = \frac{100}{1 + (\text{IC}_{50} / X)^{\text{HillSlope}}}$ (negative `HillSlope` for descending inhibition, matching standard Hill convention).
2. **Linear-Concentration (`X = [C]`) Special Dose–Response Counterparts & Mechanistic Pharmacology:**
   - Linear-concentration (`X = [C]`) counterparts for Asymmetrical 5PL ($Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{\left[1 + (2^{1/S} - 1)(\text{EC}_{50}/X)^{\text{HillSlope}}\right]^S}$; Gottschalk & Dunn 2005; Liao & Liu 2009), Biphasic, Bell-shaped, Operational Partial Agonist, Gaddum/Schild $\text{EC}_{50}$ shift, Allosteric $\text{EC}_{50}$ shift, $\text{EC}_F$, and Absolute $\text{IC}_{50}$ (Sebaugh 2011).
   - **Black–Leff Operational Model — Receptor Depletion (`operational_model_depletion_log` & `operational_model_depletion_conc`; Black & Leff 1983):** Globally shares `Basal`, `EffectMax`, agonist affinity $K_A$ (or $\log_{10} K_A$), and transducer slope $n$ across untreated control (`A`) and alkylated/receptor-depleted datasets (`B`, `C`, …), while fitting a local efficacy ratio $\tau_d$ (or $\log_{10}\tau_d$) per dataset:
     $$Y = \text{Basal} + \frac{\text{EffectMax} - \text{Basal}}{1 + \left(\frac{K_A + [A]}{\tau_d [A]}\right)^n}$$
   - **Competitive Gaddum/Schild with Unity Slope (`ec50_shift_gaddum_schild_unity_log` & `_conc`; Arunlakshana & Schild 1959; Colquhoun 2007):** Constrains `SchildSlope = 1` so $\text{p}A_2 = \text{p}K_B = -\log_{10} K_B$ directly estimates competitive antagonist equilibrium dissociation affinity across dataset column-title concentrations $[B]_d$:
     $$\log_{10}\text{EC}_{50,d} = \log_{10}\text{EC}_{50,\text{ctrl}} + \log_{10}\!\left(1 + \frac{[B]_d}{10^{-\text{p}K_B}}\right)$$
     Derived $K_B = 10^{-\text{p}K_B}$ reverses lower/upper confidence-limit ordering upon sign negation.
   - **Parallel Dose–Response Potency Fold-Shift (`ec50_shift_ratio_log` & `ec50_shift_ratio_conc`):** Shares `Bottom`, `Top`, and `HillSlope`; dataset `<A>` uses $\text{EC}_{50,\text{ctrl}}$ ($\log_{10}\text{EC}_{50,\text{ctrl}}$) and datasets `<~A>` use $\text{EC}_{50,\text{ctrl}} \cdot \text{EC}_{50\text{Ratio}}$ ($\log_{10}\text{EC}_{50,\text{ctrl}} + \log_{10}\text{EC}_{50\text{Ratio}}$), enabling a direct 1-df test against null $\text{EC}_{50\text{Ratio}} = 1$.
3. **Receptor Binding Additions:**
   - **One-site total saturation accounting for ligand depletion (`one_site_total_ligand_depletion`; Swillens 1995):** Fits total bound radioligand $Y$ (`cpm`) vs. total added radioligand $X$ (`cpm`) without separate nonspecific-binding wells, holding specific radioactivity `SpecAct` (`cpm/fmol`) and assay volume `Vol` (`mL`) constant with $K_{d,\text{cpm}} = K_{d,\text{nM}} \cdot \text{Vol} \cdot 1000 \cdot \text{SpecAct}$:
     $$a = -(1 + \text{NS}), \quad b = K_{d,\text{cpm}}(1 + \text{NS}) + X(1 + 2\text{NS}) + B_{\max}, \quad c = -X(\text{NS}\cdot K_{d,\text{cpm}} + \text{NS}\cdot X + B_{\max}), \quad Y = \frac{-b + \sqrt{b^2 - 4ac}}{2a}$$
   - **Two-site competitive binding fitting $\log \text{IC}_{50}$ (`two_sites_fit_logic50`):**
     $$Y = \text{Bottom} + (\text{Top} - \text{Bottom})\!\left[\frac{f_{\text{Hi}}}{1 + 10^{X - \log\text{IC}_{50,\text{Hi}}}} + \frac{1 - f_{\text{Hi}}}{1 + 10^{X - \log\text{IC}_{50,\text{Lo}}}}\right], \qquad f_{\text{Hi}} \in (0, 1)$$
   - **Allosteric modulator radioligand titration (`allosteric_modulator_competition`; Christopoulos & Kenakin 2002):** Ternary complex modulation of fixed radioligand $[\text{Hot}]$ (constant `RadioligandNM`, affinity `HotKdNM`) titrated against $X = \log_{10}[A]\text{ (M)}$ with modulator affinity $\log_{10} K_B$ and cooperativity factor $\alpha = 10^{\log_{10}\alpha}$:
     $$K_{\text{app}} = K_{d,\text{hot}} \frac{1 + [A]/K_B}{1 + \alpha [A]/K_B}, \qquad Y = Y_0 \cdot \frac{[\text{Hot}] + K_{d,\text{hot}}}{[\text{Hot}] + K_{\text{app}}}$$
4. **Exponentials, Intersecting Lines, Peak Areas, Waves, Growth, Radiobiology & Electrophysiology:**
   - **Two-phase exponential association (`two_phase_association`):**
     $$Y = Y_0 + (\text{Plateau} - Y_0)\!\left[p_{\text{fast}}\big(1 - e^{-K_{\text{fast}} X}\big) + (1 - p_{\text{fast}})\big(1 - e^{-K_{\text{slow}} X}\big)\right], \qquad p_{\text{fast}} = 0.01 \cdot \text{PercentFast}$$
     reporting $\tau_{\text{fast}} = 1/K_{\text{fast}}$, $\tau_{\text{slow}} = 1/K_{\text{slow}}$, $t_{1/2,\text{fast}} = \ln(2)/K_{\text{fast}}$, and $t_{1/2,\text{slow}} = \ln(2)/K_{\text{slow}}$.
   - **Crossing point of two lines (`two_lines_intersection`):** Global fit across two (or more) datasets $Y = Y_{\text{cross}} + \text{Slope}_d \cdot (X - X_{\text{cross}})$ with `SharedGlobal` $(X_{\text{cross}}, Y_{\text{cross}})$ and `Local` $\text{Slope}_d$, yielding direct SEs and profile-likelihood CIs for the intersection coordinates.
   - **Area-parameterized Gaussian (`gaussian_area`) & Lognormal (`lognormal_area`) peaks:**
     $$Y_{\text{Gauss}} = \frac{\text{Area}}{\sigma\sqrt{2\pi}} \exp\!\left(-\frac{(X - \mu)^2}{2\sigma^2}\right), \qquad Y_{\text{Lognorm}} = \frac{A}{X}\exp\!\left(-\frac{[\ln(X/\text{GeoMean})]^2}{2[\ln(\text{GeoSD})]^2}\right)$$
     reporting both peak `Amplitude` and integrated `Area` ($\text{Area}_{\text{Lognorm}} = A\sqrt{2\pi}\ln(\text{GeoSD})$).
   - **Zero-baseline 3-parameter Standard Sine Wave (`standard_sine_wave`):** $Y = \text{Amplitude}\cdot\sin\!\left(\frac{2\pi X}{\text{Wavelength}} + \text{PhaseShift}\right)$ (derived `Frequency = 1 / Wavelength`).
   - **Log of exponential growth (`log_exponential_growth`):** $Y = \ln Y_0 + k X$ for natural-log-transformed population counts under homoscedastic log-space scatter (reporting $Y_0 = e^{\ln Y_0}$ and doubling time $\ln(2)/k$).
   - **Complete 6-form Linear–Quadratic radiation response family (Chadwick & Leenhouts 1973; Brenner 2008):** Alongside surviving fraction $S(D) = e^{-(\alpha D + \beta D^2)}$ (`linear_quadratic_survival`), provide `linear_quadratic_percent_surviving` ($100 S(D)$), `linear_quadratic_number_surviving` ($Y_0 S(D)$), `linear_quadratic_fraction_dead` ($1 - S(D)$), `linear_quadratic_percent_dead` ($100[1 - S(D)]$), and `linear_quadratic_number_dead` ($Y_0[1 - S(D)]$).
   - **Boltzmann voltage-gated conductance sigmoid (`boltzmann_sigmoid`; Hodgkin & Huxley 1952):**
     $$Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + \exp\!\big((V_{50} - X)/\text{Slope}\big)}$$
     where $V_{50}$ is half-activation membrane potential and $\text{Slope} = RT / (z F)$ in potential units.
   - **Two-term power series (`power_series_two_term`):** $Y = A X^B + C X^D$ (guarded for $X = 0$ with negative exponents and $X < 0$ with non-integer exponents).

---

## 4. User-Defined Equation Language (`crates/fit/src/dsl/`)

`curvebook-fit` compiles user-entered equations into a bounded, bounds-checked stack bytecode VM (`MAX_VM_STEPS`, parse-depth and AST-node caps; zero recursion at evaluation time, zero panics on division by zero or domain errors):
- **Multiline intermediate assignments** ending in `Y = ...`,
- **Piecewise / conditional logic:** `IF(condition, true_expr, false_expr)` plus `AND`, `OR`, `NOT`, `<`, `<=`, `>`, `>=`, `=`, `<>`,
- **Dataset-scoped lines:** `<A> Y = ...`, `<B> Y = ...`, `<~A> Y = ...`, `<B:D> Y = ...` enabling global mechanistic models across heterogeneous experimental readouts (e.g., total vs. nonspecific binding, wild-type vs. mutant),
- **Column & multi-variable bindings:** Reading auxiliary independent variables from column titles or `MultipleVariables` predictors,
- **Implicit equations** ($Y$ appearing on both sides solved via safeguarded Brent/Newton inner root-finder) and **first-order ODEs** ($dY/dX = g(X, Y; \theta)$ integrated with adaptive Dormand–Prince RK45 and step-count ceiling),
- **Built-in scalar mathematical & special-function library** (`BuiltinFn` in `crates/fit/src/dsl/ast.rs`; overlaps the in-table formula functions in Section 4 of [`data-tables-and-transforms.md`](data-tables-and-transforms.md)): elementary/trig/hyperbolic functions, `erf`/`erfc`, `normcdf`/`norminv`, Bessel (`besselj0`, `besselj1`, `besseljn`, `bessely0`, `bessely1`, `besselyn`, `besseli0`, `besseli1`, `besselk0`, `besselk1`), gamma/beta/digamma (`gamma`, `lngamma`, `beta`, `digamma`/`psi`), and hypergeometric functions (`hyp1f1`, `hyp2f1`, `confluent_u`).

### 4.1 Planned Custom-Equation Built-In Function Additions *(target specification — backed by `curvebook-math` primitives)*

Expose the remaining probability, special, and unit-conversion functions in `crates/fit/src/dsl/` (`BuiltinFn` in `ast.rs`, name resolution in `compile.rs`, evaluation in `vm.rs`):
- **Regularized incomplete beta & gamma:** `ibeta(a, b, x)` ($I_x(a,b)$), `igamma(a, x)` ($P(a,x)$), `igammac(a, x)` ($Q(a,x)$),
- **Continuous & discrete probability distributions (tail-safe via `curvebook-math::dist`):**
  - Normal: `zdist(z)` (one-tailed $\Phi(z)$), `zinv(p)` ($\Phi^{-1}(p)$), `zpdf(z)` ($\phi(z)$), and 3-argument `normdist(x, mean, sd)`, `norminv(p, mean, sd)`, `normpdf(x, mean, sd)`,
  - Student *t*: `tdist(t, df)` (one-tailed upper tail), `tinv(p, df)` (two-tailed critical quantile), `tpdf(t, df)`,
  - Snedecor *F*: `fdist(f, df1, df2)` (upper tail), `finv(p, df1, df2)`, `fpdf(f, df1, df2)`,
  - Chi-square: `chidist(x2, df)` (upper tail), `chisqc(x2, df)` (lower cumulative $P(\chi^2_\nu \le x^2)$), `chiinv(p, df)`,
  - Binomial upper cumulative tail: `binomial(k, n, p)` $= \sum_{j = \lceil k \rceil}^{n} \binom{n}{j} p^j (1 - p)^{n - j}$,
- **Algebraic helpers & special-function aliases:** `fact(n)` ($n!$ for non-negative integers), `logit(p)` $= \ln(p/(1-p))$ for $p \in (0,1)$, `probit(p)` $= 5 + \Phi^{-1}(p)$, `perc(x)` $= x / 100$, `deg(rad)`, `rad(deg)`, `sqr(x)` $= x^2$, `int(x)` (truncate toward zero), `sgn(x)`, infix/function `mod(x, y)`, `is_defined(x)` (non-blank/finite predicate for multivariable rows), general integer-order modified Bessel `besseli(n, x)` / `besselk(n, x)`, and canonical aliases `gammaln` ($\equiv$ `lngamma`), `hypgeometricm` ($\equiv$ `hyp1f1`), `hypgeometricu` ($\equiv$ `confluent_u`), and `hypgeometricf` ($\equiv$ `hyp2f1`).

### 4.2 Planned User-Defined `[InitialValues]` Rule Grammar *(target specification)*

Allow user-defined equations to store reusable data-driven starting-value rules of the form `multiplier * Basis` (or `multiplier / Basis`), evaluated independently per dataset unless overridden manually:
- **Literal constant:** `c * (InitialValue)` (data-independent starting estimate $c$),
- **Range anchors:** `YMIN`, `YMAX`, `YMID` $= \frac{1}{2}(Y_{\min} + Y_{\max})$, `XMIN`, `XMAX`, `XMID` $= \frac{1}{2}(X_{\min} + X_{\max})$,
- **Linearly interpolated cross-anchors:** `XatYmin`, `XatYmid`, `XatYmax` and `YatXmin`, `YatXmid`, `YatXmax`,
- **Monotonicity sign:** `SIGN(YatXmax - YatXmin)` $\in \{+1, -1\}$ (e.g., seeding positive vs. negative `HillSlope`),
- **Column-title statistics:** arithmetic mean of numeric dataset column titles (`MeanColumnTitle`) and $\log_{10}(\text{MeanColumnTitle})$,
- **Endpoint & midpoint empirical slopes (`SlopeAtXmin`, `SlopeAtXmid`, `SlopeAtXmax` and their reciprocals):**
  - When $N \ge 30$: OLS slope $\Delta Y / \Delta X$ across the first, middle, or last $\lfloor 0.10 N \rfloor$ sorted $(X, Y)$ points;
  - When $N < 30$: two-point secant slope on the first two or last two points, and two-point (even $N$) or three-point OLS (odd $N$) slope on the central points for `SlopeAtXmid`.

---

## 5. Simple Linear, Deming, Simple Logistic & Multivariable Regression Specifications

### 5.1 Simple Linear Regression, Fixed-Point Constraints & Standard Addition (`crates/fit/src/linear.rs`)
- **Implemented core:** OLS slope & $Y$-intercept ($Y = \beta_0 + \beta_1 X$) or forced-origin regression ($Y = \beta_1 X$), $R^2$, residual SD $s_{y \cdot x}$, slope $\ne 0$ *F*/*t* test, Working–Hotelling / Student-*t* confidence & prediction bands, runs test, ANCOVA multi-dataset slope & intercept comparison (`compare_linear_lines`), bidirectional unknown interpolation (`Y` from `X` and exact quadratic Fieller `X` from `Y` calibration & mean-response intervals, `R4.6`), and domain-restricted fitting (`x_range_filter` & `[plot_x_min, plot_x_max]`, `R13.1`).
- **Planned extensions *(target specification)*:**
  1. **Arbitrary fixed-point constraint `(X0, Y0)`:** Fit $Y - Y_0 = \beta_1 (X - X_0)$ ($\text{df} = N - 1$), generalizing `force_through_origin` $(0, 0)$.
  2. **$X$-intercept Fieller interval & Standard Addition Method:** Always report $\hat{X}_{\text{int}} = -\hat{\beta}_0 / \hat{\beta}_1$ with exact Fieller (1940) confidence limits (emitting `InverseIntervalSet` unbounded/exclusive regions when slope is not significantly non-zero). For matrix-effect-corrected **Standard Addition** assays (known spike concentrations $X \ge 0$ added to constant unknown matrix), $|\hat{X}_{\text{int}}| = -\hat{X}_{\text{int}}$ estimates unknown sample concentration with negated, order-swapped Fieller limits $[-U_{X_{\text{int}}}, -L_{X_{\text{int}}}]$, paired with an option to extend the plotted regression line down to $X = \hat{X}_{\text{int}}$ ($Y = 0$).

### 5.2 Deming (Model II) Errors-in-Variables Regression (`Deming 1943`; `Linnet 1990, 1993`)
- **Implemented core (`crates/fit/src/linear.rs`):** Closed-form orthogonal ($\lambda = 1$) and general variance-ratio ($\lambda = \sigma_X^2 / \sigma_Y^2$, `R4.6`) Deming slope and intercepts with delete-1 jackknife SEs, *t*-based CIs, and zero-slope hypothesis test; omits $R^2$ by design because vertical explained-variance fraction is undefined when both $X$ and $Y$ carry measurement error.
- **Planned document/UI wiring *(target specification)*:** Persist unequal error parameters (`DemingErrorSpec`: `EqualError` $\lambda = 1$, explicit `(sd_x, sd_y)` computing $\lambda = \text{SD}_X^2 / \text{SD}_Y^2$, or direct `variance_ratio_lambda`) on `LinearRegressionSpec` in `crates/doc/src/analysis.rs` (which currently exposes only boolean `deming_model_ii`), plus a duplicate-pair repeatability helper $\hat{\sigma}_{\text{err}} = \sqrt{\frac{1}{2m}\sum_{j=1}^m d_j^2}$ from $m$ duplicate sample pairs.

### 5.3 Planned Standalone XY Simple Logistic Regression *(target specification — `Hosmer, Lemeshow & Sturdivant 2013`; `Tjur 2009`)*
Provide a dedicated `SimpleLogisticRegression` analysis directly on single-subcolumn `XY` tables where $Y_i \in \{0, 1\}$ (rejecting non-binary $Y$ values with a structured `StatsError` / `FitError`):
- **Model & MLE:** $\ln\!\left(\frac{P(Y=1 \mid X)}{1 - P(Y=1 \mid X)}\right) = \beta_0 + \beta_1 X$ solved by Newton–Raphson / Fisher scoring IRLS.
- **Reported parameters & intervals:** $\hat{\beta}_0, \hat{\beta}_1$, odds ratios $e^{\hat{\beta}_0}, e^{\hat{\beta}_1}$, **`X at 50%`** ($X_{50} = -\hat{\beta}_0 / \hat{\beta}_1$ where $P = 0.5$), asymptotic SEs, and asymmetrical profile-likelihood CIs (including propagated odds-ratio and $X_{50}$ intervals).
- **Hypothesis tests, goodness-of-fit & classification:**
  - Wald $Z$-test and likelihood-ratio test ($G^2 = D_0 - D_1 \sim \chi^2_1$ against intercept-only null) for $\beta_1 = 0$,
  - Model deviance $D_1 = -2 \ell(\hat{\beta})$, $\text{AIC}_c$, **Tjur's coefficient of discrimination** ($\bar{\hat{p}}_{Y=1} - \bar{\hat{p}}_{Y=0}$; Tjur 2009), and **Cox–Snell generalized $R^2$** ($1 - \exp(-G^2 / N)$),
  - Empirical ROC curve + AUC with SE/CI,
  - Optional **`Row Classification`** output tab copying observed $X_i$ alongside predicted probabilities $\hat{P}(X_i)$ and automatically predicting $\hat{P}(X_{\text{new}})$ (+ CI) for appended `(X_new, blank Y)` rows without requiring extra configuration,
  - Sigmoid probability curve overlay evaluated across auto or user-specified `[plot_x_min, plot_x_max]`,
  - Explicit detection and plain-language reporting of **perfect separation**, **quasi-perfect separation**, and collinearity (`Heinze & Schemper 2002`) rather than diverging parameter estimates.

### 5.4 Planned Multivariable Linear, Poisson & Logistic GLM Extensions *(target specification — cross-referenced with [`statistics-implementation.md`](statistics-implementation.md#53-roc-curves-method-comparison-multiple-regression-imputation-pca--clustering))*
Extend `GlmSpec` (`crates/doc/src/analysis.rs` & `crates/stats/src/regression.rs`) beyond main-effect `predictor_columns`, `reference_levels`, and `compute_vif`:
1. **Model formula terms & intercept control:** Explicit two-way ($A \times B$) and three-way ($A \times B \times C$) interaction terms, polynomial powers ($X^2, X^3$), predictor transforms, and `include_intercept: bool` (with documented adjusted-$R^2$ / null-deviance conventions when the intercept is omitted).
2. **Nested GLM comparison:** Compare full vs. reduced `GlmSpec` models on identical complete-case rows via Extra-SS *F*-test (linear), deviance likelihood-ratio $\chi^2$ test (logistic & Poisson), and $\Delta\text{AIC}_c$.
3. **Differential weighting & Poisson overdispersion:** Support observation weights ($1/Y^2, 1/|Y|, 1/|Y|^k, 1/X, 1/X^2$, or weight column) in multiple linear regression, and report Pearson $\chi^2 / \text{df}$ dispersion ratio (plus optional quasi-Poisson / robust sandwich SE scaling) in Poisson regression.
4. **Row-level influence & residual diagnostics:** Leverage $h_{ii}$ (diagonal of hat matrix $W^{1/2} X (X^\top W X)^{-1} X^\top W^{1/2}$), raw / standardized / studentized / Pearson / deviance residuals, **Cook's distance** $D_i$ (Cook 1977), and **DFBETAs**, with selectable residual vs. predicted $\hat{Y}$, residual vs. predictor $X_j$, homoscedasticity, Q–Q, and Cook's-distance plots.
5. **Out-of-sample row prediction & logistic cutoff:** Automatically predict $\hat{Y}$ (or $\hat{\mu}, \hat{P}$) with confidence and prediction intervals for `MultipleVariables` rows having complete predictors and blank $Y$; expose configurable classification probability threshold $p_{\text{cut}} \in (0, 1)$ (default `0.5`) for confusion matrices, sensitivity/specificity, PPV/NPV, and positive/negative predictive summaries.

---

## 6. Planned Model-Free Curve Fitting (`Spline / LOWESS`) & Theoretical Distribution Plotting *(target specification)*

### 6.1 Dedicated XY `FitSplineLowess` Analysis (`crates/transform/src/curve.rs` & `crates/doc/src/analysis.rs`)
Expose a first-class model-free curve-fitting analysis on `XY` tables producing both a smooth `FittedCurveOverlay` (`segment_count` polyline vertices) and optional standard-curve interpolation:
1. **Natural Cubic Spline (`de Boor 1978`):** $C^2$ piecewise cubic polynomial passing through every distinct sorted knot $(x_g, \bar{y}_g)$ with zero second derivatives at endpoints ($S''(x_1) = S''(x_G) = 0$).
2. **Akima Subspline (`Akima 1970`):** $C^1$ piecewise cubic interpolation computing local nodal slopes from five-point secant differences $m_i = \frac{|d_{i+1} - d_i| d_{i-1} + |d_{i-1} - d_{i-2}| d_i}{|d_{i+1} - d_i| + |d_{i-1} - d_{i-2}|}$ to suppress Runge/overshoot oscillations around sharp transitions.
3. **Smoothing Spline (`Reinsch 1967`):** Penalized cubic B-spline minimizing $\sum_g w_g (\bar{y}_g - S(x_g))^2 + \lambda \int [S''(x)]^2\,dx$ parameterized by user-selected knot count / effective degrees of freedom.
4. **LOWESS (`Cleveland 1979`):** Tricube-weighted local linear regression with Tukey bisquare robustness passes, offering `Coarse`, `Medium`, and `Fine` window presets (plus custom fraction $f \in (0, 1]$), enforcing `segment_count >= 4 * N` and emitting an assumption warning when $N < 20$.
5. **Dense Point-to-Point Curve:** Piecewise linear interpolation in data space resampled onto `segment_count` $(X, Y)$ points (distinct from straight graph-space segment connectors when rendered on logarithmic axes), enabling AUC integration or piecewise-linear standard-curve lookup.
6. **Curve XY Output & Nonparametric Standard-Curve Interpolation:** Optional `emit_curve_coordinates_tab: bool` outputting the sampled $(X, Y)$ table, and optional `interpolate_unknowns: bool` solving $S(x) = y_{\text{unk}}$ across spline/Akima/LOWESS/point-to-point segments for unpaired `(blank X, Y)` rows (and evaluating $S(x_{\text{unk}})$ for `(X, blank Y)` rows).

### 6.2 Theoretical Curve Families & Probability Distribution Plotting (`Plot a function`)
Support generating one or more theoretical curve columns without an input `DataSheet` over $[X_{\min}, X_{\max}]$ (`linear` or `log` grid spacing, configurable point/step count):
- **Dataset-conditional tail-shading families:** Multi-dataset DSL expressions (`<A> Y = pdf(X)`, `<B> Y = IF(X <= -c, pdf(X), 0)`, `<C> Y = IF(X >= c, pdf(X), 0)`) combined with Info-sheet `HookedConstant` parameters ($\nu_1, \nu_2, \lambda, n, p$, critical cutoff $c$) so editing a parameter on an `InfoSheet` reactively updates shaded one- or two-tailed $z$, Student $t$, Snedecor $F$, and $\chi^2$ density graphs.
- **Discrete probability mass functions:** Integer-step evaluation ($\Delta X = 1$ over $X = 0, 1, \dots, M$) for $\text{Binomial}(n, p)$ and $\text{Poisson}(\lambda)$ renderable as superimposed bar/spike charts with highlighted tail regions.

---

**Mandatory test gate:** `crates/fit/tests/` (`nist_strd.rs`) validates all 27 NIST StRD nonlinear regression benchmarks (`Misra1a`–`Misra1d`, `Chwirut1`–`Chwirut2`, `Lanczos1`–`Lanczos3`, `Gauss1`–`Gauss3`, `DanWood`, `Kirby2`, `Hahn1`, `Nelson`, `MGH09`, `MGH10`, `MGH17`, `BoxBOD`, `Rat42`, `Rat43`, `Bennett5`, `Eckerle4`, `Thurber`, `Roszman1`, `ENSO`) from both Start 1 (far) and Start 2 (near) initial values, asserting LRE $\ge 6.5$ on estimates and residual SD, $\ge 7.0$ on RSS, and $\ge 5.0$ on SEs (documented `Lanczos1` machine-floor exemptions); the suite skips when the external corpus is absent unless `CURVEBOOK_REQUIRE_CORPUS=1` (`cargo xtask corpus`).
