//! Scientific graph scene compiler for LumenCraft: maps typed data tables, error-bar summaries
//! (SD, SEM, 95% CI, range, IQR), fitted regression curves, survival step functions, segmented axes
//! and pairwise significance brackets into device-independent figure geometry.
//!
//! Layer L2 domain crate. Governing spec: `docs/graphing-and-layouts.md` and `docs/graphing-parity.md`.
#![forbid(unsafe_code)]

/// High-level graph families corresponding to typed scientific data tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphFamily {
    XyScatterAndLine,
    ColumnBarBoxViolin,
    GroupedComparison,
    ContingencyBar,
    SurvivalKaplanMeier,
    PartsOfWhole,
    MultipleVariablesBubbleHeatmap,
    NestedSubcolumn,
}

#[cfg(test)]
mod tests {
    use super::GraphFamily;

    #[test]
    fn eight_graph_families_defined() {
        assert_ne!(GraphFamily::XyScatterAndLine, GraphFamily::SurvivalKaplanMeier);
    }
}
