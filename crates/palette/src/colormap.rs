//! Continuous perceptual colormaps (Viridis, Cividis, Plasma, Diverging Blue–Red, Grayscale) and
//! custom user colour ramps for heatmaps and multivariate bubble plots.
//!
//! Control stops derive from public-domain colormaps:
//! - Viridis & Plasma: van der Walt & Smith (CC0 1.0 Public Domain Dedication).
//! - Cividis: Nuñez, Anderton & Renslow, *PLOS ONE* 13(7):e0199239 (2018, public domain).
//!
//! The Viridis, Cividis, and Plasma 17-stop tables hold exact entries of the published 256-entry
//! lookup tables: stop `i` (`t = i / 16`) is LUT entry `idx(i) = floor(i * 255 / 16 + 1/2)` (0,
//! 16, 32, 48, 64, 80, 96, 112, 128, 143, 159, 175, 191, 207, 223, 239, 255), with each `[0, 1]`
//! channel `c` quantised to `round(255 * c)`. Generated from `viridisLite` 0.4.2 `viridis.map`
//! (option `"D"` = Viridis, `"E"` = Cividis, `"C"` = Plasma) with R 4.5.2:
//! `d <- viridisLite::viridis.map; d <- d[d$opt == opt, ]; i <- floor((0:16) * 255 / 16 + 0.5) + 1;`
//! `rgb(d$R[i], d$G[i], d$B[i])`.

use crate::space::{Rgba8, lerp_oklab};

/// 17 exact entries of the published 256-entry Viridis LUT at `t = 0/16 .. 16/16` (see module docs
/// for the index rule).
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
    Rgba8::rgb(31, 159, 136),
    Rgba8::rgb(39, 173, 129),
    Rgba8::rgb(61, 188, 116),
    Rgba8::rgb(92, 200, 99),
    Rgba8::rgb(129, 211, 77),
    Rgba8::rgb(170, 220, 50),
    Rgba8::rgb(213, 226, 26),
    Rgba8::rgb(253, 231, 37),
];

/// 17 exact entries (`t = 0/16 .. 16/16`, module-doc index rule) of the published 256-entry CC0
/// Cividis LUT of Nuñez, Anderton & Renslow (*PLOS ONE* 13(7):e0199239, 2018; `viridisLite`).
pub const CIVIDIS_STOPS: [Rgba8; 17] = [
    Rgba8::rgb(0, 32, 77),
    Rgba8::rgb(0, 44, 105),
    Rgba8::rgb(6, 54, 110),
    Rgba8::rgb(45, 66, 108),
    Rgba8::rgb(65, 77, 107),
    Rgba8::rgb(82, 89, 108),
    Rgba8::rgb(97, 100, 111),
    Rgba8::rgb(111, 112, 115),
    Rgba8::rgb(125, 124, 120),
    Rgba8::rgb(139, 135, 121),
    Rgba8::rgb(155, 148, 119),
    Rgba8::rgb(172, 161, 116),
    Rgba8::rgb(188, 175, 111),
    Rgba8::rgb(206, 188, 104),
    Rgba8::rgb(224, 203, 94),
    Rgba8::rgb(242, 218, 80),
    Rgba8::rgb(255, 234, 70),
];

/// 17 exact entries (`t = 0/16 .. 16/16`, module-doc index rule) of the published 256-entry CC0
/// Plasma LUT of van der Walt & Smith (2015; Matplotlib `_plasma_data` / `viridisLite`).
pub const PLASMA_STOPS: [Rgba8; 17] = [
    Rgba8::rgb(13, 8, 135),
    Rgba8::rgb(49, 5, 151),
    Rgba8::rgb(76, 2, 161),
    Rgba8::rgb(102, 0, 167),
    Rgba8::rgb(126, 3, 168),
    Rgba8::rgb(149, 17, 161),
    Rgba8::rgb(170, 35, 149),
    Rgba8::rgb(188, 53, 135),
    Rgba8::rgb(204, 71, 120),
    Rgba8::rgb(217, 88, 106),
    Rgba8::rgb(229, 107, 93),
    Rgba8::rgb(240, 127, 79),
    Rgba8::rgb(248, 148, 65),
    Rgba8::rgb(253, 171, 51),
    Rgba8::rgb(253, 195, 40),
    Rgba8::rgb(249, 221, 37),
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

/// Hard ceiling on discrete colormap samples returned by [`ColormapId::sample_discrete`].
pub const MAX_DISCRETE_SAMPLES: usize = 4_096;

/// Built-in continuous colormap identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
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

    /// Sample `n` evenly spaced discrete colours across `[0.0, 1.0]`, clamped to
    /// [`MAX_DISCRETE_SAMPLES`].
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn sample_discrete(self, n: usize) -> Vec<Rgba8> {
        match n.min(MAX_DISCRETE_SAMPLES) {
            0 => Vec::new(),
            1 => vec![self.sample(0.5)],
            count => {
                let denom = (count - 1) as f64;
                (0..count).map(|i| self.sample((i as f64) / denom)).collect()
            }
        }
    }
}

/// Maximum number of custom colour stops permitted in a deserialized or validated ramp.
pub const MAX_CUSTOM_STOPS: usize = 256;

/// Validation errors returned when constructing or validating a [`ColorRamp::Custom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRampError {
    /// More than [`MAX_CUSTOM_STOPS`] custom stops were provided.
    TooManyStops { count: usize, max: usize },
    /// A stop had a non-finite (`NaN` or `±inf`) position.
    NonFinitePosition { index: usize },
}

impl std::fmt::Display for ColorRampError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyStops { count, max } => {
                write!(f, "custom color ramp has {count} stops, exceeding maximum of {max}")
            }
            Self::NonFinitePosition { index } => {
                write!(f, "custom color ramp stop at index {index} has a non-finite position")
            }
        }
    }
}

impl std::error::Error for ColorRampError {}

/// Positioned colour stop in `[0.0, 1.0]` for user-defined gradients.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ColorStop {
    pub position: f64,
    pub color: Rgba8,
}

/// User-configurable continuous colour ramp (either a named colormap or custom stops).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum ColorRamp {
    Named { id: ColormapId, reversed: bool },
    Custom { stops: Vec<ColorStop> },
}

impl<'de> serde::Deserialize<'de> for ColorRamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        enum ColorRampDe {
            Named {
                id: ColormapId,
                reversed: bool,
            },
            Custom {
                #[serde(deserialize_with = "deserialize_bounded_stops")]
                stops: Vec<ColorStop>,
            },
        }

        match ColorRampDe::deserialize(deserializer)? {
            ColorRampDe::Named { id, reversed } => Ok(Self::Named { id, reversed }),
            ColorRampDe::Custom { stops } => Ok(Self::Custom { stops }),
        }
    }
}

fn deserialize_bounded_stops<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<ColorStop>, D::Error> {
    struct StopsVisitor;

    impl<'de> serde::de::Visitor<'de> for StopsVisitor {
        type Value = Vec<ColorStop>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "a sequence of at most {MAX_CUSTOM_STOPS} ColorStop entries")
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let hint = seq.size_hint().unwrap_or(0).min(MAX_CUSTOM_STOPS);
            let mut stops = Vec::with_capacity(hint);
            while let Some(stop) = seq.next_element::<ColorStop>()? {
                if stops.len() >= MAX_CUSTOM_STOPS {
                    return Err(serde::de::Error::custom(ColorRampError::TooManyStops { count: stops.len() + 1, max: MAX_CUSTOM_STOPS }));
                }
                if !stop.position.is_finite() {
                    return Err(serde::de::Error::custom(ColorRampError::NonFinitePosition { index: stops.len() }));
                }
                stops.push(stop);
            }
            normalize_custom_stops(&mut stops);
            Ok(stops)
        }
    }

    deserializer.deserialize_seq(StopsVisitor)
}

impl Default for ColorRamp {
    fn default() -> Self {
        Self::Named { id: ColormapId::Viridis, reversed: false }
    }
}

impl ColorRamp {
    /// Construct and normalize a validated custom colour ramp.
    ///
    /// Rejects ramps exceeding [`MAX_CUSTOM_STOPS`] or containing non-finite stop
    /// positions; otherwise clamps positions to `[0.0, 1.0]`, stably sorts by
    /// position, and collapses redundant stops at identical positions.
    pub fn custom(mut stops: Vec<ColorStop>) -> Result<Self, ColorRampError> {
        validate_custom_stops(&stops)?;
        normalize_custom_stops(&mut stops);
        Ok(Self::Custom { stops })
    }

    /// Validate that custom stops stay within [`MAX_CUSTOM_STOPS`] and have finite positions.
    pub fn validate(&self) -> Result<(), ColorRampError> {
        match self {
            Self::Named { .. } => Ok(()),
            Self::Custom { stops } => validate_custom_stops(stops),
        }
    }

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

fn validate_custom_stops(stops: &[ColorStop]) -> Result<(), ColorRampError> {
    if stops.len() > MAX_CUSTOM_STOPS {
        return Err(ColorRampError::TooManyStops { count: stops.len(), max: MAX_CUSTOM_STOPS });
    }
    for (index, stop) in stops.iter().enumerate() {
        if !stop.position.is_finite() {
            return Err(ColorRampError::NonFinitePosition { index });
        }
    }
    Ok(())
}

fn normalize_custom_stops(stops: &mut Vec<ColorStop>) {
    for stop in stops.iter_mut() {
        let clamped = stop.position.clamp(0.0, 1.0);
        stop.position = if clamped == 0.0 { 0.0 } else { clamped };
    }
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    if stops.len() <= 1 {
        return;
    }

    let len = stops.len();
    let mut write = 0usize;
    let mut i = 0usize;
    while i < len {
        let Some(&first) = stops.get(i) else { break };
        let mut end = i + 1;
        while stops.get(end).is_some_and(|s| s.position == first.position) {
            end += 1;
        }
        let last = stops.get(end - 1).copied().unwrap_or(first);
        if let Some(slot) = stops.get_mut(write) {
            *slot = first;
            write += 1;
        }
        // Keep the trailing stop when the colour jumps at this position so hard step gradients are preserved.
        if last.color != first.color
            && let Some(slot) = stops.get_mut(write)
        {
            *slot = last;
            write += 1;
        }
        i = end;
    }
    stops.truncate(write);
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

fn is_normalized_stops(stops: &[ColorStop]) -> bool {
    let mut prev = -1.0_f64;
    for s in stops {
        let p = s.position;
        if !p.is_finite() || !(0.0..=1.0).contains(&p) || p < prev || (p == 0.0 && p.is_sign_negative()) {
            return false;
        }
        prev = p;
    }
    true
}

fn interpolate_stops(left: ColorStop, right: ColorStop, t: f64) -> Rgba8 {
    let span = right.position - left.position;
    if span <= 1e-12 {
        return right.color;
    }
    lerp_oklab(left.color, right.color, (t - left.position) / span)
}

fn sample_custom_stops(stops: &[ColorStop], t: f64) -> Rgba8 {
    if stops.is_empty() {
        return Rgba8::BLACK;
    }
    if stops.len() <= 64 {
        if is_normalized_stops(stops) {
            return sample_normalized_stops(stops, t);
        }
        return sample_custom_stops_unsorted(stops, t);
    }
    sample_large_custom_stops(stops, t)
}

fn sample_normalized_stops(stops: &[ColorStop], t: f64) -> Rgba8 {
    let idx = stops.partition_point(|s| s.position < t);
    if idx == 0 {
        return stops.first().map_or(Rgba8::BLACK, |s| s.color);
    }
    let Some(&right) = stops.get(idx) else {
        return stops.last().map_or(Rgba8::BLACK, |s| s.color);
    };
    let Some(&left) = stops.get(idx - 1) else {
        return right.color;
    };
    interpolate_stops(left, right, t)
}

#[inline(always)]
fn stop_word(s: &ColorStop) -> u64 {
    let col = (s.color.r as u64) | ((s.color.g as u64) << 8) | ((s.color.b as u64) << 16) | ((s.color.a as u64) << 24);
    s.position.to_bits() ^ col.rotate_left(17)
}

fn fingerprint_stops(stops: &[ColorStop]) -> (u64, u64) {
    let mut a0 = (stops.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut a1 = 0xBF58_476D_1CE4_E5B9_u64;
    let mut b0 = (stops.as_ptr() as usize as u64).wrapping_mul(0xA076_1D64_78BD_642F);
    let mut b1 = 0xE703_7ED1_A0B4_28DB_u64;
    let mut fold = |s: &ColorStop| {
        let w = stop_word(s);
        a0 = a0.rotate_left(7).wrapping_add(w);
        a1 = a1.rotate_left(11) ^ w.wrapping_mul(0x94D0_49BB_1331_11EB);
        b0 = b0.rotate_left(13) ^ w;
        b1 = b1.rotate_left(19).wrapping_add(w.rotate_left(29));
    };

    if stops.len() <= MAX_CUSTOM_STOPS {
        for s in stops {
            fold(s);
        }
    } else {
        let last = stops.len() - 1;
        for k in 0..MAX_CUSTOM_STOPS {
            let idx = (k * last) / (MAX_CUSTOM_STOPS - 1);
            if let Some(s) = stops.get(idx) {
                fold(s);
            }
        }
    }
    let h0 = (a0 ^ a1.rotate_left(31)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let h1 = (b0 ^ b1.rotate_left(37)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (h0, h1)
}

struct CachedRamp {
    len: usize,
    fingerprint: (u64, u64),
    normalized: Vec<ColorStop>,
}

std::thread_local! {
    static CUSTOM_RAMP_CACHE: std::cell::RefCell<Option<CachedRamp>> = const { std::cell::RefCell::new(None) };
}

fn sample_large_custom_stops(stops: &[ColorStop], t: f64) -> Rgba8 {
    let fp = fingerprint_stops(stops);
    let len = stops.len();
    let res = CUSTOM_RAMP_CACHE.try_with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            return None;
        };
        let needs_rebuild = match slot.as_ref() {
            Some(entry) => entry.len != len || entry.fingerprint != fp,
            None => true,
        };
        if needs_rebuild {
            let mut valid = Vec::with_capacity(stops.len());
            for s in stops {
                if s.position.is_finite() {
                    let clamped = s.position.clamp(0.0, 1.0);
                    let pos = if clamped == 0.0 { 0.0 } else { clamped };
                    valid.push(ColorStop { position: pos, color: s.color });
                }
            }
            valid.sort_by(|a, b| a.position.total_cmp(&b.position));
            *slot = Some(CachedRamp { len, fingerprint: fp, normalized: valid });
        }
        slot.as_ref().map(|entry| if entry.normalized.is_empty() { Rgba8::BLACK } else { sample_normalized_stops(&entry.normalized, t) })
    });
    match res {
        Ok(Some(color)) => color,
        _ => sample_custom_stops_unsorted(stops, t),
    }
}

fn sample_custom_stops_unsorted(stops: &[ColorStop], t: f64) -> Rgba8 {
    let mut best_left_pos = -1.0_f64;
    let mut best_left_idx = usize::MAX;
    let mut best_right_pos = 2.0_f64;
    let mut best_right_idx = usize::MAX;
    for (idx, s) in stops.iter().enumerate() {
        let raw = s.position;
        if !raw.is_finite() {
            continue;
        }
        let p = raw.clamp(0.0, 1.0);
        if p < t {
            if p >= best_left_pos {
                best_left_pos = p;
                best_left_idx = idx;
            }
        } else if p < best_right_pos {
            best_right_pos = p;
            best_right_idx = idx;
        }
    }
    match (stops.get(best_left_idx), stops.get(best_right_idx)) {
        (None, None) => Rgba8::BLACK,
        (None, Some(right)) => right.color,
        (Some(left), None) => left.color,
        (Some(left), Some(right)) => {
            interpolate_stops(ColorStop { position: best_left_pos, color: left.color }, ColorStop { position: best_right_pos, color: right.color }, t)
        }
    }
}
