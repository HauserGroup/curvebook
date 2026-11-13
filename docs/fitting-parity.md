# Nonlinear Regression & Curve Fitting Parity (`lumencraft-fit`)

> Governing mathematical & solver specification: [`curve-fitting-implementation.md`](curve-fitting-implementation.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G4`, `G7`, `G9`)
>
> **Status:** `lumencraft-fit` (`L2`) and `lumencraft-engine` (`L5`) curve-fitting pipelines are implemented and verified (`cargo test -p lumencraft-fit --offline`: `55 passed, 0 failed` including `16` lib unit, `18` `d1_nist_and_pharma_oracles`, `16` `dsl_fuzz_props`, `1` `nist_strd` across all 27 NIST nonlinear datasets, and `4` `prop_fit`, plus `e2e_workflows` & CLI Slice 0, `0` Clippy warnings). Sub-bullets lacking full wiring or remaining unimplemented keep `[ ]` with explicit notes below.

## Solver, Weighting & Robust Optimization
- [x] Damped Levenberg–Marquardt nonlinear least squares with Moré trust-region column scaling (QR augmented step)
- [x] Nelder–Mead simplex warm-start / fallback for poorly conditioned starting parameters
- [x] Exact analytical Jacobians for built-in equations + adaptive central-difference Jacobians for custom equations
- [x] Differential weighting: unweighted, $1/Y$, $1/Y^2$, $1/|Y|^k$, $1/X$, $1/X^2$, $1/\text{SD}^2$, and IRLS Poisson deviance regression
- [x] Lorentzian / Cauchy ($t_1$) robust nonlinear regression *(uses OLS initial starting pass before Cauchy IRLS)*
- [x] Automatic ROUT outlier identification & exclusion during nonlinear regression ($Q$ FDR threshold)
- [ ] Unified nonlinear regression directly from `MultipleVariables` tables across multi-factor combinations (Prism 11.0/11.1) *(`extract_from_multivar` supports explicit `[x..., y]` columns and single categorical grouping variable splitting; Cartesian multi-factor subgroup combinations not yet implemented)*

## Parameter Constraints, Global Fitting & Confidence Intervals
- [ ] Fixed constants, data-set column constants, and parameters hooked to Info/Result constants (with cycle guard) *(Fixed `ParameterScope::Constant` and `HookedConstant(HookSource)` with cycle checks are verified end-to-end; per-dataset-column constant rules in `ParameterRule` are not yet modeled independently)*
- [x] Box and inequality bounds ($\theta > c$, $\theta < c$, $a \le \theta \le b$) with `HitConstraint` reporting
- [x] Global nonlinear regression sharing selected parameters across datasets while keeping others local
- [x] Symmetrical asymptotic standard errors, t-based CIs, parameter covariance/correlation matrix, Dependency, and Hougaard skewness *(all computed in `CovarianceDiagnostics` / `FittedParameterSummary`; `cov.ambiguous` currently emits finite Wald bounds rather than `CiBound::Open`, and covariance/correlation matrices remain on `NonlinearFitResult::covariance` rather than an extra `ResultTab`)*
- [x] Asymmetrical **profile-likelihood** confidence intervals (Venzon–Moolgavkar root tracing, graceful `CiBound::Open` on incomplete plateaus) *(note: tangent warm-start extrapolation can overshoot on extremely flat sigmoid tails)*

## Model Comparison, Interpolation & Diagnostics
- [ ] Extra-sum-of-squares *F*-test and likelihood-ratio test comparing nested models or parameter invariance across datasets *(Extra-SS *F*-test verified; standalone likelihood-ratio test `LRT` not implemented in `ModelComparisonSpec`)*
- [x] Small-sample corrected Akaike Information Criterion ($\text{AIC}_c$) and Akaike model probability weights
- [ ] Replicates test for lack of fit, runs test, homoscedasticity test, and residual normality battery (D'Agostino–Pearson, Shapiro–Wilk, AD, KS) *(replicates lack-of-fit, runs test, and residual Shapiro–Wilk & D'Agostino–Pearson $K^2$ verified; homoscedasticity test and residual AD/KS checks in `fit` pending)*
- [ ] Residual tables & plots (raw, weighted, studentized; vs. $X$, vs. $\hat{Y}$, Q–Q, column comparison) *(solver records `residuals` & `weighted_residuals` and `lumencraft-chart` compiles `DiagnosticPlotKind::ResidualPlot`; `populate_nonlinear_result_sheet` does not yet emit a `ResultTabKind::Residuals` tab, and studentized/Q–Q residual tabs remain pending)*
- [ ] Confidence bands and prediction bands evaluated smoothly in linear and logarithmic axis coordinates *(101-point delta-method confidence & prediction bands verified across `x` in `solver.rs` and rendered in `lumencraft-chart`; logarithmic-grid spacing for wide-decade non-log-X curves pending)*
- [x] Standard-curve inverse interpolation ("Interpolate unknowns from standard curve" with replicate side-by-side/stacked layouts and propagated CIs) *(verified across linear regression and nonlinear `populate_nonlinear_result_sheet` `ResultTabKind::InterpolatedUnknowns` tab)*
- [ ] Conditional range filtering ("Don't fit points when…", including generated X series) and Theoretical Curve Plotting (`Plot a function`) *(`TransformAnalysisSpec::Simulate` covers curve simulation in `lumencraft-transform`; `NonlinearFitSpec` conditional exclusion filters & direct `fit` `Plot a function` remain pending)*

## Built-In Equation Library & User-Defined Equation VM
- [ ] **Lines & Polynomials:** Simple linear & Deming Model II regression, line through origin, segmental linear, smooth hinge, semi-log/log-log/probit lines, centered & standard 1st–6th order polynomials *(Simple OLS, Deming Model II, origin line, segmental linear, smooth hinge, 2nd–6th polynomials, power series, and logarithmic curve verified; centered polynomials and named semi-log/log-log/probit lines pending)*
- [x] **Dose–Response — Stimulation & Inhibition:** `log(agonist)` / `log(inhibitor)` vs. response & normalized response (3PL & 4PL variable Hill slope), plus linear `[Agonist]` / `[Inhibitor]` 3PL & 4PL variants
- [x] **Dose–Response — Special (`X = [C]` & `X = log[C]`):** Asymmetrical 5PL, Biphasic, Bell-shaped, Black–Leff Operational Model (Depletion & Partial Agonist), Gaddum/Schild $\text{EC}_{50}$ shift (`pA2` & fixed `SchildSlope=1`), $\text{EC}_{50}$ shift, Allosteric $\text{EC}_{50}$ shift, $\text{EC}_{\text{anything}}$ ($\text{EC}_F$), and Absolute $\text{IC}_{50}$
- [ ] **Receptor Binding:** One-site & two-site saturation (specific, total+NS, global total/NS, ligand depletion, Hill slope, allosteric), one-site & two-site competitive binding (Cheng–Prusoff Fit $K_i$ & Fit $\log \text{IC}_{50}$, homologous, ligand depletion), dissociation/association kinetics, and Motulsky–Mahan kinetics of competitive binding *(11 core saturation, Cheng–Prusoff Fit $K_i$/$\log \text{IC}_{50}$, and association/dissociation kinetic models verified in `pharma.rs` & `d1_nist_and_pharma_oracles`; global total+NS, ligand-depletion, allosteric saturation, homologous competition, and Motulsky–Mahan models pending)*
- [x] **Enzyme Kinetics:** Michaelis–Menten, $k_{\text{cat}}$, allosteric sigmoidal, substrate inhibition, competitive / noncompetitive / uncompetitive / mixed-model inhibition, and Morrison tight-binding $K_i$
- [x] **Exponential & Growth:** One-/two-/three-phase decay & association, plateau-then-exponential, Malthusian exponential growth, Logistic growth (with inflection-point transform), Gompertz growth, exponential plateau, Beta growth-and-decline
- [x] **Peaks, Waves & Radiobiology:** Gaussian, Log-Gaussian, Cumulative Gaussian, two-Gaussian sum, Lorentzian, standard/damped/nonzero-baseline sine & sinc waves, and Linear–Quadratic radiation cell-survival family
- [ ] **Custom Equation Bytecode VM (`crates/fit::dsl`):** Multiline algebraic equations, `IF`/`AND`/`OR` piecewise segments, `<A>`/`<B>` dataset-conditional equations, implicit equations ($Y$ on both sides), first-order RK45 differential equations ($dY/dX$), parameter transforms, and initial-value rules *(algebraic formulas, `IF`/`AND`/`OR`, `<A>`/`<B>` filters, implicit `brent_root`, and Dormand–Prince RK45 ODEs verified via `16` `dsl_fuzz_props` + unit tests; dedicated parameter-transform / `InitialValues` rule blocks & `<A>` local-variable scoping edge case remain partial)*
