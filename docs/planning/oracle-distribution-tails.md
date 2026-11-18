# Distribution tail oracle scope

Follow-up to [#28](https://github.com/HauserGroup/curvebook/issues/28). This draft proposes D1 verification for `curvebook-math`. Existing distribution tests include analytical values and round trips. The additional fixtures target tails and numerical regime changes where forward and inverse functions can agree while sharing an error.

## Proposed scope

Add 12–16 independently generated input cases across regularized incomplete beta and gamma and the noncentral *t*, *F*, and χ² distributions. Include small shape or degrees of freedom, unbalanced beta shapes, both signs of the noncentral *t* shift, large noncentrality, and points on both sides of selected approximation boundaries. Include ordinary probabilities as controls.

Use published definitions and independently evaluated high-precision integrals or series. Cross-check representative noncentral cases with a versioned reference implementation where its domain and accuracy apply. Record the generator, version, precision, and input parameters. Use direct calculations of small tail probabilities and log tails where the public API exposes them.

## Acceptance criteria

- [ ] Compare lower and upper tails to independent expected values. Include at least one case per family where forming the smaller tail by subtraction would lose important digits.
- [ ] Where inverse or confidence-limit APIs exist, compare the result directly to an independently generated quantile or limit. Keep round trips as supplementary checks.
- [ ] Include cases on both sides of at least two numerical regime boundaries and document the relevant error metric. Use relative error for small representable probabilities and absolute log error for log probabilities.
- [ ] Record source, inputs, expected outputs, and justified tolerances beside the fixtures. Exercise them in the normal offline Rust test suite with existing assertions and tolerances intact.

Reference material includes [NIST DLMF, Chapter 8](https://dlmf.nist.gov/8) and [Boost.Math noncentral t distribution and reference testing](https://live.boost.org/doc/libs/1_80_0/libs/math/doc/html/math_toolkit/dist_ref/dists/nc_t_dist.html).
