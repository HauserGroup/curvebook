//! Numerical primitives, special functions, tail-safe probability distributions, dense linear
//! algebra, quadrature, root-finding, deterministic PRNG, and axis scale transforms for CurveBook.
//!
//! Layer L0 foundation crate: depends on no higher workspace crates; uses `serde` for schema
//! serialization alongside pure-std numerical kernels.
//! Governing spec: `docs/implementation/statistics-implementation.md` (§1), `docs/implementation/graphing-and-layouts.md` (§3.1),
//! and `docs/architecture.md`.
//!
//! # Example
//!
//! ```
//! use curvebook_math::{AxisScale, ConfidenceLevel, LogitDomain, PValueFormat};
//! use curvebook_math::dist::student_t_two_sided_p;
//! use curvebook_math::sum::SummaryMoments;
//!
//! let moments = SummaryMoments::from_slice(&[10.0, 12.0, 14.0, 16.0]).expect("finite sample");
//! assert_eq!(moments.count, 4);
//! assert_eq!(moments.mean, 13.0);
//! assert!((moments.sem().expect("n >= 2") - 1.290_994_448_735_805_6).abs() < 1e-14);
//!
//! let p = student_t_two_sided_p(4.0, 12.0).expect("valid df");
//! assert_eq!(PValueFormat::Apa.format_p(p), ".002");
//! assert_eq!(ConfidenceLevel::DEFAULT_95.alpha(), 0.05);
//!
//! let logit = AxisScale::Logit(LogitDomain::Proportion);
//! assert_eq!(logit.transform(0.5), Some(0.0));
//! assert_eq!(logit.inverse(0.0), Some(0.5));
//! ```
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]
#![deny(missing_docs)]

pub mod dist;
pub mod error;
pub mod linalg;
pub mod normality;
pub mod pvalue;
pub mod quad;
pub mod quantile;
pub mod rng;
pub mod roots;
pub mod rout;
pub mod special;
pub mod sum;

pub use dist::noncentral::{noncentral_chi2_ci, noncentral_f_ci, noncentral_t_ci};
pub use error::MathError;
pub use linalg::{
    Cholesky, DenseMatrix, LeastSquaresSolution, PivotedQr, Svd, SymEigen, cholesky, parameter_dependencies, qr, qr_col_pivoted,
    solve_linear_least_squares, svd, sym_eigen,
};
pub use normality::{NormalityResult, anderson_darling, average_ranks, dagostino_pearson, shapiro_wilk};
pub use pvalue::{ConfidenceLevel, PValueFormat};
pub use quantile::quantile_type7_sorted;
pub use rng::DeterministicRng;
pub use rout::{RoutCoreResult, compute_rsdr, rout_detect_outliers_core};

/// Percentage vs unit-proportion domain for [`AxisScale::Logit`] (`docs/implementation/graphing-and-layouts.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LogitDomain {
    /// Percentages `u in (0, 100)`.
    #[default]
    Percent,
    /// Unit proportions `u in (0, 1)`.
    Proportion,
}

/// Scale transformations supported on scientific graph axes (`docs/implementation/graphing-and-layouts.md` §3.1).
///
/// Every scale defines a guarded forward map [`Self::transform`] (`g(u)`) and inverse [`Self::inverse`]
/// (`g^{-1}(t)`); out-of-domain or non-finite inputs yield `None` rather than `NaN` or `±Inf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum AxisScale {
    /// Identity scale `g(u) = u` over finite `u in (-inf, +inf)`.
    #[default]
    Linear,
    /// Common (base-10) logarithm `g(u) = log10(u)` over strictly positive `u > 0` (`g^{-1}(t) = 10^t`).
    Log10,
    /// Binary (base-2) logarithm `g(u) = log2(u)` over strictly positive `u > 0` (`g^{-1}(t) = 2^t`).
    Log2,
    /// Natural (base-*e*) logarithm `g(u) = ln(u)` over strictly positive `u > 0` (`g^{-1}(t) = exp(t)`).
    Ln,
    /// Logit log-odds transform `ln(p / (1 - p))` over either percentages `u in (0, 100)` or unit
    /// proportions `u in (0, 1)` as selected by [`LogitDomain`].
    Logit(LogitDomain),
    /// Probit (normal probability) scale `g(u) = Phi^{-1}(u / 100)` over percentages `u in (0, 100)`,
    /// where `Phi^{-1}` is the standard normal quantile function.
    Probit,
    /// Reciprocal scale `g(u) = 1 / u` over finite non-zero `u != 0` (useful for Lineweaver–Burk and
    /// Arrhenius-type reciprocal axes).
    Reciprocal,
}

impl AxisScale {
    /// Alias for [`Self::Ln`] (`ln(u)`).
    pub const NATURAL_LOG: Self = Self::Ln;

    /// Percentage-domain logit scale (`u in (0, 100)`).
    pub const LOGIT_PERCENT: Self = Self::Logit(LogitDomain::Percent);

    /// Unit-proportion logit scale (`u in (0, 1)`).
    pub const LOGIT_PROPORTION: Self = Self::Logit(LogitDomain::Proportion);

    /// Alias for [`Self::Probit`] (`Phi^{-1}(u / 100)`).
    pub const PROBABILITY: Self = Self::Probit;

    /// Guarded forward coordinate bijection `g(u)`, returning `None` (never `NaN` or `+-Inf`) when
    /// `u` lies outside the scale's mathematical domain:
    /// - `Linear`: `u` finite,
    /// - `Log10`, `Log2`, `Ln`: strictly positive `u > 0`,
    /// - `Logit(LogitDomain::Percent)`: percentage `u in (0, 100)` (`ln(u / (100 - u))`),
    /// - `Logit(LogitDomain::Proportion)`: proportion `u in (0, 1)` (`ln(u / (1 - u))`),
    /// - `Probit`: percentage `u in (0, 100)` (`Phi^{-1}(u / 100)`),
    /// - `Reciprocal`: non-zero `u != 0`.
    #[must_use]
    pub fn transform(self, u: f64) -> Option<f64> {
        if !u.is_finite() {
            return None;
        }
        let out = match self {
            Self::Linear => u,
            Self::Log10 => {
                if u <= 0.0 {
                    return None;
                }
                u.log10()
            }
            Self::Log2 => {
                if u <= 0.0 {
                    return None;
                }
                u.log2()
            }
            Self::Ln => {
                if u <= 0.0 {
                    return None;
                }
                u.ln()
            }
            Self::Logit(LogitDomain::Percent) => {
                if !(u > 0.0 && u < 100.0) {
                    return None;
                }
                if (25.0..=75.0).contains(&u) { ((2.0 * u - 100.0) / (100.0 - u)).ln_1p() } else { u.ln() - (100.0 - u).ln() }
            }
            Self::Logit(LogitDomain::Proportion) => {
                if !(u > 0.0 && u < 1.0) {
                    return None;
                }
                u.ln() - (-u).ln_1p()
            }
            Self::Probit => {
                if !(u > 0.0 && u < 100.0) {
                    return None;
                }
                dist::norm_ppf(u / 100.0).ok()?
            }
            Self::Reciprocal => {
                if u == 0.0 {
                    return None;
                }
                1.0 / u
            }
        };
        out.is_finite().then_some(out)
    }

    /// Guarded inverse transform `g^{-1}(t)` mapping axis-space `t` back to data-space `u`.
    #[must_use]
    pub fn inverse(self, t: f64) -> Option<f64> {
        if !t.is_finite() {
            return None;
        }
        let out = match self {
            Self::Linear => t,
            Self::Log10 => 10.0_f64.powf(t),
            Self::Log2 => t.exp2(),
            Self::Ln => t.exp(),
            Self::Logit(LogitDomain::Percent) => {
                if t >= 0.0 {
                    100.0 / (1.0 + (-t).exp())
                } else {
                    let e = t.exp();
                    100.0 * e / (1.0 + e)
                }
            }
            Self::Logit(LogitDomain::Proportion) => {
                if t >= 0.0 {
                    1.0 / (1.0 + (-t).exp())
                } else {
                    let e = t.exp();
                    e / (1.0 + e)
                }
            }
            Self::Probit => 100.0 * dist::norm_cdf(t),
            Self::Reciprocal => {
                if t == 0.0 {
                    return None;
                }
                1.0 / t
            }
        };
        let in_domain = match self {
            Self::Linear => out.is_finite(),
            Self::Log10 | Self::Log2 | Self::Ln => out.is_finite() && out > 0.0,
            Self::Logit(LogitDomain::Percent) | Self::Probit => out.is_finite() && out > 0.0 && out < 100.0,
            Self::Logit(LogitDomain::Proportion) => out.is_finite() && out > 0.0 && out < 1.0,
            Self::Reciprocal => out.is_finite() && out != 0.0,
        };
        in_domain.then_some(out)
    }

    /// Map domain coordinate `u` into normalized segment coordinate `[0, 1]` over `[min, max]`.
    #[must_use]
    pub fn normalize_in_domain(self, u: f64, domain_min: f64, domain_max: f64) -> Option<f64> {
        let g_u = self.transform(u)?;
        let g_min = self.transform(domain_min)?;
        let g_max = self.transform(domain_max)?;
        let span = g_max - g_min;
        if !span.is_finite() || span == 0.0 {
            return None;
        }
        let s = (g_u - g_min) / span;
        s.is_finite().then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::{AxisScale, LogitDomain};

    #[test]
    fn axis_scale_variants_are_distinct() {
        assert_eq!(AxisScale::default(), AxisScale::Linear);
        assert_eq!(LogitDomain::default(), LogitDomain::Percent);
        assert_ne!(AxisScale::Linear, AxisScale::Log10);
        assert_ne!(AxisScale::Log2, AxisScale::Ln);
        assert_ne!(AxisScale::Logit(LogitDomain::Percent), AxisScale::Logit(LogitDomain::Proportion));
    }

    #[test]
    fn axis_scale_transforms_and_inverses_roundtrip_without_nans() {
        let scales = [
            (AxisScale::Linear, -12.5),
            (AxisScale::Log10, 1e-6),
            (AxisScale::Log2, 8.0),
            (AxisScale::Ln, std::f64::consts::E),
            (AxisScale::Reciprocal, 4.0),
            (AxisScale::Logit(LogitDomain::Percent), 25.0),
            (AxisScale::Logit(LogitDomain::Proportion), 0.25),
            (AxisScale::Probit, 97.5),
        ];
        for (scale, u) in scales {
            let t = scale.transform(u).unwrap();
            let u_back = scale.inverse(t).unwrap();
            assert!((u_back - u).abs() < 1e-12, "{scale:?}: {u} -> {t} -> {u_back}");
        }
        assert!(AxisScale::Log10.transform(0.0).is_none());
        assert!(AxisScale::Log10.transform(-1.0).is_none());
        assert!(AxisScale::Reciprocal.transform(0.0).is_none());
        assert!(AxisScale::Logit(LogitDomain::Percent).transform(0.0).is_none());
        assert!(AxisScale::Logit(LogitDomain::Percent).transform(100.0).is_none());
        assert!(AxisScale::Logit(LogitDomain::Proportion).transform(0.0).is_none());
        assert!(AxisScale::Logit(LogitDomain::Proportion).transform(1.0).is_none());
        assert!(AxisScale::Logit(LogitDomain::Proportion).transform(25.0).is_none());
        assert!(AxisScale::Probit.transform(100.0).is_none());
        assert!((AxisScale::Probit.normalize_in_domain(50.0, 5.0, 95.0).unwrap() - 0.5).abs() < 1e-14);
        assert!((AxisScale::Logit(LogitDomain::Proportion).normalize_in_domain(0.5, 0.1, 0.9).unwrap() - 0.5).abs() < 1e-14);
    }
}
