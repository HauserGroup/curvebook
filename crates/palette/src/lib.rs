//! Scientific colour spaces, colorblind-safe categorical palettes, continuous heatmap colormaps,
//! data-point symbol glyphs and line dash styles for LumenCraft.
//!
//! Layer L0 foundation crate (after `lumencraft-math`).
//! Governing spec: `docs/graphing-and-layouts.md` (§5) and `docs/architecture.md`.
#![forbid(unsafe_code)]

/// RGBA colour in linear or sRGB 8-bit channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    /// Construct an opaque RGB colour.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: u8::MAX }
    }
}

#[cfg(test)]
mod tests {
    use super::Rgba8;

    #[test]
    fn opaque_rgb_sets_full_alpha() {
        let c = Rgba8::rgb(31, 119, 180);
        assert_eq!(c.a, 255);
    }
}
