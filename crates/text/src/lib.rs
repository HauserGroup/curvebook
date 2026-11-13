//! Scientific typography, axis tick formatting, superscripts/subscripts, Greek symbols and p-value
//! label layout for LumenCraft.
//!
//! Layer L2 crate. Governing spec: `docs/graphing-and-layouts.md` (§3.3) and `docs/development.md` (§Fonts).
#![forbid(unsafe_code)]

include!(concat!(env!("OUT_DIR"), "/bundled_fonts.rs"));

/// Whether bundled fonts are compiled into this build of `lumencraft-text`.
#[must_use]
pub const fn has_bundled_fonts_feature() -> bool {
    cfg!(feature = "bundled-fonts")
}

#[cfg(test)]
mod tests {
    use super::BUNDLED_FONTS;

    #[test]
    fn bundled_fonts_slice_is_accessible() {
        let _ = BUNDLED_FONTS.len();
    }
}
