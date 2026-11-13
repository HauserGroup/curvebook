//! Numerical primitives, special functions, tail-safe probability distributions, dense linear
//! algebra, quadrature, root-finding, deterministic PRNG, and axis scale transforms for LumenCraft.
//!
//! Layer L0 foundation crate: depends on no higher workspace crates; uses `serde` for schema
//! serialization alongside pure-std numerical kernels.
//! Governing spec: `docs/statistics-implementation.md` (§1), `docs/graphing-and-layouts.md` (§3.1),
//! and `docs/architecture.md`.
//!
//! # Example
//!
//! ```
//! use lumencraft_math::{AxisScale, ConfidenceLevel, LogitDomain, PValueFormat};
//! use lumencraft_math::dist::student_t_two_sided_p;
//! use lumencraft_math::sum::SummaryMoments;
//!
//! let moments = SummaryMoments::from_slice(&[10.0, 12.0, 14.0, 16.0]).expect("finite sample");
//! assert_eq!(moments.mean, 13.0);
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

pub mod dist;
pub mod error;
pub mod linalg;
pub mod pvalue;
pub mod quad;
pub mod rng;
pub mod roots;
pub mod special;
pub mod sum;

pub use dist::noncentral::{noncentral_chi2_ci, noncentral_f_ci, noncentral_t_ci};
pub use error::MathError;
pub use linalg::{
    Cholesky, DenseMatrix, LeastSquaresSolution, PivotedQr, Svd, SymEigen, cholesky, parameter_dependencies, qr, qr_col_pivoted,
    solve_linear_least_squares, svd, sym_eigen,
};
pub use pvalue::{ConfidenceLevel, PValueFormat};
pub use rng::DeterministicRng;

/// Percentage vs unit-proportion domain for [`AxisScale::Logit`] (`docs/graphing-and-layouts.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LogitDomain {
    /// Percentages `u in (0, 100)`.
    #[default]
    Percent,
    /// Unit proportions `u in (0, 1)`.
    Proportion,
}

/// Scale transformations supported on scientific graph axes (`docs/graphing-and-layouts.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum AxisScale {
    #[default]
    Linear,
    Log10,
    Log2,
    Ln,
    Logit(LogitDomain),
    Probit,
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
                (u / (100.0 - u)).ln()
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
        out.is_finite().then_some(out)
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
