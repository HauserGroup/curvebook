//! Continuous perceptual colormaps (Viridis, Cividis, Plasma, Diverging Blue–Red, Grayscale) and
//! custom user colour ramps for heatmaps and multivariate bubble plots.
//!
//! Control stops derive from public-domain colormaps:
//! - Viridis & Plasma: van der Walt & Smith (CC0 1.0 Public Domain Dedication).
//! - Cividis: Nuñez, Anderton & Renslow, *PLOS ONE* 13(7):e0199239 (2018, public domain).

use crate::space::{Rgba8, lerp_oklab};

/// Exact 17-point lookup table for Viridis (`t = 0/16 .. 16/16`).
pub const VIRIDIS_STOPS: [Rgba8; 17] = [
    Rgba8::rgb(68, 1, 84),
    Rgba8::rgb(72, 24, 106),
    Rgba8::rgb(71, 45, 123),
    Rgba8::rgb(66, 64, 134),
    Rgba8::rgb(59, 82, 139),
    Rgba8::rgb(51, 99, 141),
    Rgba8::rgb(44, 114, 142),
    Rgba8::rgb(38, 130, 142),
    Rgba8::rgb(33, 145, 140),
    Rgba8::rgb(31, 160, 136),
    Rgba8::rgb(40, 174, 128),
    Rgba8::rgb(63, 188, 115),
    Rgba8::rgb(94, 201, 98),
    Rgba8::rgb(132, 212, 75),
    Rgba8::rgb(173, 220, 48),
    Rgba8::rgb(216, 226, 25),
    Rgba8::rgb(253, 231, 37),
];

/// Exact 17-point lookup table for Cividis (`t = 0/16 .. 16/16`).
pub const CIVIDIS_STOPS: [Rgba8; 17] = [
    Rgba8::rgb(0, 32, 77),
    Rgba8::rgb(0, 42, 100),
    Rgba8::rgb(20, 53, 108),
    Rgba8::rgb(46, 65, 108),
    Rgba8::rgb(66, 77, 107),
    Rgba8::rgb(84, 89, 108),
    Rgba8::rgb(101, 102, 112),
    Rgba8::rgb(118, 115, 117),
    Rgba8::rgb(135, 129, 121),
    Rgba8::rgb(153, 143, 121),
    Rgba8::rgb(172, 158, 118),
    Rgba8::rgb(191, 173, 112),
    Rgba8::rgb(210, 189, 103),
    Rgba8::rgb(228, 205, 91),
    Rgba8::rgb(245, 221, 77),
    Rgba8::rgb(254, 230, 68),
    Rgba8::rgb(255, 234, 70),
];

/// Exact 17-point lookup table for Plasma (`t = 0/16 .. 16/16`).
pub const PLASMA_STOPS: [Rgba8; 17] = [
    Rgba8::rgb(13, 8, 135),
    Rgba8::rgb(46, 5, 150),
    Rgba8::rgb(75, 3, 161),
    Rgba8::rgb(102, 0, 167),
    Rgba8::rgb(126, 3, 168),
    Rgba8::rgb(150, 19, 161),
    Rgba8::rgb(171, 36, 148),
    Rgba8::rgb(190, 56, 133),
    Rgba8::rgb(206, 75, 117),
    Rgba8::rgb(220, 95, 103),
    Rgba8::rgb(233, 116, 88),
    Rgba8::rgb(243, 138, 73),
    Rgba8::rgb(250, 161, 58),
    Rgba8::rgb(253, 185, 45),
    Rgba8::rgb(252, 210, 37),
    Rgba8::rgb(246, 235, 35),
    Rgba8::rgb(240, 249, 33),
];

/// Symmetric perceptual diverging stops (`-1 -> 0 -> +1`) for correlation matrices.
pub const DIVERGING_BLUE_RED_STOPS: [Rgba8; 9] = [
    Rgba8::rgb(0x21, 0x66, 0xAC),
    Rgba8::rgb(0x43, 0x93, 0xC3),
    Rgba8::rgb(0x92, 0xC5, 0xDE),
    Rgba8::rgb(0xD1, 0xE5, 0xF0),
    Rgba8::rgb(0xF7, 0xF7, 0xF7),
    Rgba8::rgb(0xFD, 0xDB, 0xC7),
    Rgba8::rgb(0xF4, 0xA5, 0x82),
    Rgba8::rgb(0xD6, 0x60, 0x4D),
    Rgba8::rgb(0xB2, 0x18, 0x2B),
];

/// Monochrome continuous ramp from black (`0.0`) to white (`1.0`).
pub const GRAYSCALE_STOPS: [Rgba8; 2] = [Rgba8::BLACK, Rgba8::WHITE];

/// Built-in continuous colormap identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColormapId {
    #[default]
    Viridis,
    Cividis,
    Plasma,
    DivergingBlueRed,
    Grayscale,
}

impl ColormapId {
    pub const ALL: [Self; 5] = [Self::Viridis, Self::Cividis, Self::Plasma, Self::DivergingBlueRed, Self::Grayscale];

    /// Display label for inspector dropdowns.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Viridis => "Viridis",
            Self::Cividis => "Cividis (CVD-Optimized)",
            Self::Plasma => "Plasma",
            Self::DivergingBlueRed => "Diverging Blue-Red",
            Self::Grayscale => "Grayscale",
        }
    }

    /// Reference control stops uniformly spaced on `[0.0, 1.0]`.
    #[must_use]
    pub const fn stops(self) -> &'static [Rgba8] {
        match self {
            Self::Viridis => &VIRIDIS_STOPS,
            Self::Cividis => &CIVIDIS_STOPS,
            Self::Plasma => &PLASMA_STOPS,
            Self::DivergingBlueRed => &DIVERGING_BLUE_RED_STOPS,
            Self::Grayscale => &GRAYSCALE_STOPS,
        }
    }

    /// Evaluate the colormap at normalized position `t` in `[0.0, 1.0]` using perceptual Oklab
    /// interpolation between bracketing control stops. Non-finite `t` maps safely to `0.0`.
    #[must_use]
    pub fn sample(self, t: f64) -> Rgba8 {
        sample_uniform_stops(self.stops(), t)
    }

    /// Sample `n` evenly spaced discrete colours across `[0.0, 1.0]`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn sample_discrete(self, n: usize) -> Vec<Rgba8> {
        match n {
            0 => Vec::new(),
            1 => vec![self.sample(0.5)],
            count => {
                let denom = (count - 1) as f64;
                (0..count).map(|i| self.sample((i as f64) / denom)).collect()
            }
        }
    }
}

/// Positioned colour stop in `[0.0, 1.0]` for user-defined gradients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    pub position: f64,
    pub color: Rgba8,
}

/// User-configurable continuous colour ramp (either a named colormap or custom stops).
#[derive(Debug, Clone, PartialEq)]
pub enum ColorRamp {
    Named { id: ColormapId, reversed: bool },
    Custom { stops: Vec<ColorStop> },
}

impl Default for ColorRamp {
    fn default() -> Self {
        Self::Named { id: ColormapId::Viridis, reversed: false }
    }
}

impl ColorRamp {
    /// Sample at normalized position `t` in `[0.0, 1.0]`.
    #[must_use]
    pub fn sample(&self, t: f64) -> Rgba8 {
        let clamped = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Self::Named { id, reversed } => {
                let u = if *reversed { 1.0 - clamped } else { clamped };
                id.sample(u)
            }
            Self::Custom { stops } => sample_custom_stops(stops, clamped),
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn sample_uniform_stops(stops: &[Rgba8], t: f64) -> Rgba8 {
    let Some(&first) = stops.first() else {
        return Rgba8::BLACK;
    };
    if stops.len() == 1 {
        return first;
    }
    let u = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let max_idx = stops.len() - 1;
    let scaled = u * (max_idx as f64);
    let idx = (scaled.floor() as usize).min(max_idx);
    if idx >= max_idx {
        return stops.last().copied().unwrap_or(first);
    }
    let frac = scaled - (idx as f64);
    let c0 = stops.get(idx).copied().unwrap_or(first);
    let c1 = stops.get(idx + 1).copied().unwrap_or(c0);
    lerp_oklab(c0, c1, frac)
}

fn sample_custom_stops(stops: &[ColorStop], t: f64) -> Rgba8 {
    let mut valid: Vec<ColorStop> = stops
        .iter()
        .copied()
        .filter(|s| s.position.is_finite())
        .map(|s| ColorStop { position: s.position.clamp(0.0, 1.0), color: s.color })
        .collect();
    valid.sort_by(|a, b| a.position.total_cmp(&b.position));

    let Some(&first) = valid.first() else {
        return Rgba8::BLACK;
    };
    if t <= first.position {
        return first.color;
    }
    for pair in valid.windows(2) {
        let [left, right] = pair else { continue };
        if t <= right.position {
            let span = right.position - left.position;
            if span <= 1e-12 {
                return right.color;
            }
            return lerp_oklab(left.color, right.color, (t - left.position) / span);
        }
    }
    valid.last().map_or(first.color, |last| last.color)
}
