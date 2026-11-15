# Nonlinear Regression & Curve Fitting Parity (`lumencraft-fit`)

> Governing mathematical & solver specification: [`curve-fitting-implementation.md`](curve-fitting-implementation.md) · Roadmap: [`ROADMAP.md`](ROADMAP.md#r4-nonlinear-regression-solver-equation-library--custom-dslode) ([`G4`/`G7`/`G9` mapping](gaps.md))
>
> **Status:** `lumencraft-fit` (`L2`) and `lumencraft-engine` (`L5`) curve-fitting pipelines are implemented and verified (`cargo test -p lumencraft-fit --offline`, including `d1_nist_and_pharma_oracles`, `d1_edge_matrix`, `dsl_fuzz_props`, `nist_strd` across all `27` NIST nonlinear datasets, `prop_fit`, `perf_96_and_fit_parity`, `equations_and_dsl_blocks`, and Wave 5 `pharma_binding_and_dsl`, `0` Clippy warnings; current per-crate counts in [`STATUS.md`](STATUS.md)). The library ships **`117`** built-in `ParametricModel` equations (`47` pharmacology/dose–response/receptor-binding/enzyme-kinetics, `43` curves/polynomials/growth/peaks/waves, and `27` NIST StRD), the `96`-curve 4PL batch benchmark (`lumencraft-cli --release perf --json`) runs in `22.669 ms` fit / `26.25 ms` total (`<= 50 ms`), and Wave 5 `G9` caps custom DSL AST size (`MAX_AST_NODES = 4_096`) with iterative `Clone`/`Debug` on `Expr`.

## Solver, Weighting & Robust Optimization
- [x] Damped Levenberg–Marquardt nonlinear least squares with Moré trust-region column scaling (QR augmented step), plus sub-`50 ms` `fit_independent_datasets` fast path (`22.669 ms` fit / `26.25 ms` total on `96`-curve 4PL)
- [x] Nelder–Mead simplex warm-start / fallback for poorly conditioned starting parameters
- [x] Exact analytical Jacobians for built-in equations + adaptive central-difference Jacobians for custom equations
- [x] Differential weighting: unweighted, $1/Y$, $1/Y^2$, $1/|Y|^k$, $1/X$, $1/X^2$, $1/\text{SD}^2$, and IRLS Poisson deviance regression
- [x] Lorentzian / Cauchy ($t_1$) robust nonlinear regression *(uses OLS initial starting pass before Cauchy IRLS)*
- [x] Automatic ROUT outlier identification & exclusion during nonlinear regression ($Q$ FDR threshold)
- [x] Unified nonlinear regression directly from `MultipleVariables` tables across multi-factor combinations (Prism 11.0/11.1) *(`MultivarExtractionSpec` supports explicit `x_columns`, `y_column`, and `group_columns` for single and Cartesian multi-factor subgroup splitting)*

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
- [x] Residual tables & plots (raw, weighted, studentized; vs. $X$, vs. $\hat{Y}$, Q–Q, column comparison) *(solver records residuals, `populate_nonlinear_result_sheet` emits `ResultTabKind::Residuals` when `emit_residuals_tab: true`, and `lumencraft-chart` compiles `DiagnosticPlotKind::ResidualPlot`)*
- [x] Confidence bands and prediction bands evaluated smoothly in linear and logarithmic axis coordinates *(`build_curve_overlay` uses log-grid spacing when `x_max / x_min >= 100.0` & `x_min > 0` alongside 101-point delta-method confidence & prediction bands)*
- [x] Standard-curve inverse interpolation ("Interpolate unknowns from standard curve" with replicate side-by-side/stacked layouts and propagated CIs) *(verified across linear regression and nonlinear `populate_nonlinear_result_sheet` `ResultTabKind::InterpolatedUnknowns` tab)*
- [x] Conditional range filtering ("Don't fit points when…", including generated X series) and Theoretical Curve Plotting (`Plot a function`) *(`x_range_filter`, `y_range_filter`, and `plot_function_only` implemented on `NonlinearFitSpec` alongside `TransformAnalysisSpec::Simulate`)*

## Built-In Equation Library & User-Defined Equation VM
- [x] **Lines & Polynomials:** Simple linear & Deming Model II regression, line through origin, segmental linear, smooth hinge, semi-log/log-log/probit lines (`semi_log_line`, `log_log_line`, `probit_line`), centered (`centered_poly_1..6`) & standard polynomials
- [x] **Dose–Response — Stimulation & Inhibition:** `log(agonist)` / `log(inhibitor)` vs. response & normalized response (3PL & 4PL variable Hill slope), plus linear `[Agonist]` / `[Inhibitor]` 3PL & 4PL variants
- [x] **Dose–Response — Special (`X = [C]` & `X = log[C]`):** Asymmetrical 5PL, Biphasic, Bell-shaped, Black–Leff Operational Model (Depletion & Partial Agonist), Gaddum/Schild $\text{EC}_{50}$ shift (`pA2` & fixed `SchildSlope=1`), $\text{EC}_{50}$ shift, Allosteric $\text{EC}_{50}$ shift, $\text{EC}_{\text{anything}}$ ($\text{EC}_F$), and Absolute $\text{IC}_{50}$
- [x] **Receptor Binding:** Core saturation, competitive (`Cheng–Prusoff` Fit $K_i$ / Fit $\log \text{IC}_{50}$), dissociation/association kinetics, plus `one_site_total_and_ns_global`, `two_site_total_and_ns_global`, `one_site_specific_ligand_depletion`, `one_site_competition_ligand_depletion`, `allosteric_modulator_saturation`, `homologous_one_site_competition`, and `motulsky_mahan_kinetics` *(all `47` built-in pharmacology/binding/enzyme models verified in `pharma_binding_and_dsl`)*
- [x] **Enzyme Kinetics:** Michaelis–Menten, $k_{\text{cat}}$, allosteric sigmoidal, substrate inhibition, competitive / noncompetitive / uncompetitive / mixed-model inhibition, and Morrison tight-binding $K_i$
- [x] **Exponential & Growth:** One-/two-/three-phase decay & association, plateau-then-exponential, Malthusian exponential growth, Logistic growth (with inflection-point transform), Gompertz growth, exponential plateau, Beta growth-and-decline
- [x] **Peaks, Waves & Radiobiology:** Gaussian, Log-Gaussian, Cumulative Gaussian, two-Gaussian sum, Lorentzian, standard/damped/nonzero-baseline sine & sinc waves, and Linear–Quadratic radiation cell-survival family
- [x] **Custom Equation Bytecode VM (`crates/fit::dsl`):** Multiline algebraic equations, `IF`/`AND`/`OR` piecewise segments, `<A>`..`<Z>`/`<0>`..`<9999>`/`<~A>`/`<B:D>` dataset-conditional selectors, implicit equations ($Y$ on both sides via `brent_root`), first-order RK45 differential equations ($dY/dX$), `[Transforms]` / `[InitialValues]` blocks, and Wave 5 `G9` `MAX_AST_NODES = 4_096` parser cap with iterative `Clone`/`Debug` on `Expr`
