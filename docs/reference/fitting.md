# Curve Fitting & Custom Equation Reference

CurveBook's fitting subsystem (`curvebook-fit`, Layer `L2`) implements simple linear regression, Deming (Model II) regression, trust-region Levenberg–Marquardt nonlinear least squares, weighted least squares (WLS), Poisson deviance IRLS, Cauchy robust regression, ROUT outlier elimination, profile-likelihood intervals, and a bytecode-compiled algebraic/implicit/ODE custom equation DSL.

See also the corresponding entries in the [Analyses Reference](analyses.md#analysis-linear-regression) ([Linear Regression](analyses.md#analysis-linear-regression), [Nonlinear Regression](analyses.md#analysis-nonlinear-regression)) and [Command Reference](../commands.md).

---

## Fitting Methods & Objective Functions (`FitMethod`) {#fitting-methods}

Configured via `NonlinearFitSpec::method` (engine parameter `method`, defaulting to `LeastSquares`):

1. **`LeastSquares` (Ordinary & Weighted Sum of Squares):**
   - Minimizes $\text{SS}_w(\theta) = \sum_i w_i \left(y_i - f(x_i; \theta)\right)^2$ using a trust-region **Levenberg–Marquardt** algorithm with adaptive column scaling, trust-radius updates, and `ftol`/`xtol`/`gtol` termination tests following Levenberg (1944), Marquardt (1963), Moré (1978), and MINPACK-1 (Moré, Garbow & Hillstrom 1980).
   - Warm-start stabilization: when automatic initial guesses produce large residuals or fail evaluation at a corner, a Nelder–Mead simplex warm start (Nelder & Mead 1965; Lagarias et al. 1998) runs prior to Levenberg–Marquardt polishing (`use_simplex_warm_start`).
   - Jacobians use exact analytical derivatives (`ParametricModel::grad`) where implemented and central finite differences (`~eps^(1/3)` step size, Nocedal & Wright 2006) elsewhere.
   - Summarized `MeanSdN` / `MeanSemN` inputs account for within-point sample size $n_i$ and within-replicate sum of squares $(n_i - 1)\,\text{SD}_i^2$ (`validated_replicate_moments`) when computing overall residual degrees of freedom, goodness-of-fit $\text{RSS}$, and lack-of-fit tests.

2. **`PoissonDeviance` (Log-Likelihood / Poisson Count IRLS):**
   - Minimizes exact Poisson deviance $D(y, \mu) = 2 \sum_i \left[y_i \ln(y_i / \mu_i) - (y_i - \mu_i)\right]$ via Iteratively Reweighted Least Squares (Green 1984; McCullagh & Nelder 1989).
   - Assumes non-negative integer/count responses $y_i \ge 0$ and strictly positive fitted means $\mu_i > 0$. Explicit weighting schemes are ignored because variance equals the Poisson mean ($\text{Var}(Y) = \mu$).
   - Model selection uses Poisson log-likelihood $\ell$, `AICc`, `BIC`, and `LikelihoodRatioTest` between nested models rather than Gaussian sum-of-squares *F*-tests.

3. **`RobustCauchy` (Outlier-Resistant Lorentzian M-Estimator):**
   - Minimizes the Lorentzian / Cauchy loss $\sum_i \ln\!\left(1 + \left(r_i / \text{RSDR}\right)^2\right)$ using IRLS (Holland & Welsch 1977; Motulsky & Brown 2006) without removing points.
   - Residual scale is estimated robustly from the 68.27th percentile of sorted $|r_i|$ corrected by $N / (N - K)$ (`compute_rsdr`).
   - Following Motulsky & Brown (2006), asymptotic standard errors, symmetric CIs, and Gaussian least-squares bands are intentionally omitted (`InferenceAvailability` marks them unavailable) because standard normal Hessian variance assumptions do not hold under heavy-tailed Lorentzian loss. Requires raw replicates (`RequiresRawReplicates` on pre-summarized inputs).

4. **`RoutOutlierElimination { fdr_q_percent }` (Default $Q = 1.0\%$):**
   - Implements Motulsky & Brown (2006) two-stage outlier identification:
     1. Fits a Cauchy robust curve (`RobustCauchy`) to prevent leverage masking by outliers, computing scaled residuals relative to `RSDR`.
     2. Computes two-tailed *t*-test *P*-values (`df = N - K`) on the furthest ~30% of observations (`start = floor(0.70 * N)`) and applies the Benjamini–Hochberg (1995) step-up FDR procedure at desired false discovery rate $Q \in (0, 100]\%$ (default `1.0`%).
     3. Flags detected outliers in a diagnostic `RoutPointAudit` table and refits retained inliers with `LeastSquares` (preserving configured weighting, asymptotic SEs, profile CIs, and bands on the inlier set). Requires raw replicates (`N > K`).

---

## Differential Weighting (`WeightingScheme`) {#weighting}

Configured via `NonlinearFitSpec::weighting` (or `weighting` in `fit.run`; default `None`):

| Variant | Weight $w_i$ | Statistical Variance Model | Domain Guard |
|---|---|---|---|
| `None` | $1$ | Constant residual SD (homoscedastic) | Finite $Y$ |
| `RelativeSquare` (`1/y2`) | $1 / Y_i^2$ | Constant relative error (%CV; Motulsky & Christopoulos 2004) | Rejects $Y_i = 0$ with `ImpossibleWeights` |
| `Relative` (`1/y`) | $1 / |Y_i|$ | Poisson-like variance $\propto |Y|$ | Rejects $Y_i = 0$ with `ImpossibleWeights` |
| `GeneralPowerY(k)` | $1 / |Y_i|^k$ | Power variance $\text{Var}(Y) \propto |Y|^k$ | Rejects $Y_i = 0$ when $k > 0$ or overflow |
| `InverseX` (`1/x`) | $1 / |X_i|$ | Variance proportional to predictor magnitude | Rejects $X_i = 0$ |
| `InverseX2` (`1/x2`) | $1 / X_i^2$ | Variance proportional to $X^2$ | Rejects $X_i = 0$ |
| `ObservedSd` | $1 / \text{SD}_i^2$ | Inverse empirical variance at summarized rows | Requires pre-summarized `MeanSdN` / `MeanSemN` with finite $\text{SD}_i > 0$ |

Weights are computed deterministically from **observed** values (or explicit `explicit_weight > 0`) rather than dynamically from fitted $\hat{y}_i$, ensuring objective stability; invalid zero or non-finite denominators return `FitError::ImpossibleWeights` rather than clamping.

---

## Parameter Sharing, Constants & Box Bounds (`ParameterRule`) {#sharing-and-constraints}

Each rule in `parameter_rules` targets an exact equation parameter name across datasets (`crates/doc/src/analysis.rs`):

- **Scopes (`ParameterScope`):**
  - `Local`: Independent free parameter estimated separately for each fitted dataset.
  - `SharedGlobal`: One shared global parameter jointly optimized across all datasets simultaneously (verified invariant under dataset row/column permutations in `d1_edge_matrix.rs`).
  - `Constant(v)`: Fixed literal value consuming `0` degrees of freedom.
  - `HookedConstant(HookSource)`: Dynamically fixed to an upstream `InfoSheet` constant or `ResultSheet` scalar through the reactive DAG.
  - `PerDatasetConstant(Vec<(usize, f64)>)`: Fixes specific dataset indices while leaving unlisted datasets `Local`.
  - `ColumnTitleConstant`: Parses the leading numeric scalar from each dataset's column header (e.g., inhibitor/antagonist concentration `[B]` for Gaddum–Schild / competitive models).
- **Initial guesses & inclusive box bounds:** `initial_override` overrides the equation's data-driven initializer; `lower_bound` and `upper_bound` enforce inclusive constraints $\theta_k \in [L_k, U_k]$ projected cleanly during trust-region iterations. Parameters converging onto active bounds are annotated `ParameterNote::HitLowerBound` or `ParameterNote::HitUpperBound`; equal bounds fix the parameter value with zero free parameters.

---

## Uncertainty, Diagnostics & Model Comparison {#uncertainty-and-comparison}

### Covariance, Collinearity & Confidence Intervals {#uncertainty-and-ci}

- **Numerical Rank & Collinearity (`CovarianceDiagnostics`):** QR with column pivoting and SVD determine `numerical_rank` and the unscaled covariance $(J^\top W J)^{-1}$. Each free parameter reports R² against all other parameters (`dependencies[j]`): if $\text{Dependency}_j > 0.9999$ or rank is deficient, `ambiguous = true`, `FitStatus::Ambiguous`, and `ParameterNote::Ambiguous` warn of structural non-identifiability (Bates & Watts 1988).
- **Asymptotic Wald Standard Errors & Symmetric CIs:** $\text{SE}(\hat{\theta}_j) = \hat{\sigma}\sqrt{[(J^\top W J)^{-1}]_{jj}}$ where $\hat{\sigma} = \sqrt{\text{SS}_w / df}$, yielding $\hat{\theta}_j \pm t_{1-\alpha/2,\,df}\,\text{SE}(\hat{\theta}_j)$ at `AnalysisSpec::confidence_level` (default `95%`).
- **Venzon–Moolgavkar Profile-Likelihood CIs (`profile_likelihood_ci`):** Traces exact likelihood contours $\text{SS}(\theta_j) \le \text{SS}_{\min}\!\left(1 + \frac{F_{1-\alpha,\,1,\,df}}{df}\right)$ (or deviance threshold $\chi^2_{1-\alpha, 1}$ for `PoissonDeviance`) via Brent root finding (Venzon & Moolgavkar 1988; Bates & Watts 1988). When the likelihood surface flattens before reaching the threshold on either side, that side emits `CiBound::Open` and `ParameterNote::UnboundedCi` instead of a spurious finite interval.
- **Hougaard Parameter-Effects Skewness:** Computes $|g_{1,j}|$ curvature (`compute_hougaard_skewness`, Hougaard 1985; Ratkowsky 1990) to quantify departure from linear asymptotic normality ($|g_{1}| < 0.25$ indicates nearly linear behavior; $> 1.0$ flags severe skewness favoring profile-likelihood CIs).

### Residual Diagnostics {#residual-diagnostics}

When enabled in `NonlinearFitSpec` / `SolverConfig`, CurveBook emits non-mutating diagnostic tabs and [`QOL-7`](qol.md#active-qol) checklist items:

- **Wald–Wolfowitz Runs Test (`runs_test`, `compute_runs_test`):** Tests sign runs above/below the curve to diagnose systematic curve curvature/mis-specification (Wald & Wolfowitz 1940; Swed & Eisenhart 1943).
- **Replicates Lack-of-Fit *F*-Test (`replicates_lack_of_fit`, `replicates_lack_of_fit_test`):** Partitions residual SS into within-X-replicate pure error and model lack-of-fit SS (`Draper & Smith 1998`).
- **Residual Normality Battery (`normality_of_residuals`):** Shapiro–Wilk (Royston 1995), D'Agostino–Pearson $K^2$, Anderson–Darling $A^{2*}$, and Lilliefors KS over weighted residuals.
- **Homoscedasticity Check (`homoscedasticity_test`):** Spearman rank correlation between $|r_i|$ and $|\hat{y}_i|$ (`homoscedasticity_spearman_test`).

### Model Comparison (`ModelComparisonSpec`) {#model-comparison}

- **`ExtraSumOfSquaresF`:** Compares two nested Gaussian models via $F = \frac{(\text{SS}_{\text{null}} - \text{SS}_{\text{alt}})/(df_{\text{null}} - df_{\text{alt}})}{\text{SS}_{\text{alt}}/df_{\text{alt}}}$ (Motulsky & Christopoulos 2004).
- **`Aicc`:** Evaluates bias-corrected AICc $\text{AIC}_c = N\ln(\text{SS}/N) + 2K_{\text{eff}} + \frac{2K_{\text{eff}}(K_{\text{eff}}+1)}{N - K_{\text{eff}} - 1}$ (`K_eff = K + 1` including variance parameter; Hurvich & Tsai 1989; Akaike 1974), reporting $\Delta\text{AIC}_c$, Akaike model weights $w_i$, and evidence ratios for both nested and non-nested candidate models (Burnham & Anderson 2002).
- **`LikelihoodRatioTest`:** Evaluates $\Delta D = D_{\text{null}} - D_{\text{alt}} \sim \chi^2_{df_{\text{null}} - df_{\text{alt}}}$ for nested Poisson deviance fits (Wilks 1938).

---

## Bands & Unknown Interpolation {#bands-and-interpolation}

- **Confidence & Prediction Bands (`confidence_bands`, `prediction_bands`):** Along the evaluation grid (`curve_overlay_segments = 256` default), delta-method standard error $\text{SE}(\hat{y}(x)) = \hat{\sigma}\sqrt{g(x)^\top (J^\top W J)^{-1} g(x)}$ yields two-sided $1 - \alpha$ mean-response **confidence bands**, while $\hat{\sigma}\sqrt{w_{\text{new}}^{-1} + g(x)^\top (J^\top W J)^{-1} g(x)}$ yields single-future-observation **prediction bands** (`new_observation_inv_weight` consistent with the chosen `WeightingScheme`).
- **Unknown Interpolation (`interpolate_unknowns`):**
  - Unpaired `(empty, Y)` rows interpolate $X$ from $Y$ by bracketed grid scan + `brent_root` (Brent 1973), computing asymmetric confidence and prediction intervals by intersecting $y_{\text{target}}$ with the fitted curve bands (`band_crossing_interval`, falling back to `CiBound::Open` when non-crossing or outside top/bottom plateaus — annotated `"AboveTopPlateau"` / `"BelowBottomPlateau"`).
  - Unpaired `(X, empty)` rows evaluate $Y(X)$ forward with delta-method confidence and prediction intervals.
  - For straight-line OLS, calibration uses exact Fieller (1954) quadratic set classification (`Bounded`, `HalfLine`, `TwoRays`, `AllReals`, `Undefined`; Seber & Lee 2003, §6.1.3).

---

## Built-In Equation Library & Custom DSL (`CompiledDslModel`) {#builtin-and-dsl}

### Built-In Equation Namespaces (`EquationCategory`) {#builtin-equations}

`curvebook-fit::library` provides built-in `ParametricModel` implementations with published functional-form citations, analytical gradients, intrinsic default bounds, and derived scalars (e.g., `EC50`, `IC50`, `Km`, `Span`, `t_half`, `Tau`), organized across categories including:

- **Dose–Response Stimulation & Inhibition** (Hill / 3PL / 4PL / 5PL log & linear concentration, $\log\text{EC}_F$, Gaddum–Schild & allosteric $\text{EC}_{50}$ shifts, Black–Leff operational agonism, bell-shaped, biphasic, absolute $\text{IC}_{50}$)
- **Receptor Binding** (Langmuir one-site specific/Hill, one/two-site total & non-specific global sharing, ligand-depletion Swillens/Wang, homologous competition DeBlasi, allosteric saturation Ehlert, Cheng–Prusoff $K_i$, association/dissociation kinetics Motulsky–Mahan)
- **Enzyme Kinetics** (Michaelis–Menten, substrate inhibition Haldane, Morrison tight-binding, competitive/noncompetitive/uncompetitive/mixed inhibition Segel)
- **Exponential, Growth, Peaks, Waves, Radiobiology & Standard Curves** (one/two/three-phase decay & association, plateau delays, logistic/Gompertz/Beta growth, linear-quadratic survival, Gaussian/Lorentzian/Weibull/log-Gaussian peaks, damped/sinc waves, centered & standard polynomials, segmental & Watts–Bacon smooth hinge, power-law, log-log, probit)
- **NIST StRD Benchmark Models** (dedicated exact-Jacobian models for the NIST StRD Nonlinear Least Squares corpus).

### Custom Algebraic, Implicit & ODE DSL (`crates/fit/src/dsl.rs`) {#custom-dsl}

Custom equations (`custom_dsl`) compile to a bounded, pure-Rust WASM-safe stack VM supporting:

1. **Explicit & Multiline Local Formulas:** `Y = ...`, intermediate local variable assignments (`top = ...`), and dataset-indexed prefixes (`<A>`, `<~A>`, `<B:D>`) for global/shared multi-curve models.
2. **Short-Circuit Conditional Logic:** `IF(cond, then_expr, else_expr)` with comparisons (`<`, `<=`, `>`, `>=`, `==`, `!=`) and `AND`/`OR`/`NOT`.
3. **Implicit Equations:** Equations containing `Y` on both sides or nonlinear in `Y` (`LHS = RHS`), solved via bracket scanning and Brent's root finder (`brent_root`, returning `ImplicitSolveFailed` when unbracketed).
4. **First-Order ODE Models:** `dY/dX = ...`, `dY/dt = ...`, or `D(Y, X) = ...` with initial condition `Y(x0) = ...`, integrated deterministically using adaptive 5th-order **Dormand–Prince RK45** (`integrate_rk45`, Dormand & Prince 1980; Hairer, Nørsett & Wanner 1993) with cached trajectories per evaluation.
5. **DSL Built-in Math & Special Functions (`BuiltinFn`):**
   `abs`, `sqrt`, `cbrt`, `exp`, `exp10`, `ln`, `log10`, `log2`, `pow`, `sin`/`cos`/`tan`/`asin`/`acos`/`atan`/`atan2`, `sinh`/`cosh`/`tanh`/`asinh`/`acosh`/`atanh`, `floor`/`ceil`/`round`/`trunc`/`sign`, `min`/`max`/`clamp`/`hypot`, `erf`/`erfc`, `norm_cdf`/`norm_inv`, `gamma`/`ln_gamma`/`beta`/`digamma`, integer/order Bessel functions `j0`/`j1`/`jn`/`y0`/`y1`/`yn`/`i0`/`i1`/`k0`/`k1` (up to order `100,000`), Kummer confluent hypergeometric `1F1`, Tricomi `confluent_u`, and Gauss hypergeometric `2F1` (Abramowitz & Stegun 1964; NIST DLMF).

| DSL Safety Constant | Value | Protected Boundary |
|---|---:|---|
| `MAX_DSL_SOURCE_BYTES` | `1,048,576` (`1 MiB`) | Maximum custom equation source size |
| `MAX_DSL_LINES` | `16,384` | Maximum source lines per equation |
| `MAX_PARSE_DEPTH` | `128` | Maximum recursive expression nesting |
| `MAX_AST_NODES` | `4,096` | Maximum AST nodes parsed per equation |
| `MAX_TOTAL_OPCODES` | `500,000` | Maximum bytecode instructions emitted |
| `MAX_VM_STEPS` | `100,000` | Maximum VM instructions per `(x, theta)` evaluation |
| `MAX_ODE_STEPS` | `50,000` | Maximum RK45 adaptive steps per curve integration |
| `DEFAULT_MAX_ODE_STEPS_PER_FIT` | `300,000` | Total RK45 step budget across one nonlinear fit execution |

---

## Literature Citations & Oracle Verification (`D1` / NIST StRD) {#verification-and-citations}

- **NIST StRD Certification (`crates/fit/tests/nist_strd.rs` & `d1_nist_and_pharma_oracles.rs`):** Verified against the NIST StRD Nonlinear Least Squares suite from both `start_1` and `start_2` (`LRE >= 6.5` digits on estimated $\hat{\beta}$, `LRE >= 6.5` on `RSD`, `LRE >= 7.0` on `RSS`, `LRE >= 5.0` on asymptotic parameter SEs except `Lanczos1` with `RSS < 1e-22`, `RSD < 1e-11`, and `SE LRE >= 2.5`).
- **Analytical & `D1` Reference Suites (`d1_edge_matrix.rs`, `d1_nist_and_pharma_oracles.rs`):** Verify exact recovery (`<= 1e-6`–`1e-12` relative/absolute agreement and normal-equation gradient norm `|J^T r| <= 1e-5`–`1e-6`) across pharmacology 3PL/4PL/5PL, binding global models, enzyme kinetics, multi-phase decays, collinear flat plateaus, active box constraints, 10-decade `1/Y^2` & `1/SD^2` weighting, `20%`-outlier ROUT recovery, Poisson deviance IRLS, Extra-SS *F* / AICc comparisons vs. R 4.5.2, and Deming jackknife / Fieller linear calibration.
- **Core References:** Bates & Watts (1988); Brent (1973); Burnham & Anderson (2002); Deming (1943); Dormand & Prince (1980); Draper & Smith (1998); Fieller (1954); Green (1984); Holland & Welsch (1977); Hougaard (1985); Hurvich & Tsai (1989); Levenberg (1944); Marquardt (1963); McCullagh & Nelder (1989); Moré (1978); Motulsky & Brown (2006); Motulsky & Christopoulos (2004); Nelder & Mead (1965); Nocedal & Wright (2006); Ratkowsky (1990); Seber & Lee (2003); Seber & Wild (1989); Venzon & Moolgavkar (1988).
