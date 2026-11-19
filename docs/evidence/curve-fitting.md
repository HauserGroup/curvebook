# Nonlinear Regression & Curve Fitting Evidence (`curvebook-fit`)

> Governing mathematical & solver specification: [`curve-fitting-implementation.md`](../implementation/curve-fitting-implementation.md) · Roadmap: [`ROADMAP.md`](../history/roadmap/r4.md#r4-nonlinear-regression-solver-equation-library--custom-dslode) ([`G4`/`G7`/`G9` mapping](../history/legacy-ids-and-defects.md))
>
> **Status:** `curvebook-fit` (`L2`) and `curvebook-engine` (`L5`) curve-fitting pipelines are implemented and verified (`cargo test -p curvebook-fit --offline`, including `d1_nist_and_pharma_oracles`, `d1_edge_matrix`, `dsl_fuzz_props`, `nist_strd` across all `27` NIST nonlinear datasets, `prop_fit`, `perf_96_and_fit_parity`, `equations_and_dsl_blocks`, and Wave 5 `pharma_binding_and_dsl`, `0` Clippy warnings; current per-crate and built-in `ParametricModel` counts in [`STATUS.md`](../STATUS.md)). The `96`-curve 4PL batch benchmark (`curvebook-cli --release perf --json`) runs in `22.669 ms` fit / `26.25 ms` total (`<= 50 ms`), and Wave 5 `G9` caps custom DSL AST size (`MAX_AST_NODES = 4_096`) with iterative `Clone`/`Debug` on `Expr`.

## Solver, Weighting & Robust Optimization
- [x] Damped Levenberg–Marquardt nonlinear least squares with Moré trust-region column scaling (QR augmented step), plus sub-`50 ms` `fit_independent_datasets` fast path (`22.669 ms` fit / `26.25 ms` total on `96`-curve 4PL)
- [x] Nelder–Mead simplex warm-start / fallback for poorly conditioned starting parameters
- [x] Exact analytical Jacobians for built-in equations + adaptive central-difference Jacobians for custom equations
- [x] Differential weighting: unweighted, $1/Y$, $1/Y^2$, $1/|Y|^k$, $1/X$, $1/X^2$, $1/\text{SD}^2$, and IRLS Poisson deviance regression
- [x] Lorentzian / Cauchy ($t_1$) robust nonlinear regression *(uses OLS initial starting pass before Cauchy IRLS)*
- [x] Automatic ROUT outlier identification & exclusion during nonlinear regression ($Q$ FDR threshold)
- [x] Unified nonlinear regression directly from `MultipleVariables` tables across multi-factor combinations *(`MultivarExtractionSpec` supports explicit `x_columns`, `y_column`, and `group_columns` for single and Cartesian multi-factor subgroup splitting)*

## Parameter Constraints, Global Fitting & Confidence Intervals
- [x] Fixed constants, data-set column constants, and parameters hooked to Info/Result constants (with cycle guard) *(`ParameterScope::{Constant, PerDatasetConstant, ColumnTitleConstant}` and `HookedConstant(HookSource)` verified end-to-end with cycle detection)*
- [x] Box and inequality bounds ($\theta > c$, $\theta < c$, $a \le \theta \le b$) with `HitConstraint` reporting
- [x] Global nonlinear regression sharing selected parameters across datasets while keeping others local
- [x] Symmetrical asymptotic standard errors, t-based CIs, parameter covariance/correlation matrix, Dependency, and Hougaard skewness *(computed in `CovarianceDiagnostics` / `FittedParameterSummary`; `cov.ambiguous` emits `CiBound::Open`, and `ResultTabKind::CovarianceMatrix` is emitted when `emit_covariance_tab` is enabled)*
- [x] Asymmetrical **profile-likelihood** confidence intervals (Venzon–Moolgavkar root tracing, graceful `CiBound::Open` on incomplete plateaus) *(note: tangent warm-start extrapolation can overshoot on extremely flat sigmoid tails)*

## Model Comparison, Interpolation & Diagnostics
- [x] Extra-sum-of-squares *F*-test and likelihood-ratio test comparing nested models or parameter invariance across datasets *(`ModelComparisonSpec::LikelihoodRatioTest` verified alongside Extra-SS *F*-test)*
- [x] Small-sample corrected Akaike Information Criterion ($\text{AIC}_c$) and Akaike model probability weights
- [x] Replicates test for lack of fit, runs test, homoscedasticity test, and residual normality battery (D'Agostino–Pearson, Shapiro–Wilk, AD, KS) *(`homoscedasticity_spearman_test`, `anderson_darling_test`, and `lilliefors_ks_test` verified alongside lack-of-fit, runs test, Shapiro–Wilk, and D'Agostino–Pearson $K^2$)*
- [x] Residual tables & plots (raw, weighted, studentized; vs. $X$, vs. $\hat{Y}$, Q–Q, column comparison) *(solver records residuals, `populate_nonlinear_result_sheet` emits `ResultTabKind::Residuals` when `emit_residuals_tab: true`, and `curvebook-chart` compiles `DiagnosticPlotKind::ResidualPlot`)*
- [x] Confidence bands and prediction bands evaluated smoothly in linear and logarithmic axis coordinates *(`build_curve_overlay` uses log-grid spacing when `x_max / x_min >= 100.0` & `x_min > 0` alongside 101-point delta-method confidence & prediction bands)*
- [x] Standard-curve inverse interpolation ("Interpolate unknowns from standard curve" with replicate side-by-side/stacked layouts and propagated CIs) *(verified across linear regression and nonlinear `populate_nonlinear_result_sheet` `ResultTabKind::InterpolatedUnknowns` tab)*
- [x] Conditional range filtering ("Don't fit points when…", including generated X series) and Theoretical Curve Plotting (`Plot a function`) *(`x_range_filter`, `y_range_filter`, and `plot_function_only` implemented on `NonlinearFitSpec` alongside `TransformAnalysisSpec::Simulate`)*

## Built-In Equation Library & User-Defined Equation VM
- [x] **Lines & Polynomials:** Simple linear & Deming Model II regression, line through origin, segmental linear, smooth hinge, semi-log/log-log/probit lines (`semi_log_line`, `log_log_line`, `probit_line`), centered (`centered_poly_1..6`) & standard polynomials
- [x] **Dose–Response — Stimulation & Inhibition:** `log(agonist)` / `log(inhibitor)` vs. response & normalized response (3PL & 4PL variable Hill slope), plus linear `[Agonist]` / `[Inhibitor]` unnormalized 3PL & 4PL variants
- [x] **Dose–Response — Special (`X = log[C]`):** Asymmetrical 5PL, Biphasic, Bell-shaped, Black–Leff Operational Model (Partial Agonist), Gaddum/Schild $\text{EC}_{50}$ shift (`pA2` & `SchildSlope`), Allosteric $\text{EC}_{50}$ shift, $\text{EC}_{\text{anything}}$ ($\text{EC}_F$), and Absolute $\text{IC}_{50}$
- [x] **Receptor Binding:** Core saturation, competitive (`Cheng–Prusoff` Fit $K_i$ / One-site Fit $\log \text{IC}_{50}$), dissociation/association kinetics, plus `one_site_total_and_ns_global`, `two_site_total_and_ns_global`, `one_site_specific_ligand_depletion`, `one_site_competition_ligand_depletion`, `allosteric_modulator_saturation`, `homologous_one_site_competition`, and `motulsky_mahan_kinetics` *(built-in pharmacology/binding/enzyme models verified in `pharma_binding_and_dsl`; live count in [`STATUS.md`](../STATUS.md))*
- [x] **Enzyme Kinetics:** Michaelis–Menten, $k_{\text{cat}}$, allosteric sigmoidal, substrate inhibition, competitive / noncompetitive / uncompetitive / mixed-model inhibition, and Morrison tight-binding $K_i$
- [x] **Exponential & Growth:** One-/two-/three-phase decay, one-phase association, plateau-then-exponential decay/association, Malthusian exponential growth, Logistic growth (with inflection-point transform), Gompertz growth, exponential plateau, Beta growth-and-decline
- [x] **Peaks, Waves & Radiobiology:** Gaussian, Log-Gaussian, Cumulative Gaussian, two-Gaussian sum, Lorentzian, Weibull peak, baseline-parameterized sine / damped sine & sinc waves, and Linear–Quadratic surviving-fraction cell-survival (`linear_quadratic_survival`)
- [x] **Custom Equation Bytecode VM (`crates/fit/src/dsl.rs`):** Multiline algebraic equations, `IF`/`AND`/`OR` piecewise segments, `<A>`..`<Z>`/`<0>`..`<9999>`/`<~A>`/`<B:D>` dataset-conditional selectors, implicit equations ($Y$ on both sides via `brent_root`), first-order RK45 differential equations ($dY/dX$), `[Transforms]` / `[InitialValues]` blocks, and Wave 5 `G9` `MAX_AST_NODES = 4_096` parser cap with iterative `Clone`/`Debug` on `Expr`

---

## Planned Curve-Fitting & Regression Completeness Scope

Target specifications detailed in [`curve-fitting-implementation.md`](../implementation/curve-fitting-implementation.md) §§1.4, 2.5, 3.5, 4.1–4.2, 5, and 6 (partially scheduled under [`R13.1`](../ROADMAP.md#r131-domain-restricted-regression-fit-identifiability-replicate-unknown-interpolation--outlier-overlays)):

### Solver, Weighting, Diagnostics & Comparison Extensions
- [ ] Iteratively reweighted curve-predicted weighting (`1 / Y_curve^2`, `1 / |Y_curve|`, `1 / |Y_curve|^k`) from an initial unweighted pass, matching `(Mean, SD, n)` sufficient statistics (`§1.4`)
- [ ] Three-way parameter identifiability (`IdentifyUnstableParameters` greedy Hessian $\kappa(H) > 10/\sqrt{\varepsilon_{\text{mach}}}$ reduction, `IdentifyAmbiguousFits` dependency $> 0.9999$, and `ReportAll`), midpoint extrapolation warnings, and SE suppression under profile-likelihood CI mode (`§2.5`, `R13.1`)
- [ ] ROUT `"Identify & count only"` mode alongside elimination, analyzable `"Cleaned data"` output tab, union-of-outliers synchronization when comparing models, and graph outlier marker overlays (`§1.4`, `R13.1`)
- [ ] Replicate independence toggle (`IndividualReplicates` vs. `RowMeansOnly` on replicate and `Mean/SD/N` tables) and `Quick` / `Medium` / `Strict` convergence presets (`§1.4`)
- [ ] Full 4-question `ModelComparisonSpec` (two equations, selected parameter invariance across datasets, single global curve vs. separate curves, parameter vs. theoretical value) plus ambiguous/flagged pre-check (`§2.5`)
- [ ] Parameter summary output table + linked secondary bar/XY graph, configurable output significant digits, and 6-rule `FitFlagRules` (`R²`, `N`, dependency, normality, runs/lack-of-fit, outlier count) (`§2.5`)
- [ ] Multi-response `mv_y_columns` variable assignment on `MultipleVariables` nonlinear regression (`M` responses $\times G$ groups) (`§1.4`)
- [ ] Dual `Interpolated Mean X` + `Interpolated Replicates` standard-curve tabs (`R13.1`) with analyzable output for $10^X$ / $e^X$ antilog concentration transforms (`§2.5`)

### Built-In Equation Library & Custom DSL Completeness
- [ ] Linear-concentration (`X = [C]`) normalized 3PL/4PL stimulation & inhibition models (`agonist_vs_response_normalized[_var]`, `inhibitor_vs_response_normalized[_var]`) (`§3.5`)
- [ ] Linear-concentration (`X = [C]`) Special Dose–Response counterparts plus **Operational Model — Receptor Depletion** (`log` & `[C]`), **Gaddum/Schild (`SchildSlope = 1`)** (`log` & `[C]`), and **$\text{EC}_{50}$ potency-ratio shift** (`log` & `[C]`) (`§3.5`)
- [ ] Receptor binding additions: **One-site total saturation with ligand depletion** (Swillens 1995), **Two-site Fit $\log \text{IC}_{50}$**, and **Allosteric modulator radioligand titration** (`§3.5`)
- [ ] Curve & radiobiology additions: **`two_phase_association`**, **`two_lines_intersection`** (crossing point with SE/profile CI), Area-parameterized Gaussian & Lognormal peaks, 3-parameter zero-baseline `standard_sine_wave`, `log_exponential_growth`, 5 remaining Linear–Quadratic survival/mortality forms (`percent`/`number` surviving & `fraction`/`percent`/`number` dead), **`boltzmann_sigmoid`**, and 4-parameter **`power_series_two_term`** (`§3.5`)
- [ ] Custom DSL distribution & special functions (`ibeta`, `igamma`, `igammac`, `binomial`, `z`/`norm`/`t`/`f`/`chi` CDF-quantile-PDF family, `fact`, `logit`, `probit`, `perc`, `deg`, `rad`, `sqr`, `is_defined`, integer-order `besseli`/`besselk`, and special-function aliases) (`§4.1`)
- [ ] Reusable user-defined `[InitialValues]` rule multipliers (`YMIN`/`YMID`/`YMAX`, `XatYmid`, `SIGN(YatXmax - YatXmin)`, column-title mean/log-mean, and `SlopeAtXmin`/`Xmid`/`Xmax` + reciprocals) (`§4.2`)

### Linear, Deming, Logistic, Multivariable GLM & Spline/LOWESS Extensions
- [ ] Simple linear regression arbitrary fixed-point `(X0, Y0)` constraint, domain-restricted `[X_min, X_max]` fitting (`R13.1`), and **Standard Addition Method** negated $X$-intercept Fieller CI + line extension to $Y = 0$ (`§5.1`)
- [ ] Persist unequal Deming measurement-error $(\text{SD}_X, \text{SD}_Y)$ / $\lambda = \sigma_X^2 / \sigma_Y^2$ on `LinearRegressionSpec` with duplicate-pair repeatability helper (`§5.2`)
- [ ] Standalone XY **`SimpleLogisticRegression`** (`X at 50%`, odds ratios + profile CIs, LRT & Wald tests, Tjur & Cox–Snell $R^2$, ROC + AUC, automatic `(X, blank Y)` `Row Classification` prediction, sigmoid curve overlay, and separation detection) (`§5.3`)
- [ ] Multivariable `GlmSpec` extensions: 2-/3-way interactions, polynomial terms, `include_intercept: bool`, nested GLM comparison (*F* / LRT / $\Delta\text{AIC}_c$), linear weights & Poisson overdispersion, Cook's distance / leverage / DFBETAs / residual plots, out-of-sample row prediction, and configurable logistic cutoff (`§5.4`)
- [ ] Dedicated XY **`FitSplineLowess`** analysis (Natural Cubic Spline, Akima subspline, knot-controlled Smoothing Spline, Coarse/Medium/Fine LOWESS with $\ge 4N$ segments, log-aware point-to-point curve, XY coordinates tab, and nonparametric standard-curve unknown interpolation) plus multi-dataset Info-hooked theoretical distribution plotting (`§6.1`–`§6.2`)
