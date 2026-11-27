# Curve fitting and nonlinear regression reference

<!-- Audience: Tier 2 Technical Reference — complete nonlinear regression algorithms, weighting formulas, constraints, diagnostics, DSL/ODE specs, and citations belong here; keep desktop-gui.md and get-started.md concise for beginners. -->

This technical reference gives the exact regression objectives, weighting formulas, parameter constraints, uncertainty diagnostics, and rules for custom equations and ODEs. For an interactive walkthrough of running a curve fit and viewing its live graph, start with the [Desktop GUI overview](desktop-gui.md).

The [Parametric Equations Registry](registry/equations.html) has compact tables of the built-in models, their parameters, and their defaults. For sheet-level options and the table types each analysis accepts, see [Nonlinear Regression](analyses.md#analysis-nonlinear-regression), [Linear Regression](analyses.md#analysis-linear-regression), [Simple Logistic Regression](analyses.md#analysis-simple-logistic-regression), and the [Command Reference](../commands.md).

## Fitting methods and estimation objectives {#fitting-methods}

Nonlinear regression fits a parametric model $y = f(x; \theta) + \varepsilon$ to the active (non-excluded) observations. You can run it with [`analyze.run`](../commands.md), `fit.run`, or `analysis.add` with `nonlinear`. The input can be raw replicates or [pre-summarized XY subcolumns](tables.md#subcolumn-modes) (`MeanSdN`, `MeanSemN`, `MeanCvN`). To fit only part of the predictor range, set optional bounds with `x_range_filter`.

Choose the fitting method that matches your data:

- Least squares for roughly Gaussian scatter.
- Poisson deviance for non-negative counts.
- Robust Cauchy regression to down-weight outliers without deleting them.
- ROUT to flag outliers and refit the remaining clean inliers.

> [!NOTE]
> Under weighted least squares, Poisson deviance, or replicate-moment weighting, the optimizer minimizes `weighted_ss`. The reported `rss` and the unweighted $R^2 = 1 - \text{rss} / \text{TSS}$ stay in the original units of the response $Y$ (Motulsky & Christopoulos 2004).

### 1. Least squares (`method: "least_squares"`)

Least squares is the default method. Use it when the scatter around the curve is roughly Gaussian, or becomes uniform once the residuals are weighted. It minimizes the weighted sum of squared residuals:

$$SS_w(\theta) = w_1 (y_1 - \hat{y}_1)^2 + w_2 (y_2 - \hat{y}_2)^2 + ... + w_N (y_N - \hat{y}_N)^2$$

- **Core optimizer:** CurveBook uses trust-region Levenberg–Marquardt (Levenberg 1944; Marquardt 1963; Moré 1978 / MINPACK-1 `lmder`) and solves each step by column-scaled QR decomposition of the augmented Jacobian $[J; \sqrt{\lambda} D]$. A step is accepted when $\rho \ge 10^{-4}$. The trust region contracts by a factor in $[0.1, 0.5]$ and expands up to $\text{max}(\delta, 2 |D p|)$, and active faces are projected onto the box bounds.
- **Tolerances and iteration limits:** The default stopping thresholds are `ftol = 1e-15` for relative residual reduction, `xtol = 1e-14` for the parameter step, `gtol = 1e-13` for the projected gradient cosine, and `stall_gtol = 1e-4` for stalls. You can choose a `convergence_strictness` preset (`Default`, `Quick`, `Medium`, `Strict`) and set `max_iters` (default `1000`, hard cap `50,000`). The `replicate_fit_mode` option decides how replicate subcolumns enter the fit: as individual replicates (`IndividualReplicates`, the default) or as unweighted row means (`RowMeansOnly`).
- **Warm start and rescue:** When `use_simplex_warm_start` is `true` (the default), a bounded Nelder–Mead simplex search (Nelder & Mead 1965; Lagarias et al. 1998) first finds an initial basin. The same search also rescues stalled or non-finite starting points.
- **Jacobians:** Built-in models use analytical derivatives $df / d\theta_j$ where they are available. Other models, including all custom DSL equations, use central finite differences with step $h = 10^{-6} \cdot \text{max}(|\theta_j|, 1)$. At active bounds they switch to interior one-sided differences with $h = 1.5 \times 10^{-8} \cdot \text{max}(|\theta_j|, 1)$ (Nocedal & Wright 2006).

### 2. Poisson deviance (`method: "poisson_deviance"`)

Use Poisson deviance for non-negative count data ($y_i \ge 0$) when the predicted mean $\hat{\mu}_i = f(x_i; \theta)$ is strictly positive. The method minimizes the exact Poisson log-likelihood deviance (McCullagh & Nelder 1989):

$$D(\theta) = 2 \cdot [ y_1 \ln(y_1 / \hat{\mu}_1) - (y_1 - \hat{\mu}_1) ] + ... + 2 \cdot [ y_N \ln(y_N / \hat{\mu}_N) - (y_N - \hat{\mu}_N) ]$$

A term with $y_i = 0$ contributes $2 \hat{\mu}_i$, so the objective stays smooth at zero counts.

- **Solver:** CurveBook uses iteratively reweighted least squares (IRLS; Green 1984) with working weights $1 / \hat{\mu}_i$. Inner trust-region Levenberg–Marquardt updates repeat until the relative change in deviance drops below $10^{-10}$.
- **Domain guards:** A negative observed count ($y_i < 0$) makes the fit fail with `ImpossibleWeights`. Custom Gaussian `weighting` options are ignored; the method always uses the exact Poisson variance $V(\mu) = \mu$.
- **Inference:** The dispersion is fixed at $\phi = 1$, and the parameter covariance comes from the asymptotic Fisher information. Curve bands use two-sided normal $z$ quantiles, and the model criteria $\text{AIC}_c$ and $\text{BIC}$ use the exact Poisson log-likelihood.

### 3. Robust Cauchy regression (`method: "robust_cauchy"`)

Use robust Cauchy regression when outliers distort the least-squares estimates and you want to down-weight extreme residuals without deleting points. This Lorentzian M-estimator minimizes a logarithmic loss (Motulsky & Brown 2006):

$$L(\theta) = \ln(1 + (r_1 / \text{RSDR})^2) + ... + \ln(1 + (r_N / \text{RSDR})^2)$$

The Cauchy IRLS solver scales each residual by the robust standard deviation of the residuals (`RSDR`). CurveBook computes `RSDR` as the $68.27\text{th}$ percentile of the weighted residual magnitudes, multiplied by $N / (N - K_{\text{free}})$. Every observation stays in the dataset; no point is deleted.

> [!IMPORTANT]
> Under `robust_cauchy`, CurveBook intentionally suppresses parametric standard errors, confidence intervals, parameter dependency, and Hougaard skewness, and shows them as empty cells or open intervals. It also omits the residual SD ($S_{y.x}$), $\text{AIC}_c$, $\text{BIC}$, studentized residuals, replicate lack-of-fit tests, and confidence and prediction bands. The reason is that the standard Gaussian linearization covariance is invalid under the non-quadratic Cauchy loss. The best-fit parameters, unweighted $R^2$, and `RSDR` are still reported. Use `rout_outlier_elimination` when you need parametric inference on the clean inliers.

### 4. ROUT outlier elimination (`method: "rout_outlier_elimination"`)

Use ROUT when you want automatic outlier screening followed by standard parametric inference (SEs, CIs, bands, and model comparisons) on the retained inliers. CurveBook implements the three-stage ROUT procedure (Motulsky & Brown 2006):

1. **Robust Cauchy fit:** CurveBook fits the model with `robust_cauchy` to get a resistant baseline. It then computes `RSDR` from the base-weighted residuals $r_{w,i} = \sqrt{w_i} (y_i - \hat{y}_i)$. Weighting (such as `relative_square`, $1/Y^2$) puts the residuals on a uniform scale before screening.
2. **FDR screening of the extreme tail:** Each residual gets a two-sided Student-$t$ $P$ value from $t_i = |r_{w,i}| / \text{RSDR}$. Benjamini–Hochberg (1995) step-up FDR control runs only across the 30% of observations furthest from the curve (`start = floor(0.70 * N)`). The other ~70% of residuals, those closest to the curve, are never tested (`q_value = 1.0`, `is_outlier = false`).
3. **Least-squares refit:** CurveBook refits the retained inliers by ordinary or weighted least squares, starting from the robust Cauchy parameters.

- **FDR threshold and modes:** `fdr_q_percent` sets the false discovery rate $Q$ in percent (default `1.0`, meaning $Q = 1\%$). Nonlinear regression requires $0 < Q \le 50$, whereas [univariate column outlier detection](analyses.md#analysis-outlier-detection) allows $(0, 100]$. Set `outlier_handling` to `Eliminate`, `IdentifyAndCountOnly`, or `None`; `IdentifyAndCountOnly` flags points without removing them from the least-squares fit. Turn on `emit_cleaned_data_tab` to add a cleaned data tab, without the flagged outliers, that you can analyze further.
- **Too few inliers:** If ROUT excludes so many points that no residual degrees of freedom remain, the solver returns `InsufficientData` or a non-converged warning. The diagnostics tab lists the coordinates and $q$-values of the detected outliers, and the source [data table](concepts.md#cell-exclusions) is not changed.

### Optimizer status outcomes

| Status | Meaning |
| :--- | :--- |
| `Converged` | The stopping criteria are met and the Jacobian has full rank, so the fit is identifiable. |
| `Ambiguous` | The objective converged, but the Jacobian is rank-deficient, `Dependency > 0.9999`, or the objective has an open plateau. |
| `HitMaxIters` | The iteration cap was reached before the strict stopping tolerances were met. |
| `Stalled` | The trust-region step cannot reduce the objective any further (`stall_gtol`). |
| `Diverged` | The objective became non-finite or exceeded numerical limits. |

## Weighting schemes and replicate summary moments {#weighting-and-moments}

Use weighting when the experimental scatter grows or shrinks systematically with $Y$ or $X$, for example when the percentage CV is constant across assay concentrations. The `weighting` option scales the residuals before least-squares, Cauchy, and ROUT fitting. For response-dependent schemes, `weighting_source` chooses between fixed weights from the observed values (`ObservedY`) and weights from the curve height that IRLS updates (`PredictedY`). Weighting is ignored under `poisson_deviance`, and explicit cell-level weights take precedence when they are attached. See also [Parametric Equations Registry: Weighting Schemes](registry/equations.html#weighting-schemes).

| Key | Formula | Error model and guard |
| :--- | :--- | :--- |
| `none` | $w_i = 1$ | Uniform absolute SD across $Y$. |
| `relative_square` | $w_i = 1 / Y_i^2$ | Constant relative scatter (%CV); requires $Y_i \ne 0$. |
| `relative` | $w_i = 1 / |Y_i|$ | Variance proportional to the mean response; requires $Y_i \ne 0$. |
| `general_power_y` | $w_i = 1 / |Y_i|^k$ | Power-law variance with a finite exponent `weighting_power` ($k$); requires $Y_i \ne 0$ when $k > 0$. |
| `inverse_x` | $w_i = 1 / |X_i|$ | Scatter proportional to $\sqrt{|X|}$; requires $X_i \ne 0$. |
| `inverse_x2` | $w_i = 1 / X_i^2$ | Scatter relative to the predictor's magnitude; requires $X_i \ne 0$. |
| `observed_sd` | $w_i = 1 / \text{SD}_i^2$ | Empirical variance from pre-summarized data; requires $\text{SD}_i > 0$ in every active row. |

If any weight is non-finite, has a zero denominator, or overflows, the fit stops with an `ImpossibleWeights` error.

### Replicate summary subcolumns (`MeanSdN`, `MeanSemN`, `MeanCvN`)

When an XY table uses a [pre-summarized subcolumn mode](tables.md#subcolumn-modes) (`MeanSdN`, `MeanSemN`, `MeanCvN`) and the fit uses neither explicit weights nor `observed_sd`, the solver reconstructs the exact replicate sufficient statistics from $(\text{mean}_i, \text{SD}_i, n_i)$:

- Each row's weighted residual is scaled by $\sqrt{w_i \cdot n_i}$.
- The total sample size is $N = n_1 + ... + n_M$.
- The pooled within-row variance adds $w_i (n_i - 1) \text{SD}_i^2$ to both `weighted_ss` and the unweighted `rss`. As a result, the estimates, parameter SEs, $\text{AIC}_c$, and replicate lack-of-fit tests exactly match those of an equivalent fit to the raw replicates (Motulsky & Christopoulos 2004).

Under `observed_sd`, each summary row contributes a single point with weight $1 / \text{SD}_i^2$, which requires a finite $\text{SD}_i > 0$.

## Parameter scopes, bounds, and dynamic hooks {#parameter-scopes}

Use `parameter_rules` to fix known assay constants, enforce physical bounds, or share parameters across datasets in a global fit. Each rule names a parameter with `name` (matched case-insensitively) and sets its scope, starting value, or limits:

| `scope` value | Behavior and free parameters |
| :--- | :--- |
| `local` | The default. Fits an independent value for each active dataset. |
| `shared_global` | Pools the selected datasets and fits one shared value. |
| `constant` | Fixes the parameter to a finite scalar `value` in every dataset (0 free parameters). |
| `hooked_constant` | Takes a finite scalar from an info constant (`info_constant`) or from an up-to-date result scalar (`result_scalar`). |
| `per_dataset_constant` | Fixes a constant for each listed dataset, given as `(data_set_index, value)` pairs in `per_dataset_constants`; datasets that are not listed stay `local`. |
| `column_title_constant` | Reads a finite numeric constant from each dataset's column header. |

Parameter rule details and guards:

- **Hook validation:** A `hooked_constant` rule fails with an error if the referenced constant or scalar is missing, non-finite, or dirty.
- **Column-title unit suffixes:** `column_title_constant` accepts scientific notation and these unit suffixes: `M`, `mM`, `uM`, `nM`, `pM`, `fM`, `%`, `s`, `ms`, `min`, `h`, `Hz`, and `deg`. A header that cannot be parsed stops the fit with an error.
- **Initial guesses and box bounds:** When `initial_override` is omitted, model heuristics compute starting values from each dataset, or from the pooled data for `shared_global` parameters. `lower_bound` and `upper_bound` must be finite, with $\text{lower} \le \text{upper}$, and they must intersect the model's intrinsic domain bounds (such as positive rate constants). Setting $\text{lower} = \text{upper}$ fixes the parameter (0 free parameters).
- **Relational constraints:** The solver enforces pairwise parameter inequalities directly, such as `KFast > KSlow` in two-phase decay or `KdLo > KdHi` in two-site binding. Custom equations can declare such constraints in a `[Constraints]` section.
- **Derived-scalar uncertainty:** Derived scalars from built-in models and from custom `[Transforms]` sections report propagated SEs and CIs. Examples include `EC50 = 10^LogEC50`, `KB = 10^-pKB`, `t_half = ln(2)/K`, `Span = Top - Bottom`, and the analytical inflection points of logistic and Gompertz models. CurveBook propagates the uncertainty either by exact monotonic mapping of the interval endpoints (swapping the bounds when the transform's derivative is negative) or by the multi-parameter delta method.
- **Model default rules:** Some built-in equations apply default rules unless a user rule overrides them. For example, centered polynomials fix `XCenter` at the sample mean $\bar{x}$ of the active predictor values to keep the Jacobian well conditioned.

## Parameter uncertainty, diagnostics, and safeguards {#uncertainty-and-diagnostics}

### Profile-likelihood and Wald confidence intervals

Nonlinear parameter estimates often have asymmetric uncertainty distributions, so CurveBook reports profile-likelihood confidence intervals by default instead of symmetric Wald limits.

- **Wald asymptotic covariance:** For least squares, $\text{Cov}(\hat{\theta}) = s_w^2 (J^T J)^{-1}$, with $s_w^2 = SS_w / \text{df}$ and $\text{df} = N - K_{\text{free}}$. For Poisson deviance, $\text{Cov}(\hat{\theta}) = (J^T W J)^{-1}$ with $W = \text{diag}(\hat{\mu}_i)$. Symmetric Wald intervals use the Student $t_{\text{df}, \alpha/2}$ quantile for least squares and the normal $z_{\alpha/2}$ quantile for Poisson.
- **Venzon–Moolgavkar (1988) profile-likelihood intervals (`profile_likelihood_ci: true`, the default):** For each free parameter $\theta_j$, CurveBook re-optimizes the nuisance parameters $\theta_{-j}$ with a reduced trust-region Levenberg–Marquardt solve (`ftol = 1e-12`, `xtol = 1e-12`). An outward bracketing scan, refined by Brent root finding (`rel_tol = 1e-7`), then solves:

$$SS_{\text{prof}}(\theta_j) = SS_{\text{min}} \cdot (1 + \frac{1}{\text{df}} F_{1 - \alpha}(1, \text{df}))$$

Under `poisson_deviance`, the critical threshold is a deviance increase of $\Delta D(\theta_j) = \chi^2_{1, 1 - \alpha}$. If the profiled objective reaches a boundary, the search horizon, or a flat plateau before it crosses the threshold, CurveBook reports an open (unbounded) endpoint rather than substituting symmetric Wald limits (Bates & Watts 1988; Seber & Wild 1989).

### Hougaard Skewness ($g_1$)

Hougaard's (1985) third-order skewness $g_1$ measures how closely a parameter's sampling distribution behaves like that of a linear model. When $1 \le K_{\text{free}} \le 10$, $\text{df} > 0$, and the fit is identifiable, CurveBook computes $g_1$ for each parameter from central approximations of the second-derivative tensor:

- $|g_1| < 0.1$: effectively linear behavior; Wald and profile intervals agree closely.
- $0.1 \le |g_1| < 0.25$: mild parameter-effects nonlinearity.
- $0.25 \le |g_1| \le 1.0$: substantial skewness; asymmetric profile-likelihood intervals are materially more accurate than symmetric intervals.
- $|g_1| > 1.0$: extreme nonlinearity; CurveBook adds a warning to the diagnostic checklist.

### Collinearity and fit identifiability

The Bates & Watts dependency measures, for each free parameter, how much collinearity with the other parameters inflates its variance:

$$R_j^2 = 1 - \frac{1}{(J^T J)_{jj} \cdot [(J^T J)^{-1}]_{jj}}$$

- **Collinearity warning:** When `Dependency > 0.99`, CurveBook shows a collinearity warning on the parameter and in the diagnostic checklist.
- **Identifiability reporting (`identifiability_mode`):** Choose `IdentifyUnstableParameters` to mark unstable parameters by greedy reduction of the Hessian condition number. Choose `IdentifyAmbiguousFits` to flag rank deficiency, $R_j^2 > 0.9999$, or open profile-likelihood bounds. The third option is `ReportAll`.
- **Standard errors and batch flags:** `show_standard_errors` overrides the default SE visibility. By default, SEs are hidden when asymmetric profile-likelihood CIs are reported and shown when symmetric Wald CIs are reported. `fit_flag_rules` evaluates automatic batch quality flags for $R^2$, $N$, dependency, residual normality, runs or lack of fit, and outlier count.

### Residual diagnostics, range filters, and threshold alerts

- **Predictor range filter (`x_range_filter`):** This filter restricts the fit to active rows with $X \in [x_{\text{min}}, x_{\text{max}}]$, where an omitted side is unbounded. CurveBook counts the excluded points. Curve overlays can optionally be drawn over a separate `[plot_x_min, plot_x_max]` range.
- **Unweighted $R^2$ alert (`flag_r_squared_below`):** This optional threshold lies in $[0, 1]$. CurveBook adds a checklist warning whenever the unweighted $R^2$ falls below it.
- **Internally studentized residuals:** CurveBook computes $e_i = r_{w,i} / (s_w \sqrt{1 - h_{ii}})$ from the tangent-plane leverage $h_{ii} = J_i (J^T J)^{-1} J_i^T$ (Cook & Weisberg 1982). It does so only when the fit is identifiable and $K_{\text{free}} \le 64$.
- **Replicates lack-of-fit test:** This test splits the residual variance (or deviance) at replicated $X$ values into pure error and lack of fit, and reports an $F$ statistic and $P$ value. It accounts for weighting, Poisson deviance, and summary moments.
- **Runs test:** CurveBook runs a Wald–Wolfowitz runs test along ascending $X$ and combines the datasets in global fits.
- **Normality and homoscedasticity tests:** CurveBook tests the weighted or Pearson residuals with Shapiro–Wilk, Anderson–Darling, and Lilliefors Kolmogorov–Smirnov tests. Outside the range where Shapiro–Wilk applies, it uses D'Agostino–Pearson $K^2$ instead. It also computes the Spearman rank correlation of $|r_{w,i}|$ with $\hat{y}_i$ when $n \ge 4$.
- **Goodness-of-fit metrics:** Least-squares fits report the unweighted $R^2$, `rss`, `weighted_ss`, adjusted $R^2$, Willett & Singer (1988) weighted $R^2$, `RMSE`, and the residual standard deviation ($S_{y.x}$). Under `poisson_deviance`, CurveBook also reports the Poisson negative log-likelihood, residual deviance, pseudo-$R^2$, Pearson $\chi^2$, and dispersion ratio.
- **Bands and AUC:** CurveBook samples delta-method confidence bands for the mean and prediction bands for single observations over `curve_overlay_segments` segments (default 256; see the [Graphs Reference](graphs.md)). It also computes the trapezoidal area under the curve (`fit_auc`) for the fitted curve and for the empirical data.

> [!NOTE]
> Every assumption checklist item in CurveBook is diagnostic only. It reports objective metrics and methodological limits but never recommends alternative models or transformations (see [Quality of Life Capabilities](qol.md#active-qol)).

## Model comparison and interpolation of unknowns {#comparison-and-interpolation}

### Comparing models (`comparison`)

Use the extra-sum-of-squares $F$-test (or the deviance likelihood ratio test) when one model is a simpler, nested case of the other. Use $\text{AIC}_c$ to compare nested or non-nested models fitted to exactly the same active data and weights.

1. **Nested extra-sum-of-squares $F$-test (`extra_sum_of_squares_f` / `shared_parameter_comparison`):** This test compares a simpler model with the fuller model ($\text{df}_1 > \text{df}_2$). The simpler model is either a constrained or null model given by `alternative_equation_id`, or the same model with globally shared parameters:

$$F = \frac{(SS_{\text{null}} - SS_{\text{alt}}) / (\text{df}_{\text{null}} - \text{df}_{\text{alt}})}{SS_{\text{alt}} / \text{df}_{\text{alt}}}$$

Under `poisson_deviance` or `likelihood_ratio_test`, CurveBook instead runs an asymptotic likelihood ratio test that compares the deviance difference $\Delta D$ with $\chi^2_{\Delta \text{df}}$. The model comparison tab reports $F$ or $\chi^2$, $\text{df}$, the $P$ value, the preferred model, and the percentage difference in residual SD.

2. **Corrected Akaike Information Criterion (`aicc`):** This is the small-sample corrected AIC, which requires identical active data and weights for both models (Burnham & Anderson 2002):

$$\text{AIC}_c = N \ln\left(\frac{SS_w}{N}\right) + 2 (K + 1) + \frac{2 (K + 1)(K + 2)}{N - K - 2}$$

For the exact Poisson log-likelihood $\ln L$, $\text{AIC}_c = -2 \ln L + 2K + \frac{2K(K+1)}{N - K - 1}$, where $K$ excludes fixed constants. CurveBook also reports $\Delta \text{AIC}_c$, the normalized Akaike weights $w_{\text{alt}}$ and $w_{\text{null}}$, and the evidence ratio. $\text{AIC}_c$ is not available in `robust_cauchy` mode.

### Forward prediction and inverse interpolation (`interpolate_unknowns`)

When `interpolate_unknowns` is on, each `(X, blank)` row gets a predicted $\hat{Y}$ from the fitted curve. Each `(blank, Y)` row gets an $\hat{X}$ that CurveBook finds numerically by bracketed Brent and damped Newton root search.

- **Curve-mean and replicate intervals:** The delta-method curve variance $\sigma_{\text{mean}}^2(x) = g(x)^T \text{Cov}(\hat{\theta}) g(x)$, where $g(x)$ is the gradient of $f(x; \hat{\theta})$ with respect to the parameters, gives curve-mean bands (`Interpolated Mean X`). Adding the observation scatter, $\sigma_{\text{pred}}^2(x) = \sigma_{\text{mean}}^2(x) + s_w^2 / w(x)$, gives inverse prediction intervals for a single replicate (`Interpolated Replicates`) whenever the active data contain replicate scatter ($s_w > 0$). Forward and inverse intervals both propagate standard errors as $\text{SE}(\hat{X}) = \sigma / |df / dX|$ evaluated at $\hat{X}$.
- **Extrapolation and no-root guards:** Rows with $X$ outside $[X_{\text{min}}, X_{\text{max}}]$, or with a target $Y$ outside the curve's monotonic range, are flagged `Extrapolated`. Rows with no real root (such as a $Y$ beyond a 4PL asymptote) are left empty.

## Custom equation DSL and first-order ODE models {#custom-dsl-and-ode}

User-defined nonlinear equations (`custom_dsl`) compile to bytecode for a sandboxed, stack-based virtual machine (VM) and use central finite-difference Jacobians. The DSL supports explicit equations, dataset-specific branches, implicit equations solved by root finding, initial-value rules, derived transforms, and first-order ODEs.

### Grammar, sections, and precedence

An equation is a series of newline-separated statements, organized into optional sections: `[Equation]` (the default), `[InitialValues]`, `[Constraints]`, and `[Transforms]`. Line comments start with `;`, `//`, or `#`.

**Operator precedence (highest to lowest):**

1. Parentheses `(...)`, numeric literals, variable and parameter identifiers, and function calls `f(...)`
2. Exponentiation `^` or `**`, which is right-associative and binds tighter than prefix unary operators: `-X^2` parses as `-(X^2)`, and `X^-2` parses as `X^(-2)`
3. Prefix unary `+`, `-`, `!` (`NOT`)
4. Multiplicative `*`, `/`, `%` (left-associative)
5. Additive `+`, `-` (left-associative)
6. Comparisons `==`, `=`, `!=`, `<>`, `<`, `<=`, `>`, `>=`
7. Logical `&&` (`AND`), then `||` (`OR`)

**Statement forms:**

- **Explicit assignment:** `Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))`. A bare right-hand-side expression also works.
- **Local helper variables:** Define a helper such as `span = Top - Bottom` before the target `Y = ...` line.
- **Conditional branching:** `IF(cond, then_expr, else_expr)`.
- **Dataset-specific lines (`<A>`, `<B>`, `<A:C>`, `<2>`):** Prefix a line with angle brackets that contain column letters (`A`–`Z`, …), 1-based dataset indices, or ranges. The line then applies only to those datasets, so a shared fit can use different expressions or assignments for different datasets. Free identifiers outside any dataset scope become fitted parameters, whereas assignments inside a dataset scope restrict their parameters to those datasets.
- **Initial-value rules (`[InitialValues]`):** Formulas that compute starting values from the active `(X, Y)` points and numeric column titles. The supported summary tokens are `YMIN`, `YMAX`, `XMIN`, `XMAX`, `XMID`, `YMID`, `XATYMIN`, `XATYMAX`, `XATY50`, `XATYMID`, `YATXMIN`, `YATXMID`, `YATXMAX`, `SPAN`, `SIGN`, `INITIALVALUE`, `MEANCOLUMNTITLE`, `LOGMEANCOLUMNTITLE`, `SLOPEATXMIN`, `SLOPEATXMID`, and `SLOPEATXMAX`.
- **Derived scalar transforms (`[Transforms]`):** Post-fit algebraic expressions (such as `EC50 = 10^LogEC50`) are evaluated at the converged estimates and appear in `derived_scalars` and in the summary tabs.
- **Built-in constants and predictors:** The predictor is `X` (or `T` for an ODE time axis, and `X1`, `X2`, … for multiple predictors). The response is `Y`, and the dataset index is `DS` or `DATASET`. The built-in constants are `PI`, `E`, `LN2`, `LN10`, `SQRT2`, `INF`, and `NAN`.

### Built-in DSL functions

| Category | Supported identifiers (case-insensitive) |
| :--- | :--- |
| Elementary and arithmetic | `abs`, `sqrt`, `cbrt`, `sqr`, `exp`, `exp10`, `ln` (`loge`), `log` (`log10`), `log2`, `pow(a,b)`, `mod(x,y)`, `hypot(a,b)`, `min(a,b,...)`, `max(a,b,...)`, `clamp(x,lo,hi)`, `fact(n)`, `perc(x)`, `deg(rad)`, `rad(deg)`, `is_defined(x)` |
| Rounding and sign | `floor`, `ceil`, `round`, `trunc` (`int`), `sign` (`sgn`) |
| Trigonometric and inverse | `sin`, `cos`, `tan`, `asin`, `acos`, `atan` (`arctan`), `atan2(y,x)` |
| Hyperbolic and inverse | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` |
| Error, normal, and probability | `erf`, `erfc`, `logit(p)`, `probit(p)`, `normcdf` (`zdist`), `normpdf` (`zpdf`), `norminv` (`zinv`), 3-arg `normdist(x,mean,sd)` / `norminv(p,mean,sd)` / `normpdf(x,mean,sd)`, `tdist(t,df)` / `tinv(p,df)` / `tpdf(t,df)`, `fdist(f,df1,df2)` / `finv(p,df1,df2)` / `fpdf(f,df1,df2)`, `chidist(x2,df)` / `chisqc(x2,df)` / `chiinv(p,df)`, `binomial(k,n,p)` |
| Gamma, Beta, and Bessel | `gamma`, `lngamma` (`gammaln`, `lgamma`), `beta(a,b)`, `ibeta(a,b,x)`, `igamma(a,x)`, `igammac(a,x)`, `digamma` (`psi`), `besselj0`, `besselj1`, `besseljn`, `bessely0`, `bessely1`, `besselyn`, `besseli0`, `besseli1`, `besseli(n,x)`, `besselk0`, `besselk1`, `besselk(n,x)` |
| Hypergeometric | `hyp1f1` (`hypgeometricm`), `hyp2f1` (`hypgeometricf`), `confluent_u` (`hypgeometricu`) |

### Implicit equations (`LHS = RHS`)

An `[Equation]` target compiles as an implicit root problem, $F(X, Y; \theta) = \text{LHS} - \text{RHS} = 0$, whenever `Y` appears on both sides or the left-hand side is not plain `Y`:

- The solver scans multi-decade, scale-augmented probe grids outward from 0 in both the positive and negative directions to bracket a sign change. It refines each bracket with safeguarded Brent root finding (`rel_tol = 1e-13`) and re-brackets tightly to `[0, root]` near zero.
- If it finds no bracket with a sign change, the solver falls back to damped Newton iteration. It returns a domain failure if the residual $|F|$ exceeds the tolerance or is non-finite.

### First-order ODE models (`dY/dX` / `dY/dT` / `D(Y, X)`)

A single first-order ODE is written as a derivative line, such as `dY/dX = -k * Y`, `dY/dT = r * Y * (1 - Y / K)`, or `D(Y, X) = ...`. Pair it with an initial condition, `Y(0) = Y0` or `Y(t0) = expr`. If you write `Y0 = ...`, $t_0$ defaults to 0.

- **Integrator:** CurveBook integrates with the adaptive embedded 5(4) Dormand–Prince RK45 method (Dormand & Prince 1980), using `rtol = 1e-8` and the mixed error control $\text{err} \le \text{atol} \cdot y_{\text{ref}} + \text{rtol} \cdot \text{max}(|y|, |y_{\text{new}}|)$ (Hairer, Nørsett & Wanner 1993 §II.4). It supports both forward ($t_{\text{end}} > t_0$) and backward ($t_{\text{end}} < t_0$) integration.
- **Trajectory cache:** CurveBook sorts the observations in each dataset by ascending $X$ so that continuous trajectories can be reused across points and overlays. It caches up to 4 trajectories, with up to 4,096 knots each.

### Sandboxed DSL and ODE resource caps

Every custom equation runs within strict compile-time and runtime limits that prevent unbounded recursion and runaway ODE integration:

| Resource limit | Value | Purpose |
| :--- | ---: | :--- |
| Source size | 1,048,576 bytes | Maximum custom equation source size (1 MiB). |
| Source lines | 16,384 | Maximum source lines per equation. |
| Parse nesting depth | 128 | Maximum depth of nested parentheses or calls. |
| AST nodes | 4,096 | Maximum AST nodes per equation. |
| Compiled opcodes | 100,000 | Maximum VM opcodes across all equation, initial-value, and transform blocks. |
| Dataset selector indices | 50,000 | Maximum `<...>` dataset selector indices. |
| Dataset branch classes | 4,096 | Maximum distinct `<...>` dataset branch programs. |
| Bessel order | 100,000 | Maximum integer order $|n|$ for `besseljn` (`jn`) and `besselyn` (`yn`). |
| VM steps per evaluation | 100,000 | Maximum bytecode steps per scalar evaluation. |
| ODE steps per point | 50,000 | Maximum adaptive RK45 steps per curve point. |
| ODE VM ops per evaluation | 5,000,000 | Maximum VM operations per ODE trajectory evaluation. |
| Global ODE steps per fit | 300,000 | Default RK45 step budget per fit (scaled by 1 to 4 datasets). |

### Reusable user equation library and theoretical distribution plotting

- **Atomic user equation library:** You can save, list, inspect, delete, and run reusable custom DSL equations by `equation_id`, including all of their sections (`[Equation]`, `[InitialValues]`, `[Constraints]`, and `[Transforms]`). CurveBook stores them inside `CURVEBOOK_CONFIG_DIR`. The commands are [`fit.saveUserEquation`](../commands.md), `fit.listUserEquations`, `fit.getUserEquation`, `fit.deleteUserEquation`, and `fit.cloneBuiltinEquation`, which copies any built-in equation into an editable custom template.
- **Theoretical probability distribution plotting (`plot_distribution`):** Set `equation_id: "plot_distribution"` and supply `plot_distribution` options to draw exact theoretical curves without fitting any data. Continuous distributions (`Normal`, `StudentT`, `FisherF`, `ChiSquare`) produce PDF and CDF curves with configurable tail shading (`LowerTail`, `UpperTail`, `TwoTailed`, `Between`). Discrete distributions (`Binomial`, `Poisson`) produce integer-step PMF and step-CDF curves. Optional parameter hooks take values from Info or Result sheets. The output includes chainable `analyzable_data` alongside the chart overlays.

## Published references and oracle verification {#oracle-verification}

Under CurveBook's clean-room policy, every fitting algorithm and built-in curve model is verified offline against public benchmark datasets, the primary literature, and independent reference implementations. The full verification record is in [Curve-Fitting Evidence on GitHub](https://github.com/HauserGroup/curvebook/blob/main/docs/evidence/curve-fitting.md).

- **NIST StRD nonlinear benchmarks:** Parameter estimates, asymptotic SEs, residual sums of squares, and residual SDs match the certified NIST StRD values for the Lower, Average, and Higher difficulty datasets. This holds from both `start_1` and `start_2`, including the logarithmic transforms for `Nelson`. Algebraic reparameterizations of the built-in pharmacology, kinetics, growth, Gaussian, Boltzmann, power-series, and polynomial equations are verified as well.
- **Dose-response and pharmacology (R 4.5.2 `stats::nls`, `stats::lm`):** Verification covers 4PL and 5PL models with log and linear concentration (Motulsky & Christopoulos 2004; Gottschalk & Dunn 2005; Liao & Liu 2009). It also covers Gaddum–Schild competitive antagonism and the unity-slope `pKB` (Arunlakshana & Schild 1959; Colquhoun 2007). Black–Leff operational partial agonism and receptor depletion (Black & Leff 1983) and Christopoulos–Kenakin allosteric modulation (Christopoulos & Kenakin 2002) are covered too.
- **Binding and enzyme kinetics (R 4.5.2 `stats::nls`, `stats::lm`):** Verification covers Cheng–Prusoff and two-site receptor binding (Cheng & Prusoff 1973) and Swillens ligand-depletion saturation (Swillens 1995). For enzymes, it covers Michaelis–Menten, Hill allosteric, Haldane substrate inhibition, and Morrison tight-binding kinetics (Cornish-Bowden 1974; Morrison 1969).
- **Growth, peaks, and radiobiology (R 4.5.2 `stats::nls`, `stats::lm`):** Verification covers multi-phase exponential decay and association, the Verhulst (1838) logistic, the Hodgkin & Huxley (1952) Boltzmann sigmoid, and Gompertz (1825) growth, including the analytical inflection points. It also covers the Watts & Bacon (1974) smooth hinge, the Limpert et al. (2001) log-Gaussian and area-parameterized lognormal, the cumulative Gaussian (`percent`, `fraction`, `counts`), the Lorentzian and the sum of two Lorentzians, the Padé `[1/1]`, and Weibull (1951) peaks. Classic legacy reparameterizations and the Linear–Quadratic radiobiology family (Chadwick & Leenhouts 1973; Brenner 2008) are covered too.
- **Uncertainty, relational constraints, GOF, and theoretical distributions (`mpmath 1.3.0`, R 4.5.2):** These results are checked against 80- and 120-digit `mpmath` and against R (`stats::nls`, `stats::glm`, `stats::lm`). Coverage includes Venzon–Moolgavkar (1988) profile-likelihood and Poisson deviance intervals (including open bounds), relational inequality constraints (`KFast > KSlow`, `KdLo > KdHi`, `LogKiLo > LogKiHi`), and monotonic or delta-method CIs for derived scalars. It also includes adjusted and weighted $R^2$, Poisson deviance GOF, Bates & Watts (1988) / Seber & Wild (1989) linearization covariance, Hougaard (1985) skewness, Cook & Weisberg (1982) leverage residuals, and exact grids for continuous and discrete theoretical distributions.
- **Model comparison, ROUT, Deming, and ODEs (`mpmath 1.3.0`, R 4.5.2):** Verification covers Burnham & Anderson (2002) $\text{AIC}_c$ weights, Motulsky & Brown (2006) ROUT detection at $Q = 1\%$ on raw and weighted curves, and Sebaugh (2011) sparse-curve identifiability. It also covers Deming (1943) / Linnet (1990) jackknife regression, Brent (1973) implicit roots over 400 orders of magnitude, and Hairer, Nørsett & Wanner (1993) RK45 ODE fits.
- **Model-free splines and LOWESS (R 4.5.2):** These are checked against R (`stats::splinefun`, `stats::smooth.spline`, `stats::lowess`) and against analytical oracles for Akima step overshoot. Coverage includes natural cubic splines (de Boor 1978), Akima subsplines (Akima 1970), Reinsch smoothing splines (Reinsch 1967; Hutchinson & de Hoog 1985), Cleveland LOWESS (Cleveland 1979), and dense point-to-point interpolation of unknowns (see [Fit Spline / LOWESS Curve](analyses.md#analysis-fit-spline-lowess)).
