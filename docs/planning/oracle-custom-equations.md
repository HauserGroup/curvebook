# Custom equation oracle scope

Add independent numerical checks for custom implicit equations and fitted ordinary differential equations. Existing tests already exercise pole rejection, two-sided exponential sweeps, and analytic solutions for logistic and oscillatory ODEs. The next cases target scale sensitivity in implicit roots and evaluation across datasets and parameter sets. [Custom equation evaluator](../../crates/fit/src/dsl.rs) · [Existing ODE oracles](../../crates/fit/tests/r46_shapiro_rk45_oracles.rs)

## Proposed scope

Use a monotone implicit equation with a unique root, such as Y + Y³ = A X, and multiply the entire residual by several positive scales. Evaluate the root with an independent high-precision bracketed solve. For a first-order forced ODE with a nonzero initial time, derive the expected trajectory by an integrating factor. Use two noisy datasets with distinct local amplitudes and one shared rate for an independent least-squares fit to the closed form. [SciPy bracketing solver reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.brentq.html) · [SciPy independent ODE solver reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.integrate.solve_ivp.html)

## Acceptance criteria

- [ ] Compare the compiled implicit equation's root at negative, near-zero, and positive predictors against independent stored values. Check the original unscaled residual and check root agreement across the documented scale range.
- [ ] Compare forced ODE predictions before, at, and after the nonzero initial time. Alternate datasets and parameter vectors, revisit previously evaluated times, and compare every result independently to its own closed-form value.
- [ ] Fit the noisy two-dataset fixture through the public fit entry point. Compare the shared rate, both local amplitudes, fitted responses, and residual sum of squares against an independent least-squares solve of the closed form.
- [ ] Record equation text, evaluation order, synthetic observations, reference precision, and justified tolerances. Run the fixtures in the ordinary offline Rust suite. Retain existing root-domain and ODE budget checks.

Built-in model predictions and fits are covered by [#31](https://github.com/HauserGroup/curvebook/pull/31).
