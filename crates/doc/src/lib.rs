//! Document model for LumenCraft projects: sheet families (`Data`, `Info`, `Analyses`, `Graphs`,
//! `Layouts`), the eight typed scientific data-table schemas, replicate subcolumns, cell-exclusion
//! masks, and reactive links between tables, analyses and figures.
//!
//! Layer L1 crate: pure data structures and structural sharing, free of statistical solvers or
//! rendering backends. Governing spec: `docs/data-tables-and-transforms.md`.
#![forbid(unsafe_code)]

/// The eight typed scientific data-table families in a LumenCraft project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Xy,
    Column,
    Grouped,
    Contingency,
    Survival,
    PartsOfWhole,
    MultipleVariables,
    Nested,
}

/// Top-level sheet sections in a LumenCraft project navigator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetSection {
    Data,
    Info,
    Analyses,
    Graphs,
    Layouts,
}

#[cfg(test)]
mod tests {
    use super::{SheetSection, TableKind};

    #[test]
    fn eight_table_families_defined() {
        let all = [
            TableKind::Xy,
            TableKind::Column,
            TableKind::Grouped,
            TableKind::Contingency,
            TableKind::Survival,
            TableKind::PartsOfWhole,
            TableKind::MultipleVariables,
            TableKind::Nested,
        ];
        assert_eq!(all.len(), 8);
        assert_ne!(SheetSection::Data, SheetSection::Graphs);
    }
}
