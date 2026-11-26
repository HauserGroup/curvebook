//! Shared SBS microplate grid geometry and well-role model (`QOL-2` / `R17.7`).
//!
//! Kept in `curvebook-doc` (`L1`) so both `curvebook-transform` (`L2`) and `curvebook-format`
//! (`L3`, versioned `.cbplate` persistence) share one canonical type definition without cross-layer
//! duplication.

use serde::{Deserialize, Serialize};

use crate::error::DocError;

/// Maximum number of wells in any supported SBS microplate (`384`).
pub const MAX_PLATE_WELLS: usize = PlateGeometry::Wells384.well_count();

/// Standard SBS microplate grid geometries (`96`-well `8 × 12` or `384`-well `16 × 24`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlateGeometry {
    /// 8 rows (`A`–`H`) × 12 columns (`1`–`12`) = 96 wells.
    Wells96,
    /// 16 rows (`A`–`P`) × 24 columns (`1`–`24`) = 384 wells.
    Wells384,
}

impl PlateGeometry {
    /// Number of plate rows (`8` for 96-well, `16` for 384-well).
    #[must_use]
    pub const fn rows(self) -> usize {
        match self {
            Self::Wells96 => 8,
            Self::Wells384 => 16,
        }
    }

    /// Number of plate columns (`12` for 96-well, `24` for 384-well).
    #[must_use]
    pub const fn cols(self) -> usize {
        match self {
            Self::Wells96 => 12,
            Self::Wells384 => 24,
        }
    }

    /// Total number of wells (`96` or `384`).
    #[must_use]
    pub const fn well_count(self) -> usize {
        self.rows() * self.cols()
    }

    /// Row-major well index for zero-based `(row, col)`.
    pub fn well_index(self, row: usize, col: usize) -> Result<usize, DocError> {
        if row >= self.rows() || col >= self.cols() {
            return Err(DocError::CellOutOfBounds { row, col, subcol: 0 });
        }
        Ok(row * self.cols() + col)
    }

    /// Human-readable canonical plate well label such as `"A01"` or `"P24"`.
    pub fn well_label(self, row: usize, col: usize) -> Result<String, DocError> {
        let _ = self.well_index(row, col)?;
        let row_byte = u8::try_from(row).map_err(|_| DocError::CellOutOfBounds { row, col, subcol: 0 })?;
        let row_char = char::from(b'A'.saturating_add(row_byte));
        Ok(format!("{row_char}{:02}", col + 1))
    }

    /// Human-readable canonical well label for a row-major flat index `0..self.well_count()`.
    pub fn label_for_index(self, idx: usize) -> Result<String, DocError> {
        if idx >= self.well_count() {
            return Err(DocError::CellOutOfBounds { row: idx / self.cols(), col: idx % self.cols(), subcol: 0 });
        }
        let cols = self.cols();
        self.well_label(idx / cols, idx % cols)
    }

    /// Parse a canonical or unpadded well identifier (`"A01"`, `"A1"`, `"P24"`, case-insensitive)
    /// into its zero-based row-major index for this plate geometry, returning `None` if invalid or
    /// outside this geometry.
    #[must_use]
    pub fn parse_well_id(self, raw: &str) -> Option<usize> {
        let trimmed = raw.trim();
        let mut bytes = trimmed.bytes();
        let row_b = bytes.next()?.to_ascii_uppercase();
        if !row_b.is_ascii_uppercase() {
            return None;
        }
        let row = usize::from(row_b - b'A');
        if row >= self.rows() {
            return None;
        }
        let col_str = trimmed.get(1..)?;
        if col_str.is_empty() || !col_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let col_one: usize = col_str.parse().ok()?;
        if col_one == 0 || col_one > self.cols() {
            return None;
        }
        Some(row * self.cols() + (col_one - 1))
    }
}

/// Experimental role assigned to each well in a microplate layout template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlateWellRole {
    /// Experimental treatment sample with group/compound name, *X* concentration/dilution, and replicate index.
    Treatment {
        /// Compound or treatment group column title on the mapped output [`crate::DataSheet`].
        group_name: String,
        /// Concentration, dilution, or time *X* coordinate assigned to this well (finite `f64`).
        dilution_x: f64,
        /// Zero-based replicate subcolumn index within `(group_name, dilution_x)`.
        replicate: usize,
    },
    /// Background blank well used optionally for mean blank subtraction.
    Blank,
    /// Assay positive control well.
    PositiveControl,
    /// Excluded or unused well whose reading must never enter treatment groups or blank means.
    Excluded,
}
