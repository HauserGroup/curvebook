# Curve fitting & nonlinear regression reference

This reference covers CurveBook's parametric linear and nonlinear regression algorithms, residual weighting, parameter sharing and bounds, profile-likelihood uncertainty, diagnostic safeguards, model comparison, unknown interpolation, custom equation DSL, and adaptive ODE solver.

For the full catalog of built-in equations, parameters, and defaults, see the [Parametric Equations Registry](registry/equations.html). For sheet-level execution and table compatibility, see [Nonlinear Regression](analyses.md#analysis-nonlinear-regression), [Linear Regression](analyses.md#analysis-linear-regression), [Simple Logistic Regression](analyses.md#analysis-simple-logistic-regression), and the [Command Reference](../commands.md).

## Fitting methods & estimation objectives {#fitting-methods}

Nonlinear regression fits a parametric model $y = f(x; \theta) + \varepsilon$ over active (non-excluded) observations via [`analyze.run`](../commands.md), `fit.run`, or `analysis.add` (`nonlinear`). Inputs can be raw replicates or [pre-summarized XY subcolumns](tables.md#subcolumn-modes) (`MeanSdN`, `MeanSemN`, `MeanCvN`), optionally restricted to a predictor interval with `x_range_filter`.

> [!NOTE]
> Under weighted least squares, Poisson deviance, or replicate-moment weighting, `weighted_ss` is the objective minimized by the optimizer. Reported `rss` and unweighted $R^2 = 1 - \text{rss} / \text{TSS}$ remain expressed in original response $Y$ units (Motulsky & Christopoulos 2004).

### 1. Least squares (`method: "least_squares"`)

Minimizes the weighted sum of squared residuals:

$$SS_w(\theta) = w_1 (y_1 - \hat{y}_1)^2 + w_2 (y_2 - \hat{y}_2)^2 + ... + w_N (y_N - \hat{y}_N)^2$$

- **Core optimizer:** Uses trust-region Levenberg–Marquardt (Levenberg 1944; Marquardt 1963; Moré 1978 / MINPACK-1 `lmder`) with column-scaled QR decomposition on the augmented Jacobian $[J; \sqrt{\lambda} D]$. Steps accept at $\rho \ge 10^{-4}$, contract in $[0.1, 0.5]$, expand up to $\text{max}(\delta, 2 |D p|)$, and project active faces onto box bounds.
- **Tolerances & budgets:** Relative residual reduction `ftol = 1e-15`, parameter step `xtol = 1e-14`, projected gradient cosine `gtol = 1e-13`, and stall threshold `stall_gtol = 1e-4`. Default `max_iters` is `1000` (hard cap 50,000).
- **Warm start & rescue:** When `use_simplex_warm_start` is `true` (default), bounded Nelder–Mead simplex search (Nelder & Mead 1965; Lagarias et al. 1998) locates an initial basin or rescues stalled and non-finite starts.
- **Jacobians:** Built-in models use analytical derivatives $df / d\theta_j$ where available. Other models (including all custom DSL equations) use central finite differences $h = 10^{-6} \cdot \text{max}(|\theta_j|, 1)$, switching to interior one-sided differences $h = 1.5 \times 10^{-8} \cdot \text{max}(|\theta_j|, 1)$ at active bounds (Nocedal & Wright 2006).

### 2. Poisson deviance (`method: "poisson_deviance"`)

For non-negative count data $y_i \ge 0$ with strictly positive predicted mean $\hat{\mu}_i = f(x_i; \theta) > 0$, minimizes exact Poisson log-likelihood deviance (McCullagh & Nelder 1989):

$$D(\theta) = 2 \cdot [ y_1 \ln(y_1 / \hat{\mu}_1) - (y_1 - \hat{\mu}_1) ] + ... + 2 \cdot [ y_N \ln(y_N / \hat{\mu}_N) - (y_N - \hat{\mu}_N) ]$$

Terms with $y_i = 0$ contribute $2 \hat{\mu}_i$ smoothly.

- **Solver:** Uses iteratively reweighted least squares (IRLS; Green 1984) with working weights $1 / \hat{\mu}_i$ around inner trust-region Levenberg–Marquardt updates until relative deviance change drops below $10^{-10}$.
- **Domain guards:** Negative observed counts ($y_i < 0$) fail with `ImpossibleWeights`. Custom Gaussian `weighting` options are ignored in favor of exact Poisson variance $V(\mu) = \mu$.
- **Inference:** Fixes dispersion at $\phi = 1$, using asymptotic Fisher information covariance, two-sided normal $z$ quantiles for curve bands, and exact Poisson log-likelihood $\text{AIC}_c$ and $\text{BIC}$.

### 3. Robust Cauchy regression (`method: "robust_cauchy"`)

Outlier-resistant Lorentzian M-estimator minimizing the logarithmic loss (Motulsky & Brown 2006):

$$L(\theta) = \ln(1 + (r_1 / \text{RSDR})^2) + ... + \ln(1 + (r_N / \text{RSDR})^2)$$

Cauchy IRLS optimizes around the Robust Standard Deviation of Residuals (`RSDR`), computed from the $68.27\text{th}$ percentile of weighted residual magnitudes scaled by $\sqrt{N / (N - K_{\text{free}})}$. Observations stay in the dataset without binary deletion.

> [!IMPORTANT]
> Under `robust_cauchy`, parametric standard errors, confidence intervals, parameter dependency, Hougaard skewness, residual SD ($S_{y.x}$), $\text{AIC}_c$, $\text{BIC}$, studentized residuals, replicate lack-of-fit tests, and confidence/prediction bands are **intentionally suppressed** (reported as empty cells or open intervals). Standard Gaussian linearization covariance is invalid under non-quadratic Cauchy loss; best-fit parameters, unweighted $R^2$, and `RSDR` remain reported. Use `rout_outlier_elimination` when you need parametric inference on clean inliers.

### 4. ROUT outlier elimination (`method: "rout_outlier_elimination"`)

Implements the three-stage ROUT procedure (Motulsky & Brown 2006):

1. **Robust Cauchy fit:** Fits the model with `robust_cauchy` to establish a resistant baseline and computes `RSDR` on base-weighted residuals $r_{w,i} = \sqrt{w_i} (y_i - \hat{y}_i)$. Weighting (such as `relative_square` $1/Y^2$) maps residuals onto a uniform scale before testing.
2. **FDR screening over extreme tail:** Evaluates two-sided Student-$t$ $P$ values against `RSDR` ($t_i = |r_{w,i}| / \text{RSDR}$) and applies Benjamini–Hochberg (1995) step-up FDR control **only** across the top 30% of observations furthest from the curve (`start = floor(0.70 * N)`). The remaining lower ~70% of residuals are never tested (`q_value = 1.0`, `is_outlier = false`).
3. **Least-squares refit:** Re-optimizes retained inliers with ordinary or weighted least squares, seeded from the robust Cauchy parameters.

- **FDR threshold:** `fdr_q_percent` is specified in percentage points (default `1.0` for $Q = 1\%$) and must satisfy $0 < Q \le 50$ in nonlinear regression (compare [univariate column outlier detection](analyses.md#analysis-outlier-detection), which allows $(0, 100]$).
- **Insufficient inliers:** If ROUT excludes too many points to retain residual degrees of freedom, the solver returns `InsufficientData` or a non-converged warning. Detected outlier coordinates and $q$-values appear in the diagnostics tab without mutating the source [data table](concepts.md#cell-exclusions).

### Optimizer status outcomes

| Status | Meaning |
| :--- | :--- |
| `Converged` | Stopping criteria satisfied with a full-rank identifiable Jacobian. |
| `Ambiguous` | Converged objective with a rank-deficient Jacobian, `Dependency > 0.9999`, or open plateau. |
| `HitMaxIters` | Iteration cap reached before meeting strict stopping tolerances. |
| `Stalled` | Trust-region step cannot achieve further objective reduction (`stall_gtol`). |
| `Diverged` | Objective became non-finite or exceeded numerical limits. |

## Weighting schemes & replicate summary moments {#weighting-and-moments}

The `weighting` option scales residuals before least-squares, Cauchy, and ROUT fitting. It is ignored by `poisson_deviance`, and explicit cell-level weights take precedence when attached. See also [Parametric Equations Registry: Weighting Schemes](registry/equations.html#weighting-schemes).

| Canonical key | Formula | Error model & guard |
| :--- | :--- | :--- |
| `none` | $w_i = 1$ | Uniform absolute SD across $Y$. |
| `relative_square` | $w_i = 1 / Y_i^2$ | Constant relative %CV scatter; requires $Y_i \ne 0$. |
| `relative` | $w_i = 1 / |Y_i|$ | Variance proportional to mean response; requires $Y_i \ne 0$. |
| `general_power_y` | $w_i = 1 / |Y_i|^k$ | Power variance via finite `weighting_power` ($k$); requires $Y_i \ne 0$ when $k > 0$. |
| `inverse_x` | $w_i = 1 / |X_i|$ | Scatter proportional to $\sqrt{|X|}$; requires $X_i \ne 0$. |
| `inverse_x2` | $w_i = 1 / X_i^2$ | Relative predictor-magnitude scatter; requires $X_i \ne 0$. |
| `observed_sd` | $w_i = 1 / \text{SD}_i^2$ | Pre-summarized empirical variance; requires $\text{SD}_i > 0$ at every active row. |

Any non-finite, zero-denominator, or overflowing weight fails cleanly with `ImpossibleWeights`.

### Replicate summary subcolumns (`MeanSdN`, `MeanSemN`, `MeanCvN`)

When the source XY table uses a [pre-summarized subcolumn mode](tables.md#subcolumn-modes) (`MeanSdN`, `MeanSemN`, `MeanCvN`) and `weighting` is `none` (or without explicit weights and not `observed_sd`), the solver reconstructs exact replicate-sufficient statistics from $(\text{mean}_i, \text{SD}_i, n_i)$:

- Each row's weighted residual scales by $\sqrt{w_i \cdot n_i}$.
- Total sample size sums $N = n_1 + ... + n_M$.
- Pooled within-row variance contributes $w_i (n_i - 1) \text{SD}_i^2$ into `weighted_ss` and unweighted `rss`, matching an equivalent raw-replicate fit identically for estimates, parameter SEs, $\text{AIC}_c$, and replicate lack-of-fit tests (Motulsky & Christopoulos 2004).

Under `observed_sd`, each summary row contributes one point weighted by $1 / \text{SD}_i^2$ (requiring finite $\text{SD}_i > 0$).

## Parameter scopes, bounds & dynamic hooks {#parameter-scopes}

Each entry in `parameter_rules` targets a case-insensitive parameter `name` and controls its initial guess, box constraints, sharing across datasets, or constant value:

| Canonical `scope` | Behavior & free parameters |
| :--- | :--- |
| `local` | Default; fits an independent value separately for each active dataset. |
| `shared_global` | Pools selected datasets to optimize one shared parameter. |
| `constant` | Fixes the parameter to a finite scalar `value` across all datasets (0 free parameters). |
| `hooked_constant` | Resolves a finite scalar from an info constant (`info_constant`) or up-to-date results scalar (`result_scalar`). |
| `per_dataset_constant` | Fixes dataset-indexed `(data_set_index, value)` constants (`per_dataset_constants`); omitted datasets stay `local`. |
| `column_title_constant` | Parses a finite numeric constant from each dataset column header. |

Parameter rule details and guards:

- **Hook validation:** `hooked_constant` fails cleanly if the referenced constant or scalar is missing, non-finite, or dirty.
- **Column-title unit suffixes:** `column_title_constant` accepts scientific notation and unit suffixes (`M`, `mM`, `uM`, `nM`, `pM`, `fM`, `%`, `s`, `ms`, `min`, `h`, `Hz`, `deg`), failing cleanly if a header cannot be parsed.
- **Initial guesses and box bounds:** When `initial_override` is omitted, model heuristics compute dataset-aware (or pooled `shared_global`) starting values. `lower_bound` and `upper_bound` must be finite with $\text{lower} \le \text{upper}$ and intersect with intrinsic domain bounds (such as positive rate constants). Setting $\text{lower} = \text{upper}$ fixes the parameter (0 free parameters).
- **Model default rules:** Some built-in equations apply defaults when no user rule overrides them—for example, centered polynomials fix `XCenter` to the active predictor sample mean $\bar{x}$ to preserve Jacobian conditioning.

## Parameter uncertainty, diagnostics & safeguards {#uncertainty-and-diagnostics}

### Profile-likelihood & Wald confidence intervals

- **Wald asymptotic covariance:** $\text{Cov}(\hat{\theta}) = s_w^2 (J^T J)^{-1}$ with $s_w^2 = SS_w / \text{df}$ ($\text{df} = N - K_{\text{free}}$) for least squares, or $\text{Cov}(\hat{\theta}) = (J^T W J)^{-1}$ ($W = \text{diag}(\hat{\mu}_i)$) for Poisson deviance. Symmetric Wald intervals use Student $t_{\text{df}, \alpha/2}$ (least squares) or normal $z_{\alpha/2}$ (Poisson).
- **Venzon–Moolgavkar (1988) profile-likelihood intervals (`profile_likelihood_ci: true`, default):** For each free parameter $\theta_j$, nuisance parameters $\theta_{-j}$ are re-optimized via reduced trust-region Levenberg–Marquardt (`ftol = 1e-12`, `xtol = 1e-12`) across an outward bracket scan refined by Brent root finding (`rel_tol = 1e-7`) to solve:

$$SS_{\text{prof}}(\theta_j) = SS_{\text{min}} \cdot (1 + \frac{1}{\text{df}} F_{1 - \alpha}(1, \text{df}))$$

Under `poisson_deviance`, the critical threshold is the deviance increase $\Delta D(\theta_j) = \chi^2_{1, 1 - \alpha}$. If the profiled objective hits a boundary, search horizon, or flat plateau before reaching the threshold, CurveBook reports an open/unbounded endpoint rather than substituting symmetric Wald limits (Bates & Watts 1988; Seber & Wild 1989).

### Hougaard Skewness ($g_1$)

When $1 \le K_{\text{free}} \le 10$, $\text{df} > 0$, and the fit is identifiable, CurveBook evaluates Hougaard's (1985) third-order skewness measure $g_1$ per parameter from central second-derivative tensor approximations:

- $|g_1| < 0.1$: effectively linear behavior (Wald and profile intervals agree closely).
- $0.1 \le |g_1| < 0.25$: mild parameter-effects nonlinearity.
- $0.25 \le |g_1| \le 1.0$: substantial skewness; asymmetric profile-likelihood intervals are materially more accurate than symmetric intervals.
- $|g_1| > 1.0$: extreme nonlinearity; emits a diagnostic checklist warning.

### Collinearity & fit identifiability

Variance-inflation collinearity per free parameter is reported via Bates & Watts dependency:

$$R_j^2 = 1 - \frac{1}{(J^T J)_{jj} \cdot [(J^T J)^{-1}]_{jj}}$$

- `Dependency > 0.99` emits a collinearity warning on the parameter and diagnostic checklist.
- A fit is marked `Ambiguous` (with affected parameters flagged `Unstable`) when rank-revealing QR detects rank deficiency or $R_j^2 > 0.9999$. It also triggers when profile-likelihood intervals return open bounds on unidentifiable asymptotes or EC50 plateaus (such as sparse dose-response curves; Sebaugh 2011).

### Residual diagnostics, range filters & threshold alerts

- **Predictor range filter (`x_range_filter`):** Restricts fit observations to active rows in $X \in [x_{\text{min}}, x_{\text{max}}]$ (unbounded on omitted sides) and counts excluded points cleanly. Curve overlays can optionally render over an independent `[plot_x_min, plot_x_max]` range.
- **Unweighted $R^2$ alert (`flag_r_squared_below`):** Optional threshold in $[0, 1]$ emitting a checklist warning whenever unweighted $R^2$ drops below the threshold.
- **Internally studentized residuals:** $e_i = r_{w,i} / (s_w \sqrt{1 - h_{ii}})$ using tangent-plane leverage $h_{ii} = J_i (J^T J)^{-1} J_i^T$ (Cook & Weisberg 1982) when identifiable and $K_{\text{free}} \le 64$.
- **Replicates lack-of-fit test:** Partitions residual variance or deviance across replicate $X$ values into pure error and lack of fit, reporting an $F$-statistic and $P$ value (honoring weighting, Poisson deviance, and summary moments).
- **Runs test:** Wald–Wolfowitz runs test along ascending $X$, combined across datasets in global fits.
- **Normality & homoscedasticity battery:** Shapiro–Wilk (falling back to D'Agostino–Pearson $K^2$ outside SW range), Anderson–Darling, and Lilliefors Kolmogorov–Smirnov on weighted or Pearson residuals, plus Spearman rank correlation of $|r_{w,i}|$ vs $\hat{y}_i$ ($n \ge 4$).
- **Bands & AUC:** Delta-method mean confidence bands and observation prediction bands sampled over `curve_overlay_segments` (default 256; see [Graphs Reference](graphs.md)), plus fitted-curve vs. empirical trapezoidal Area Under the Curve (`fit_auc`).

> [!NOTE]
> Every assumption checklist item in CurveBook is strictly diagnostics-only: it reports objective metrics and methodological limits without recommending alternative models or transformations (see [Quality of Life Capabilities](qol.md#active-qol)).

## Model comparison & unknown interpolation {#comparison-and-interpolation}

### Comparing models (`comparison`)

1. **Nested extra-sum-of-squares $F$-test (`extra_sum_of_squares_f` / `shared_parameter_comparison`):** Compares a simpler constrained or null model (`alternative_equation_id` or globally shared parameters) against the fuller model ($\text{df}_1 > \text{df}_2$):

$$F = \frac{(SS_{\text{null}} - SS_{\text{alt}}) / (\text{df}_{\text{null}} - \text{df}_{\text{alt}})}{SS_{\text{alt}} / \text{df}_{\text{alt}}}$$

Under `poisson_deviance` or `likelihood_ratio_test`, CurveBook runs an asymptotic deviance-difference $\chi^2$ Likelihood Ratio Test comparing $\Delta D$ against $\chi^2_{\Delta \text{df}}$. Output appears in a model comparison tab with $F$ or $\chi^2$, $\text{df}$, $P$ value, preferred model label, and percentage difference in residual SD.

2. **Corrected Akaike Information Criterion (`aicc`):** Evaluates small-sample corrected AIC for identical active data and weights (Burnham & Anderson 2002):

$$\text{AIC}_c = N \ln\left(\frac{SS_w}{N}\right) + 2 (K + 1) + \frac{2 (K + 1)(K + 2)}{N - K - 2}$$

Under exact Poisson log-likelihood $\ln L$, $\text{AIC}_c = -2 \ln L + 2K + \frac{2K(K+1)}{N - K - 1}$ (where $K$ excludes fixed constants). CurveBook also reports $\Delta \text{AIC}_c$, normalized Akaike weights $w_{\text{alt}}$ and $w_{\text{null}}$, and the evidence ratio. $\text{AIC}_c$ is unavailable in `robust_cauchy` mode.

### Forward prediction & inverse interpolation (`interpolate_unknowns`)

When `interpolate_unknowns` is enabled, `(X, blank)` rows are predicted forward ($\hat{Y}$) and `(blank, Y)` rows are inverted numerically ($\hat{X}$) along each fitted curve using bracketed Brent and damped Newton root search.

- **Curve-mean vs. replicate intervals:** Delta-method curve uncertainty $\sigma_{\text{mean}}^2(x) = g(x)^T \text{Cov}(\hat{\theta}) g(x)$ (where $g(x)$ is the parameter gradient of $f(x; \hat{\theta})$) produces curve-mean bands (`Interpolated Mean X`). Adding observation scatter $\sigma_{\text{pred}}^2(x) = \sigma_{\text{mean}}^2(x) + s_w^2 / w(x)$ yields single-replicate inverse prediction intervals (`Interpolated Replicates`) whenever active data contain replicate scatter ($s_w > 0$). Forward and inverse intervals propagate SEs via $\text{SE}(\hat{X}) = \sigma / |df / dX|$ at $\hat{X}$.
- **Extrapolation & no-root guards:** Rows with $X$ outside $[X_{\text{min}}, X_{\text{max}}]$ or target $Y$ outside the curve's monotonic range are flagged `Extrapolated`. Rows with no real root (such as $Y$ beyond a 4PL asymptote) remain empty.

## Custom equation DSL & first-order ODE models {#custom-dsl-and-ode}

User-defined nonlinear equations (`custom_dsl`) compile into a sandboxed stack-based bytecode VM. Custom models use central finite-difference Jacobians and support explicit equations, dataset-branch selectors, implicit roots, initial-value rules, derived transforms, and first-order ODEs.

### Grammar, sections & precedence

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

- **Explicit assignment:** `Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))` (or a bare right-hand expression).
- **Local helper variables:** `span = Top - Bottom` before the target `Y = ...` line.
- **Conditional branching:** `IF(cond, then_expr, else_expr)`.
- **Dataset-specific lines (`<A>`, `<B>`, `<A:C>`, `<2>`):** Prefix lines with angle-bracket column letters (`A`–`Z`, …), 1-based dataset indices, or ranges to apply distinct expressions or parameter assignments across datasets in a shared fit. Unscoped free identifiers become optimizable parameters; dataset-scoped assignments restrict parameters to those datasets.
- **Initial-value rules (`[InitialValues]`):** Supply deterministic starting formulas using summary tokens computed from active `(X, Y)` points: `YMIN`, `YMAX`, `XMIN`, `XMAX`, `XATYMIN`, `XATYMAX`, `XMID`, `YMEAN`, `YMEDIAN`, `XMEAN`, `XMEDIAN`, `YAT0`, `SLOPE`, and related tokens.
- **Derived scalar transforms (`[Transforms]`):** Post-fit algebraic expressions (such as `EC50 = 10^LogEC50`) evaluated at converged estimates and reported in `derived_scalars` and summary tabs.
- **Built-in constants & predictors:** Predictor `X` (or `T` for ODE time axis; multi-predictor `X0`, `X1`, …), response `Y`, and constants `PI`, `E`, `LN2`, `LN10`, `LOG10E`, `LOG2E`, `SQRT2`, `PHI`.

### Built-in DSL functions

| Category | Supported identifiers (case-insensitive) |
| :--- | :--- |
| Elementary & arithmetic | `abs`, `sqrt`, `cbrt`, `exp`, `exp10`, `ln` (`loge`), `log` (`log10`), `log2`, `pow(a,b)`, `hypot(a,b)`, `min(a,b)`, `max(a,b)`, `clamp(x,lo,hi)` |
| Rounding & sign | `floor`, `ceil`, `round`, `trunc`, `sign` (`sgn`) |
| Trigonometric & inverse | `sin`, `cos`, `tan`, `asin`, `acos`, `atan` (`arctan`), `atan2(y,x)` |
| Hyperbolic & inverse | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` |
| Error & normal | `erf`, `erfc`, `normcdf` (`phi`), `norminv` (`probit`) |
| Gamma, Beta & Bessel | `gamma`, `lngamma` (`lgamma`), `beta(a,b)`, `digamma` (`psi`), `besselj0` (`j0`), `besselj1` (`j1`), `besseljn` (`jn`), `bessely0` (`y0`), `bessely1` (`y1`), `besselyn` (`yn`), `besseli0` (`i0`), `besseli1` (`i1`), `besselk0` (`k0`), `besselk1` (`k1`) |
| Hypergeometric | `hyp1f1` (`m(a,b,z)`), `hyp2f1(a,b,c,z)`, `confluent_u` (`u(a,b,z)`) |

### Implicit equations (`LHS = RHS`)

Any `[Equation]` target where `Y` appears on both sides (or where `LHS` is not plain `Y`) compiles as an implicit root problem $F(X, Y; \theta) = \text{LHS} - \text{RHS} = 0$:

- Scans multi-decade, scale-augmented positive and negative probe grids outward from 0 to bracket a sign change, then refines via safeguarded Brent root finding (`rel_tol = 1e-13`, with tight `[0, root]` re-bracketing near zero).
- Falls back to damped Newton iteration when no sign-change bracket is found, returning a domain failure if residual $|F| > \text{tol}$ or non-finite.

### First-order ODE models (`dY/dX` / `dY/dT` / `D(Y, X)`)

Single-equation first-order ODEs specify a derivative line (`dY/dX = -k * Y`, `dY/dT = r * Y * (1 - Y / K)`, or `D(Y, X) = ...`). Pair it with an initial condition (`Y(0) = Y0` or `Y(t0) = expr`; $t_0$ defaults to 0 when `Y0 = ...` is given).

- **Integrator:** Uses adaptive embedded 5(4) Dormand–Prince RK45 (Dormand & Prince 1980) with `rtol = 1e-8` and mixed error control $\text{err} \le \text{atol} \cdot y_{\text{ref}} + \text{rtol} \cdot \text{max}(|y|, |y_{\text{new}}|)$ (Hairer, Nørsett & Wanner 1993 §II.4). Both forward ($t_{\text{end}} > t_0$) and backward ($t_{\text{end}} < t_0$) sweeps are supported.
- **Trajectory cache:** Sorts observations per dataset along ascending $X$ to reuse continuous trajectories across points and overlays (up to 4 cached trajectories and 4,096 knots per trajectory).

### Sandboxed DSL & ODE resource caps

All custom equations run inside strict compile-time and runtime limits to prevent unbounded recursion or runaway ODE integration:

| Resource limit | Value | Purpose |
| :--- | ---: | :--- |
| Source size | 1,048,576 bytes | Maximum custom equation source size (1 MiB). |
| Source lines | 16,384 | Maximum source lines per equation. |
| Parse nesting depth | 128 | Maximum nested parenthesis or call depth. |
| AST nodes | 4,096 | Maximum AST nodes per equation. |
| Compiled opcodes | 100,000 | Maximum VM opcodes across all equation, init, and transform blocks. |
| Dataset selector indices | 50,000 | Maximum `<...>` dataset selector indices. |
| Dataset branch classes | 4,096 | Maximum distinct `<...>` dataset branch programs. |
| Bessel order | 100,000 | Maximum integer order $|n|$ for `jn` and `yn`. |
| VM steps per evaluation | 100,000 | Maximum bytecode steps per scalar evaluation. |
| ODE steps per point | 50,000 | Maximum adaptive RK45 steps per curve point. |
| ODE VM ops per evaluation | 5,000,000 | Maximum VM operations per ODE trajectory evaluation. |
| Global ODE steps per fit | 300,000 | Default per-fit RK45 step budget (scaled by 1 to 4 datasets). |

## Published references & oracle verification {#oracle-verification}

Every fitting algorithm and built-in curve model is verified offline under CurveBook's clean-room policy against public benchmark corpora, primary literature, and independent reference implementations. See the full verification record in [Curve-Fitting Evidence on GitHub](https://github.com/HauserGroup/curvebook/blob/main/docs/evidence/curve-fitting.md).

- **NIST StRD nonlinear benchmarks:** Parameter estimates, asymptotic SEs, residual sums of squares, and residual SDs match certified NIST StRD values across Lower, Average, and Higher difficulty datasets (`start_1` and `start_2`, including logarithmic transforms for `Nelson`). Algebraic reparameterizations are also verified across built-in pharmacology, kinetics, growth, Gaussian, Boltzmann, power-series, and polynomial equations.
- **Dose-response & pharmacology (R 4.5.2 `stats::nls`, `stats::lm`):** Covers 4PL and 5PL log- and linear-concentration models (Motulsky & Christopoulos 2004; Gottschalk & Dunn 2005; Liao & Liu 2009). Also verifies Gaddum–Schild competitive antagonism and unity-slope `pKB` (Arunlakshana & Schild 1959; Colquhoun 2007), Black–Leff operational partial agonism and receptor depletion (Black & Leff 1983), and Christopoulos–Kenakin allosteric modulation (Christopoulos & Kenakin 2002).
- **Binding & enzyme kinetics (R 4.5.2 `stats::nls`, `stats::lm`):** Covers Cheng–Prusoff and two-site receptor binding (Cheng & Prusoff 1973) and Swillens ligand-depletion saturation (Swillens 1995). Enzyme models cover Michaelis–Menten, Hill allosteric, Haldane substrate inhibition, and Morrison tight-binding kinetics (Cornish-Bowden 1974; Morrison 1969).
- **Growth, peaks & radiobiology (R 4.5.2 `stats::nls`, `stats::lm`):** Verifies multi-phase exponential decay and association, Verhulst (1838) logistic, Hodgkin & Huxley (1952) Boltzmann sigmoid, and Gompertz (1825) growth. Also covers Watts & Bacon (1974) smooth hinge, Limpert et al. (2001) log-Gaussian and area-parameterized lognormal, Weibull (1951) peaks, and the Linear–Quadratic radiobiology family (Chadwick & Leenhouts 1973; Brenner 2008).
- **Uncertainty & diagnostics (`mpmath 1.3.0`, R 4.5.2):** Verified against 80- and 120-digit `mpmath` and R (`stats::nls`, `stats::glm`, `stats::lm`). Covers Venzon–Moolgavkar (1988) profile-likelihood and Poisson deviance intervals (including open bounds), Bates & Watts (1988) / Seber & Wild (1989) linearization covariance, Hougaard (1985) skewness, and Cook & Weisberg (1982) leverage residuals.
- **Model comparison, ROUT, Deming & ODEs (`mpmath 1.3.0`, R 4.5.2):** Verifies Burnham & Anderson (2002) $\text{AIC}_c$ weights, Motulsky & Brown (2006) ROUT $Q = 1\%$ detection on raw and weighted curves, and Sebaugh (2011) sparse-curve identifiability. Also verifies Deming (1943) / Linnet (1990) jackknife regression, Brent (1973) implicit roots over 400 orders of magnitude, and Hairer, Nørsett & Wanner (1993) RK45 ODE fits.
- **Model-free splines & LOWESS (R 4.5.2):** Verified against R (`stats::splinefun`, `stats::smooth.spline`, `stats::lowess`) and analytical Akima step-overshoot oracles. Covers natural cubic splines (de Boor 1978), Akima subsplines (Akima 1970), Reinsch smoothing splines (Reinsch 1967; Hutchinson & de Hoog 1985), Cleveland LOWESS (Cleveland 1979), and dense point-to-point unknown interpolation (see [Fit Spline / LOWESS Curve](analyses.md#analysis-fit-spline-lowess)).
