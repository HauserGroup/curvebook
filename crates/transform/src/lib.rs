//! Data-table transformations, normalization, baseline removal, fraction-of-total, transpose,
//! row/column pruning and Monte Carlo data simulation for LumenCraft.
//!
//! Layer L2 domain crate. Governing spec: `docs/data-tables-and-transforms.md` (§4–§5).
#![forbid(unsafe_code)]

/// Built-in families of table transformations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Normalize,
    RemoveBaseline,
    Transpose,
    FractionOfTotal,
    PruneRows,
}

#[cfg(test)]
mod tests {
    use super::TransformKind;

    #[test]
    fn transform_kinds_are_distinct() {
        assert_ne!(TransformKind::Normalize, TransformKind::Transpose);
    }
}
