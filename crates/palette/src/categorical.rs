//! License-clean, colorblind-safe categorical palettes from published scientific literature:
//! - Okabe & Ito (2008), *Color Universal Design (CUD)* / Wong (2011), *Nature Methods* 8:441.
//! - Paul Tol (2021), *Colour Schemes*, SRON/EPS/TN/09-002 (Bright, Muted, High-Contrast, Vibrant).
//! - Publication monochrome grayscale steps for print journals.

use crate::cvd::{CvdKind, simulate_cvd};
use crate::space::Rgba8;

// Named Okabe–Ito (2008) universal-design colour swatches.
pub const OKABE_ITO_BLUE: Rgba8 = Rgba8::rgb(0x00, 0x72, 0xB2);
pub const OKABE_ITO_VERMILION: Rgba8 = Rgba8::rgb(0xD5, 0x5E, 0x00);
pub const OKABE_ITO_BLUISH_GREEN: Rgba8 = Rgba8::rgb(0x00, 0x9E, 0x73);
pub const OKABE_ITO_ORANGE: Rgba8 = Rgba8::rgb(0xE6, 0x9F, 0x00);
pub const OKABE_ITO_SKY_BLUE: Rgba8 = Rgba8::rgb(0x56, 0xB4, 0xE9);
pub const OKABE_ITO_REDDISH_PURPLE: Rgba8 = Rgba8::rgb(0xCC, 0x79, 0xA7);
pub const OKABE_ITO_YELLOW: Rgba8 = Rgba8::rgb(0xF0, 0xE4, 0x42);
pub const OKABE_ITO_BLACK: Rgba8 = Rgba8::rgb(0x00, 0x00, 0x00);

/// All 8 Okabe–Ito (2008) colorblind-safe swatches ordered so multi-series plots begin with
/// high-contrast chromatic series on white figure paper.
pub const OKABE_ITO: &[Rgba8] = &[
    OKABE_ITO_BLUE,
    OKABE_ITO_VERMILION,
    OKABE_ITO_BLUISH_GREEN,
    OKABE_ITO_ORANGE,
    OKABE_ITO_SKY_BLUE,
    OKABE_ITO_REDDISH_PURPLE,
    OKABE_ITO_YELLOW,
    OKABE_ITO_BLACK,
];

/// Paul Tol's 7-colour Bright qualitative scheme (SRON/EPS/TN/09-002).
pub const PAUL_TOL_BRIGHT: &[Rgba8] = &[
    Rgba8::rgb(0x44, 0x77, 0xAA), // blue
    Rgba8::rgb(0xEE, 0x66, 0x77), // red/rose
    Rgba8::rgb(0x22, 0x88, 0x33), // green
    Rgba8::rgb(0xCC, 0xBB, 0x44), // yellow
    Rgba8::rgb(0x66, 0xCC, 0xEE), // cyan
    Rgba8::rgb(0xAA, 0x33, 0x77), // purple
    Rgba8::rgb(0xBB, 0xBB, 0xBB), // grey
];

/// Paul Tol's 9-colour Muted qualitative scheme (SRON/EPS/TN/09-002).
pub const PAUL_TOL_MUTED: &[Rgba8] = &[
    Rgba8::rgb(0x33, 0x22, 0x88), // indigo
    Rgba8::rgb(0x88, 0xCC, 0xEE), // cyan
    Rgba8::rgb(0x44, 0xAA, 0x99), // teal
    Rgba8::rgb(0x11, 0x77, 0x33), // green
    Rgba8::rgb(0x99, 0x99, 0x33), // olive
    Rgba8::rgb(0xDD, 0xCC, 0x77), // sand
    Rgba8::rgb(0xCC, 0x66, 0x77), // rose
    Rgba8::rgb(0x88, 0x22, 0x55), // wine
    Rgba8::rgb(0xAA, 0x44, 0x99), // purple
];

/// Paul Tol's 3-colour High-Contrast qualitative scheme (also distinct in monochrome print).
pub const PAUL_TOL_HIGH_CONTRAST: &[Rgba8] = &[
    Rgba8::rgb(0x00, 0x44, 0x88), // dark blue
    Rgba8::rgb(0xDD, 0xAA, 0x33), // ochre
    Rgba8::rgb(0xBB, 0x55, 0x66), // red
];

/// Paul Tol's 7-colour Vibrant qualitative scheme (SRON/EPS/TN/09-002).
pub const PAUL_TOL_VIBRANT: &[Rgba8] = &[
    Rgba8::rgb(0x00, 0x77, 0xBB), // blue
    Rgba8::rgb(0x33, 0xBB, 0xEE), // cyan
    Rgba8::rgb(0x00, 0x99, 0x88), // teal
    Rgba8::rgb(0xEE, 0x77, 0x33), // orange
    Rgba8::rgb(0xCC, 0x33, 0x11), // red
    Rgba8::rgb(0xEE, 0x33, 0x77), // magenta
    Rgba8::rgb(0xBB, 0xBB, 0xBB), // grey
];

/// Monochrome grayscale palette for black-and-white print journal submission.
pub const MONOCHROME_PRINT: &[Rgba8] = &[
    Rgba8::rgb(0x14, 0x14, 0x14),
    Rgba8::rgb(0x52, 0x52, 0x52),
    Rgba8::rgb(0x85, 0x85, 0x85),
    Rgba8::rgb(0xB8, 0xB8, 0xB8),
    Rgba8::rgb(0x33, 0x33, 0x33),
    Rgba8::rgb(0x6B, 0x6B, 0x6B),
    Rgba8::rgb(0x9E, 0x9E, 0x9E),
    Rgba8::rgb(0xD1, 0xD1, 0xD1),
];

/// Identifier for built-in qualitative scientific colour palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaletteId {
    #[default]
    OkabeIto,
    PaulTolBright,
    PaulTolMuted,
    PaulTolHighContrast,
    PaulTolVibrant,
    MonochromePrint,
}

impl PaletteId {
    pub const ALL: [Self; 6] =
        [Self::OkabeIto, Self::PaulTolBright, Self::PaulTolMuted, Self::PaulTolHighContrast, Self::PaulTolVibrant, Self::MonochromePrint];

    /// Human-readable display title.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OkabeIto => "Okabe-Ito (Colorblind-Safe)",
            Self::PaulTolBright => "Paul Tol Bright",
            Self::PaulTolMuted => "Paul Tol Muted",
            Self::PaulTolHighContrast => "Paul Tol High-Contrast",
            Self::PaulTolVibrant => "Paul Tol Vibrant",
            Self::MonochromePrint => "Monochrome Print",
        }
    }

    /// Ordered colour swatches in this palette (guaranteed non-empty).
    #[must_use]
    pub const fn swatches(self) -> &'static [Rgba8] {
        match self {
            Self::OkabeIto => OKABE_ITO,
            Self::PaulTolBright => PAUL_TOL_BRIGHT,
            Self::PaulTolMuted => PAUL_TOL_MUTED,
            Self::PaulTolHighContrast => PAUL_TOL_HIGH_CONTRAST,
            Self::PaulTolVibrant => PAUL_TOL_VIBRANT,
            Self::MonochromePrint => MONOCHROME_PRINT,
        }
    }

    /// Deterministically pick a series colour by cyclic 0-based index.
    #[must_use]
    pub fn color_at(self, index: usize) -> Rgba8 {
        let list = self.swatches();
        let Some(&first) = list.first() else {
            return Rgba8::BLACK;
        };
        list.get(index % list.len()).copied().unwrap_or(first)
    }

    /// Materialize a [`CategoricalPalette`] descriptor.
    #[must_use]
    pub fn descriptor(self) -> CategoricalPalette {
        CategoricalPalette { id: self, name: self.label(), swatches: self.swatches().to_vec() }
    }
}

/// Qualitative colour palette (either built-in or user-customized).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoricalPalette {
    pub id: PaletteId,
    pub name: &'static str,
    pub swatches: Vec<Rgba8>,
}

impl CategoricalPalette {
    /// Deterministically fetch a swatch by cyclic 0-based series index, falling back to black if empty.
    #[must_use]
    pub fn color_at(&self, index: usize) -> Rgba8 {
        if self.swatches.is_empty() {
            return Rgba8::BLACK;
        }
        let slot = index % self.swatches.len();
        self.swatches.get(slot).copied().unwrap_or(Rgba8::BLACK)
    }

    /// Return a copy of this palette simulated under a colour-vision deficiency at `severity` in `[0.0, 1.0]`.
    #[must_use]
    pub fn simulate_cvd(&self, kind: CvdKind, severity: f64) -> Self {
        Self { id: self.id, name: self.name, swatches: self.swatches.iter().map(|&c| simulate_cvd(c, kind, severity)).collect() }
    }

    /// Minimum pairwise perceptual `Delta E_OK` distance across all distinct swatches (`0.0` if `< 2` swatches).
    #[must_use]
    pub fn min_pairwise_oklab_distance(&self) -> f64 {
        if self.swatches.len() < 2 {
            return 0.0;
        }
        let labs: Vec<_> = self.swatches.iter().map(|c| c.to_oklab()).collect();
        let mut min_d = f64::INFINITY;
        for (i, a) in labs.iter().enumerate() {
            for b in labs.iter().skip(i + 1) {
                min_d = min_d.min(a.distance(*b));
            }
        }
        if min_d.is_finite() { min_d } else { 0.0 }
    }
}
