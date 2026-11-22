//! Typed cell values, non-destructive cell exclusions (`excluded: bool`), inline `*` exclusion
//! token parsing, replicate subcolumn modes, and pre-summarized `(Mean, SD/SEM/%CV, N)` moment
//! reconstruction (`docs/implementation/data-tables-and-transforms.md` §1.2, §2, §2.1, §6.1).

use crate::error::{DocError, SummaryPointError, validate_subcolumns};

/// Typed content of a single grid cell (`docs/implementation/data-tables-and-transforms.md` §1.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum CellValue {
    /// Unpopulated cell (missing observation; non-finite `NaN`/`±Inf` inputs also normalize to `Empty` via [`Self::from_finite_or_empty`]).
    #[default]
    Empty,
    /// Numeric scalar observation in the column's data units (finite `f64` when constructed via [`Self::from_finite_or_empty`] or [`Cell::number`]).
    Number(f64),
    /// Categorical label, row/group identifier, or free-form string token.
    Text(String),
}

impl CellValue {
    /// Construct a finite numeric cell value, mapping `NaN` or `±Inf` cleanly to [`Self::Empty`]
    /// (`docs/implementation/data-tables-and-transforms.md` §4.1).
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
/// and persist in `.curvebook` archives while downstream statistical tests, fits, transforms, and
/// graphs mask them out without mutating raw observations (`docs/implementation/data-tables-and-transforms.md` §1.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    /// Underlying empty, numeric, or text payload stored in the cell.
    pub value: CellValue,
    /// When `true`, the observation is retained visibly in the sheet (`*` badge) but masked out of [`Self::active_number`], [`Self::active_text`], statistical tests, fits, transforms, and plotted series.
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
    /// decimal separators (`.` vs `,`) (`docs/implementation/data-tables-and-transforms.md` §6.1).
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

/// Subcolumn structure within each data-set column (`docs/implementation/data-tables-and-transforms.md` §2).
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
    /// pre-summarized tables with `RequiresRawReplicates` (`docs/implementation/data-tables-and-transforms.md` §2.1).
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
        if !center.is_finite() {
            return None;
        }
        match self {
            Self::Single | Self::RawReplicates(_) => None,
            Self::MeanSdN | Self::MeanSemN => {
                let d = subcells.get(1)?.active_number()?;
                if !d.is_finite() || d < 0.0 {
                    return None;
                }
                let lower = center - d;
                let upper = center + d;
                (lower.is_finite() && upper.is_finite()).then_some(ExplicitErrorBounds { center, lower, upper })
            }
            Self::MeanCvN => {
                let cv = subcells.get(1)?.active_number()?;
                if !cv.is_finite() || cv < 0.0 || center == 0.0 {
                    return None;
                }
                let sd = (cv / 100.0) * center.abs();
                let lower = center - sd;
                let upper = center + sd;
                (sd.is_finite() && lower.is_finite() && upper.is_finite()).then_some(ExplicitErrorBounds { center, lower, upper })
            }
            Self::MeanLowHigh => {
                let down = subcells.get(1)?.active_number()?;
                let up = subcells.get(2)?.active_number()?;
                if !down.is_finite() || !up.is_finite() || down < 0.0 || up < 0.0 {
                    return None;
                }
                let lower = center - down;
                let upper = center + up;
                (lower.is_finite() && upper.is_finite()).then_some(ExplicitErrorBounds { center, lower, upper })
            }
            Self::UpperLowerLimits | Self::MedianIqr => {
                let low = subcells.get(1)?.active_number()?;
                let high = subcells.get(2)?.active_number()?;
                if !low.is_finite() || !high.is_finite() {
                    return None;
                }
                let (lower, upper) = if low <= high { (low, high) } else { (high, low) };
                Some(ExplicitErrorBounds { center, lower, upper })
            }
        }
    }
}

/// Plottable `(center, lower_endpoint, upper_endpoint)` resolved from pre-summarized subcolumns.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExplicitErrorBounds {
    /// Central point location in data-axis units (`Mean` for parametric/interval modes or `Median` for [`SubcolumnMode::MedianIqr`]).
    pub center: f64,
    /// Lower error-bar endpoint in data-axis units (`center - SD`, `center - SEM`, `center - (%CV/100)*|center|`, `center - (-Error)`, `LowerLimit`, or `Q1`).
    pub lower: f64,
    /// Upper error-bar endpoint in data-axis units (`center + SD`, `center + SEM`, `center + (%CV/100)*|center|`, `center + (+Error)`, `UpperLimit`, or `Q3`).
    pub upper: f64,
}

/// Visual layout of replicate subcolumns in a `Grouped` table (`docs/implementation/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateStacking {
    /// Display replicate subcolumns (`Y1 .. Yn`) horizontally side-by-side within each row (`row = factor A level`, `subcolumn = subject/replicate`).
    #[default]
    SideBySide,
    /// Display replicates stacked vertically across consecutive rows under a single column.
    Stacked,
}

/// Rigorously reconstructed parametric summary observation from `MeanSdN`, `MeanSemN`, or
/// `MeanCvN` subcolumns (`docs/implementation/data-tables-and-transforms.md` §2.1):
///
/// `SS_within = (n - 1) * SD^2`, where `SD = SEM * sqrt(n)` or `(%CV / 100) * |Mean|`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SummarizedPoint {
    /// Sample arithmetic mean in data units.
    pub mean: f64,
    /// Reconstructed sample standard deviation (`>= 0`, finite) in data units (`SD`, `SEM * sqrt(n)`, or `(%CV / 100) * |mean|`).
    pub sd: f64,
    /// Integer replicate count `n >= 1.0` stored as `f64` for exact moment algebra.
    pub n: f64,
    /// Reconstructed within-cell sum of squared deviations `(n - 1) * sd^2` (`0.0` when `n == 1.0`).
    pub ss_within: f64,
    /// Within-cell degrees of freedom `n - 1.0` (`>= 0.0`).
    pub df_within: f64,
}

impl SummarizedPoint {
    /// Reconstruct `(mean, sd, n, ss_within, df_within)` from a 3-cell subcolumn slice.
    ///
    /// Returns `Ok(None)` when the mean cell is empty or excluded. Flags missing `N`, `N < 1`,
    /// fractional `N`, negative/non-finite spread, undefined `%CV` at `mean == 0`, or overflowing
    /// `sd`/`ss_within` as explicit [`SummaryPointError`] diagnostics rather than silently assuming `N = 1`.
    pub fn reconstruct(mode: SubcolumnMode, subcells: &[Cell]) -> Result<Option<Self>, SummaryPointError> {
        if !mode.supports_parametric_moments() {
            return Err(SummaryPointError::NotParametricSummaryMode { mode });
        }
        let Some(mean) = subcells.first().and_then(Cell::active_number) else {
            return Ok(None);
        };
        let spread = subcells.get(1).and_then(Cell::active_number).ok_or(SummaryPointError::MissingSpread)?;
        if !spread.is_finite() || spread < 0.0 {
            return Err(SummaryPointError::NegativeSpread { spread });
        }
        let n = subcells.get(2).and_then(Cell::active_number).ok_or(SummaryPointError::MissingSampleSize)?;
        if !n.is_finite() || n < 1.0 || n.fract() != 0.0 {
            return Err(SummaryPointError::SampleSizeBelowOne { n });
        }
        let sd = match mode {
            SubcolumnMode::MeanSdN => spread,
            SubcolumnMode::MeanSemN => spread * n.sqrt(),
            SubcolumnMode::MeanCvN => {
                if mean == 0.0 {
                    return Err(SummaryPointError::MissingSpread);
                }
                (spread / 100.0) * mean.abs()
            }
            _ => return Err(SummaryPointError::NotParametricSummaryMode { mode }),
        };
        let df_within = n - 1.0;
        let sd_sq = sd * sd;
        let ss_within = df_within * sd_sq;
        if !sd.is_finite() || !sd_sq.is_finite() || !df_within.is_finite() || !ss_within.is_finite() || ss_within < 0.0 {
            return Err(SummaryPointError::NegativeSpread { spread });
        }
        Ok(Some(Self { mean, sd, n, ss_within, df_within }))
    }
}
