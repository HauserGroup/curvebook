//! Publication-quality SVG figure and multi-panel layout export for LumenCraft.
//!
//! Layer L3 crate. Governing spec: `docs/graphing-and-layouts.md` (§6) and `docs/file-format-parity.md`.
#![forbid(unsafe_code)]

/// Default SVG XML namespace URI.
pub const SVG_XMLNS: &str = "http://www.w3.org/2000/svg";

#[cfg(test)]
mod tests {
    use super::SVG_XMLNS;

    #[test]
    fn svg_namespace_is_standard() {
        assert!(SVG_XMLNS.starts_with("http://"));
    }
}
