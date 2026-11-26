//! Colour-vision-deficiency (CVD) simulation in linear RGB using physiologically-based
//! per-severity cone-shift matrices from Machado, Oliveira & Fernandes (2009),
//! *A Physiologically-based Model for Simulation of Color Vision Deficiency*,
//! *IEEE Trans. Vis. Comput. Graph.* 15(6):1291–1298 (Table 1 / precomputed sRGB severity
//! tables `0.0, 0.1, ..., 1.0` with piecewise linear interpolation between adjacent published
//! severities for anomalous trichromacy, and ITU-R BT.709 luminance projection for achromatopsia).

use crate::space::{LinearRgba, Rgba8};

type Matrix3 = [[f64; 3]; 3];

/// Type of colour-vision deficiency simulated for figure accessibility previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CvdKind {
    /// L-cone (red-sensitive) deficiency (protanomaly / protanopia).
    Protanopia,
    /// M-cone (green-sensitive) deficiency (deuteranomaly / deuteranopia).
    Deuteranopia,
    /// S-cone (blue-sensitive) deficiency (tritanomaly / tritanopia).
    Tritanopia,
    /// Rod monochromacy / full grayscale luminance check.
    Achromatopsia,
}

const IDENTITY_MATRIX: Matrix3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Precomputed Machado, Oliveira & Fernandes (2009) linear sRGB matrices for protanomaly /
/// protanopia across severities `0.0, 0.1, ..., 1.0` (2 nm L-cone spectral shift per 0.1 step).
const PROTAN_MATRICES: [Matrix3; 11] = [
    IDENTITY_MATRIX,
    [[0.856_167, 0.182_038, -0.038_205], [0.029_342, 0.955_115, 0.015_544], [-0.002_880, -0.001_563, 1.004_443]],
    [[0.734_766, 0.334_872, -0.069_637], [0.051_840, 0.919_198, 0.028_963], [-0.004_928, -0.004_209, 1.009_137]],
    [[0.630_323, 0.465_641, -0.095_964], [0.069_181, 0.890_046, 0.040_773], [-0.006_308, -0.007_724, 1.014_032]],
    [[0.539_009, 0.579_343, -0.118_352], [0.082_546, 0.866_121, 0.051_332], [-0.007_136, -0.011_959, 1.019_095]],
    [[0.458_064, 0.679_578, -0.137_642], [0.092_785, 0.846_313, 0.060_902], [-0.007_494, -0.016_807, 1.024_301]],
    [[0.385_450, 0.769_005, -0.154_455], [0.100_526, 0.829_802, 0.069_673], [-0.007_442, -0.022_190, 1.029_632]],
    [[0.319_627, 0.849_633, -0.169_261], [0.106_241, 0.815_969, 0.077_790], [-0.007_025, -0.028_051, 1.035_076]],
    [[0.259_411, 0.923_008, -0.182_420], [0.110_296, 0.804_340, 0.085_364], [-0.006_276, -0.034_346, 1.040_622]],
    [[0.203_876, 0.990_338, -0.194_214], [0.112_975, 0.794_542, 0.092_483], [-0.005_222, -0.041_043, 1.046_265]],
    [[0.152_286, 1.052_583, -0.204_868], [0.114_503, 0.786_281, 0.099_216], [-0.003_882, -0.048_116, 1.051_998]],
];

/// Precomputed Machado, Oliveira & Fernandes (2009) linear sRGB matrices for deuteranomaly /
/// deuteranopia across severities `0.0, 0.1, ..., 1.0` (2 nm M-cone spectral shift per 0.1 step).
const DEUTAN_MATRICES: [Matrix3; 11] = [
    IDENTITY_MATRIX,
    [[0.866_435, 0.177_704, -0.044_139], [0.049_567, 0.939_063, 0.011_370], [-0.003_453, 0.007_233, 0.996_220]],
    [[0.760_729, 0.319_078, -0.079_807], [0.090_568, 0.889_315, 0.020_117], [-0.006_027, 0.013_325, 0.992_702]],
    [[0.675_425, 0.433_850, -0.109_275], [0.125_303, 0.847_755, 0.026_942], [-0.007_950, 0.018_572, 0.989_378]],
    [[0.605_511, 0.528_560, -0.134_071], [0.155_318, 0.812_366, 0.032_316], [-0.009_376, 0.023_176, 0.986_200]],
    [[0.547_494, 0.607_765, -0.155_259], [0.181_692, 0.781_742, 0.036_566], [-0.010_410, 0.027_275, 0.983_136]],
    [[0.498_864, 0.674_741, -0.173_604], [0.205_199, 0.754_872, 0.039_929], [-0.011_131, 0.030_969, 0.980_162]],
    [[0.457_771, 0.731_899, -0.189_670], [0.226_409, 0.731_012, 0.042_579], [-0.011_595, 0.034_333, 0.977_261]],
    [[0.422_823, 0.781_057, -0.203_881], [0.245_752, 0.709_602, 0.044_646], [-0.011_843, 0.037_423, 0.974_421]],
    [[0.392_952, 0.823_610, -0.216_562], [0.263_559, 0.690_210, 0.046_232], [-0.011_910, 0.040_281, 0.971_630]],
    [[0.367_322, 0.860_646, -0.227_968], [0.280_085, 0.672_501, 0.047_413], [-0.011_820, 0.042_940, 0.968_881]],
];

/// Precomputed Machado, Oliveira & Fernandes (2009) linear sRGB matrices for tritanomaly /
/// tritanopia across severities `0.0, 0.1, ..., 1.0`.
const TRITAN_MATRICES: [Matrix3; 11] = [
    IDENTITY_MATRIX,
    [[0.926_670, 0.092_514, -0.019_184], [0.021_191, 0.964_503, 0.014_306], [0.008_437, 0.054_813, 0.936_750]],
    [[0.895_720, 0.133_330, -0.029_050], [0.029_997, 0.945_400, 0.024_603], [0.013_027, 0.104_707, 0.882_266]],
    [[0.905_871, 0.127_791, -0.033_662], [0.026_856, 0.941_251, 0.031_893], [0.013_410, 0.148_296, 0.838_294]],
    [[0.948_035, 0.089_490, -0.037_526], [0.014_364, 0.946_792, 0.038_844], [0.010_853, 0.193_991, 0.795_156]],
    [[1.017_277, 0.027_029, -0.044_306], [-0.006_113, 0.958_479, 0.047_634], [0.006_379, 0.248_708, 0.744_913]],
    [[1.104_996, -0.046_633, -0.058_363], [-0.032_137, 0.971_635, 0.060_503], [0.001_336, 0.317_922, 0.680_742]],
    [[1.193_214, -0.109_812, -0.083_402], [-0.058_496, 0.979_410, 0.079_086], [-0.002_346, 0.403_492, 0.598_854]],
    [[1.257_728, -0.139_648, -0.118_081], [-0.078_003, 0.975_409, 0.102_594], [-0.003_316, 0.501_214, 0.502_102]],
    [[1.278_864, -0.125_333, -0.153_531], [-0.086_438, 0.963_215, 0.123_223], [-0.002_237, 0.601_550, 0.400_687]],
    [[1.255_528, -0.076_749, -0.178_779], [-0.078_411, 0.930_809, 0.147_602], [0.004_733, 0.691_367, 0.303_900]],
];

/// ITU-R BT.709 / sRGB luminance projection matrix for rod monochromacy (achromatopsia).
const ACHROMA_MATRIX: Matrix3 = [[0.212_600, 0.715_200, 0.072_200], [0.212_600, 0.715_200, 0.072_200], [0.212_600, 0.715_200, 0.072_200]];

fn lerp_matrix(a: Matrix3, b: Matrix3, t: f64) -> Matrix3 {
    [
        [a[0][0] + (b[0][0] - a[0][0]) * t, a[0][1] + (b[0][1] - a[0][1]) * t, a[0][2] + (b[0][2] - a[0][2]) * t],
        [a[1][0] + (b[1][0] - a[1][0]) * t, a[1][1] + (b[1][1] - a[1][1]) * t, a[1][2] + (b[1][2] - a[1][2]) * t],
        [a[2][0] + (b[2][0] - a[2][0]) * t, a[2][1] + (b[2][1] - a[2][1]) * t, a[2][2] + (b[2][2] - a[2][2]) * t],
    ]
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn machado_matrix(table: &[Matrix3; 11], severity: f64) -> Matrix3 {
    let scaled = severity.clamp(0.0, 1.0) * 10.0;
    let lo = (scaled.floor() as usize).min(10);
    let hi = (lo + 1).min(10);
    let frac = (scaled - (lo as f64)).clamp(0.0, 1.0);
    let m0 = table.get(lo).copied().unwrap_or(IDENTITY_MATRIX);
    let m1 = table.get(hi).copied().unwrap_or(m0);
    if lo == hi || frac == 0.0 { m0 } else { lerp_matrix(m0, m1, frac) }
}

fn simulation_matrix(kind: CvdKind, severity: f64) -> Matrix3 {
    match kind {
        CvdKind::Protanopia => machado_matrix(&PROTAN_MATRICES, severity),
        CvdKind::Deuteranopia => machado_matrix(&DEUTAN_MATRICES, severity),
        CvdKind::Tritanopia => machado_matrix(&TRITAN_MATRICES, severity),
        CvdKind::Achromatopsia => lerp_matrix(IDENTITY_MATRIX, ACHROMA_MATRIX, severity),
    }
}

/// Simulate colour-vision deficiency at `severity` in `[0.0, 1.0]` (`0.0` = normal trichromacy,
/// `0.1..=0.9` = anomalous trichromacy via Machado, Oliveira & Fernandes 2009 per-severity
/// cone-shift tables with piecewise interpolation between adjacent 0.1 steps, `1.0` = full
/// dichromacy / monochromacy). Non-finite severities map safely to `0.0`.
#[must_use]
pub fn simulate_cvd(color: Rgba8, kind: CvdKind, severity: f64) -> Rgba8 {
    let s = if severity.is_finite() { severity.clamp(0.0, 1.0) } else { 0.0 };
    if s <= 0.0 {
        return color;
    }
    let lin = color.to_linear();
    let mat = simulation_matrix(kind, s);

    let sim_r = mat[0][0] * lin.r + mat[0][1] * lin.g + mat[0][2] * lin.b;
    let sim_g = mat[1][0] * lin.r + mat[1][1] * lin.g + mat[1][2] * lin.b;
    let sim_b = mat[2][0] * lin.r + mat[2][1] * lin.g + mat[2][2] * lin.b;

    LinearRgba { r: sim_r.clamp(0.0, 1.0), g: sim_g.clamp(0.0, 1.0), b: sim_b.clamp(0.0, 1.0), a: lin.a }.to_rgba8()
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

    #[test]
    fn gap_03_07_machado_2009_per_severity_matrices_and_interpolation() {
        use super::{CvdKind, DEUTAN_MATRICES, PROTAN_MATRICES, TRITAN_MATRICES, simulate_cvd, simulation_matrix};
        use crate::space::{LinearRgba, Rgba8};

        // 1. Every row of every published Machado (2009) matrix sums to 1.0 (white/gray invariance).
        for (name, table) in [("protan", &PROTAN_MATRICES), ("deutan", &DEUTAN_MATRICES), ("tritan", &TRITAN_MATRICES)] {
            for (step, mat) in table.iter().enumerate() {
                for (row_idx, row) in mat.iter().enumerate() {
                    let sum: f64 = row.iter().sum();
                    assert!((sum - 1.0).abs() <= 2e-6, "{name} step={step} row={row_idx}: sum={sum}");
                }
            }
        }

        // 2. At severity 0.5, simulation uses Machado (2009)'s published 0.5 cone-shift matrix,
        //    NOT the naive linear blend `0.5 * I + 0.5 * M(1.0)`!
        let pure_red = Rgba8::rgb(255, 0, 0);
        for (kind, m05, m10) in [
            (CvdKind::Protanopia, PROTAN_MATRICES[5], PROTAN_MATRICES[10]),
            (CvdKind::Deuteranopia, DEUTAN_MATRICES[5], DEUTAN_MATRICES[10]),
            (CvdKind::Tritanopia, TRITAN_MATRICES[5], TRITAN_MATRICES[10]),
        ] {
            let got = simulate_cvd(pure_red, kind, 0.5);
            let want = LinearRgba { r: m05[0][0].clamp(0.0, 1.0), g: m05[1][0].clamp(0.0, 1.0), b: m05[2][0].clamp(0.0, 1.0), a: 1.0 }.to_rgba8();
            assert_eq!(got, want, "{kind:?} at severity 0.5 must match published Machado (2009) M(0.5)");

            let naive_r = 0.5 * (1.0 + m10[0][0]);
            assert!((m05[0][0] - naive_r).abs() > 0.08, "{kind:?}: M(0.5) must differ materially from 0.5*(I + M(1.0))");
        }

        // 3. Intermediate severity 0.55 interpolates linearly between published 0.5 and 0.6 matrices.
        let m55 = simulation_matrix(CvdKind::Deuteranopia, 0.55);
        for r in 0..3 {
            for c in 0..3 {
                let mid = 0.5 * (DEUTAN_MATRICES[5][r][c] + DEUTAN_MATRICES[6][r][c]);
                assert!((m55[r][c] - mid).abs() < 1e-12, "m55[{r}][{c}]={} vs {mid}", m55[r][c]);
            }
        }

        // 4. Neutral gray invariance holds at intermediate severities too.
        let gray = Rgba8::rgb(128, 128, 128);
        for &sev in &[0.0, 0.25, 0.5, 0.75, 1.0, f64::NAN] {
            for kind in [CvdKind::Protanopia, CvdKind::Deuteranopia, CvdKind::Tritanopia, CvdKind::Achromatopsia] {
                let sim = simulate_cvd(gray, kind, sev);
                assert!(sim.r.abs_diff(128) <= 1 && sim.g.abs_diff(128) <= 1 && sim.b.abs_diff(128) <= 1);
            }
        }
    }
}
