//! Typed cell values, non-destructive cell exclusions (`excluded: bool`), inline `*` exclusion
//! token parsing, replicate subcolumn modes, and pre-summarized `(Mean, SD/SEM/%CV, N)` moment
//! reconstruction (`docs/data-tables-and-transforms.md` §1.2, §2, §2.1, §6.1).

use crate::error::{DocError, SummaryPointError, validate_subcolumns};

/// Typed content of a single grid cell (`docs/data-tables-and-transforms.md` §1.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum CellValue {
    #[default]
    Empty,
    Number(f64),
    Text(String),
}

impl CellValue {
    /// Construct a finite numeric cell value, mapping `NaN` or `±Inf` cleanly to [`Self::Empty`]
    /// (`docs/data-tables-and-transforms.md` §4.1).
    #[must_use]
    pub fn from_finite_or_empty(value: f64) -> Self {
        if value.is_finite() { Self::Number(value) } else { Self::Empty }
    }

    /// True if the cell holds no value (`Empty`, or whitespace-only text).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Number(_) => false,
            Self::Text(s) => s.trim().is_empty(),
        }
    }

    /// Return `Some(x)` iff this cell holds a finite IEEE-754 `f64`.
    #[must_use]
    pub fn finite_f64(&self) -> Option<f64> {
        match self {
            Self::Number(v) if v.is_finite() => Some(*v),
            _ => None,
        }
    }
}

/// Grid cell pairing a [`CellValue`] with a non-destructive exclusion mask (`excluded: bool`).
///
/// Excluded cells remain visible in the spreadsheet (rendered in blue italics with an `*` badge)
/// and persist in `.lumencraft` archives while downstream statistical tests, fits, transforms, and
/// graphs mask them out without mutating raw observations (`docs/data-tables-and-transforms.md` §1.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub value: CellValue,
    pub excluded: bool,
}

impl Cell {
    /// Empty, unexcluded cell.
    #[must_use]
    pub const fn empty() -> Self {
        Self { value: CellValue::Empty, excluded: false }
    }

    /// Finite numeric cell (`NaN` / `±Inf` mapped safely to `Empty`).
    #[must_use]
    pub fn number(value: f64) -> Self {
        Self { value: CellValue::from_finite_or_empty(value), excluded: false }
    }

    /// Excluded numeric cell (`45.6*`).
    #[must_use]
    pub fn excluded_number(value: f64) -> Self {
        Self { value: CellValue::from_finite_or_empty(value), excluded: true }
    }

    /// Text / categorical token cell.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self { value: CellValue::Text(value.into()), excluded: false }
    }

    /// Excluded text / categorical token cell.
    #[must_use]
    pub fn excluded_text(value: impl Into<String>) -> Self {
        Self { value: CellValue::Text(value.into()), excluded: true }
    }

    /// True when the underlying value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Return `Some(x)` iff `!self.excluded` and `self.value` is a finite `f64`.
    #[must_use]
    pub fn active_number(&self) -> Option<f64> {
        if self.excluded { None } else { self.value.finite_f64() }
    }

    /// Return `Some(&str)` iff `!self.excluded` and `self.value` is non-empty text.
    #[must_use]
    pub fn active_text(&self) -> Option<&str> {
        if self.excluded {
            return None;
        }
        match &self.value {
            CellValue::Text(s) if !s.trim().is_empty() => Some(s.as_str()),
            _ => None,
        }
    }

    /// Parse an imported CSV/TSV/clipboard token, honoring inline asterisk exclusion syntax
    /// (`45.6*` or `*45.6` -> `CellValue::Number(45.6)` with `excluded = true`) and regional
    /// decimal separators (`.` vs `,`) (`docs/data-tables-and-transforms.md` §6.1).
    #[must_use]
    pub fn parse_import_token(raw: &str, decimal_separator: char, parse_asterisk_exclusion: bool) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("na") || trimmed.eq_ignore_ascii_case("nan") {
            return Self::empty();
        }

        let (core, starred) = if parse_asterisk_exclusion {
            if let Some(stripped) = trimmed.strip_suffix('*') {
                (stripped.trim(), true)
            } else if let Some(stripped) = trimmed.strip_prefix('*') {
                (stripped.trim(), true)
            } else {
                (trimmed, false)
            }
        } else {
            (trimmed, false)
        };

        if core.is_empty() {
            return Self::text(trimmed);
        }

        let normalized = if decimal_separator == ',' { core.replace(',', ".") } else { core.to_owned() };
        if let Ok(num) = normalized.parse::<f64>()
            && num.is_finite()
        {
            return Self { value: CellValue::Number(num), excluded: starred };
        }

        Self { value: CellValue::Text(if starred { core.to_owned() } else { trimmed.to_owned() }), excluded: starred }
    }
}

/// Subcolumn structure within each data-set column (`docs/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SubcolumnMode {
    /// Single value per row (`1` subcolumn).
    #[default]
    Single,
    /// `n` raw replicate subcolumns (`Y1 .. Yn`, `1 <= n <= MAX_SUBCOLUMNS`).
    RawReplicates(usize),
    /// Pre-summarized `[Mean, SD, N]` (`3` subcolumns).
    MeanSdN,
    /// Pre-summarized `[Mean, SEM, N]` (`3` subcolumns).
    MeanSemN,
    /// Pre-summarized `[Mean, %CV, N]` (`3` subcolumns).
    MeanCvN,
    /// Pre-summarized `[Mean, -Error, +Error]` (`3` subcolumns).
    MeanLowHigh,
    /// Pre-summarized `[Mean, LowerLimit, UpperLimit]` (`3` subcolumns).
    UpperLowerLimits,
    /// Pre-summarized `[Median, Q1, Q3]` (`3` subcolumns).
    MedianIqr,
}

impl SubcolumnMode {
    /// Number of physical subcolumns per data-set column.
    #[must_use]
    pub const fn subcolumn_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::RawReplicates(n) => n,
            Self::MeanSdN | Self::MeanSemN | Self::MeanCvN | Self::MeanLowHigh | Self::UpperLowerLimits | Self::MedianIqr => 3,
        }
    }

    /// Validate that replicate counts lie within `1..=MAX_SUBCOLUMNS`.
    pub fn validate(self) -> Result<(), DocError> {
        match self {
            Self::RawReplicates(n) => validate_subcolumns(n),
            _ => Ok(()),
        }
    }

    /// True if values represent pre-computed summary statistics rather than raw observations.
    ///
    /// Nonparametric rank tests, normality tests, outlier tests, and robust regression refuse
    /// pre-summarized tables with `RequiresRawReplicates` (`docs/data-tables-and-transforms.md` §2.1).
    #[must_use]
    pub const fn is_pre_summarized(self) -> bool {
        matches!(self, Self::MeanSdN | Self::MeanSemN | Self::MeanCvN | Self::MeanLowHigh | Self::UpperLowerLimits | Self::MedianIqr)
    }

    /// True if parametric `(Mean, SD, N)` within-point sum of squares can be reconstructed rigorously.
    #[must_use]
    pub const fn supports_parametric_moments(self) -> bool {
        matches!(self, Self::MeanSdN | Self::MeanSemN | Self::MeanCvN)
    }

    /// Second-tier subcolumn header labels for the spreadsheet grid.
    #[must_use]
    pub fn subcolumn_labels(self) -> Vec<String> {
        match self {
            Self::Single => vec![String::from("Y")],
            Self::RawReplicates(n) => (1..=n).map(|i| format!("Y{i}")).collect(),
            Self::MeanSdN => vec![String::from("Mean"), String::from("SD"), String::from("N")],
            Self::MeanSemN => vec![String::from("Mean"), String::from("SEM"), String::from("N")],
            Self::MeanCvN => vec![String::from("Mean"), String::from("%CV"), String::from("N")],
            Self::MeanLowHigh => vec![String::from("Mean"), String::from("-Error"), String::from("+Error")],
            Self::UpperLowerLimits => vec![String::from("Mean"), String::from("Lower"), String::from("Upper")],
            Self::MedianIqr => vec![String::from("Median"), String::from("Q1"), String::from("Q3")],
        }
    }

    /// Resolve `(center, lower_endpoint, upper_endpoint)` in data-axis coordinates from a
    /// pre-summarized 3-cell subcolumn slice (`MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`,
    /// `UpperLowerLimits`, or `MedianIqr`).
    #[must_use]
    pub fn explicit_bounds(self, subcells: &[Cell]) -> Option<ExplicitErrorBounds> {
        let center = subcells.first()?.active_number()?;
        match self {
            Self::Single | Self::RawReplicates(_) => None,
            Self::MeanSdN | Self::MeanSemN => {
                let d = subcells.get(1)?.active_number()?;
                (d >= 0.0).then_some(ExplicitErrorBounds { center, lower: center - d, upper: center + d })
            }
            Self::MeanCvN => {
                let cv = subcells.get(1)?.active_number()?;
                if cv < 0.0 {
                    return None;
                }
                let sd = (cv / 100.0) * center.abs();
                Some(ExplicitErrorBounds { center, lower: center - sd, upper: center + sd })
            }
            Self::MeanLowHigh => {
                let down = subcells.get(1)?.active_number()?;
                let up = subcells.get(2)?.active_number()?;
                (down >= 0.0 && up >= 0.0).then_some(ExplicitErrorBounds { center, lower: center - down, upper: center + up })
            }
            Self::UpperLowerLimits | Self::MedianIqr => {
                let low = subcells.get(1)?.active_number()?;
                let high = subcells.get(2)?.active_number()?;
                let (lower, upper) = if low <= high { (low, high) } else { (high, low) };
                Some(ExplicitErrorBounds { center, lower, upper })
            }
        }
    }
}

/// Plottable `(center, lower_endpoint, upper_endpoint)` resolved from pre-summarized subcolumns.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExplicitErrorBounds {
    pub center: f64,
    pub lower: f64,
    pub upper: f64,
}

/// Visual layout of replicate subcolumns in a `Grouped` table (`docs/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateStacking {
    #[default]
    SideBySide,
    Stacked,
}

/// Rigorously reconstructed parametric summary observation from `MeanSdN`, `MeanSemN`, or
/// `MeanCvN` subcolumns (`docs/data-tables-and-transforms.md` §2.1):
///
/// `SS_within = (n - 1) * SD^2`, where `SD = SEM * sqrt(n)` or `(%CV / 100) * |Mean|`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SummarizedPoint {
    pub mean: f64,
    pub sd: f64,
    pub n: f64,
    pub ss_within: f64,
    pub df_within: f64,
}

impl SummarizedPoint {
    /// Reconstruct `(mean, sd, n, ss_within, df_within)` from a 3-cell subcolumn slice.
    ///
    /// Returns `Ok(None)` when the mean cell is empty or excluded. Flags missing `N`, `N < 1`, or
    /// negative spread as explicit [`SummaryPointError`] diagnostics rather than silently assuming `N = 1`.
    pub fn reconstruct(mode: SubcolumnMode, subcells: &[Cell]) -> Result<Option<Self>, SummaryPointError> {
        if !mode.supports_parametric_moments() {
            return Err(SummaryPointError::NotParametricSummaryMode { mode });
        }
        let Some(mean) = subcells.first().and_then(Cell::active_number) else {
            return Ok(None);
        };
        let spread = subcells.get(1).and_then(Cell::active_number).ok_or(SummaryPointError::MissingSpread)?;
        if spread < 0.0 {
            return Err(SummaryPointError::NegativeSpread { spread });
        }
        let n = subcells.get(2).and_then(Cell::active_number).ok_or(SummaryPointError::MissingSampleSize)?;
        if n < 1.0 {
            return Err(SummaryPointError::SampleSizeBelowOne { n });
        }
        let sd = match mode {
            SubcolumnMode::MeanSdN => spread,
            SubcolumnMode::MeanSemN => spread * n.sqrt(),
            SubcolumnMode::MeanCvN => (spread / 100.0) * mean.abs(),
            _ => return Err(SummaryPointError::NotParametricSummaryMode { mode }),
        };
        let df_within = n - 1.0;
        let ss_within = df_within * sd * sd;
        Ok(Some(Self { mean, sd, n, ss_within, df_within }))
    }
}
