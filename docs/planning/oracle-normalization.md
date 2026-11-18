# Normalization and baseline oracle scope

Add numerical fixtures for normalization, baseline removal, and uncertainty propagation in mathematical transforms. The current suite includes basic transform values, a finite-difference propagation check, and a negative-divisor bounds regression. New fixtures will independently check reported anchors, rowwise arithmetic, and uncertainty on summarized measurements. [Transform specifications](../implementation/data-tables-and-transforms.md#5-derived-transforms-cratestransform) · [Existing transform tests](../../crates/transform/tests/unit_transform.rs)

## Proposed scope

Use small hand-computable sheets with unequal dataset ranges, inactive observations, and summarized SD or SEM. Derive expected values using exact arithmetic where practical and evaluate nonlinear endpoints and derivatives independently. Treat the two uncertainty modes according to their documented output layouts. [JCGM guide to uncertainty propagation](https://doi.org/10.59161/JCGM100-2008E)

## Acceptance criteria

- [ ] Compare per-dataset and pooled anchors and every transformed value for both fraction and percentage output. Include an excluded extreme and a blank row, assert the emitted anchor values, and check that coincident anchors produce the documented typed error.
- [ ] Use distinct leading and trailing windows to derive the baseline and interpolate it over row positions. Verify subtraction and ratio or percent change, including a zero-denominator row and a missing or excluded anchor observation.
- [ ] Verify transformed centers and analytical first-order spreads for summarized SD or SEM. Independently calculate both endpoints for a nonlinear monotone transform and check lower and upper ordering for a decreasing transform. Include an out-of-domain case and assert the documented missing-value or warning behavior.
- [ ] Exercise the public transform functions and check one typed analysis result. Record source inputs, expected cells and scalars, uncertainty conventions, and justified tolerances. Run the fixtures in the ordinary offline Rust suite with existing tolerances intact.

Curve smoothing and AUC, including the separate clipped fitted-curve case, are covered by [#30](https://github.com/HauserGroup/curvebook/pull/30).
