//! Numerical primitives, special functions, tail-safe probability distributions, dense linear
//! algebra, quadrature, root-finding, deterministic PRNG, and axis scale transforms for LumenCraft.
//!
//! Layer L0 foundation crate: depends on no higher workspace crates and zero external crates.
//! Governing spec: `docs/statistics-implementation.md` (§1), `docs/graphing-and-layouts.md` (§3.1),
//! and `docs/architecture.md`.
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

/// Scale transformations supported on scientific graph axes (`docs/graphing-and-layouts.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisScale {
    Linear,
    Log10,
    Log2,
    NaturalLog,
    Reciprocal,
    Logit,
    Probability,
}

impl AxisScale {
    /// Guarded forward coordinate bijection `g(u)`, returning `None` (never `NaN` or `+-Inf`) when
    /// `u` lies outside the scale's mathematical domain:
    /// - `Linear`: `u` finite,
    /// - `Log10`, `Log2`, `NaturalLog`: strictly positive `u > 0`,
    /// - `Reciprocal`: non-zero `u != 0`,
    /// - `Logit`: percentage `u in (0, 100)` (`ln(u / (100 - u))`),
    /// - `Probability` (probit): percentage `u in (0, 100)` (`Phi^{-1}(u / 100)`).
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
            Self::NaturalLog => {
                if u <= 0.0 {
                    return None;
                }
                u.ln()
            }
            Self::Reciprocal => {
                if u == 0.0 {
                    return None;
                }
                1.0 / u
            }
            Self::Logit => {
                if !(u > 0.0 && u < 100.0) {
                    return None;
                }
                (u / (100.0 - u)).ln()
            }
            Self::Probability => {
                if !(u > 0.0 && u < 100.0) {
                    return None;
                }
                dist::norm_ppf(u / 100.0).ok()?
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
            Self::NaturalLog => t.exp(),
            Self::Reciprocal => {
                if t == 0.0 {
                    return None;
                }
                1.0 / t
            }
            Self::Logit => {
                if t >= 0.0 {
                    100.0 / (1.0 + (-t).exp())
                } else {
                    let e = t.exp();
                    100.0 * e / (1.0 + e)
                }
            }
            Self::Probability => 100.0 * dist::norm_cdf(t),
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
    use super::AxisScale;

    #[test]
    fn axis_scale_variants_are_distinct() {
        assert_ne!(AxisScale::Linear, AxisScale::Log10);
        assert_ne!(AxisScale::Log2, AxisScale::NaturalLog);
    }

    #[test]
    fn axis_scale_transforms_and_inverses_roundtrip_without_nans() {
        let scales = [
            (AxisScale::Linear, -12.5),
            (AxisScale::Log10, 1e-6),
            (AxisScale::Log2, 8.0),
            (AxisScale::NaturalLog, std::f64::consts::E),
            (AxisScale::Reciprocal, 4.0),
            (AxisScale::Logit, 25.0),
            (AxisScale::Probability, 97.5),
        ];
        for (scale, u) in scales {
            let t = scale.transform(u).unwrap();
            let u_back = scale.inverse(t).unwrap();
            assert!((u_back - u).abs() < 1e-12, "{scale:?}: {u} -> {t} -> {u_back}");
        }
        assert!(AxisScale::Log10.transform(0.0).is_none());
        assert!(AxisScale::Log10.transform(-1.0).is_none());
        assert!(AxisScale::Reciprocal.transform(0.0).is_none());
        assert!(AxisScale::Logit.transform(0.0).is_none());
        assert!(AxisScale::Logit.transform(100.0).is_none());
        assert!(AxisScale::Probability.transform(100.0).is_none());
        assert!((AxisScale::Probability.normalize_in_domain(50.0, 5.0, 95.0).unwrap() - 0.5).abs() < 1e-14);
    }
}
