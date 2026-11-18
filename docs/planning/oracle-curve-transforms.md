# Curve transform oracle scope

Follow-up to [#28](https://github.com/HauserGroup/curvebook/issues/28). This draft proposes D1 verification for `curvebook-transform`. Existing tests cover a basic baseline crossing, uniform-grid AUC uncertainty, and exact polynomial differentiation. The added fixtures focus on clipped and irregular-grid calculations and numerical values from smoothing.

## Proposed scope

Add 6–8 independent analytical or numerical cases across AUC, Savitzky–Golay, and LOWESS:

- Use piecewise-linear curves with irregular spacing, a nonzero baseline, and an interior crossing. Compute signed areas by hand or with exact rational arithmetic. For clipped calculations, use bounds inside segments and derive weights for the original observations.
- Compute Savitzky–Golay fitted values on an irregular grid from an independently specified local polynomial least-squares problem. Choose data with residual variation so exact polynomial reproduction alone cannot satisfy the checks.
- Compute LOWESS reference predictions for a small dataset with an influential outlier. Match the documented span, local weights, and robustness iterations before comparing values.

## Acceptance criteria

- [ ] Assert positive, negative, and net AUC and the propagated standard error on an irregular grid. Include clipping inside at least one segment and verify interpolation and weights.
- [ ] Assert numerical Savitzky–Golay values at both endpoints and interior positions.
- [ ] Assert LOWESS fitted values with and without robustness iterations at representative positions, including near the outlier.
- [ ] Include a sheet-level case to verify exclusions and uncertainty reach the calculation from the normal entry point.
- [ ] Record source, inputs, expected outputs, method conventions, and justified tolerances. Exercise the fixtures in offline Rust CI with existing assertions and tolerances intact.
