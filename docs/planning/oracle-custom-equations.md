# Custom equation oracle scope

Add independent numerical checks for custom implicit equations and fitted ordinary differential equations. Existing tests already exercise pole rejection, two-sided exponential sweeps, and analytic solutions for logistic and oscillatory ODEs. The next cases target scale sensitivity in implicit roots and evaluation across datasets and parameter sets. [Custom equation evaluator](../../crates/fit/src/dsl.rs) · [Existing ODE oracles](../../crates/fit/tests/r46_shapiro_rk45_oracles.rs)

## Proposed scope

Use a monotone implicit equation with a unique root, such as Y + Y³ = A X, and multiply the entire residual by several positive scales. Evaluate the root with an independent high-precision bracketed solve. For a first-order forced ODE with a nonzero initial time, derive the expected trajectory by an integrating factor. Use two noisy datasets with distinct local amplitudes and one shared rate for an independent least-squares fit to the closed form. [SciPy bracketing solver reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.brentq.html) · [SciPy independent ODE solver reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.integrate.solve_ivp.html)

## Acceptance criteria

- [x] Compare the compiled implicit equation's root at negative, near-zero, and positive predictors against independent stored values. Check the original unscaled residual and check root agreement across the documented scale range.
- [x] Compare forced ODE predictions before, at, and after the nonzero initial time. Alternate datasets and parameter vectors, revisit previously evaluated times, and compare every result independently to its own closed-form value.
- [x] Fit the noisy two-dataset fixture through the public fit entry point. Compare the shared rate, both local amplitudes, fitted responses, and residual sum of squares against an independent least-squares solve of the closed form.
- [x] Record equation text, evaluation order, synthetic observations, reference precision, and justified tolerances. Run the fixtures in the ordinary offline Rust suite. Retain existing root-domain and ODE budget checks.

Built-in model predictions and fits are covered by [#31](https://github.com/HauserGroup/curvebook/pull/31).

## Fixtures, inputs, and tolerances

All new offline checks live in the [custom equation oracle tests](../../crates/fit/tests/d1_custom_equations.rs), with stored IEEE-754 binary64 constants generated independently by the [Python reference generator](../../crates/fit/tests/reference/custom_equations.py).

### 1. Monotone implicit root scale sensitivity

- **Equation text:**
  - Canonical form: `Y + Y^3 = A * X`
  - Literal scaled residuals: `<s> * (Y + Y^3 - A * X) = 0`
  - Parameterized scaled residual: `S * (Y + Y^3 - A * X) = 0`
- **Inputs & documented scale range:** `A = 2.5`; predictors `X = [-3.0, -0.4, -1e-9, 0.0, 1e-9, 0.4, 3.0]` spanning negative, near-zero, zero, and positive values; literal positive scales `1e-200`, `1e-100`, `1e-50`, `1e-24`, `1e-12`, `1e-6`, `0.000125`, `1`, `7.5`, `1e6`, `1e12`, `1e24`, `1e50`, `1e100`, `1e200` (400 orders of magnitude), plus parameterized `S in {1e-100, 1.0, 1e100}`. Scales in `[1e-200, 1e200]` keep `s * |F(y)|` inside IEEE-754 normal binary64 range across all bracketing probes and local derivative perturbations (`delta >= 1e-7 * |y|`) for these predictors.
- **Justified tolerances:** `eval_implicit` refines sign brackets with Brent's method at `rel_tol = 1e-13` and `abs_tol ~ 1e-14 * bracket_scale`. Root comparisons against independent bracketed solutions and across scales use `abs_tol = 1e-15`, `rel_tol = 1e-12`; original unscaled residual `|y + y^3 - A * X|` is bounded by `max(1e-15, 1e-11 * (1 + 3 * y^2) * |y|)`.

### 2. Forced ODE with nonzero initial time (`t0 = 1.5`)

- **Equation text:**
  ```text
  Y(1.5) = Y0
  <A> dY/dX = -k * Y + q * cos(w * X)
  <B> dY/dX = -k * Y + q * exp(-r * (X - 1.5))
  ```
- **Closed form via integrating factor $\mu(X) = e^{k X}$:**
  - Dataset `<A>` (`ds_idx = 0`): $Y_A(X) = Y_0 e^{-k(X - t_0)} + \frac{q}{k^2 + w^2}\big[k\cos(wX) + w\sin(wX) - e^{-k(X - t_0)}(k\cos(w t_0) + w\sin(w t_0))\big]$.
  - Dataset `<B>` (`ds_idx = 1`): $Y_B(X) = Y_0 e^{-k(X - t_0)} + \frac{q}{k - r}\big[e^{-r(X - t_0)} - e^{-k(X - t_0)}\big]$.
- **Parameter vectors `[k, q, w, r, Y0]` & times:**
  - `P0 = [0.6, 2.5, 1.8, 1.1, 3.0]`
  - `P1 = [1.25, -1.5, 2.5, 0.4, -2.0]`
  - `P2 = [0.35, 1.2, 0.9, 0.85, 4.5]`
  - Times `t = [-0.5, 0.25, 1.0, 1.5, 2.25, 3.5, 5.0]` (indices `0..=6`: `0..=2` before `t0`, `3` at `t0`, `4..=6` after `t0`).
- **Evaluation order:** Six `(parameter set, dataset)` trajectories exceed the four-slot `ODE_TRAJECTORY_CACHE` capacity. The test runs an explicit 51-step interleaved schedule `(param_idx, ds_idx, time_idx)`:
  1. Phase 1 (`P0`, `DS0` non-monotone): time indices `5, 1, 3, 6, 0` (`3.5, 0.25, 1.5, 5.0, -0.5`).
  2. Phase 2 (`P0`, `DS1` dataset switch): time indices `6, 0, 3, 4` (`5.0, -0.5, 1.5, 2.25`).
  3. Phase 3 (`P1`, `DS0` and `DS1` parameter switch): `(1,0,4), (1,0,2), (1,0,5)` then `(1,1,1), (1,1,5), (1,1,2), (1,1,3)`.
  4. Phase 4 (revisit `P0`, `DS0` while warm-cached): unvisited interior points `4, 2`, then warm-cache revisits `5, 1, 3` (verified bitwise identical to Phase 1).
  5. Phase 5 (`P2`, `DS0` and `DS1` cache eviction): `(2,0,[6,0,4,2,3])` then `(2,1,[0,6,1,5])`.
  6. Phase 6 (remaining grid & post-eviction revisits): `(0,1,[1,2,5,6,0])`, `(1,0,[0,1,3,6,4])`, `(1,1,[0,4,6,1])`, `(2,0,[1,5])`, `(2,1,[2,3,4])`, `(0,0,[6,0])`.
- **Justified tolerances:** At `t == t0 = 1.5`, `abs_tol = 1e-14`. Everywhere else, Dormand–Prince RK45 runs with `rtol = 1e-8`, `atol = 1e-10`; across `|t - t0| <= 3.5` and backward amplification bounded by `exp(1.25 * 2) ~ 12.2`, predictions match closed form within `abs_tol = 5e-7`, `rel_tol = 5e-7`.

### 3. Noisy two-dataset shared-rate ODE fit

- **Equation text:**
  ```text
  Y0 = A
  dY/dX = -k * Y
  ```
  with `k` configured as `SharedGlobal` (`initial_override = 0.3`, `lower_bound = 0.0`) and `A` configured as `Local` (`initial_override = 50.0`), fitted through both `fit_nonlinear` and typed `DataSheet` + `execute_analysis` (`custom_dsl`).
- **Synthetic observations:**
  - `X   = [0.0, 0.5, 1.0, 2.0, 3.5, 5.0, 7.5, 10.0]`
  - `DS1 = [79.1, 68.4, 55.6, 40.7, 22.9, 14.6, 5.4, 2.8]`
  - `DS2 = [35.8, 28.2, 25.3, 16.9, 10.8, 5.7, 2.8, 0.9]`
- **Justified tolerances:** Finite-difference LM Jacobians (`h = 1e-6 * max(|theta|, 1)`) over RK45 trajectories (`rtol = 1e-8`, `atol = 1e-10`) yield `abs_tol = 1e-5` on estimates (`k`, `A [DS1]`, `A [DS2]`), asymptotic SEs, and all 16 fitted responses, and `abs_tol = 1e-6` on total and per-dataset RSS.

### 4. Retained checks & independent corroboration

- Existing pole-rejection, domain-guard, and ODE step-budget tests in `crates/fit/tests/` remain unchanged, and `d1_custom_equations.rs` retains explicit domain (`1 / (Y - A) = 0`, `Y^2 + 1 = 0`, `NaN` predictor) and stiff ODE budget (`OdeIntegrationFailed`) checks.
- **Reference precision & corroboration:** The [Python generator](../../crates/fit/tests/reference/custom_equations.py) uses `mpmath` 1.3.0 (`bracketed_brent_solve`, Cardano cubic cross-check, integrating-factor closed forms, and normal-equations `findroot` to `1e-60`) and emits identical binary64 constants at 80 and 120 decimal digits. R 4.5.2 independently corroborated all implicit roots (`stats::uniroot`), forced ODE closed forms and classical RK4 trajectories, and the shared-rate nonlinear least-squares estimates, SEs, and RSS (`stats::nls`).
