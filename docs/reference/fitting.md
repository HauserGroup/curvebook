# Curve Fitting & Nonlinear Regression Reference

This reference documents CurveBook's parametric linear and nonlinear regression algorithms, residual weighting schemes, global parameter sharing and constant constraints, profile-likelihood uncertainty, R13.1 diagnostic safeguards, model comparison, unknown interpolation, custom equation DSL, and adaptive ODE solver. For the full compiled catalog of built-in equations, parameters, and solver defaults, see the [Parametric Equations Registry](registry/equations.html). For sheet-level analysis execution and table compatibility, see [Statistical Analyses (`#analysis-nonlinear-regression`)](analyses.md#analysis-nonlinear-regression), [Linear Regression (`#analysis-linear-regression`)](analyses.md#analysis-linear-regression), [Simple Logistic Regression (`#analysis-simple-logistic-regression`)](analyses.md#analysis-simple-logistic-regression), and the [Command Reference](../commands.md).

## Fitting Methods & Estimation Objectives {#fitting-methods}

Nonlinear regression fits a parametric model $y = f(x; \theta) + \varepsilon$ over active (non-excluded) observations via [`analyze.run`](../commands.md), `fit.run`, or `analysis.add` (`nonlinear` / `AnalysisKind::NonlinearRegression`). Observations may come from raw replicates or [pre-summarized XY subcolumns](tables.md#subcolumn-modes) (`MeanSdN`, `MeanSemN`, `MeanCvN`), and can be filtered by predictor interval using `x_range_filter`.

> [!NOTE]
> Under weighted least squares, `PoissonDeviance`, or replicate-moment weighting, `weighted_ss` is the objective minimized by the optimizer, while reported `rss` and unweighted $R^2 = 1 - \text{rss} / \text{TSS}$ remain expressed in original response $Y$ units (Motulsky & Christopoulos 2004).

### 1. Least Squares (`method: "least_squares"`)

Minimizes the weighted sum of squared residuals:

$$SS_w(\theta) = w_1 (y_1 - \hat{y}_1)^2 + w_2 (y_2 - \hat{y}_2)^2 + ... + w_N (y_N - \hat{y}_N)^2$$

- **Core optimizer:** Trust-region Levenberg–Marquardt (Levenberg 1944; Marquardt 1963; Moré 1978 / MINPACK-1 `lmder`) using column-scaled QR decomposition on the augmented Jacobian $[J; \sqrt{\lambda} D]$, quadratic trust-region step updates ($\rho \ge 10^{-4}$ acceptance, contraction in $[0.1, 0.5]$, expansion up to $\text{max}(\delta, 2 |D p|)$), and active-face projection onto box constraints.
- **Tolerances & budgets:** Relative residual reduction `ftol = 1e-15`, parameter step `xtol = 1e-14`, projected gradient cosine `gtol = 1e-13` (and stall check `stall_gtol = 1e-4`), default `max_iters = 1000` (`HARD_MAX_ITERS = 50000`).
- **Warm start & rescue:** Before or during difficult starts, when `use_simplex_warm_start` is active (default `true`), bounded Nelder–Mead simplex optimization (Nelder & Mead 1965; Lagarias et al. 1998) locates an initial basin or rescues non-finite / stalled starts.
- **Jacobians:** Built-in models supply analytical derivatives $df / d\theta_j$ where available; models without analytical gradients (including all custom DSL equations) use central finite differences $h = 10^{-6} \cdot \text{max}(|\theta_j|, 1)$, switching to interior one-sided differences $h = 1.5 \times 10^{-8} \cdot \text{max}(|\theta_j|, 1)$ at an active bound (Nocedal & Wright 2006).

### 2. Poisson Deviance (`method: "poisson_deviance"`)

For non-negative integer or count data $y_i \ge 0$ with strictly positive predicted mean $\hat{\mu}_i = f(x_i; \theta) > 0$, minimizes exact Poisson log-likelihood deviance (McCullagh & Nelder 1989):

$$D(\theta) = 2 \cdot [ y_1 \ln(y_1 / \hat{\mu}_1) - (y_1 - \hat{\mu}_1) ] + ... + 2 \cdot [ y_N \ln(y_N / \hat{\mu}_N) - (y_N - \hat{\mu}_N) ]$$

where terms with $y_i = 0$ contribute $2 \hat{\mu}_i$ smoothly via `deviance_bd0`.

- **Solver:** Iteratively reweighted least squares (IRLS; Green 1984) assigning working weights $1 / \hat{\mu}_i$ around inner trust-region Levenberg–Marquardt updates until relative deviance change drops below $10^{-10}$.
- **Domain guards:** Negative observed counts ($y_i < 0$) raise `FitError::ImpossibleWeights`, and custom Gaussian `weighting` choices are ignored in favor of exact Poisson variance $V(\mu) = \mu$.
- **Inference:** Dispersion is fixed at $\phi = 1$, using asymptotic Fisher information covariance, normal two-sided $z$ quantiles for curve bands, and exact Poisson log-likelihood $\text{AIC}_c$ and $\text{BIC}$.

### 3. Robust Cauchy Regression (`method: "robust_cauchy"`)

Outlier-resistant Lorentzian M-estimator minimizing the logarithmic loss (Motulsky & Brown 2006):

$$L(\theta) = \ln(1 + (r_1 / \text{RSDR})^2) + ... + \ln(1 + (r_N / \text{RSDR})^2)$$

solved by Cauchy IRLS around the Robust Standard Deviation of Residuals (`RSDR`), defined from the $68.27\text{th}$ percentile of weighted residual magnitudes scaled by $\sqrt{N / (N - K_{\text{free}})}$. Points remain in the dataset without binary deletion.

> [!IMPORTANT]
> Under `robust_cauchy`, parametric standard errors, confidence intervals, parameter dependency, Hougaard skewness, residual SD ($S_{y.x}$), $\text{AIC}_c$, $\text{BIC}$, studentized residuals, replicate lack-of-fit tests, and confidence/prediction bands are **intentionally suppressed** (`InferenceAvailability::NotApplicableInRobustMode` — empty cells / `Open` CIs), because standard Gaussian linearization covariance is invalid under non-quadratic Cauchy loss. Best-fit parameters, unweighted $R^2$, and `RSDR` remain reported. Use `rout_outlier_elimination` when parametric inference on clean inliers is required.

### 4. ROUT Outlier Elimination (`method: "rout_outlier_elimination"`)

Implements the three-stage ROUT procedure of Motulsky & Brown (2006):

1. **Robust Cauchy fit:** Fit the model with `robust_cauchy` to locate a resistant baseline and compute `RSDR` on base-weighted residuals $r_{w,i} = \sqrt{w_i} (y_i - \hat{y}_i)$, ensuring heteroscedastic weighting schemes (such as `relative_square` $1/Y^2$) map residuals onto a uniform scale before testing.
2. **FDR screening over extreme tail:** Evaluate two-sided Student-$t$ $P$ values against `RSDR` ($t_i = |r_{w,i}| / \text{RSDR}$) and apply Benjamini–Hochberg (1995) step-up FDR control **only** across the top $30\%$ of observations furthest from the curve (`start = floor(0.70 * N)`). The remaining lower $\approx 70\%$ of residuals are never tested (`q_value = 1.0`, `is_outlier = false`).
3. **Least-squares refit:** Re-optimize retained non-outlier points using ordinary or weighted least squares, seeded from the robust Cauchy parameters.

- **Parameter constraint:** `fdr_q_percent` is specified in percentage points (default `1.0` for $Q = 1\%$) and must satisfy $0 < Q \le 50$ in nonlinear regression (compare [univariate column outlier detection](analyses.md#analysis-outlier-detection), which allows $(0, 100]$).
- **Empty/insufficient inliers:** If ROUT excludes too many points to retain residual degrees of freedom, the solver returns `FitError::InsufficientData` or surfaces a non-converged warning; excluded outlier coordinates and $q$-values are tabulated in the diagnostics sheet without mutating the source [`DataSheet`](concepts.md#cell-exclusions).

### Optimizer Status Outcomes (`FitStatus`)

| Status | Meaning |
| :--- | :--- |
| `Converged` | Gradient, step, or objective criteria satisfied with identifiable full-rank Jacobian |
| `Ambiguous` | Converged objective but rank-deficient Jacobian, collinearity `Dependency > 0.9999`, or unbounded identifiability plateau |
| `HitMaxIters` | Iteration cap reached before strict stopping tolerance |
| `Stalled` | Trust-region radius or projected step cannot achieve further reduction (`stall_gtol`) |
| `Diverged` | Objective failed to remain finite or exceeded numerical limits |

## Weighting Schemes & Replicate Summary Moments {#weighting-and-moments}

The `weighting` field (`WeightingScheme`) scales residual errors prior to least-squares and Cauchy/ROUT fitting (ignored by `poisson_deviance`; explicit cell-level weights, when attached, take precedence over the global scheme). See also [Parametric Equations Registry: Weighting Schemes](registry/equations.html#weighting-schemes).

| Canonical Key | Formula | Typical Error Model & Guard |
| :--- | :--- | :--- |
| `none` | $w_i = 1$ | Homoscedastic uniform absolute SD across $Y$ |
| `relative_square` | $w_i = 1 / Y_i^2$ | Constant relative `%CV` scatter; undefined if any $Y_i = 0$ |
| `relative` | $w_i = 1 / |Y_i|$ | Poisson-like variance proportional to mean response; undefined if any $Y_i = 0$ |
| `general_power_y` | $w_i = 1 / |Y_i|^k$ | Power variance via finite `weighting_power` ($k$); undefined if $Y_i = 0$ when $k > 0$ |
| `inverse_x` | $w_i = 1 / |X_i|$ | Scatter proportional to $\sqrt{|X|}$; undefined if any $X_i = 0$ |
| `inverse_x2` | $w_i = 1 / X_i^2$ | Relative predictor-magnitude scatter; undefined if any $X_i = 0$ |
| `observed_sd` | $w_i = 1 / \text{SD}_i^2$ | Pre-summarized empirical variance; requires $\text{SD}_i > 0$ at every active row |

Any non-finite, zero-denominator, or overflowing weight produces `FitError::ImpossibleWeights`.

### Replicate Summary Subcolumns (`MeanSdN`, `MeanSemN`, `MeanCvN`)

When the source XY table uses a [pre-summarized subcolumn mode](tables.md#subcolumn-modes) (`MeanSdN`, `MeanSemN`, `MeanCvN`) and `weighting` is `none` (or on pre-summarized inputs without explicit weights and not using `observed_sd`), the solver reconstructs exact replicate-sufficient statistics from `(mean_i, sd_i, n_i)`:

- Each row's weighted residual scales by $\sqrt{w_i \cdot n_i}$.
- Total sample size sums $N = n_1 + ... + n_M$.
- Pooled within-row replicate variance contributes $w_i (n_i - 1) \text{SD}_i^2$ into total `weighted_ss` and unweighted `rss`, matching an equivalent raw-replicate fit identically for least-squares estimates, parameter SEs, $\text{AIC}_c$, and replicate lack-of-fit tests (Motulsky & Christopoulos 2004).

Under `observed_sd`, each pre-summarized row contributes one summary point weighted by $1 / \text{SD}_i^2$ (requiring finite $\text{SD}_i > 0$).

## Parameter Scopes, Bounds & Dynamic Hooks {#parameter-scopes}

Each entry in `parameter_rules` (`ParameterRule`) targets a case-insensitive model parameter `name` (`ParametricModel::param_names`) and controls initial guess, box constraints, sharing across datasets, or constant fixation (`ParameterScope`).

| Canonical `scope` | Behavior & Free Parameters Consumed |
| :--- | :--- |
| `local` | Default; fits an independent parameter value separately for each active dataset |
| `shared_global` | Pools all selected datasets to optimize a single shared parameter (`is_shared = true`) |
| `constant` | Fixes the parameter to a finite scalar `value` across all datasets ($0$ free parameters) |
| `hooked_constant` | Resolves a finite scalar from an upstream `InfoSheet` constant (`info_constant`) or up-to-date `ResultSheet` scalar (`result_scalar`); fails cleanly if missing, non-finite, or `Dirty` |
| `per_dataset_constant` | Supplies dataset-indexed `(data_set_index, value)` fixed constants (`per_dataset_constants`); datasets omitted from the list remain free `local` parameters |
| `column_title_constant` | Parses a finite numeric constant from each dataset column header (supports scientific notation and unit suffixes such as `M`, `mM`, `uM`, `nM`, `pM`, `fM`, `%`, `s`, `ms`, `min`, `h`, `Hz`, `deg`; fails if unparseable) |

- **Initial guesses (`initial_override`) & bounds (`lower_bound`, `upper_bound`):** When `initial_override` is omitted, deterministic model heuristics compute dataset-aware initial guesses (or pooled guesses for `shared_global`). Box bounds must be finite with $\text{lower} \le \text{upper}$ and are intersected with intrinsic domain bounds (e.g., positive rate constants). Setting $\text{lower} = \text{upper}$ converts a free slot into a fixed constant consuming $0$ free parameters.
- **Default model rules:** Certain models supply `default_parameter_rules` when no user rule overrides them—for example, centered polynomials fix `XCenter` to the active predictor sample mean $\bar{x}$ to preserve Jacobian conditioning.

## Parameter Uncertainty, Diagnostics & R13.1 Safeguards {#uncertainty-and-diagnostics}

### Profile-Likelihood & Wald Confidence Intervals

- **Wald asymptotic covariance:** $\text{Cov}(\hat{\theta}) = s_w^2 (J^T J)^{-1}$ where $s_w^2 = SS_w / \text{df}$ ($\text{df} = N - K_{\text{free}}$) for least squares, or $\text{Cov}(\hat{\theta}) = (J^T W J)^{-1}$ ($W = \text{diag}(\hat{\mu}_i)$) for Poisson deviance. Symmetric Wald intervals use Student $t_{\text{df}, \alpha/2}$ (least squares) or normal $z_{\alpha/2}$ (Poisson).
- **Venzon–Moolgavkar (1988) profile-likelihood intervals (`profile_likelihood_ci: true`, default):** For each free parameter $\theta_j$, nuisance parameters $\theta_{-j}$ are re-optimized via reduced trust-region LM (`run_reduced_lm_with_domain`, tolerance `ftol = 1e-12`, `xtol = 1e-12`) across an outward bracket scan refined by Brent root finding (`rel_tol = 1e-7`) to solve:

$$SS_{\text{prof}}(\theta_j) = SS_{\text{min}} \cdot (1 + \frac{1}{\text{df}} F_{1 - \alpha}(1, \text{df}))$$

(or deviance increase $\Delta D(\theta_j) = \chi^2_{1, 1 - \alpha}$ in `poisson_deviance`). If the profiled objective fails to reach the critical threshold before hitting a boundary, search horizon, or flat plateau, the endpoint reports `CiBound::Open` (`open` / `unbounded`), alerting users without truncating to symmetric Wald approximations (Bates & Watts 1988; Seber & Wild 1989).

### Hougaard Skewness ($g_1$)

When $1 \le K_{\text{free}} \le 10$, $\text{df} > 0$, and the fit is identifiable, `compute_hougaard` evaluates Hougaard's (1985) third-order skewness measure $g_1$ per parameter from central second-derivative tensor approximations:

- $|g_1| < 0.1$: effectively linear behavior (Wald and profile CIs agree closely).
- $0.1 \le |g_1| < 0.25$: mild parameter-effects nonlinearity.
- $0.25 \le |g_1| \le 1.0$: substantial skewness; asymmetric profile-likelihood intervals are materially more accurate than symmetric intervals.
- $|g_1| > 1.0$: extreme nonlinearity; triggers a diagnostic checklist entry (`QOL-7`, `remediation: None`).

### Collinearity & Fit Identifiability (`identify_unstable_parameters`)

Variance-inflation collinearity per free parameter is reported via Bates & Watts dependency:

$$R_j^2 = 1 - \frac{1}{(J^T J)_{jj} \cdot [(J^T J)^{-1}]_{jj}}$$

- `Dependency > 0.99`: emits a collinearity warning on the parameter and checklist.
- `FitIdentifiability::Ambiguous` (`FitStatus::Ambiguous`, `ParameterNote::Unstable`, checklist metric `"Dependency > 0.9999 or rank-deficient Jacobian"`): raised when rank-revealing QR detects numerical rank deficiency, $R_j^2 > 0.9999$ (`MAX_DEPENDENCY = 1 - 1 / 10000`), or profile-likelihood intervals return `CiBound::Open` on unidentifiable asymptotes/EC50 plateaus (for instance, sparse dose-response curves lacking upper/lower concentration coverage per Sebaugh 2011).

### Residual Diagnostics, Range Filters & Threshold Alerts

- **Inclusive predictor filter (`x_range_filter`):** Restricts fit observations to active rows satisfying $X \in [x_{\text{min}}, x_{\text{max}}]$ (unbounded on omitted sides) while optionally evaluating curve overlays over an independent `[plot_x_min, plot_x_max]` display range. Counts points excluded by the range filter cleanly.
- **Unweighted $R^2$ alert (`flag_r_squared_below`):** Optional threshold in $[0, 1]$ emitting an `R squared below threshold` diagnostic checklist entry (`ChecklistStatus::Warn`, `remediation: None`) whenever unweighted $R^2$ drops below threshold.
- **Internally studentized residuals:** $e_i = r_{w,i} / (s_w \sqrt{1 - h_{ii}})$ using tangent-plane leverage $h_{ii} = J_i (J^T J)^{-1} J_i^T$ (Cook & Weisberg 1982) when identifiable and $K_{\text{free}} \le 64$.
- **Replicates lack-of-fit test:** Partitions residual variance/deviance across exact replicate predictor groups into `Pure Error` and `Lack of Fit`, returning an $F$-statistic and $P$ value (honoring weighting, Poisson deviance, and replicate summary moments).
- **Runs test:** Wald–Wolfowitz runs test sorted along ascending $X$, combined across datasets in global fits.
- **Normality & homoscedasticity battery:** Shapiro–Wilk (falling back to D'Agostino–Pearson $K^2$ outside SW range), Anderson–Darling, and Lilliefors Kolmogorov–Smirnov on weighted/Pearson residuals, plus Spearman rank correlation of $|r_{w,i}|$ vs $\hat{y}_i$ ($n \ge 4$).
- **Confidence & prediction bands + AUC:** Delta-method mean confidence bands and observation prediction bands sampled over `curve_overlay_segments` (`256` by default; [Graphs Reference](graphs.md)), plus fitted-curve vs empirical trapezoidal Area Under the Curve (`fit_auc`).

> [!NOTE]
> Following `QOL-7`, every assumption checklist item emitted by CurveBook sets `remediation: None` and reports purely objective metrics and limitations without recommending alternative models or transformations ([Quality of Life Capabilities](qol.md#active-qol)).

## Model Comparison & Unknown Interpolation {#comparison-and-interpolation}

### Comparing Models (`comparison`)

1. **Nested Extra-Sum-of-Squares $F$-test (`extra_sum_of_squares_f` / `shared_parameter_comparison`):** Compares a simpler constrained/null model (`alternative_equation_id` or globally shared parameters via `SharedGlobal`) against the fuller model ($\text{df}_1 > \text{df}_2$):

$$F = \frac{(SS_{\text{null}} - SS_{\text{alt}}) / (\text{df}_{\text{null}} - \text{df}_{\text{alt}})}{SS_{\text{alt}} / \text{df}_{\text{alt}}}$$

Under `poisson_deviance` or `likelihood_ratio_test`, performs an asymptotic deviance difference $\chi^2$ Likelihood Ratio Test (comparing $\Delta D$ against $\chi^2_{\Delta \text{df}}$). Emits a `ModelComparison` tab with $F$ / $\chi^2$, $\text{df}$, $P$ value, preferred model label, and percentage difference in residual SD.
2. **Corrected Akaike Information Criterion (`aicc`):** Evaluates small-sample corrected AIC for identical active observations and weights (Burnham & Anderson 2002):

$$\text{AIC}_c = N \ln\left(\frac{SS_w}{N}\right) + 2 (K + 1) + \frac{2 (K + 1)(K + 2)}{N - K - 2}$$

(using $-2 \ln L + 2K + \frac{2K(K+1)}{N - K - 1}$ under exact Poisson log-likelihood $\ln L$, where $K$ excludes fixed constants), alongside $\Delta \text{AIC}_c$, normalized Akaike weights $w_{\text{alt}}, w_{\text{null}}$, and evidence ratio. Unavailable (`None`) in `robust_cauchy` mode.

### Forward Prediction & Inverse Interpolation (`interpolate_unknowns`)

When `interpolate_unknowns` is enabled, unpaired `(X, blank)` rows are predicted forward ($\hat{Y}$) and `(blank, Y)` rows are inverted numerically ($\hat{X}$) along each fitted dataset's curve via bracketed Brent + damped Newton root search:

- **Confidence intervals & modes (`Mean` vs `Replicates`):** Delta-method curve uncertainty $\sigma_{\text{mean}}^2(x) = g(x)^T \text{Cov}(\hat{\theta}) g(x)$ (where $g(x)$ is the parameter gradient of $f(x; \hat{\theta})$) yields the curve-mean band (`Interpolated Mean X` when $Y$ represents an observed single/averaged mean), while adding observation scatter $\sigma_{\text{pred}}^2(x) = \sigma_{\text{mean}}^2(x) + s_w^2 / w(x)$ yields single-replicate inverse prediction intervals (`Interpolated Replicates`) whenever active data include replicate scatter ($s_w > 0$). Forward and inverse intervals propagate SEs via $\text{SE}(\hat{X}) = \sigma / |df / dX|$ evaluated at $\hat{X}$.
- **Extrapolation & non-finite guards:** Any inverse or forward row with predictor outside $[X_{\text{min}}, X_{\text{max}}]$ or target $Y$ outside the curve's monotonic image sets an explicit `Extrapolated` warning flag (or `Empty` when no real root exists, e.g., $Y$ beyond a 4PL asymptote).

## Custom Equation DSL & First-Order ODE Models {#custom-dsl-and-ode}

User-defined nonlinear models compile through `CompiledDslModel::compile` (`custom_dsl`) into a stack-based sandboxed bytecode VM implementing `ParametricModel`. Custom models use central finite-difference Jacobians and support explicit equations, dataset-branch selectors, implicit roots, initial-value rules, derived transforms, and first-order ODEs.

### Grammar, Sections & Precedence

Equations are newline-delimited statements organized into optional `[Equation]` (default), `[InitialValues]`, and `[Transforms]` sections. Line comments start with `;`, `//`, or `#`.

**Operator precedence (highest to lowest):**
1. Parentheses `(...)`, numeric literals, variable/parameter identifiers, and function calls `f(...)`
2. Exponentiation `^` or `**` (right-associative; binds tighter than prefix unary operators, so `-X^2` parses as `-(X^2)` while `X^-2` parses as `X^(-2)`)
3. Prefix unary `+`, `-`, `!` (`NOT`)
4. Multiplicative `*`, `/`, `%` (left-associative)
5. Additive `+`, `-` (left-associative)
6. Comparisons `==`, `=`, `!=`, `<>`, `<`, `<=`, `>`, `>=`
7. Logical `&&` (`AND`), then `||` (`OR`)

**Statement forms:**
- **Explicit assignment:** `Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))` (or a bare expression rhs).
- **Local helper variables:** `span = Top - Bottom` prior to the target `Y = ...` line.
- **Conditional piecewise branching:** `IF(cond, then_expr, else_expr)`.
- **Dataset-specific lines (`<A>`, `<B>`, `<A:C>`, `<2>`):** Prefix lines with angle-bracket column letters (`A`..`Z`.. or 1-based dataset indices) or ranges to apply distinct expressions or parameter assignments across datasets in a shared fit. Unscoped free identifiers become optimizable parameters; dataset-scoped assignments restrict parameters cleanly.
- **Initial-value rules (`[InitialValues]`):** Supply deterministic starting formulas using built-in summary tokens calculated from active `(X, Y)` points: `YMIN`, `YMAX`, `XMIN`, `XMAX`, `XATYMIN`, `XATYMAX`, `XMID`, `YMEAN`, `YMEDIAN`, `XMEAN`, `XMEDIAN`, `YAT0`, `SLOPE`, etc.
- **Derived scalar transforms (`[Transforms]`):** Post-fit algebraic expressions (e.g., `EC50 = 10^LogEC50`) evaluated at converged estimates and emitted into `derived_scalars` and summary tabs.
- **Built-in constants & predictors:** Predictor `X` (or `T` for ODE time axis; multi-predictor `X0`, `X1`, ...), dependent response `Y`, mathematical constants `PI`, `E`, `LN2`, `LN10`, `LOG10E`, `LOG2E`, `SQRT2`, `PHI`.

### Built-In DSL Functions

| Category | Supported Identifiers (Case-Insensitive) |
| :--- | :--- |
| Elementary & arithmetic | `abs`, `sqrt`, `cbrt`, `exp`, `exp10`, `ln` (`loge`), `log` (`log10`), `log2`, `pow(a,b)`, `hypot(a,b)`, `min(a,b)`, `max(a,b)`, `clamp(x,lo,hi)` |
| Rounding & sign | `floor`, `ceil`, `round`, `trunc`, `sign` (`sgn`) |
| Trigonometric & inverse | `sin`, `cos`, `tan`, `asin`, `acos`, `atan` (`arctan`), `atan2(y,x)` |
| Hyperbolic & inverse | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` |
| Error & normal | `erf`, `erfc`, `normcdf` (`phi`), `norminv` (`probit`) |
| Gamma, Beta & Bessel | `gamma`, `lngamma` (`lgamma`), `beta(a,b)`, `digamma` (`psi`), `besselj0` (`j0`), `besselj1` (`j1`), `besseljn` (`jn`), `bessely0` (`y0`), `bessely1` (`y1`), `besselyn` (`yn`), `besseli0` (`i0`), `besseli1` (`i1`), `besselk0` (`k0`), `besselk1` (`k1`) |
| Hypergeometric | `hyp1f1` (`m(a,b,z)`), `hyp2f1(a,b,c,z)`, `confluent_u` (`u(a,b,z)`) |

### Implicit Equations (`LHS = RHS`)

Any `[Equation]` target with `Y` appearing on both sides (or an algebraic equation where `LHS` is not plain `Y`) compiles as an implicit root problem $F(X, Y; \theta) = \text{LHS} - \text{RHS} = 0$:

- Scans multi-decade, scale-augmented positive and negative probe grids outward from $0$ to bracket a sign change, then refines via safeguarded Brent root finding (`rel_tol = 1e-13`, with `[0, root]` tight re-bracketing near zero).
- Falls back to damped Newton iteration when no sign-change bracket is found. Returns `None` (domain failure) if residual $|F| > \text{tol}$ or non-finite.

### First-Order ODE Models (`dY/dX` / `dY/dT` / `D(Y, X)`)

Single-equation first-order ODEs specify a derivative line (`dY/dX = -k * Y`, `dY/dT = r * Y * (1 - Y / K)`, or `D(Y, X) = ...`) paired with an initial condition (`Y(0) = Y0` or `Y(t0) = expr`, defaulting to $t_0 = 0$ if `Y0 = ...` is given):

- **Integrator:** Adaptive embedded 5(4) Dormand–Prince RK45 (`Dormand & Prince 1980`) with mixed step-error control $\text{err} \le \text{atol} \cdot y_{\text{ref}} + \text{rtol} \cdot \text{max}(|y|, |y_{\text{new}}|)$ (`rtol = 1e-8`, Hairer, Nørsett & Wanner 1993 §II.4), supporting both forward ($t_{\text{end}} > t_0$) and backward ($t_{\text{end}} < t_0$) sweeps.
- **Trajectory reuse cache:** Observations are sorted per dataset along ascending $X$, allowing continuous trajectory frontier advancement and reuse (`MAX_CACHED_ODE_TRAJECTORIES = 4`, `MAX_CACHED_ODE_KNOTS = 4,096`) across curve points and overlays.

### Sandboxed DSL & ODE Resource Caps

All custom equations execute inside strict static compile-time and runtime limits to prevent unbounded recursion or runaway ODE steps:

| Resource Limit Constant | Value | Purpose |
| :--- | :--- | :--- |
| `MAX_DSL_SOURCE_BYTES` | `1,048,576` | Maximum custom equation source size (`1 MiB`) |
| `MAX_DSL_LINES` | `16,384` | Maximum source lines per equation |
| `MAX_PARSE_DEPTH` | `128` | Maximum nested parenthesis/call AST recursion depth |
| `MAX_AST_NODES` | `4,096` | Maximum AST nodes per equation |
| `MAX_TOTAL_OPCODES` | `100,000` | Maximum compiled VM opcodes across all programs/init/transforms |
| `MAX_TOTAL_DATASET_INDICES` | `50,000` | Maximum `<...>` dataset selector indices |
| `MAX_DATASET_CLASSES` | `4,096` | Maximum distinct `<...>` dataset branch programs |
| `MAX_BESSEL_ORDER` | `100,000` | Maximum integer order $\|n\|$ for `jn` / `yn` |
| `MAX_VM_STEPS` | `100,000` | Maximum bytecode steps per scalar evaluation |
| `MAX_ODE_STEPS` | `50,000` | Maximum adaptive RK45 steps per single curve point |
| `MAX_ODE_VM_OPS_PER_EVAL` | `5,000,000` | Maximum VM ops per single ODE trajectory evaluation |
| `DEFAULT_MAX_ODE_STEPS_PER_FIT` | `300,000` | Global per-fit RK45 step budget (scaled by `clamp(datasets.len(), 1, 4)`) |

## Published D1 References & Oracle Verification {#oracle-verification}

Every fitting algorithm and built-in curve model is verified offline under CurveBook's clean-room **Decision D1** policy against public reference corpora and primary statistical literature:

- **NIST StRD Nonlinear Least Squares (`nist_strd`, `d1_nist_reparam_builtins`):** Certified parameter estimates, asymptotic SEs, residual sums of squares, and residual standard deviations across Lower, Average, and Higher difficulty benchmarks with both `start_1` and `start_2` starting guesses (including logarithmic response transformations for the `Nelson` model), plus algebraic NIST StRD reparameterization checks across 18 user-facing built-in pharmacology, kinetics, growth, Gaussian, Boltzmann, power-series, and polynomial equation IDs.
- **Pharmacology, Biochemistry, Radiobiology & Growth Oracles (`d1_r18_3_extended_equations`):** 4PL/5PL log- and linear-concentration dose-response (`Motulsky & Christopoulos 2004`; `Gottschalk & Dunn 2005`; `Liao & Liu 2009`), Gaddum–Schild competitive antagonism and unity-slope `pKB` (`Arunlakshana & Schild 1959`; `Colquhoun 2007`), Black–Leff operational partial agonism and receptor depletion (`Black & Leff 1983`), Cheng–Prusoff and two-site receptor binding (`Cheng & Prusoff 1973`), Swillens ligand-depletion total saturation (`Swillens 1995`), Christopoulos–Kenakin allosteric modulator titration (`Christopoulos & Kenakin 2002`), Michaelis–Menten / Hill allosteric / Haldane substrate inhibition / Morrison tight-binding enzyme kinetics (`Cornish-Bowden 1974`; `Morrison 1969`), multi-phase exponential decay and two-phase association, association–dissociation binding, `Verhulst (1838)` logistic, `Hodgkin & Huxley (1952)` Boltzmann sigmoid, `Gompertz (1825)`, `Watts & Bacon (1974)` smooth hyperbolic hinge, `Limpert et al. (2001)` log-Gaussian and area-parameterized lognormal, `Weibull (1951)` peaks, and the complete 6-form Linear–Quadratic radiobiology family (`Chadwick & Leenhouts 1973`; `Brenner 2008`) verified against `R 4.5.2` `stats::nls` / `stats::lm`.
- **Fit Uncertainty, ROUT & Comparison Oracles:** High-precision `mpmath 1.3.0` (80/120-digit decimal equality) and `R 4.5.2` (`stats::nls`, `stats::glm`, `stats::lm`) verification for Venzon–Moolgavkar (1988) profile-likelihood and Poisson deviance intervals (`CiBound::Open` boundary checks), Bates & Watts (1988) / Seber & Wild (1989) linearization covariance and Hougaard (1985) skewness, Cook & Weisberg (1982) leverage residuals, Burnham & Anderson (2002) $\text{AIC}_c$ weights, Motulsky & Brown (2006) ROUT `Q = 1%` detection on raw and weighted curves, Sebaugh (2011) sparse-curve identifiability detection, Deming (1943) / Linnet (1990) jackknife Model II regression, Brent (1973) implicit root precision over 400 orders of magnitude, and Hairer, Nørsett & Wanner (1993) RK45 ODE parameter estimation.
- **Model-Free Spline & LOWESS Oracles (`d1_r18_7_spline_lowess`):** Dedicated XY `FitSplineLowess` curves (Natural Cubic Spline `de Boor 1978`, Akima Subspline `Akima 1970`, Reinsch Smoothing Spline `Reinsch 1967` / `Hutchinson & de Hoog 1985`, Cleveland LOWESS `Cleveland 1979`, and dense data-space point-to-point resampling with nonparametric unknown interpolation; see [Fit Spline / LOWESS Curve](analyses.md#analysis-fit-spline-lowess)) verified against `R 4.5.2` (`stats::splinefun`, `stats::smooth.spline`, `stats::lowess`) and analytical Akima step-overshoot oracles.
