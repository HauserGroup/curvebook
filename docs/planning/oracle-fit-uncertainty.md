# Fit uncertainty oracle scope

Verify profile-deviance intervals for Poisson nonlinear fitting and pointwise uncertainty for a globally shared weighted fit. Existing fitting oracles exercise Gaussian profile behavior and Poisson model comparison. These cases target the numerical values of interval endpoints and the use of shared covariance in reported bands. [Solver](../../crates/fit/src/solver.rs) · [Existing solver oracles](../../crates/fit/tests/r46_solver_oracles.rs)

## Proposed scope

Use two noisy synthetic fixtures. For positive fitted Poisson means with zero counts among the observations, profile a two-parameter log-linear mean. Solve for the nuisance intercept at each fixed slope and locate the profile roots independently. For a Gaussian fit with a shared slope and two dataset-specific intercepts, solve the weighted design matrix independently using weights of 1/|X| at nonzero X. [Venzon and Moolgavkar on profile intervals](https://doi.org/10.2307/2347496)

## Acceptance criteria

- [ ] Compare the Poisson estimates, fitted means, and minimized deviance to independent values. Assert both finite profile endpoints and verify that their reoptimized deviance increments match the one-degree-of-freedom chi-square cutoff for the stated confidence level.
- [ ] Compare shared and local estimates, all entries of the free-parameter covariance matrix, residual degrees of freedom, and weighted residual sum of squares against the independent weighted least-squares solution. Include unequal dataset sizes and nonzero residuals.
- [ ] Check pointwise confidence and single-observation prediction limits at multiple X values in both overlays. Calculate the mean-response variance from the independent covariance and include the documented future-observation variance for the prediction limits.
- [ ] Exercise the public fit entry point and verify representative reported interval or overlay values through the typed analysis path. Record the independent calculation, fixture values, weight and dispersion conventions, and justified per-output tolerances. Run the fixtures offline with existing tolerances intact.

The scope covers the nonlinear solver and shared weighted-fit inference. The stats crate's Poisson GLM and Cox reporting cases are tracked in [#33](https://github.com/HauserGroup/curvebook/pull/33). The weighted Gaussian estimates and fixed-knot checks are covered by [#31](https://github.com/HauserGroup/curvebook/pull/31).
