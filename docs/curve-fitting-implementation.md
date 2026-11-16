# Curve Fitting & Nonlinear Regression — Implementation Guidelines

Governing specification for `crates/fit` (`L2`). Parity checklist: [`fitting-parity.md`](fitting-parity.md).

Public behavioural & mathematical references:
- [Principles of nonlinear regression & solver details](https://www.graphpad.com/guides/prism/latest/curve-fitting/reg_how_nonlinear_regression_works.htm)
- [Asymmetrical profile-likelihood confidence intervals](https://www.graphpad.com/guides/prism/latest/curve-fitting/reg_how_confidence_intervals_are_c.htm)
- [Robust nonlinear regression & ROUT outlier elimination](https://www.graphpad.com/guides/prism/latest/curve-fitting/reg_fit_tab_2.htm)
- [Comparing nonlinear models: Extra-sum-of-squares F-test vs. AICc](https://www.graphpad.com/guides/prism/latest/curve-fitting/reg_comparing_models_overview.htm)
- [User-defined equation syntax & function catalog](https://www.graphpad.com/guides/prism/latest/curve-fitting/reg_available_functions.htm)
- Primary literature (exact optimization, profile CIs, outliers & mechanistic kinetics):
  - **Solver & profile-likelihood CIs:** Moré (1978) *Lect. Notes Math.* 630:105–116 (`10.1007/BFb0067700`); Venzon & Moolgavkar (1988) *Appl. Stat.* 37:87–94 (`10.2307/2347496`); Bates & Watts (1988) *Nonlinear Regression Analysis and Its Applications*; Motulsky & Christopoulos (2004) *Fitting Models to Biological Data using Linear and Nonlinear Regression*.
  - **Robust regression & ROUT:** Motulsky & Brown (2006) *BMC Bioinformatics* 7:123 (`10.1186/1471-2105-7-123`).
  - **Tight-binding & competitive inhibition:** Morrison (1969) *Biochim. Biophys. Acta* 185:269–286 (`10.1016/0005-2744(69)90420-3`); Cheng & Prusoff (1973) *Biochem. Pharmacol.* 22:3099–3108 (`10.1016/0006-2952(73)90196-2`).

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
   Targets the heavy-tailed Cauchy objective (Motulsky & Brown 2006), implemented as iteratively reweighted least squares (`fit_cauchy_irls` in `crates/fit/src/solver.rs`, Cauchy weights $1/(1 + z_i^2)$ with $z_i = |r_i| / \text{RSDR}$, up to 15 reweighting passes):
   $$\mathcal{L}_{\text{Cauchy}}(\theta, s) = \sum_{i=1}^N \ln\!\left(1 + \frac{w_i (y_i - f(x_i; \theta))^2}{s^2}\right)$$
   where scale $s$ is updated iteratively from the $68.27\text{th}$ percentile of weighted absolute residuals ($\text{RSDR}$). Pure Robust mode (`FitMethod::RobustCauchy`) requires raw replicates (`FitError::RequiresRawReplicates` on pre-summarized data). *Planned ([`R4.5`](ROADMAP.md#r45-suppress-least-squares-inference-in-pure-robust-fit-mode), todo):* suppress asymptotic SEs, CIs, covariance matrices, confidence bands, and *F*/AICc comparisons in pure Robust mode — `fit_nonlinear` currently reports them as for least squares and there is no `NotApplicableInRobustMode` status. Robust mode serves both for outlier-resistant interpolation and as Stage 1 of **ROUT** automatic outlier elimination ($Q$ default $1\%$).

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
- **Standard-curve interpolation (`Interpolate unknowns`):** Given rows with $Y$ entered and $X$ blank, solve $f(x; \hat{\theta}) = y_{\text{unk}}$ by a bracketing scan plus root refinement over the fitted $X$ range extended by $\pm 50\%$ of its span (`interpolate_x_from_y` in `crates/fit/src/solver.rs`), reporting $\hat{x}_{\text{unk}}$ in `InterpolatedUnknown`, a CI from the confidence-band crossings, and an out-of-range `note` ("Target Y falls outside fitted curve range") instead of an estimate when no root exists.

### 2.4 Comparing Models (`Extra-Sum-of-Squares F` vs. `AICc`)

- **Extra-sum-of-squares *F*-test** (for nested models $M_1 \subset M_2$, $\text{df}_1 > \text{df}_2 > 0$):
  $$F = \frac{(\text{SS}_1 - \text{SS}_2) / (\text{df}_1 - \text{df}_2)}{\text{SS}_2 / \text{df}_2} \sim F_{\text{df}_1 - \text{df}_2,\, \text{df}_2}$$
- **Corrected Akaike Information Criterion ($\text{AIC}_c$)** (valid for nested and non-nested models on identical included data points, $N \ge K + 3$, where $K = p_{\text{free}} + 1$ includes $\sigma^2$):
  $$\text{AIC}_c = N \ln\!\left(\frac{\text{SS}}{N}\right) + 2K + \frac{2K(K+1)}{N - K - 1}, \qquad w_{\text{Akaike}, m} = \frac{e^{-\frac{1}{2}\Delta_m}}{\sum_j e^{-\frac{1}{2}\Delta_j}}$$
  When ROUT outlier elimination is enabled alongside model comparison, outlier masks are synchronized according to documented comparison rules so both models are evaluated on a consistent point set.
- **Residual diagnostics:** Replicates test for lack of fit, runs test, homoscedasticity test, D'Agostino–Pearson / Shapiro–Wilk / Anderson–Darling / KS normality of residuals, and residual plots.

---

## 3. Canonical Built-In Equation Library (`crates/fit/src/library/`)

Every built-in model implements `ParametricModel` (`crates/fit/src/library/mod.rs`) with an immutable string ID (`id()`, listed in `PHARMA_EQUATION_IDS` / `CURVE_EQUATION_IDS` / `NIST_EQUATION_IDS`; the live model count is in [docs/STATUS.md](STATUS.md)), human title, default bounds, initial-guess rules, reported derived transforms (`derived_transforms`, e.g. $\text{EC}_{50} = 10^{\log \text{EC}_{50}}$, half-life $\ln(2)/K$, $\tau = 1/K$, span, inflection point), and optional analytical partial derivatives (`grad`):

### 3.1 Dose–Response (Stimulation, Inhibition & Special)
- **Stimulation / Inhibition (`X = log10[C]` and linear `X = [C]` variants, unnormalized & `[0, 100]` normalized):**
  - 3PL (`HillSlope = ±1`): $Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log \text{EC}_{50} - X)}}$ (inhibition: $10^{(X - \log \text{IC}_{50})}$).
  - 4PL Variable Slope: $Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log \text{EC}_{50} - X) \cdot \text{HillSlope}}}$.
- **Special Dose–Response (`X = [C]` and `X = log10[C]`):**
  - **Asymmetrical 5PL:** $Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{\left(1 + 10^{(\log X_b - X) \cdot \text{HillSlope}}\right)^S}$ where $\log X_b = \log \text{EC}_{50} + \frac{1}{\text{HillSlope}}\log_{10}(2^{1/S} - 1)$ ($S > 0$).
  - **Biphasic** (two saturating phases with fraction $F \in (0,1)$) and **Bell-shaped** ($\text{Plateau}_1 \to \text{Dip} \to \text{Plateau}_2$).
  - **Black–Leff Operational Model** (`operational_model_agonist`, partial-agonist form with `Basal`, `Emax`, `LogKA`, `LogTau`, `n`; share parameters across full + partial agonists via global fitting; avoids overflow on $\log \tau$). A separate receptor-depletion variant is not a built-in ID.
  - **Gaddum/Schild $\text{EC}_{50}$ shift** (fitting $\text{p}A_2$ and Schild slope, or fixing `SchildSlope = 1` to fit $\log K_b$), **Allosteric $\text{EC}_{50}$ shift** (ternary complex cooperativity $\alpha$).
  - **$\text{EC}_{\text{anything}}$ ($\text{EC}_F$):** Parameterizes $\log \text{EC}_F$ directly for user constant $F \in (0, 100)$ via $\log \text{EC}_{50} = \log \text{EC}_F - \frac{1}{\text{HillSlope}}\log_{10}\!\left(\frac{F}{100 - F}\right)$.
  - **Absolute $\text{IC}_{50}$:** Fits concentration where $Y = \text{Fifty}$ ($50\%$ between baseline constant and control) even when $\text{Bottom} \ne 0$.

### 3.2 Receptor Binding (Saturation, Competition & Kinetics)
- **Saturation:** Built-in in `crates/fit/src/library/pharma.rs`: One-site specific ($Y = \frac{B_{\max} X}{K_d + X}$), One-site total ($+ \text{NS}\cdot X + \text{Background}$), Global total & nonspecific (`one_site_total_and_ns_global`, `<A>` total, `<B>` nonspecific), One-site with ligand depletion (`one_site_specific_ligand_depletion`, quadratic mass-action root), Specific binding with Hill slope, Two-site specific, Allosteric modulator (`allosteric_modulator_saturation`), and Two-site total & nonspecific (`two_site_total_and_ns_global`).
- **Competition & Kinetics:** Built-in in `crates/fit/src/library/pharma.rs`: One-site & Two-site Fit $K_i$ (Cheng–Prusoff $\log \text{IC}_{50} = \log_{10}\!\big(10^{\log K_i}(1 + [\text{Hot}]/K_{d,\text{hot}})\big)$ with hooked constants `HotNM` and `HotKdNM`), Fit $\log \text{IC}_{50}$, Homologous one-site competition (`homologous_one_site_competition`), Allosteric modulator, Association/Dissociation kinetics, Motulsky–Mahan kinetics (`motulsky_mahan_kinetics`), and One-site competition with ligand depletion (`one_site_competition_ligand_depletion`).
- **Kinetics:** Dissociation, Association (one or two+ ligand concentrations globally fitting $k_{\text{on}}$ and $k_{\text{off}}$), Association-then-dissociation (`X < X0` vs. `X >= X0`), and **Motulsky–Mahan kinetics of competitive binding**.

### 3.3 Enzyme Kinetics
- **Substrate–Velocity:** Michaelis–Menten ($Y = \frac{V_{\max} X}{K_m + X}$), $k_{\text{cat}}$ ($Y = \frac{E_t k_{\text{cat}} X}{K_m + X}$, constant $E_t$), Allosteric sigmoidal ($Y = \frac{V_{\max} X^h}{K_{\text{half}}^h + X^h}$), Substrate inhibition ($Y = \frac{V_{\max} X}{K_m + X(1 + X/K_i)}$).
- **Reversible & Tight-Binding Inhibition (global over inhibitor `[I]`):** Competitive, Noncompetitive, Uncompetitive, Mixed-model ($\alpha K_i$), and **Morrison tight-binding $K_i$**:
  $$Y = v_0 \left(1 - \frac{(E_t + I + K_i^{\text{app}}) - \sqrt{(E_t + I + K_i^{\text{app}})^2 - 4 E_t I}}{2 E_t}\right), \qquad K_i^{\text{app}} = K_i \left(1 + \frac{S}{K_m}\right)$$

### 3.4 Exponentials, Lines, Polynomials, Peaks, Waves, Growth & Radiobiology
- **Exponential:** One-/two-/three-phase decay & association, plateau followed by one-phase decay/association, exponential growth ($Y = Y_0 e^{kX}$, doubling time $\ln 2 / k$).
- **Lines & Polynomials:** Straight line (including Deming Model II regression for $X$ and $Y$ error), line through origin, segmental linear ($X_0$ breakpoint), smooth hinge function, semi-log/log-log/probit lines, centered & ordinary 1st–6th order polynomials.
- **Gaussian, Sine & Growth:** Gaussian, Log-Gaussian, Cumulative Gaussian, sum of two Gaussians, Lorentzian, standard and damped sine waves (both with a `Baseline` parameter) and sinc wave, Logistic growth (reporting inflection point $\frac{1}{k}\ln\!\left(\frac{Y_M - Y_0}{Y_0}\right)$), Gompertz growth, Beta growth-and-decline, and Linear–Quadratic radiation cell-survival ($S(D) = e^{-(\alpha D + \beta D^2)}$, `linear_quadratic_survival`; percent/count or "dead" forms are not separate built-in IDs).

---

## 4. User-Defined Equation Language (`crates/fit/src/dsl.rs`)

`curvebook-fit` compiles user-entered equations into a bounded, bounds-checked stack bytecode VM (`MAX_VM_STEPS`, parse-depth and AST-node caps; zero recursion at evaluation time, zero panics on division by zero or domain errors):
- **Multiline intermediate assignments** ending in `Y = ...`,
- **Piecewise / conditional logic:** `IF(condition, true_expr, false_expr)` plus `AND`, `OR`, `NOT`, `<`, `<=`, `>`, `>=`, `=`, `<>`,
- **Dataset-scoped lines:** `<A> Y = ...`, `<B> Y = ...`, `<~A> Y = ...` enabling global mechanistic models across heterogeneous experimental readouts (e.g., total vs. nonspecific binding, wild-type vs. mutant),
- **Column & multi-variable bindings:** Reading auxiliary independent variables from column titles or `MultipleVariables` predictors,
- **Implicit equations** ($Y$ appearing on both sides solved via safeguarded Brent/Newton inner root-finder) and **first-order ODEs** ($dY/dX = g(X, Y; \theta)$ integrated with adaptive Dormand–Prince RK45 and step-count ceiling),
- **Built-in scalar mathematical & special-function library** (`BuiltinFn` in `crates/fit/src/dsl.rs`; overlaps the in-table formula functions in Section 4 of [`data-tables-and-transforms.md`](data-tables-and-transforms.md)): elementary/trig/hyperbolic functions, `erf`/`erfc`, `normcdf`/`norminv`, Bessel (`besselj0`, `besselj1`, `besseljn`, `bessely0`, `bessely1`, `besselyn`, `besseli0`, `besseli1`, `besselk0`, `besselk1`), gamma/beta/digamma (`gamma`, `lngamma`, `beta`, `digamma`/`psi`), and hypergeometric functions (`hyp1f1`, `hyp2f1`, `confluent_u`). Incomplete beta/gamma (`ibeta`, `igamma`, `igammac`) are not exposed in the DSL.

**Mandatory test gate:** `crates/fit/tests/` (`nist_strd.rs`) validates all 27 NIST StRD nonlinear regression benchmarks (`Misra1a`–`Misra1d`, `Chwirut1`–`Chwirut2`, `Lanczos1`–`Lanczos3`, `Gauss1`–`Gauss3`, `DanWood`, `Kirby2`, `Hahn1`, `Nelson`, `MGH09`, `MGH10`, `MGH17`, `BoxBOD`, `Rat42`, `Rat43`, `Bennett5`, `Eckerle4`, `Thurber`, `Roszman1`, `ENSO`) from both Start 1 (far) and Start 2 (near) initial values, asserting LRE $\ge 6.5$ on estimates and residual SD, $\ge 7.0$ on RSS, and $\ge 5.0$ on SEs (documented `Lanczos1` machine-floor exemptions); the suite skips when the external corpus is absent unless `CURVEBOOK_REQUIRE_CORPUS=1` (`cargo xtask corpus`).
