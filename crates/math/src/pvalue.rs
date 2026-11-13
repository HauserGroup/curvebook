//! Arbitrary confidence-level validation (`[0.01%, 99.99%]`) and *p*-value display formatting &
//! significance-star summaries per `docs/statistics-implementation.md` §1.2.

use crate::error::MathError;

/// Validated confidence level in `[0.01, 99.99]` percent (`alpha in [0.0001, 0.9999]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceLevel {
    percent: f64,
    alpha: f64,
}

impl ConfidenceLevel {
    /// Standard 95% confidence level (`alpha = 0.05`).
    pub const NINETY_FIVE: Self = Self { percent: 95.0, alpha: 0.05 };

    /// Construct a validated confidence level from `confidence_percent` in `[0.01, 99.99]`.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `confidence_percent` is outside `[0.01, 99.99]` or `NaN`.
    pub fn from_percent(confidence_percent: f64) -> Result<Self, MathError> {
        if !confidence_percent.is_finite() || !(0.01..=99.99).contains(&confidence_percent) {
            return Err(MathError::InvalidParameter { name: "confidence_percent", value: confidence_percent });
        }
        let alpha = (100.0 - confidence_percent) / 100.0;
        Ok(Self { percent: confidence_percent, alpha })
    }

    /// Confidence percentage in `[0.01, 99.99]`.
    #[must_use]
    pub const fn percent(self) -> f64 {
        self.percent
    }

    /// Two-sided significance level `alpha = (100 - percent) / 100`.
    #[must_use]
    pub const fn alpha(self) -> f64 {
        self.alpha
    }

    /// One-sided per-tail probability `alpha / 2`.
    #[must_use]
    pub fn half_alpha(self) -> f64 {
        0.5 * self.alpha
    }
}

impl Default for ConfidenceLevel {
    fn default() -> Self {
        Self::NINETY_FIVE
    }
}

/// *P*-value formatting and asterisk-threshold presets (`docs/statistics-implementation.md` §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PValueFormat {
    /// Leading zero, 4 decimal places (`0.1234`), floor `< 0.0001`, up to 4 stars (`****`).
    #[default]
    FourDecimal,
    /// No leading zero, 3 decimal places (`.123`), floor `< .001`, ceiling of 3 stars (`***`).
    Apa,
    /// Leading zero, 3 decimal places (`0.123`), floor `< 0.001`, ceiling of 3 stars (`***`).
    Nejm,
    /// Custom decimal count (`1..=15`) with optional scientific-notation threshold (`2.41e-9`) and 4-star thresholds.
    Custom { decimals: u8, scientific_below: Option<f64> },
}

impl PValueFormat {
    /// Return the asterisk summary (`"ns"`, `"*"`, `"**"`, `"***"`, or `"****"`) for raw `p in [0, 1]`.
    #[must_use]
    pub fn stars(self, p: f64) -> &'static str {
        if !p.is_finite() || p > 0.05 {
            return "ns";
        }
        let three_star_ceiling = matches!(self, Self::Apa | Self::Nejm);
        if p <= 0.0001 && !three_star_ceiling {
            "****"
        } else if p <= 0.001 {
            "***"
        } else if p <= 0.01 {
            "**"
        } else {
            "*"
        }
    }

    /// Format a raw `p`-value according to the selected publication style without mutating stored precision.
    #[must_use]
    pub fn format(self, p: f64) -> String {
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
                    && p > 0.0
                    && p < thresh
                {
                    let sig = d.clamp(2, 6);
                    return format!("{p:.sig$e}");
                }
                let floor = 10.0_f64.powi(-(d as i32));
                if p < floor { format!("< {floor:.d$}") } else { format!("{p:.d$}") }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfidenceLevel, PValueFormat};

    #[test]
    fn confidence_level_supports_decimal_and_extreme_levels() {
        let c999 = ConfidenceLevel::from_percent(99.9).unwrap();
        assert!((c999.alpha() - 0.001).abs() < 1e-15);
        let c_dec = ConfidenceLevel::from_percent(97.5).unwrap();
        assert!((c_dec.half_alpha() - 0.0125).abs() < 1e-15);
        assert!(ConfidenceLevel::from_percent(0.0).is_err());
        assert!(ConfidenceLevel::from_percent(100.0).is_err());
    }

    #[test]
    fn pvalue_format_presets_and_star_ceilings() {
        assert_eq!(PValueFormat::FourDecimal.format(0.000_03), "< 0.0001");
        assert_eq!(PValueFormat::FourDecimal.stars(0.000_03), "****");
        assert_eq!(PValueFormat::Apa.format(0.1234), ".123");
        assert_eq!(PValueFormat::Apa.format(0.000_03), "< .001");
        assert_eq!(PValueFormat::Apa.stars(0.000_03), "***");
        assert_eq!(PValueFormat::Nejm.format(0.1234), "0.123");
        assert_eq!(PValueFormat::Nejm.stars(0.000_03), "***");
        let custom = PValueFormat::Custom { decimals: 2, scientific_below: Some(1e-4) };
        assert_eq!(custom.format(2.41e-9), "2.41e-9");
    }
}
