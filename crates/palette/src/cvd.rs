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
