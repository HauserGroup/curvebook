//! Colour-vision-deficiency (CVD) simulation in linear RGB using physiologically-based matrices
//! from Machado, Oliveira & Fernandes (2009), *IEEE Trans. Vis. Comput. Graph.* 15(6):1291–1298.

use crate::space::{LinearRgba, Rgba8};

/// Type of colour-vision deficiency simulated for figure accessibility previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CvdKind {
    /// L-cone (red-sensitive) deficiency.
    Protanopia,
    /// M-cone (green-sensitive) deficiency.
    Deuteranopia,
    /// S-cone (blue-sensitive) deficiency.
    Tritanopia,
    /// Rod monochromacy / full grayscale luminance check.
    Achromatopsia,
}

const PROTAN_MATRIX: [[f64; 3]; 3] = [[0.152_286, 1.052_583, -0.204_868], [0.114_503, 0.786_281, 0.099_216], [-0.003_882, -0.048_116, 1.051_998]];

const DEUTAN_MATRIX: [[f64; 3]; 3] = [[0.367_322, 0.860_646, -0.227_968], [0.280_085, 0.672_501, 0.047_413], [-0.011_820, 0.042_940, 0.968_881]];

const TRITAN_MATRIX: [[f64; 3]; 3] = [[1.255_528, -0.076_749, -0.178_779], [-0.078_411, 0.930_809, 0.147_602], [0.004_733, 0.691_367, 0.303_900]];

const ACHROMA_MATRIX: [[f64; 3]; 3] = [[0.212_600, 0.715_200, 0.072_200], [0.212_600, 0.715_200, 0.072_200], [0.212_600, 0.715_200, 0.072_200]];

/// Simulate colour-vision deficiency at `severity` in `[0.0, 1.0]` (`0.0` = normal trichromacy,
/// `1.0` = full dichromacy / monochromacy).
#[must_use]
pub fn simulate_cvd(color: Rgba8, kind: CvdKind, severity: f64) -> Rgba8 {
    let s = if severity.is_finite() { severity.clamp(0.0, 1.0) } else { 0.0 };
    if s <= 0.0 {
        return color;
    }
    let lin = color.to_linear();
    let mat = match kind {
        CvdKind::Protanopia => &PROTAN_MATRIX,
        CvdKind::Deuteranopia => &DEUTAN_MATRIX,
        CvdKind::Tritanopia => &TRITAN_MATRIX,
        CvdKind::Achromatopsia => &ACHROMA_MATRIX,
    };

    let sim_r = mat[0][0] * lin.r + mat[0][1] * lin.g + mat[0][2] * lin.b;
    let sim_g = mat[1][0] * lin.r + mat[1][1] * lin.g + mat[1][2] * lin.b;
    let sim_b = mat[2][0] * lin.r + mat[2][1] * lin.g + mat[2][2] * lin.b;

    LinearRgba {
        r: (lin.r + (sim_r - lin.r) * s).clamp(0.0, 1.0),
        g: (lin.g + (sim_g - lin.g) * s).clamp(0.0, 1.0),
        b: (lin.b + (sim_b - lin.b) * s).clamp(0.0, 1.0),
        a: lin.a,
    }
    .to_rgba8()
}

/// Live non-destructive canvas preview simulation filter (`QOL-9`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CanvasPreviewFilter {
    #[default]
    Normal,
    Deuteranopia,
    Protanopia,
    Tritanopia,
    GrayscalePrint,
}

impl CanvasPreviewFilter {
    /// Canonical serialization / CLI identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Deuteranopia => "deuteranopia",
            Self::Protanopia => "protanopia",
            Self::Tritanopia => "tritanopia",
            Self::GrayscalePrint => "grayscale",
        }
    }

    /// Parse a case-insensitive preview filter name.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        let norm: String = raw.trim().to_ascii_lowercase().chars().map(|c| if c == '-' { '_' } else { c }).collect();
        match norm.as_str() {
            "normal" | "none" | "off" => Some(Self::Normal),
            "deuteranopia" | "deutan" => Some(Self::Deuteranopia),
            "protanopia" | "protan" => Some(Self::Protanopia),
            "tritanopia" | "tritan" => Some(Self::Tritanopia),
            "grayscale" | "grayscale_print" | "grayscaleprint" | "achromatopsia" | "print_bw" | "bw" => Some(Self::GrayscalePrint),
            _ => None,
        }
    }
}

impl serde::Serialize for CanvasPreviewFilter {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

struct CanvasPreviewFilterVisitor;

impl serde::de::Visitor<'_> for CanvasPreviewFilterVisitor {
    type Value = CanvasPreviewFilter;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a canvas preview filter name (`normal`, `deuteranopia`, `protanopia`, `tritanopia`, `grayscale`)")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        CanvasPreviewFilter::from_name(value)
            .ok_or_else(|| E::unknown_variant(value, &["normal", "deuteranopia", "protanopia", "tritanopia", "grayscale"]))
    }
}

impl<'de> serde::Deserialize<'de> for CanvasPreviewFilter {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(CanvasPreviewFilterVisitor)
    }
}

/// Apply a [`CanvasPreviewFilter`] to an [`Rgba8`] swatch without altering alpha (`QOL-9`).
#[must_use]
pub fn simulate_preview_filter(color: Rgba8, filter: CanvasPreviewFilter) -> Rgba8 {
    match filter {
        CanvasPreviewFilter::Normal => color,
        CanvasPreviewFilter::Deuteranopia => simulate_cvd(color, CvdKind::Deuteranopia, 1.0),
        CanvasPreviewFilter::Protanopia => simulate_cvd(color, CvdKind::Protanopia, 1.0),
        CanvasPreviewFilter::Tritanopia => simulate_cvd(color, CvdKind::Tritanopia, 1.0),
        CanvasPreviewFilter::GrayscalePrint => simulate_cvd(color, CvdKind::Achromatopsia, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::CanvasPreviewFilter;

    #[test]
    fn serde_uses_canonical_lowercase_strings_and_accepts_legacy_pascal_case() {
        let canonical = [
            (CanvasPreviewFilter::Normal, "\"normal\""),
            (CanvasPreviewFilter::Deuteranopia, "\"deuteranopia\""),
            (CanvasPreviewFilter::Protanopia, "\"protanopia\""),
            (CanvasPreviewFilter::Tritanopia, "\"tritanopia\""),
            (CanvasPreviewFilter::GrayscalePrint, "\"grayscale\""),
        ];

        for (variant, expected_json) in canonical {
            let encoded = serde_json::to_string(&variant).unwrap();
            assert_eq!(encoded, expected_json);
            let decoded: CanvasPreviewFilter = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, variant);
        }

        let legacy_and_aliases = [
            ("\"Normal\"", CanvasPreviewFilter::Normal),
            ("\"Deuteranopia\"", CanvasPreviewFilter::Deuteranopia),
            ("\"Protanopia\"", CanvasPreviewFilter::Protanopia),
            ("\"Tritanopia\"", CanvasPreviewFilter::Tritanopia),
            ("\"GrayscalePrint\"", CanvasPreviewFilter::GrayscalePrint),
            ("\"grayscale-print\"", CanvasPreviewFilter::GrayscalePrint),
            ("\"grayscale_print\"", CanvasPreviewFilter::GrayscalePrint),
            ("\"achromatopsia\"", CanvasPreviewFilter::GrayscalePrint),
            ("\"bw\"", CanvasPreviewFilter::GrayscalePrint),
        ];

        for (raw_json, expected) in legacy_and_aliases {
            let decoded: CanvasPreviewFilter = serde_json::from_str(raw_json).unwrap();
            assert_eq!(decoded, expected);
        }

        assert!(serde_json::from_str::<CanvasPreviewFilter>("\"sepia\"").is_err());
        assert!(serde_json::from_str::<CanvasPreviewFilter>("123").is_err());
    }
}
