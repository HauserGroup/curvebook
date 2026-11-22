//! Perceptual colour spaces (sRGB, linear RGB, Oklab, OkLCH), print CMYK conversion, and WCAG 2.1
//! contrast helpers.

/// RGBA colour in 8-bit sRGB channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    /// Construct an opaque sRGB colour.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: u8::MAX }
    }

    /// Construct an sRGB colour with explicit alpha.
    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Return a copy of this colour with replaced alpha channel.
    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Parse `#RRGGBB`, `RRGGBB`, `#RRGGBBAA`, or `RRGGBBAA` hex notation.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        let raw = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
        match raw.len() {
            6 => {
                let r = u8::from_str_radix(raw.get(0..2)?, 16).ok()?;
                let g = u8::from_str_radix(raw.get(2..4)?, 16).ok()?;
                let b = u8::from_str_radix(raw.get(4..6)?, 16).ok()?;
                Some(Self::rgb(r, g, b))
            }
            8 => {
                let r = u8::from_str_radix(raw.get(0..2)?, 16).ok()?;
                let g = u8::from_str_radix(raw.get(2..4)?, 16).ok()?;
                let b = u8::from_str_radix(raw.get(4..6)?, 16).ok()?;
                let a = u8::from_str_radix(raw.get(6..8)?, 16).ok()?;
                Some(Self::rgba(r, g, b, a))
            }
            _ => None,
        }
    }

    /// Format as uppercase `#RRGGBB`.
    #[must_use]
    pub fn to_hex_rgb(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Format as uppercase `#RRGGBBAA`.
    #[must_use]
    pub fn to_hex_rgba(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    /// Convert from 8-bit sRGB into linear-light `[0.0, 1.0]` RGBA.
    #[must_use]
    pub fn to_linear(self) -> LinearRgba {
        LinearRgba {
            r: srgb_channel_to_linear(f64::from(self.r) / 255.0),
            g: srgb_channel_to_linear(f64::from(self.g) / 255.0),
            b: srgb_channel_to_linear(f64::from(self.b) / 255.0),
            a: f64::from(self.a) / 255.0,
        }
    }

    /// Convert to perceptual Oklab coordinates (Ottosson 2020).
    #[must_use]
    pub fn to_oklab(self) -> Oklab {
        self.to_linear().to_oklab()
    }

    /// Convert to cylindrical perceptual OkLCH coordinates.
    #[must_use]
    pub fn to_oklch(self) -> Oklch {
        self.to_oklab().to_oklch()
    }

    /// Convert to device CMYK for print PDF export.
    #[must_use]
    pub fn to_cmyk(self) -> Cmyk {
        Cmyk::from_srgb(self)
    }

    /// WCAG 2.1 relative luminance in `[0.0, 1.0]`.
    #[must_use]
    pub fn relative_luminance(self) -> f64 {
        let lin = self.to_linear();
        (0.2126 * lin.r + 0.7152 * lin.g + 0.0722 * lin.b).clamp(0.0, 1.0)
    }

    /// WCAG 2.1 contrast ratio in `[1.0, 21.0]` between two opaque colours.
    #[must_use]
    pub fn contrast_ratio(self, other: Self) -> f64 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (lighter + 0.05) / (darker + 0.05)
    }

    /// Whether normal-sized text meets WCAG 2.1 AA contrast (`>= 4.5:1`).
    #[must_use]
    pub fn meets_wcag_aa(self, background: Self) -> bool {
        self.contrast_ratio(background) >= 4.5
    }

    /// Whether large text or graphical strokes meet WCAG 2.1 AA contrast (`>= 3.0:1`).
    #[must_use]
    pub fn meets_wcag_aa_large(self, background: Self) -> bool {
        self.contrast_ratio(background) >= 3.0
    }
}

impl Default for Rgba8 {
    fn default() -> Self {
        Self::BLACK
    }
}

/// Linear-light RGBA colour with channels nominally in `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearRgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl LinearRgba {
    /// Clamp and encode linear-light channels into 8-bit sRGB.
    #[must_use]
    pub fn to_rgba8(self) -> Rgba8 {
        Rgba8 {
            r: unit_to_u8(linear_channel_to_srgb(self.r)),
            g: unit_to_u8(linear_channel_to_srgb(self.g)),
            b: unit_to_u8(linear_channel_to_srgb(self.b)),
            a: unit_to_u8(sanitize_unit(self.a)),
        }
    }

    /// Convert linear sRGB into Björn Ottosson's Oklab perceptual colour space (public domain, 2020).
    #[must_use]
    pub fn to_oklab(self) -> Oklab {
        let r = sanitize_finite(self.r);
        let g = sanitize_finite(self.g);
        let b = sanitize_finite(self.b);

        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();

        Oklab {
            l: 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
            a: 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
            b: 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
            alpha: sanitize_unit(self.a),
        }
    }
}

/// Perceptual Oklab colour (`l` in `[0, 1]`, opponent axes `a` and `b`, `alpha` in `[0, 1]`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Oklab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
    pub alpha: f64,
}

impl Oklab {
    /// Convert Oklab back to linear sRGB.
    #[must_use]
    pub fn to_linear_rgba(self) -> LinearRgba {
        let l_val = sanitize_finite(self.l);
        let a_val = sanitize_finite(self.a);
        let b_val = sanitize_finite(self.b);

        let l_ = l_val + 0.396_337_777_4 * a_val + 0.215_803_757_3 * b_val;
        let m_ = l_val - 0.105_561_345_8 * a_val - 0.063_854_172_8 * b_val;
        let s_ = l_val - 0.089_484_177_5 * a_val - 1.291_485_548_0 * b_val;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        LinearRgba {
            r: 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            g: -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            b: -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
            a: sanitize_unit(self.alpha),
        }
    }

    /// Convert Oklab to clamped 8-bit sRGB.
    #[must_use]
    pub fn to_rgba8(self) -> Rgba8 {
        self.to_linear_rgba().to_rgba8()
    }

    /// Convert to cylindrical OkLCH (`h_deg` in `[0, 360)`).
    #[must_use]
    pub fn to_oklch(self) -> Oklch {
        let a = sanitize_finite(self.a);
        let b = sanitize_finite(self.b);
        let c = a.hypot(b);
        let raw_h_deg = if c <= 1e-12 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
        let h_deg = if raw_h_deg >= 360.0 || raw_h_deg == 0.0 { 0.0 } else { raw_h_deg };
        Oklch { l: sanitize_finite(self.l), c, h_deg, alpha: sanitize_unit(self.alpha) }
    }

    /// Euclidean perceptual distance (`Delta E_OK`) between two Oklab colours.
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        let dl = sanitize_finite(self.l) - sanitize_finite(other.l);
        let da = sanitize_finite(self.a) - sanitize_finite(other.a);
        let db = sanitize_finite(self.b) - sanitize_finite(other.b);
        (dl * dl + da * da + db * db).sqrt()
    }
}

/// Cylindrical OkLCH colour (`l` lightness, `c` chroma, `h_deg` hue in degrees `[0, 360)`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h_deg: f64,
    pub alpha: f64,
}

impl Oklch {
    /// Convert cylindrical OkLCH into Cartesian Oklab.
    #[must_use]
    pub fn to_oklab(self) -> Oklab {
        let c = sanitize_finite(self.c).max(0.0);
        let rad = sanitize_finite(self.h_deg).to_radians();
        Oklab { l: sanitize_finite(self.l), a: c * rad.cos(), b: c * rad.sin(), alpha: sanitize_unit(self.alpha) }
    }

    /// Convert cylindrical OkLCH to clamped 8-bit sRGB.
    #[must_use]
    pub fn to_rgba8(self) -> Rgba8 {
        self.to_oklab().to_rgba8()
    }
}

/// Print CMYK colour with ink coverages in `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cmyk {
    pub c: f64,
    pub m: f64,
    pub y: f64,
    pub k: f64,
}

impl Cmyk {
    /// Construct a clamped CMYK ink specification.
    #[must_use]
    pub fn new(c: f64, m: f64, y: f64, k: f64) -> Self {
        Self { c: sanitize_unit(c), m: sanitize_unit(m), y: sanitize_unit(y), k: sanitize_unit(k) }
    }

    /// Convert 8-bit sRGB to device CMYK using grey-component replacement.
    #[must_use]
    pub fn from_srgb(color: Rgba8) -> Self {
        let r = f64::from(color.r) / 255.0;
        let g = f64::from(color.g) / 255.0;
        let b = f64::from(color.b) / 255.0;
        let max_rgb = r.max(g).max(b);
        if max_rgb <= 1e-12 {
            return Self { c: 0.0, m: 0.0, y: 0.0, k: 1.0 };
        }
        let k = (1.0 - max_rgb).clamp(0.0, 1.0);
        let denom = 1.0 - k;
        Self { c: ((1.0 - r - k) / denom).clamp(0.0, 1.0), m: ((1.0 - g - k) / denom).clamp(0.0, 1.0), y: ((1.0 - b - k) / denom).clamp(0.0, 1.0), k }
    }

    /// Convert device CMYK to opaque 8-bit sRGB.
    #[must_use]
    pub fn to_srgb(self) -> Rgba8 {
        let c = sanitize_unit(self.c);
        let m = sanitize_unit(self.m);
        let y = sanitize_unit(self.y);
        let k = sanitize_unit(self.k);
        let scale = 1.0 - k;
        Rgba8::rgb(unit_to_u8((1.0 - c) * scale), unit_to_u8((1.0 - m) * scale), unit_to_u8((1.0 - y) * scale))
    }
}

/// Perceptually uniform linear interpolation in Oklab space for `t` in `[0.0, 1.0]`.
#[must_use]
pub fn lerp_oklab(start: Rgba8, end: Rgba8, t: f64) -> Rgba8 {
    let t = sanitize_unit(t);
    let a = start.to_oklab();
    let b = end.to_oklab();
    Oklab { l: a.l + (b.l - a.l) * t, a: a.a + (b.a - a.a) * t, b: a.b + (b.b - a.b) * t, alpha: a.alpha + (b.alpha - a.alpha) * t }.to_rgba8()
}

///IEC 61966-2-1 sRGB to linear-light transfer function.
#[must_use]
pub fn srgb_channel_to_linear(channel: f64) -> f64 {
    let c = sanitize_unit(channel);
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

///IEC 61966-2-1 linear-light to sRGB transfer function.
#[must_use]
pub fn linear_channel_to_srgb(channel: f64) -> f64 {
    let c = sanitize_unit(channel);
    if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn sanitize_finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn sanitize_unit(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unit_to_u8(unit: f64) -> u8 {
    let scaled = (sanitize_unit(unit) * 255.0).round();
    scaled.clamp(0.0, 255.0) as u8
}
