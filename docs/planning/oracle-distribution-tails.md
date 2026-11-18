# Distribution tail oracle scope

Follow-up to [#28](https://github.com/HauserGroup/curvebook/issues/28). Existing distribution tests include analytical values and round trips. The additional D1 fixtures target tails and numerical regime changes where forward and inverse functions can agree while sharing an error.

## Proposed scope

Add 12–16 independently generated input cases across regularized incomplete beta and gamma and the noncentral *t*, *F*, and χ² distributions. Include small shape or degrees of freedom, unbalanced beta shapes, both signs of the noncentral *t* shift, large noncentrality, and points on both sides of selected approximation boundaries. Include ordinary probabilities as controls.

Use published definitions and independently evaluated high-precision integrals or series. Cross-check representative noncentral cases with a versioned reference implementation where its domain and accuracy apply. Record the generator, version, precision, and input parameters. Use direct calculations of small tail probabilities and log tails where the public API exposes them.

## Acceptance criteria

- [x] Compare lower and upper tails to independent expected values. Include at least one case per family where forming the smaller tail by subtraction would lose important digits.
- [x] Compare inverse beta and gamma values to independently generated quantiles, and all three noncentral confidence intervals to independently solved limits.
- [x] Bracket the incomplete beta continued-fraction switch and the incomplete gamma Temme threshold. Use relative error for representable probabilities and absolute error on the log scale.
- [x] Record source, inputs, expected outputs, and justified tolerances beside the fixtures. Exercise them in the normal offline Rust test suite with existing assertions and tolerances intact.

The [Rust tests](../../crates/math/tests/d1_distribution_tails.rs) cover forward probabilities, inverse quantiles, and confidence interval endpoints. The [reference generator](../../crates/math/tests/reference/distribution_tails.py) uses mpmath 1.3.0 at 120 decimal digits. The generated binary64 values agree with a second run at 160 digits. For one case in each noncentral family, both tails also agree with Boost.Math 1.92.0 to within `1e-14` relative error using its CDF and complementary CDF.

Reference material includes [NIST DLMF, Chapter 8](https://dlmf.nist.gov/8) and [Boost.Math noncentral t distribution and reference testing](https://live.boost.org/doc/libs/1_80_0/libs/math/doc/html/math_toolkit/dist_ref/dists/nc_t_dist.html).
