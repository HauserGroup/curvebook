//! Numerical primitives, special functions, probability distributions and axis scale transforms
//! for LumenCraft.
//!
//! Layer L0 foundation crate: depends on no higher workspace crates.
//! Governing spec: `docs/statistics-implementation.md` (§1) and `docs/architecture.md`.
#![forbid(unsafe_code)]

/// Scale transformations supported on scientific graph axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisScale {
    Linear,
    Log10,
    Log2,
    NaturalLog,
    Reciprocal,
    Logit,
    Probability,
}

#[cfg(test)]
mod tests {
    use super::AxisScale;

    #[test]
    fn axis_scale_variants_are_distinct() {
        assert_ne!(AxisScale::Linear, AxisScale::Log10);
        assert_ne!(AxisScale::Log2, AxisScale::NaturalLog);
    }
}
