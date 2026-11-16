//! Publication PDF and Encapsulated PostScript figure and statistical report writer for
//! CurveBook.
//!
//! Layer L3 crate. Governing spec: `docs/graphing-and-layouts.md` (§6) and `docs/file-format-parity.md`.
#![forbid(unsafe_code)]

pub mod writer;

use serde::{Deserialize, Serialize};

pub use writer::{
    MAX_PDF_OUTPUT_BYTES, MAX_PDF_PAGES, MAX_PDF_TOTAL_PRIMITIVES, PdfError, PdfExportOptions, render_pdf, render_pdf_page_refs, render_pdf_pages,
};

/// Supported PDF specification version headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PdfVersion {
    Pdf14,
    #[default]
    Pdf17,
}

/// Supported PDF colour spaces for journal submission presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PdfColorSpace {
    Srgb,
    Cmyk,
    Grayscale,
}

#[cfg(test)]
mod tests {
    use curvebook_chart::ChartScene;
    use curvebook_palette::Rgba8;

    use super::{PdfColorSpace, PdfExportOptions, PdfVersion, render_pdf, render_pdf_pages};

    #[test]
    fn pdf_color_spaces_are_distinct() {
        assert_ne!(PdfColorSpace::Srgb, PdfColorSpace::Cmyk);
    }

    #[test]
    fn render_pdf_single_and_multipage_smoke() {
        let s1 = ChartScene::new("graph-1", "Page 1", 300.0, 200.0, Some(Rgba8::WHITE));
        let s2 = ChartScene::new("graph-2", "Page 2", 300.0, 200.0, Some(Rgba8::WHITE));
        let bytes = render_pdf(&s1, &PdfExportOptions::default()).expect("render pdf");
        assert!(bytes.starts_with(b"%PDF-1.7\n"));
        assert!(bytes.ends_with(b"%%EOF\n"));

        let opts14 = PdfExportOptions { version: PdfVersion::Pdf14, ..PdfExportOptions::default() };
        let bytes14 = render_pdf(&s1, &opts14).expect("render pdf 1.4");
        assert!(bytes14.starts_with(b"%PDF-1.4\n"));
        assert!(bytes14.ends_with(b"%%EOF\n"));

        let multi = render_pdf_pages(&[s1, s2], &PdfExportOptions::default()).expect("render multipage");
        assert!(multi.starts_with(b"%PDF-1.7\n"));
    }
}
