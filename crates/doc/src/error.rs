//! Bounded dimension limits, confidence-level validation, and structured error types for
//! `curvebook-doc` (`docs/implementation/data-tables-and-transforms.md` §1.3 & §2.1).

use std::fmt;

pub use curvebook_math::ConfidenceLevel;

use crate::ids::SheetId;
use crate::layout::LayoutItemLimitError;
use crate::project::SheetFolderKind;
use crate::{SubcolumnMode, TableKind};

/// Hard ceiling on rows per data table (`docs/implementation/data-tables-and-transforms.md` §1.3).
pub const MAX_ROWS: usize = 1_000_000;

/// Hard ceiling on data-set / variable columns per table (`docs/implementation/data-tables-and-transforms.md` §1.3).
pub const MAX_DATA_SETS: usize = 1_024;

/// Hard ceiling on replicate subcolumns per data set (`docs/implementation/data-tables-and-transforms.md` §1.3).
pub const MAX_SUBCOLUMNS: usize = 512;

/// Hard ceiling on UTF-8 byte length of sheet, axis, row, and column titles.
pub const MAX_TITLE_BYTES: usize = 4_096;

/// Hard ceiling on UTF-8 byte length of user-supplied [`SheetId`] identifiers.
pub const MAX_SHEET_ID_BYTES: usize = 256;

/// Maximum total sheets across all five navigator sections in a single project.
pub const MAX_SHEETS_PER_PROJECT: usize = 500;

/// Hard ceiling on total allocated [`crate::Cell`] slots within a single data table sheet.
pub const MAX_SHEET_CELLS: usize = 30_000_000;

/// Hard ceiling on cumulative allocated [`crate::Cell`] slots across all data sheets in one project.
pub const MAX_PROJECT_CELLS: usize = 50_000_000;

/// Current `.curvebook` document schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 6;

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

/// Validate a total cell count against [`MAX_SHEET_CELLS`].
pub fn validate_sheet_cells(count: usize) -> Result<(), DocError> {
    if count > MAX_SHEET_CELLS {
        return Err(DocError::TooManyCells { count, max: MAX_SHEET_CELLS });
    }
    Ok(())
}

/// Validate a rectangular table allocation `(rows * cols * subcolumns) + (rows * extra_cols_per_row)`
/// against [`MAX_SHEET_CELLS`] with overflow-checked arithmetic prior to heap allocation.
pub fn validate_cell_grid(rows: usize, cols: usize, subcolumns: usize, extra_cols_per_row: usize) -> Result<usize, DocError> {
    let overflow = || DocError::TooManyCells { count: usize::MAX, max: MAX_SHEET_CELLS };
    let per_col = rows.checked_mul(subcolumns).ok_or_else(overflow)?;
    let data_cells = per_col.checked_mul(cols).ok_or_else(overflow)?;
    let extra_cells = rows.checked_mul(extra_cols_per_row).ok_or_else(overflow)?;
    let total = data_cells.checked_add(extra_cells).ok_or_else(overflow)?;
    validate_sheet_cells(total)?;
    Ok(total)
}

/// Validate that adding `added_cells` to a project already holding `existing_cells`
/// remains within [`MAX_PROJECT_CELLS`].
pub fn validate_project_cells(existing_cells: usize, added_cells: usize) -> Result<usize, DocError> {
    let total = existing_cells.checked_add(added_cells).ok_or(DocError::TooManyCells { count: usize::MAX, max: MAX_PROJECT_CELLS })?;
    if total > MAX_PROJECT_CELLS {
        return Err(DocError::TooManyCells { count: total, max: MAX_PROJECT_CELLS });
    }
    Ok(total)
}

/// Construct a validated [`ConfidenceLevel`] in `[0.01%, 99.99%]`, mapping invalid inputs to
/// [`DocError::InvalidConfidenceLevel`].
pub fn validate_confidence_level(percent: f64) -> Result<ConfidenceLevel, DocError> {
    ConfidenceLevel::from_percent(percent).map_err(|_| DocError::InvalidConfidenceLevel { percent })
}

/// Validation failure when reconstructing within-point parametric moments from pre-summarized
/// subcolumns (`MeanSdN`, `MeanSemN`, `MeanCvN`; `docs/implementation/data-tables-and-transforms.md` §2.1).
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryPointError {
    /// Subcolumn mode does not encode parametric `(Mean, SD/SEM/%CV, N)` moments.
    NotParametricSummaryMode {
        /// Non-parametric or raw-replicate [`SubcolumnMode`] passed to [`crate::SummarizedPoint::reconstruct`].
        mode: SubcolumnMode,
    },
    /// Mean value was entered, but the spread (`SD` / `SEM` / `%CV`) subcolumn is missing or non-numeric.
    MissingSpread,
    /// Mean value was entered, but sample size `N` is missing or non-numeric.
    MissingSampleSize,
    /// Sample size `N` is `< 1.0` or non-integer.
    SampleSizeBelowOne {
        /// Invalid sample size `N` read from the third subcolumn.
        n: f64,
    },
    /// Entered spread (`SD`, `SEM`, or `%CV`) is negative or overflows moment reconstruction.
    NegativeSpread {
        /// Offending spread value read from the second subcolumn.
        spread: f64,
    },
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

/// Structured validation and mutation errors produced by `curvebook-doc`.
#[derive(Debug, Clone, PartialEq)]
pub enum DocError {
    /// Identifier (`SheetId`, constant name, or formula key) was empty or whitespace-only.
    EmptyIdentifier,
    /// Identifier violated syntax or UTF-8 length constraints (`MAX_SHEET_ID_BYTES`, whitespace/control characters, or reserved `:`/`{{`/`}}` hook delimiters).
    InvalidIdentifier {
        /// Static explanation of which identifier rule failed.
        reason: &'static str,
    },
    /// Requested table row count exceeded [`MAX_ROWS`].
    TooManyRows {
        /// Requested row count.
        count: usize,
        /// Maximum permitted row count ([`MAX_ROWS`]).
        max: usize,
    },
    /// Requested data-set or variable column count exceeded [`MAX_DATA_SETS`].
    TooManyDataSets {
        /// Requested column count.
        count: usize,
        /// Maximum permitted column count ([`MAX_DATA_SETS`]).
        max: usize,
    },
    /// Requested replicate subcolumn count exceeded [`MAX_SUBCOLUMNS`].
    TooManySubcolumns {
        /// Requested subcolumn count.
        count: usize,
        /// Maximum permitted subcolumn count ([`MAX_SUBCOLUMNS`]).
        max: usize,
    },
    /// Requested sheet or project cell allocation exceeded [`MAX_SHEET_CELLS`] or [`MAX_PROJECT_CELLS`] (or overflowed `usize`).
    TooManyCells {
        /// Requested total cell slots (`usize::MAX` on integer overflow).
        count: usize,
        /// Applicable per-sheet or per-project cell ceiling.
        max: usize,
    },
    /// Requested layout item count on a [`crate::LayoutSpec`] exceeded [`crate::MAX_LAYOUT_ITEMS`].
    TooManyLayoutItems {
        /// Attempted number of layout items.
        count: usize,
        /// Maximum permitted layout items ([`crate::MAX_LAYOUT_ITEMS`]).
        max: usize,
    },
    /// Total sheet count across all navigator sections exceeded [`MAX_SHEETS_PER_PROJECT`].
    TooManySheets {
        /// Attempted total sheet count.
        count: usize,
        /// Maximum permitted sheets per project ([`MAX_SHEETS_PER_PROJECT`]).
        max: usize,
    },
    /// Title or label UTF-8 byte length exceeded [`MAX_TITLE_BYTES`].
    TitleTooLong {
        /// Actual UTF-8 byte length of the title.
        byte_len: usize,
        /// Maximum permitted UTF-8 byte length ([`MAX_TITLE_BYTES`]).
        max: usize,
    },
    /// Subcolumn count was `0` (`1..=MAX_SUBCOLUMNS` is required).
    InvalidSubcolumnCount {
        /// Offending zero subcolumn count.
        count: usize,
    },
    /// Requested [`SubcolumnMode`] is disallowed for the target [`TableKind`] archetype.
    IncompatibleSubcolumnMode {
        /// Table archetype rejecting the subcolumn layout.
        archetype: TableKind,
        /// Disallowed subcolumn mode.
        mode: SubcolumnMode,
    },
    /// Mutation was attempted on a [`crate::DataSheet`] with `frozen == true`.
    SheetFrozen {
        /// Identifier of the frozen data sheet.
        sheet_id: SheetId,
    },
    /// Zero-based `(row, col, subcol)` coordinate lay outside the table's allocated dimensions.
    CellOutOfBounds {
        /// Zero-based row index.
        row: usize,
        /// Zero-based data-set or variable column index.
        col: usize,
        /// Zero-based replicate or summary subcolumn index.
        subcol: usize,
    },
    /// Cell value violated archetype-specific domain invariants (for example negative counts in [`TableKind::Contingency`] or invalid censor codes in [`TableKind::Survival`]).
    InvalidCellValue {
        /// Table archetype enforcing the cell invariant.
        archetype: TableKind,
        /// Zero-based row index of the rejected cell.
        row: usize,
        /// Zero-based column index of the rejected cell.
        col: usize,
        /// Static description of the violated domain constraint.
        reason: &'static str,
    },
    /// Referenced [`SheetId`] does not exist in the project.
    SheetNotFound {
        /// Missing sheet identifier.
        sheet_id: SheetId,
    },
    /// Zero-based folder index passed to [`crate::ProjectDocument::reorder_sheet`] was `>= len`.
    SheetIndexOutOfBounds {
        /// Navigator folder whose slice bounds were exceeded.
        folder: SheetFolderKind,
        /// Out-of-bounds zero-based index.
        index: usize,
        /// Current number of sheets in `folder`.
        len: usize,
    },
    /// Attempted to insert a sheet whose [`SheetId`] already exists in the project.
    DuplicateSheetId {
        /// Colliding sheet identifier.
        sheet_id: SheetId,
    },
    /// Confidence percentage lay outside `[0.01, 99.99]` or was non-finite.
    InvalidConfidenceLevel {
        /// Offending confidence percentage.
        percent: f64,
    },
    /// Axis segment list contained `0` or `> 3` segments (`1..=3` contiguous or broken segments are supported).
    InvalidAxisSegments {
        /// Invalid segment count.
        count: usize,
    },
    /// Retargeting an [`crate::AnalysisSpec`] to a new upstream table failed because the analysis family does not accept `target_kind`.
    IncompatibleRetargetTable {
        /// Effective display title of the analysis being retargeted.
        analysis_title: String,
        /// Archetype of the incompatible target [`crate::DataSheet`].
        target_kind: TableKind,
    },
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentifier => f.write_str("identifier must not be empty"),
            Self::InvalidIdentifier { reason } => write!(f, "invalid identifier: {reason}"),
            Self::TooManyRows { count, max } => write!(f, "row count {count} exceeds maximum {max}"),
            Self::TooManyDataSets { count, max } => write!(f, "column count {count} exceeds maximum {max}"),
            Self::TooManySubcolumns { count, max } => write!(f, "subcolumn count {count} exceeds maximum {max}"),
            Self::TooManyCells { count, max } => write!(f, "sheet cell count {count} exceeds maximum {max}"),
            Self::TooManyLayoutItems { count, max } => write!(f, "layout item count {count} exceeds maximum {max}"),
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
            Self::SheetIndexOutOfBounds { folder, index, len } => {
                write!(f, "sheet index {index} out of bounds for {folder:?} folder of length {len}")
            }
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

impl From<LayoutItemLimitError> for DocError {
    fn from(err: LayoutItemLimitError) -> Self {
        Self::TooManyLayoutItems { count: err.count, max: err.max }
    }
}
