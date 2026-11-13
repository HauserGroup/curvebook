//! Native `.lumencraft` / `.lctemplate` archive serialization, delimited tabular (CSV/TSV)
//! import/export, and clean-room interoperability adapters for external scientific project
//! archives.
//!
//! Layer L3 file-format crate. Governing spec: `docs/file-format-parity.md` and `docs/data-tables-and-transforms.md`.
#![forbid(unsafe_code)]

/// Canonical file extension for native LumenCraft project archives.
pub const EXTENSION: &str = "lumencraft";

/// Canonical file extension for LumenCraft project templates.
pub const TEMPLATE_EXTENSION: &str = "lctemplate";

#[cfg(test)]
mod tests {
    use super::{EXTENSION, TEMPLATE_EXTENSION};

    #[test]
    fn native_extensions_are_non_empty() {
        assert_eq!(EXTENSION, "lumencraft");
        assert_eq!(TEMPLATE_EXTENSION, "lctemplate");
    }
}
