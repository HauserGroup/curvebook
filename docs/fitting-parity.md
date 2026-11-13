# Nonlinear Regression & Curve Fitting Parity (`lumencraft-fit`)

> Governing mathematical & solver specification: [`curve-fitting-implementation.md`](curve-fitting-implementation.md) · Ranked work queue: [`gaps.md`](gaps.md) (`G4`, `G7`, `G9`)

## Solver, Weighting & Robust Optimization
- [ ] Damped Levenberg–Marquardt nonlinear least squares with Moré trust-region column scaling (QR augmented step)
- [ ] Nelder–Mead simplex warm-start / fallback for poorly conditioned starting parameters
- [ ] Exact analytical Jacobians for built-in equations + adaptive central-difference Jacobians for custom equations
- [ ] Differential weighting: unweighted, $1/Y$, $1/Y^2$, $1/|Y|^k$, $1/X$, $1/X^2$, $1/\text{SD}^2$, and IRLS Poisson deviance regression
- [ ] Lorentzian / Cauchy ($t_1$) robust nonlinear regression
- [ ] Automatic ROUT outlier identification & exclusion during nonlinear regression ($Q$ FDR threshold)
- [ ] Unified nonlinear regression directly from `MultipleVariables` tables across multi-factor combinations (Prism 11.0/11.1)

## Parameter Constraints, Global Fitting & Confidence Intervals
- [ ] Fixed constants, data-set column constants, and parameters hooked to Info/Result constants (with cycle guard)
- [ ] Box and inequality bounds ($\theta > c$, $\theta < c$, $a \le \theta \le b$) with `HitConstraint` reporting
- [ ] Global nonlinear regression sharing selected parameters across datasets while keeping others local
- [ ] Symmetrical asymptotic standard errors, t-based CIs, parameter covariance/correlation matrix, Dependency, and Hougaard skewness
- [ ] Asymmetrical **profile-likelihood** confidence intervals (Venzon–Moolgavkar root tracing, graceful `CiBound::Open` on incomplete plateaus)

## Model Comparison, Interpolation & Diagnostics
- [ ] Extra-sum-of-squares *F*-test and likelihood-ratio test comparing nested models or parameter invariance across datasets
- [ ] Small-sample corrected Akaike Information Criterion ($\text{AIC}_c$) and Akaike model probability weights
- [ ] Replicates test for lack of fit, runs test, homoscedasticity test, and residual normality battery (D'Agostino–Pearson, Shapiro–Wilk, AD, KS)
- [ ] Residual tables & plots (raw, weighted, studentized; vs. $X$, vs. $\hat{Y}$, Q–Q, column comparison)
- [ ] Confidence bands and prediction bands evaluated smoothly in linear and logarithmic axis coordinates
- [ ] Standard-curve inverse interpolation ("Interpolate unknowns from standard curve" with replicate side-by-side/stacked layouts and propagated CIs)
- [ ] Conditional range filtering ("Don't fit points when…", including generated X series) and Theoretical Curve Plotting (`Plot a function`)

## Built-In Equation Library & User-Defined Equation VM
- [ ] **Lines & Polynomials:** Simple linear & Deming Model II regression, line through origin, segmental linear, smooth hinge, semi-log/log-log/probit lines, centered & standard 1st–6th order polynomials
- [ ] **Dose–Response — Stimulation & Inhibition:** `log(agonist)` / `log(inhibitor)` vs. response & normalized response (3PL & 4PL variable Hill slope), plus linear `[Agonist]` / `[Inhibitor]` 3PL & 4PL variants
- [ ] **Dose–Response — Special (`X = [C]` & `X = log[C]`):** Asymmetrical 5PL, Biphasic, Bell-shaped, Black–Leff Operational Model (Depletion & Partial Agonist), Gaddum/Schild $\text{EC}_{50}$ shift (`pA2` & fixed `SchildSlope=1`), $\text{EC}_{50}$ shift, Allosteric $\text{EC}_{50}$ shift, $\text{EC}_{\text{anything}}$ ($\text{EC}_F$), and Absolute $\text{IC}_{50}$
- [ ] **Receptor Binding:** One-site & two-site saturation (specific, total+NS, global total/NS, ligand depletion, Hill slope, allosteric), one-site & two-site competitive binding (Cheng–Prusoff Fit $K_i$ & Fit $\log \text{IC}_{50}$, homologous, ligand depletion), dissociation/association kinetics, and Motulsky–Mahan kinetics of competitive binding
- [ ] **Enzyme Kinetics:** Michaelis–Menten, $k_{\text{cat}}$, allosteric sigmoidal, substrate inhibition, competitive / noncompetitive / uncompetitive / mixed-model inhibition, and Morrison tight-binding $K_i$
- [ ] **Exponential & Growth:** One-/two-/three-phase decay & association, plateau-then-exponential, Malthusian exponential growth, Logistic growth (with inflection-point transform), Gompertz growth, exponential plateau, Beta growth-and-decline
- [ ] **Peaks, Waves & Radiobiology:** Gaussian, Log-Gaussian, Cumulative Gaussian, two-Gaussian sum, Lorentzian, standard/damped/nonzero-baseline sine & sinc waves, and Linear–Quadratic radiation cell-survival family
- [ ] **Custom Equation Bytecode VM (`crates/fit::dsl`):** Multiline algebraic equations, `IF`/`AND`/`OR` piecewise segments, `<A>`/`<B>` dataset-conditional equations, implicit equations ($Y$ on both sides), first-order RK45 differential equations ($dY/dX$), parameter transforms, and initial-value rules
