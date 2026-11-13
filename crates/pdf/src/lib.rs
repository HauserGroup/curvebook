//! Publication PDF and Encapsulated PostScript figure and statistical report writer for
//! LumenCraft.
//!
//! Layer L3 crate. Governing spec: `docs/graphing-and-layouts.md` (§6) and `docs/file-format-parity.md`.
#![forbid(unsafe_code)]

/// Supported PDF colour spaces for journal submission presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfColorSpace {
    Srgb,
    Cmyk,
    Grayscale,
}

#[cfg(test)]
mod tests {
    use super::PdfColorSpace;

    #[test]
    fn pdf_color_spaces_are_distinct() {
        assert_ne!(PdfColorSpace::Srgb, PdfColorSpace::Cmyk);
    }
}
