//! Scientific typography, axis tick formatting, superscripts/subscripts, Greek symbols and p-value
//! label layout for LumenCraft.
//!
//! Layer L2 crate. Governing spec: `docs/graphing-and-layouts.md` (§3.3) and `docs/development.md` (§Fonts).
#![forbid(unsafe_code)]

pub mod glyph_paths;
pub mod markup;
pub mod metrics;
pub mod number;

pub use glyph_paths::{GlyphOutlineCmd, PositionedGlyphOutline, outline_rich_text};
pub use markup::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, expand_template_hooks, parse_markup};
pub use metrics::{TextMetricsPt, measure_text_pt};
pub use number::{LocaleNumberPolicy, format_axis_tick, format_pvalue_label};

include!(concat!(env!("OUT_DIR"), "/bundled_fonts.rs"));

/// Whether bundled fonts are compiled into this build of `lumencraft-text`.
#[must_use]
pub const fn has_bundled_fonts_feature() -> bool {
    cfg!(feature = "bundled-fonts")
}

#[cfg(test)]
mod tests {
    use lumencraft_math::{AxisScale, PValueFormat};

    use super::{
        BUNDLED_FONTS, LocaleNumberPolicy, SpanScript, SpanWeight, expand_template_hooks, format_axis_tick, format_pvalue_label, measure_text_pt,
        outline_rich_text, parse_markup,
    };

    #[test]
    fn bundled_fonts_slice_is_accessible() {
        let _ = BUNDLED_FONTS.len();
    }

    #[test]
    fn markup_metrics_and_numbers_roundtrip() {
        let rich = parse_markup("<b>IC<sub>50</sub></b> (nM)");
        assert_eq!(rich.plain_text(), "IC50 (nM)");
        assert_eq!(rich.spans.len(), 3);
        assert_eq!(rich.spans[0].weight, SpanWeight::Bold);
        assert_eq!(rich.spans[1].script, SpanScript::Subscript);

        let expanded = expand_template_hooks("Slope = {{slope}}", |key| (key == "slope").then(|| String::from("1.25")));
        assert_eq!(expanded, "Slope = 1.25");

        let bounds = measure_text_pt(&rich, 10.0);
        assert!(bounds.width_pt > 0.0);
        assert!(bounds.height_pt() > 0.0);

        let outlines = outline_rich_text(&rich, 0.0, 12.0, 10.0);
        assert_eq!(outlines.len(), rich.plain_text().chars().count());

        let policy = LocaleNumberPolicy::default();
        assert_eq!(format_axis_tick(100.0, AxisScale::Linear, &policy), "100");
        assert_eq!(format_pvalue_label(0.0023, PValueFormat::FourDecimal, true, true, &policy), "P = 0.0023 **");
    }
}
