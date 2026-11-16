//! Arbitrary confidence-level validation (`[0.01%, 99.99%]`) and *p*-value display formatting &
//! significance-star summaries per `docs/statistics-implementation.md` §1.2.
//!
//! Shared as the single canonical definition across `curvebook-math` (`L0`), `curvebook-doc`
//! (`L1`), and downstream statistical/fitting/chart crates (`L2`).

use crate::error::MathError;

/// Validated statistical confidence level in `[0.01, 99.99]` percent (`alpha in [0.0001, 0.9999]`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct ConfidenceLevel(f64);

impl ConfidenceLevel {
    /// Standard 95.0% confidence level (`alpha = 0.05`).
    pub const DEFAULT_95: Self = Self(95.0);

    /// Alias for [`Self::DEFAULT_95`] (`95.0%`, `alpha = 0.05`).
    pub const NINETY_FIVE: Self = Self(95.0);

    /// Standard 99.0% confidence level (`alpha = 0.01`).
    pub const PERCENT_99: Self = Self(99.0);

    /// Construct a validated confidence level from `confidence_percent` in `[0.01, 99.99]`.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `confidence_percent` is outside `[0.01, 99.99]` or non-finite.
    pub fn from_percent(confidence_percent: f64) -> Result<Self, MathError> {
        if !confidence_percent.is_finite() || !(0.01..=99.99).contains(&confidence_percent) {
            return Err(MathError::InvalidParameter { name: "confidence_percent", value: confidence_percent });
        }
        Ok(Self(confidence_percent))
    }

    /// Construct a validated confidence level from `percent` in `[0.01, 99.99]` (alias for [`Self::from_percent`]).
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `percent` is outside `[0.01, 99.99]` or non-finite.
    pub fn new(percent: f64) -> Result<Self, MathError> {
        Self::from_percent(percent)
    }

    /// Confidence percentage in `[0.01, 99.99]`.
    #[must_use]
    pub const fn percent(self) -> f64 {
        self.0
    }

    /// Two-sided significance threshold `alpha = (100 - percent) / 100`.
    #[must_use]
    pub const fn alpha(self) -> f64 {
        (100.0 - self.0) / 100.0
    }

    /// One-sided per-tail probability `alpha / 2`.
    #[must_use]
    pub const fn two_sided_tail(self) -> f64 {
        0.5 * self.alpha()
    }

    /// Alias for [`Self::two_sided_tail`] (`alpha / 2`).
    #[must_use]
    pub const fn half_alpha(self) -> f64 {
        self.two_sided_tail()
    }
}

impl Default for ConfidenceLevel {
    fn default() -> Self {
        Self::DEFAULT_95
    }
}

impl From<ConfidenceLevel> for f64 {
    fn from(level: ConfidenceLevel) -> Self {
        level.percent()
    }
}

impl TryFrom<f64> for ConfidenceLevel {
    type Error = MathError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::from_percent(value)
    }
}

/// *P*-value and *q*-value formatting and asterisk-threshold presets (`docs/statistics-implementation.md` §1.2).
///
/// Raw `f64` probabilities are always stored unrounded in result sheets so downstream multiplicity
/// adjustments and live graph brackets never suffer rounding quantization.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum PValueFormat {
    /// Leading zero, 4 decimal places (`0.1234`), floor `< 0.0001`, up to 4 stars (`****`).
    #[default]
    FourDecimal,
    /// No leading zero, 3 decimal places (`.123`), floor `< .001`, ceiling of 3 stars (`***`).
    Apa,
    /// Leading zero, 3 decimal places (`0.123`), floor `< 0.001`, ceiling of 3 stars (`***`).
    Nejm,
    /// Custom decimal count (`1..=15`) with optional scientific-notation threshold (`2.41e-9`) and 4-star ceiling.
    Custom { decimals: u8, scientific_below: Option<f64> },
}

impl PValueFormat {
    /// Construct a [`Self::Custom`] preset with `decimals` clamped to `1..=15` and finite positive `scientific_below`.
    #[must_use]
    pub fn new_custom(decimals: u8, scientific_below: Option<f64>) -> Self {
        let clamped_decimals = decimals.clamp(1, 15);
        let cutoff = scientific_below.filter(|&v| v.is_finite() && v > 0.0);
        Self::Custom { decimals: clamped_decimals, scientific_below: cutoff }
    }

    /// Maximum number of significance stars emitted by this preset (`3` for `Apa`/`Nejm`, `4` otherwise).
    #[must_use]
    pub const fn star_ceiling(self) -> u8 {
        match self {
            Self::Apa | Self::Nejm => 3,
            Self::FourDecimal | Self::Custom { .. } => 4,
        }
    }

    /// Return the asterisk summary (`"ns"`, `"*"`, `"**"`, `"***"`, or `"****"`) for raw `p in [0, 1]`.
    #[must_use]
    pub fn stars(self, p: f64) -> &'static str {
        if !p.is_finite() || !(0.0..=1.0).contains(&p) || p > 0.05 {
            return "ns";
        }
        if p <= 0.0001 && self.star_ceiling() >= 4 {
            "****"
        } else if p <= 0.001 {
            "***"
        } else if p <= 0.01 {
            "**"
        } else {
            "*"
        }
    }

    /// Alias for [`Self::stars`] used by document/result-sheet summary helpers.
    #[must_use]
    pub fn summary_stars(self, p: f64) -> &'static str {
        self.stars(p)
    }

    /// Format a raw `p`-value in `[0.0, 1.0]` according to the selected publication style.
    #[must_use]
    pub fn format_p(self, p: f64) -> String {
        if !p.is_finite() || !(0.0..=1.0).contains(&p) {
            return String::from("NaN");
        }
        match self {
            Self::FourDecimal => {
                if p < 0.0001 {
                    String::from("< 0.0001")
                } else if p > 0.9999 && p < 1.0 {
                    String::from("> 0.9999")
                } else {
                    format!("{p:.4}")
                }
            }
            Self::Apa => {
                if p < 0.001 {
                    String::from("< .001")
                } else if p > 0.999 {
                    String::from("> .999")
                } else {
                    let s = format!("{p:.3}");
                    s.strip_prefix('0').map_or(s.clone(), str::to_owned)
                }
            }
            Self::Nejm => {
                if p < 0.001 {
                    String::from("< 0.001")
                } else if p > 0.999 && p < 1.0 {
                    String::from("> 0.999")
                } else {
                    format!("{p:.3}")
                }
            }
            Self::Custom { decimals, scientific_below } => {
                let d = usize::from(decimals.clamp(1, 15));
                if let Some(thresh) = scientific_below
                    && thresh.is_finite()
                    && p > 0.0
                    && p < thresh
                {
                    let sig = d.clamp(2, 6);
                    return format!("{p:.sig$e}");
                }
                let floor = 10.0_f64.powi(-(d as i32));
                if p < floor {
                    format!("< {floor:.d$}")
                } else if p > 1.0 - floor && p < 1.0 {
                    let ceil = 1.0 - floor;
                    format!("> {ceil:.d$}")
                } else {
                    format!("{p:.d$}")
                }
            }
        }
    }

    /// Alias for [`Self::format_p`].
    #[must_use]
    pub fn format(self, p: f64) -> String {
        self.format_p(p)
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfidenceLevel, PValueFormat};

    #[test]
    fn confidence_level_supports_decimal_and_extreme_levels() {
        assert_eq!(ConfidenceLevel::default(), ConfidenceLevel::DEFAULT_95);
        assert_eq!(ConfidenceLevel::DEFAULT_95, ConfidenceLevel::NINETY_FIVE);
        assert_eq!(ConfidenceLevel::PERCENT_99.percent(), 99.0);
        let c999 = ConfidenceLevel::from_percent(99.9).unwrap();
        assert_eq!(c999, ConfidenceLevel::new(99.9).unwrap());
        assert!((c999.alpha() - 0.001).abs() < 1e-15);
        let c_dec = ConfidenceLevel::from_percent(97.5).unwrap();
        assert!((c_dec.two_sided_tail() - 0.0125).abs() < 1e-15);
        assert!((c_dec.half_alpha() - 0.0125).abs() < 1e-15);
        assert!(ConfidenceLevel::from_percent(0.0).is_err());
        assert!(ConfidenceLevel::from_percent(100.0).is_err());
        assert!(ConfidenceLevel::new(f64::NAN).is_err());
    }

    #[test]
    fn pvalue_format_presets_edge_cases_and_star_ceilings() {
        assert_eq!(PValueFormat::FourDecimal.star_ceiling(), 4);
        assert_eq!(PValueFormat::Apa.star_ceiling(), 3);
        assert_eq!(PValueFormat::Nejm.star_ceiling(), 3);

        assert_eq!(PValueFormat::FourDecimal.format_p(0.000_03), "< 0.0001");
        assert_eq!(PValueFormat::FourDecimal.format(0.999_95), "> 0.9999");
        assert_eq!(PValueFormat::FourDecimal.stars(0.000_03), "****");
        assert_eq!(PValueFormat::FourDecimal.summary_stars(-0.1), "ns");
        assert_eq!(PValueFormat::FourDecimal.format_p(-0.1), "NaN");

        assert_eq!(PValueFormat::Apa.format_p(0.1234), ".123");
        assert_eq!(PValueFormat::Apa.format_p(0.000_03), "< .001");
        assert_eq!(PValueFormat::Apa.format_p(0.9995), "> .999");
        assert_eq!(PValueFormat::Apa.stars(0.000_03), "***");

        assert_eq!(PValueFormat::Nejm.format_p(0.1234), "0.123");
        assert_eq!(PValueFormat::Nejm.stars(0.000_03), "***");

        let custom = PValueFormat::new_custom(2, Some(1e-4));
        assert_eq!(custom.star_ceiling(), 4);
        assert_eq!(custom.format_p(2.41e-9), "2.41e-9");
        assert_eq!(custom.format_p(0.0), "< 0.01");

        let nan_cutoff = PValueFormat::Custom { decimals: 3, scientific_below: Some(f64::NAN) };
        assert_eq!(nan_cutoff.format_p(0.005), "0.005");
    }

    #[test]
    fn confidence_level_try_from_and_serde_traits_enforced() {
        fn assert_serde<T: serde::Serialize + for<'de> serde::Deserialize<'de>>() {}
        assert_serde::<ConfidenceLevel>();
        assert_serde::<PValueFormat>();
        assert_serde::<crate::AxisScale>();
        assert_serde::<crate::LogitDomain>();
        assert_serde::<crate::sum::SummaryMoments>();
        assert_serde::<crate::DeterministicRng>();

        let level = ConfidenceLevel::try_from(97.5).unwrap();
        assert_eq!(f64::from(level), 97.5);
        assert_eq!(ConfidenceLevel::try_from(0.01).unwrap().percent(), 0.01);
        assert_eq!(ConfidenceLevel::try_from(99.99).unwrap().percent(), 99.99);

        assert!(ConfidenceLevel::try_from(0.0).is_err());
        assert!(ConfidenceLevel::try_from(-5.0).is_err());
        assert!(ConfidenceLevel::try_from(100.0).is_err());
        assert!(ConfidenceLevel::try_from(100.001).is_err());
        assert!(ConfidenceLevel::try_from(f64::NAN).is_err());
        assert!(ConfidenceLevel::try_from(f64::INFINITY).is_err());
    }
}
