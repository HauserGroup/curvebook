# Curve transform oracle scope

Follow-up to [#28](https://github.com/HauserGroup/curvebook/issues/28). Existing tests cover a basic baseline crossing, uniform-grid AUC uncertainty, and exact polynomial differentiation. The additional D1 fixtures cover clipped and irregular-grid AUC and numerical values from smoothing.

## Proposed scope

Add 6–8 independent analytical or numerical cases across AUC, Savitzky–Golay, and LOWESS:

- Use piecewise-linear curves with irregular spacing, a nonzero baseline, and an interior crossing. Compute signed areas by hand or with exact rational arithmetic. For clipped calculations, use bounds inside segments and derive weights for the original observations.
- Compute Savitzky–Golay fitted values on an irregular grid from an independently specified local polynomial least-squares problem. Choose data with residual variation so exact polynomial reproduction alone cannot satisfy the checks.
- Compute LOWESS reference predictions for a small dataset with an influential outlier. Match the documented span, local weights, and robustness iterations before comparing values.

## Acceptance criteria

- [x] Assert positive, negative, and net AUC and the propagated standard error over an irregular observed grid. Verify interpolation and original-point weights when fitted-curve AUC is clipped inside its first and last segments.
- [x] Assert numerical Savitzky–Golay values at both endpoints and all interior positions for widths 5, 7, and 9.
- [x] Assert LOWESS fitted values before and after two robustness iterations, including the outlier and its neighbors.
- [x] Include a summarized XY sheet case that excludes a row and converts the remaining SD and sample size values to point standard errors.
- [x] Record source, inputs, expected outputs, method conventions, and justified tolerances. Exercise the fixtures in offline Rust tests with existing assertions and tolerances intact.

The [transform tests](../../crates/transform/tests/d1_curve_transforms.rs) use exact triangle geometry and Bailer weights for signed area and uncertainty over the observed range. The [fitted-curve tests](../../crates/fit/src/result.rs) check interval clipping using the exact integrals of the linear basis functions.

The [smoothing reference generator](../../crates/transform/tests/reference/curve_smoothing.R) uses R 4.5.2. An exact-rational calculation corroborated the local cubic predictions, and an 80-decimal-digit weighted-regression calculation corroborated LOWESS. Both checks agreed with the R values within `1e-12` in response units. The Rust tests use stored values.
