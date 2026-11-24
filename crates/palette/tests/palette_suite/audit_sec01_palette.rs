//! Regression tests covering `FA-SEC01-21` and `FA-SEC01-22` in `curvebook-palette`.

use curvebook_palette::{Oklab, Oklch};

#[test]
fn fa_sec01_21_oklab_to_oklch_hue_strictly_below_360() {
    // Subnormal/tiny negative `b` near +a axis makes atan2(b, a).to_degrees() in (-1e-16, 0.0),
    // where `raw + 360.0` previously rounded to exact `360.0`.
    let near_pos_a = [
        Oklab { l: 0.7, a: 0.1, b: -1e-18, alpha: 1.0 },
        Oklab { l: 0.7, a: 0.1, b: -5e-324, alpha: 1.0 },
        Oklab { l: 0.5, a: 1.0, b: -1e-17, alpha: 1.0 },
        Oklab { l: 0.5, a: 0.0, b: 0.0, alpha: 1.0 },
        Oklab { l: 0.5, a: -0.0, b: -0.0, alpha: 1.0 },
    ];
    for lab in near_pos_a {
        let lch = lab.to_oklch();
        assert!(lch.h_deg.is_finite(), "hue must be finite for {lab:?}");
        assert!(lch.h_deg >= 0.0 && lch.h_deg < 360.0, "expected h_deg in [0, 360), got {} for {lab:?}", lch.h_deg);
    }
    assert_eq!(Oklab { l: 0.7, a: 0.1, b: -1e-18, alpha: 1.0 }.to_oklch().h_deg, 0.0);
    assert_eq!(Oklab { l: 0.5, a: -0.0, b: -0.0, alpha: 1.0 }.to_oklch().h_deg, 0.0);

    // Distinctly negative angle in quadrant IV remains close to 360 yet strictly < 360.
    let q4 = Oklab { l: 0.65, a: 0.2, b: -0.001, alpha: 1.0 }.to_oklch();
    assert!(q4.h_deg > 359.0 && q4.h_deg < 360.0, "got {}", q4.h_deg);
}

#[test]
fn fa_sec01_22_palette_manifest_removes_unused_math_dep() {
    let manifest = include_str!("../../Cargo.toml");
    assert!(!manifest.contains("curvebook-math"), "unused `curvebook-math` dependency must stay removed from palette/Cargo.toml");
}

#[test]
fn da_05_05_gamut_mapped_huge_finite_chroma_preserves_gamut_boundary() {
    // Regression for DA-05-05: huge finite chroma (>= 2e9) must map to the same hue-preserving
    // sRGB gamut boundary as chroma 0.5 instead of collapsing to achromatic grey.
    for hue in [25.0, 140.0, 250.0, 320.0] {
        let boundary = Oklch::new(0.65, 0.5, hue).to_rgba8_gamut_mapped();
        let huge = Oklch::new(0.65, 1e12, hue).to_rgba8_gamut_mapped();
        let max_c = Oklch::new(0.65, f64::MAX, hue).to_rgba8_gamut_mapped();
        assert_eq!(huge, boundary, "chroma 1e12 must match boundary for hue {hue}");
        assert_eq!(max_c, boundary, "chroma f64::MAX must match boundary for hue {hue}");
        // Verify the mapped colour is chromatic (not achromatic grey).
        assert!(boundary.r != boundary.g || boundary.g != boundary.b, "gamut boundary at L=0.65, hue={hue} must remain chromatic, got {boundary:?}");
    }
}
