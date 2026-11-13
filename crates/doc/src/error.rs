//! Bounded dimension limits, confidence-level validation, and structured error types for
//! `lumencraft-doc` (`docs/data-tables-and-transforms.md` §1.3 & §2.1).

use std::fmt;

use crate::ids::SheetId;
use crate::{SubcolumnMode, TableKind};

/// Hard ceiling on rows per data table (`docs/data-tables-and-transforms.md` §1.3).
pub const MAX_ROWS: usize = 1_000_000;

/// Hard ceiling on data-set / variable columns per table (`docs/data-tables-and-transforms.md` §1.3).
pub const MAX_DATA_SETS: usize = 1_024;

/// Hard ceiling on replicate subcolumns per data set (`docs/data-tables-and-transforms.md` §1.3).
pub const MAX_SUBCOLUMNS: usize = 512;

/// Hard ceiling on UTF-8 byte length of sheet, axis, row, and column titles.
pub const MAX_TITLE_BYTES: usize = 4_096;

/// Maximum total sheets across all five navigator sections in a single project.
pub const MAX_SHEETS_PER_PROJECT: usize = 500;

/// Current `.lumencraft` document schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Validate a title string against [`MAX_TITLE_BYTES`].
pub fn validate_title(title: &str) -> Result<(), DocError> {
    if title.len() > MAX_TITLE_BYTES {
        return Err(DocError::TitleTooLong { byte_len: title.len(), max: MAX_TITLE_BYTES });
    }
    Ok(())
}

/// Validate a row count against [`MAX_ROWS`].
pub fn validate_rows(count: usize) -> Result<(), DocError> {
    if count > MAX_ROWS {
        return Err(DocError::TooManyRows { count, max: MAX_ROWS });
    }
    Ok(())
}

/// Validate a data-set / variable column count against [`MAX_DATA_SETS`].
pub fn validate_data_sets(count: usize) -> Result<(), DocError> {
    if count > MAX_DATA_SETS {
        return Err(DocError::TooManyDataSets { count, max: MAX_DATA_SETS });
    }
    Ok(())
}

/// Validate a subcolumn count against `1..=MAX_SUBCOLUMNS`.
pub fn validate_subcolumns(count: usize) -> Result<(), DocError> {
    if count == 0 {
        return Err(DocError::InvalidSubcolumnCount { count });
    }
    if count > MAX_SUBCOLUMNS {
        return Err(DocError::TooManySubcolumns { count, max: MAX_SUBCOLUMNS });
    }
    Ok(())
}

/// Validated statistical confidence level in `[0.01%, 99.99%]` (`docs/statistics-implementation.md` §1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceLevel(f64);

impl ConfidenceLevel {
    /// Standard 95.0% confidence level (`alpha = 0.05`).
    pub const DEFAULT_95: Self = Self(95.0);

    /// Standard 99.0% confidence level (`alpha = 0.01`).
    pub const PERCENT_99: Self = Self(99.0);

    /// Construct a validated confidence level in `[0.01, 99.99]`.
    pub fn new(percent: f64) -> Result<Self, DocError> {
        if !percent.is_finite() || !(0.01..=99.99).contains(&percent) {
            return Err(DocError::InvalidConfidenceLevel { percent });
        }
        Ok(Self(percent))
    }

    /// Confidence level expressed as a percentage (`0.01..=99.99`).
    #[must_use]
    pub const fn percent(self) -> f64 {
        self.0
    }

    /// Complementary tail significance threshold `alpha = 1.0 - percent / 100.0`.
    #[must_use]
    pub fn alpha(self) -> f64 {
        (1.0 - self.0 / 100.0).clamp(1e-6, 0.9999)
    }
}

impl Default for ConfidenceLevel {
    fn default() -> Self {
        Self::DEFAULT_95
    }
}

/// Validation failure when reconstructing within-point parametric moments from pre-summarized
/// subcolumns (`MeanSdN`, `MeanSemN`, `MeanCvN`; `docs/data-tables-and-transforms.md` §2.1).
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryPointError {
    /// Subcolumn mode does not encode parametric `(Mean, SD/SEM/%CV, N)` moments.
    NotParametricSummaryMode { mode: SubcolumnMode },
    /// Mean value was entered, but the spread (`SD` / `SEM` / `%CV`) subcolumn is missing or non-numeric.
    MissingSpread,
    /// Mean value was entered, but sample size `N` is missing or non-numeric.
    MissingSampleSize,
    /// Sample size `N` is `< 1.0`.
    SampleSizeBelowOne { n: f64 },
    /// Entered spread (`SD`, `SEM`, or `%CV`) is negative.
    NegativeSpread { spread: f64 },
}

impl fmt::Display for SummaryPointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotParametricSummaryMode { mode } => {
                write!(f, "subcolumn mode {mode:?} does not encode parametric (Mean, Spread, N) moments")
            }
            Self::MissingSpread => f.write_str("pre-summarized point is missing its SD/SEM/%CV spread value"),
            Self::MissingSampleSize => f.write_str("pre-summarized point is missing sample size N"),
            Self::SampleSizeBelowOne { n } => write!(f, "pre-summarized sample size N must be >= 1, got {n}"),
            Self::NegativeSpread { spread } => write!(f, "pre-summarized spread must be >= 0, got {spread}"),
        }
    }
}

impl std::error::Error for SummaryPointError {}

/// Structured validation and mutation errors produced by `lumencraft-doc`.
#[derive(Debug, Clone, PartialEq)]
pub enum DocError {
    EmptyIdentifier,
    TooManyRows { count: usize, max: usize },
    TooManyDataSets { count: usize, max: usize },
    TooManySubcolumns { count: usize, max: usize },
    TooManySheets { count: usize, max: usize },
    TitleTooLong { byte_len: usize, max: usize },
    InvalidSubcolumnCount { count: usize },
    IncompatibleSubcolumnMode { archetype: TableKind, mode: SubcolumnMode },
    SheetFrozen { sheet_id: SheetId },
    CellOutOfBounds { row: usize, col: usize, subcol: usize },
    InvalidCellValue { archetype: TableKind, row: usize, col: usize, reason: &'static str },
    SheetNotFound { sheet_id: SheetId },
    DuplicateSheetId { sheet_id: SheetId },
    InvalidConfidenceLevel { percent: f64 },
    InvalidAxisSegments { count: usize },
    IncompatibleRetargetTable { analysis_title: String, target_kind: TableKind },
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentifier => f.write_str("identifier must not be empty"),
            Self::TooManyRows { count, max } => write!(f, "row count {count} exceeds maximum {max}"),
            Self::TooManyDataSets { count, max } => write!(f, "column count {count} exceeds maximum {max}"),
            Self::TooManySubcolumns { count, max } => write!(f, "subcolumn count {count} exceeds maximum {max}"),
            Self::TooManySheets { count, max } => write!(f, "project sheet count {count} exceeds maximum {max}"),
            Self::TitleTooLong { byte_len, max } => write!(f, "title length {byte_len} bytes exceeds maximum {max}"),
            Self::InvalidSubcolumnCount { count } => write!(f, "subcolumn count must be >= 1, got {count}"),
            Self::IncompatibleSubcolumnMode { archetype, mode } => {
                write!(f, "subcolumn mode {mode:?} is not valid for {archetype:?} tables")
            }
            Self::SheetFrozen { sheet_id } => write!(f, "data sheet `{sheet_id}` is frozen against edits"),
            Self::CellOutOfBounds { row, col, subcol } => {
                write!(f, "cell coordinate (row={row}, col={col}, subcol={subcol}) is out of bounds")
            }
            Self::InvalidCellValue { archetype, row, col, reason } => {
                write!(f, "invalid {archetype:?} cell at (row={row}, col={col}): {reason}")
            }
            Self::SheetNotFound { sheet_id } => write!(f, "sheet `{sheet_id}` not found in project"),
            Self::DuplicateSheetId { sheet_id } => write!(f, "duplicate sheet identifier `{sheet_id}`"),
            Self::InvalidConfidenceLevel { percent } => {
                write!(f, "confidence level must be finite in [0.01, 99.99], got {percent}")
            }
            Self::InvalidAxisSegments { count } => {
                write!(f, "axis must contain 1, 2, or 3 segments, got {count}")
            }
            Self::IncompatibleRetargetTable { analysis_title, target_kind } => {
                write!(f, "analysis `{analysis_title}` is not compatible with {target_kind:?} tables")
            }
        }
    }
}

impl std::error::Error for DocError {}
