# Non-NIST fitting oracle scope

Follow-up to [#28](https://github.com/HauserGroup/curvebook/issues/28). This work adds D1 reference tests for built-in equations outside the NIST nonlinear benchmark catalog.

## Proposed scope

Map the non-NIST catalog by model and verified output. Distinguish direct comparisons against independent values from solver recovery, finite-difference checks against the production evaluator, and catalog smoke tests. Use the inventory to select 8–10 new cases across curve and pharmacology families. Start with underrepresented peak, segmented, binding, or enzyme models as the inventory warrants, and include noisy weighted and constrained fits.

Use published equations for expected predictions and a versioned independent solver or closed-form calculation for fitted results. Match parameterization, weight convention, constraints, and the definition of reported uncertainty. Record the synthetic data and perturbations so a second implementation can reproduce the inputs.

## Acceptance criteria

- [x] List each selected model and the exact outputs its reference verifies.
- [x] Assert independent predictions, fit estimates, and residual sums of squares. Check the weighted Gaussian objective separately from the ordinary RSS.
- [x] Verify standard errors for the noisy weighted Gaussian and the free coefficients in the fixed-knot fit.
- [x] Exercise both fits through typed XY data and the public analysis entry point. Check the reported parameters, predicted responses, and goodness-of-fit cells.
- [x] Record source, inputs, expected outputs, and justified tolerances. Run the fixtures in the offline Rust suite with existing assertions and tolerances intact.

## Catalog review and selected coverage

The review started with the canonical [curve](../../crates/fit/src/library/curves.rs) and [pharmacology](../../crates/fit/src/library/pharma.rs) IDs and inspected the corresponding fit tests. Existing benchmark references cover exponential and growth cases, dose response, binding, and enzyme inhibition. The [library oracles](../../crates/fit/tests/r46_library_oracles.rs) and [binding tests](../../crates/fit/tests/pharma_binding_and_dsl.rs) also contain gradient checks against the production evaluator, solver recovery cases, and catalog registration checks. Those checks establish different properties from independent predictions and estimates. Shared implementations, aliases, and indirect calls make a textual ID search insufficient for an exhaustive coverage claim.

| Catalog models selected | New assertions | Reference |
| --- | --- | --- |
| `log_gaussian` | Peak height and predictions at symmetric logarithmic offsets | Gaussian at integer standardized offsets, Limpert et al. (2001) |
| `weibull_peak` | Scaled density at zero, half scale, scale, and twice scale | Weibull (1951) |
| `damped_sine_wave` | Sign, baseline, and damping at quarter-cycle steps | Sinusoid with exact half-amplitude decay per time unit |
| `hinge_smooth_hyperbolic` | Predictions at the join and on each side | Watts & Bacon (1974) with an exact 3–4–5 triangle |
| `one_site_specific_hill` | Zero binding, half occupancy, and off-midpoint responses | Hill (1910) occupancy odds |
| `kcat_enzyme_kinetics` | Zero rate, saturation fractions, and enzyme concentration scaling | Michaelis–Menten rate with `Vmax = kcat * Et` |
| `substrate_inhibition` | Peak, paired responses on either side, and high-substrate inhibition | Haldane (1930), Segel (1975) |
| `morrison_tight_binding_ki` | Residual activity at known independently constructed occupancies | Morrison (1969) mass balance solved for total inhibitor |
| `gaussian_baseline` | Four parameter estimates and SEs, every fitted response, residual degrees of freedom, ordinary RSS, and the `1/X²` objective | High-precision weighted stationarity equations and Gauss–Newton covariance |
| `segmental_linear` | Three free estimates and SEs, fixed knot and omitted knot SE, every fitted response, residual degrees of freedom, and RSS | Independent linear least squares with a fixed knot, following Hudson (1966) |

The [Rust tests](../../crates/fit/tests/d1_non_nist_fitting.rs) contain the analytical reductions and stored fit values. The [generator](../../crates/fit/tests/reference/non_nist_fits.py) uses mpmath 1.3.0 and produced identical binary64 results at 80 and 120 decimal digits. R 4.5.2 independently corroborated the weighted Gaussian fit through `stats::nls` and the fixed-knot fit through `stats::lm` with a slope-change basis.
