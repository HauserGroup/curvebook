//! Deterministic CPU rasterizer and compositor for LumenCraft graphs and multi-figure publication
//! layouts.
//!
//! Layer L3 rendering crate. Governing spec: `docs/graphing-and-layouts.md` (§1, §6).
#![forbid(unsafe_code)]

/// Output pixel dimensions and dots-per-inch metadata for rendered scientific figures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderTargetSpec {
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
}

#[cfg(test)]
mod tests {
    use super::RenderTargetSpec;

    #[test]
    fn publication_dpi_target_constructs() {
        let spec = RenderTargetSpec { width_px: 2400, height_px: 1800, dpi: 600 };
        assert_eq!(spec.dpi, 600);
    }
}
