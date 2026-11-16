//! Scientific colour spaces, colorblind-safe categorical palettes, continuous heatmap colormaps,
//! colour-vision-deficiency simulation, data-point symbol glyphs, and line dash styles for
//! CurveBook.
//!
//! Layer L0 foundation crate (after `curvebook-math`).
//! Governing spec: `docs/implementation/graphing-and-layouts.md` (§5) and `docs/architecture.md`.
//!
//! ```
//! use curvebook_palette::{ColormapId, CvdKind, PaletteId, Rgba8};
//!
//! let primary = PaletteId::OkabeIto.color_at(0);
//! assert!(primary.meets_wcag_aa(Rgba8::WHITE));
//! assert!(PaletteId::OkabeIto.descriptor().simulate_cvd(CvdKind::Deuteranopia, 1.0).min_pairwise_oklab_distance() > 0.05);
//! assert_eq!(ColormapId::Viridis.sample(0.0), Rgba8::rgb(68, 1, 84));
//! ```
#![forbid(unsafe_code)]

pub mod categorical;
pub mod colormap;
pub mod cvd;
pub mod space;
pub mod style;

pub use categorical::{
    CategoricalPalette, MONOCHROME_PRINT, OKABE_ITO, OKABE_ITO_BLACK, OKABE_ITO_BLUE, OKABE_ITO_BLUISH_GREEN, OKABE_ITO_ORANGE,
    OKABE_ITO_REDDISH_PURPLE, OKABE_ITO_SKY_BLUE, OKABE_ITO_VERMILION, OKABE_ITO_YELLOW, PAUL_TOL_BRIGHT, PAUL_TOL_HIGH_CONTRAST, PAUL_TOL_MUTED,
    PAUL_TOL_VIBRANT, PaletteId,
};
pub use colormap::{
    CIVIDIS_STOPS, ColorRamp, ColorStop, ColormapId, DIVERGING_BLUE_RED_STOPS, GRAYSCALE_STOPS, MAX_DISCRETE_SAMPLES, PLASMA_STOPS, VIRIDIS_STOPS,
};
pub use cvd::{CanvasPreviewFilter, CvdKind, simulate_cvd, simulate_preview_filter};
pub use space::{Cmyk, LinearRgba, Oklab, Oklch, Rgba8, lerp_oklab, linear_channel_to_srgb, srgb_channel_to_linear};
pub use style::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle};

#[cfg(test)]
mod tests {
    use super::{
        CategoricalPalette, Cmyk, ColorRamp, ColorStop, ColormapId, CvdKind, LineDashStyle, LinearRgba, MAX_DISCRETE_SAMPLES, MarkerFillMode,
        MarkerShape, MarkerStyle, OKABE_ITO, Oklab, Oklch, PAUL_TOL_BRIGHT, PAUL_TOL_HIGH_CONTRAST, PAUL_TOL_MUTED, PaletteId, Rgba8, lerp_oklab,
        simulate_cvd,
    };

    #[test]
    fn opaque_rgb_sets_full_alpha() {
        let c = Rgba8::rgb(31, 119, 180);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn hex_parsing_and_formatting_round_trip() {
        let parsed = Rgba8::from_hex("#0072B2").unwrap();
        assert_eq!(parsed, Rgba8::rgb(0x00, 0x72, 0xB2));
        assert_eq!(parsed.to_hex_rgb(), "#0072B2");

        let with_alpha = Rgba8::from_hex("D55E0080").unwrap();
        assert_eq!(with_alpha, Rgba8::rgba(0xD5, 0x5E, 0x00, 0x80));
        assert_eq!(with_alpha.to_hex_rgba(), "#D55E0080");
        assert!(Rgba8::from_hex("#XYZ").is_none());
    }

    #[test]
    fn oklab_and_oklch_round_trip_within_one_lsb() {
        for &swatch in OKABE_ITO.iter().chain(PAUL_TOL_BRIGHT.iter()) {
            let round_oklab = swatch.to_oklab().to_rgba8();
            assert!(swatch.r.abs_diff(round_oklab.r) <= 1, "{swatch:?} -> {round_oklab:?}");
            assert!(swatch.g.abs_diff(round_oklab.g) <= 1, "{swatch:?} -> {round_oklab:?}");
            assert!(swatch.b.abs_diff(round_oklab.b) <= 1, "{swatch:?} -> {round_oklab:?}");

            let round_oklch = swatch.to_oklch().to_rgba8();
            assert!(swatch.r.abs_diff(round_oklch.r) <= 1);
            assert!(swatch.g.abs_diff(round_oklch.g) <= 1);
            assert!(swatch.b.abs_diff(round_oklch.b) <= 1);
        }
    }

    #[test]
    fn cmyk_round_trip_preserves_srgb_channels() {
        for &swatch in OKABE_ITO {
            let cmyk = Cmyk::from_srgb(swatch);
            let back = cmyk.to_srgb();
            assert!(swatch.r.abs_diff(back.r) <= 1);
            assert!(swatch.g.abs_diff(back.g) <= 1);
            assert!(swatch.b.abs_diff(back.b) <= 1);
        }
        assert_eq!(Cmyk::from_srgb(Rgba8::BLACK), Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 1.0 });
    }

    #[test]
    fn wcag_contrast_ratios_match_reference_bounds() {
        assert!((Rgba8::BLACK.contrast_ratio(Rgba8::WHITE) - 21.0).abs() < 1e-9);
        assert!(Rgba8::BLACK.meets_wcag_aa(Rgba8::WHITE));
        assert!(PaletteId::OkabeIto.color_at(0).meets_wcag_aa(Rgba8::WHITE));
    }

    #[test]
    fn okabe_ito_and_paul_tol_palettes_have_published_counts_and_wrap_safely() {
        assert_eq!(OKABE_ITO.len(), 8);
        assert_eq!(PAUL_TOL_BRIGHT.len(), 7);
        assert_eq!(PAUL_TOL_MUTED.len(), 9);
        assert_eq!(PAUL_TOL_HIGH_CONTRAST.len(), 3);

        for id in PaletteId::ALL {
            let desc = id.descriptor();
            assert_eq!(desc.id, Some(id));
            assert_eq!(desc.name, id.label());
            assert!(!desc.swatches.is_empty());
            assert_eq!(id.color_at(0), id.color_at(desc.swatches.len()));
            assert_eq!(desc.color_at(2), id.color_at(2));
        }
    }

    #[test]
    fn custom_categorical_palette_supports_owned_name_color_at_and_cvd() {
        let swatches = vec![Rgba8::rgb(0x1B, 0x9E, 0x77), Rgba8::rgb(0xD9, 0x5F, 0x02), Rgba8::rgb(0x75, 0x70, 0xB3)];
        let custom = CategoricalPalette::custom(String::from("Lab Assay Scheme"), swatches.clone());
        assert_eq!(custom.id, None);
        assert_eq!(custom.name, "Lab Assay Scheme");
        assert_eq!(custom.color_at(0), swatches[0]);
        assert_eq!(custom.color_at(3), swatches[0]);
        assert!(custom.min_pairwise_oklab_distance() > 0.05);

        let sim = custom.simulate_cvd(CvdKind::Deuteranopia, 1.0);
        assert_eq!(sim.id, None);
        assert_eq!(sim.name, "Lab Assay Scheme");
        assert_eq!(sim.swatches.len(), 3);
        assert_eq!(sim.color_at(1), simulate_cvd(swatches[1], CvdKind::Deuteranopia, 1.0));

        let empty = CategoricalPalette::custom("Empty", Vec::new());
        assert_eq!(empty.color_at(0), Rgba8::BLACK);
        assert!((empty.min_pairwise_oklab_distance() - 0.0).abs() < 1e-12);
    }

    #[test]
    fn sample_discrete_clamps_unbounded_counts_to_max() {
        let clamped = ColormapId::Viridis.sample_discrete(usize::MAX);
        assert_eq!(clamped.len(), MAX_DISCRETE_SAMPLES);
        assert_eq!(clamped.first().copied(), Some(ColormapId::Viridis.sample(0.0)));
        assert_eq!(clamped.last().copied(), Some(ColormapId::Viridis.sample(1.0)));
    }

    #[test]
    fn viridis_cividis_and_plasma_increase_monotonically_in_perceptual_lightness() {
        for cmap in [ColormapId::Viridis, ColormapId::Cividis, ColormapId::Plasma, ColormapId::Grayscale] {
            let samples = cmap.sample_discrete(17);
            assert_eq!(samples.len(), 17);
            for pair in samples.windows(2) {
                let l0 = pair[0].to_oklab().l;
                let l1 = pair[1].to_oklab().l;
                assert!(l1 >= l0 - 1e-3, "{cmap:?}: lightness decreased from {l0} to {l1}");
            }
        }
        assert_eq!(ColormapId::Viridis.sample(0.0), Rgba8::rgb(68, 1, 84));
        assert_eq!(ColormapId::Viridis.sample(1.0), Rgba8::rgb(253, 231, 37));
        assert!(ColormapId::Viridis.sample_discrete(0).is_empty());
        assert_eq!(ColormapId::Viridis.sample_discrete(1).len(), 1);
    }

    #[test]
    fn custom_color_ramp_and_non_finite_inputs_never_panic() {
        let ramp =
            ColorRamp::Custom { stops: vec![ColorStop { position: 1.0, color: Rgba8::WHITE }, ColorStop { position: 0.0, color: Rgba8::BLACK }] };
        assert_eq!(ramp.sample(0.0), Rgba8::BLACK);
        assert_eq!(ramp.sample(1.0), Rgba8::WHITE);
        let mid = ramp.sample(f64::NAN);
        assert_eq!(mid, Rgba8::BLACK);
        let _ = lerp_oklab(Rgba8::BLACK, Rgba8::WHITE, f64::INFINITY);
    }

    #[test]
    fn cvd_simulation_preserves_neutral_grays_and_distinguishes_okabe_ito_pairs() {
        let gray = Rgba8::rgb(128, 128, 128);
        for kind in [CvdKind::Protanopia, CvdKind::Deuteranopia, CvdKind::Tritanopia, CvdKind::Achromatopsia] {
            let sim = simulate_cvd(gray, kind, 1.0);
            assert!(sim.r.abs_diff(128) <= 1 && sim.g.abs_diff(128) <= 1 && sim.b.abs_diff(128) <= 1);
        }
        // Okabe–Ito blue vs vermilion remain perceptually separated under full deuteranopia.
        let blue_deut = simulate_cvd(PaletteId::OkabeIto.color_at(0), CvdKind::Deuteranopia, 1.0).to_oklab();
        let verm_deut = simulate_cvd(PaletteId::OkabeIto.color_at(1), CvdKind::Deuteranopia, 1.0).to_oklab();
        assert!(blue_deut.distance(verm_deut) > 0.12);
    }

    #[test]
    fn marker_and_dash_styles_expose_expected_defaults() {
        let m = MarkerStyle::default();
        assert_eq!(m.shape, MarkerShape::Circle);
        assert_eq!(m.fill_mode, MarkerFillMode::Filled);
        assert!(LineDashStyle::Solid.dash_multipliers().is_empty());
        assert_eq!(LineDashStyle::Dashed.dash_multipliers().len(), 2);
    }

    #[test]
    fn palette_and_style_types_round_trip_via_serde_json() {
        fn assert_roundtrip<T>(value: &T)
        where
            T: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
        {
            let json = serde_json::to_string(value).unwrap();
            let decoded: T = serde_json::from_str(&json).unwrap();
            assert_eq!(&decoded, value);
        }

        let rgba = Rgba8::rgba(0x1B, 0x9E, 0x77, 0xCC);
        assert_roundtrip(&rgba);
        let lin: LinearRgba = rgba.to_linear();
        assert_roundtrip(&lin);
        let lab: Oklab = rgba.to_oklab();
        assert_roundtrip(&lab);
        let lch: Oklch = rgba.to_oklch();
        assert_roundtrip(&lch);
        let cmyk: Cmyk = rgba.to_cmyk();
        assert_roundtrip(&cmyk);

        for id in PaletteId::ALL {
            assert_roundtrip(&id);
            assert_roundtrip(&id.descriptor());
        }
        assert_roundtrip(&CategoricalPalette::custom("Custom", vec![rgba, Rgba8::WHITE]));

        for cmap in ColormapId::ALL {
            assert_roundtrip(&cmap);
            assert_roundtrip(&ColorRamp::Named { id: cmap, reversed: true });
        }
        assert_roundtrip(&ColorRamp::Custom {
            stops: vec![ColorStop { position: 0.0, color: Rgba8::BLACK }, ColorStop { position: 1.0, color: rgba }],
        });

        for cvd in [CvdKind::Protanopia, CvdKind::Deuteranopia, CvdKind::Tritanopia, CvdKind::Achromatopsia] {
            assert_roundtrip(&cvd);
        }
        for shape in [
            MarkerShape::Circle,
            MarkerShape::Square,
            MarkerShape::TriangleUp,
            MarkerShape::TriangleDown,
            MarkerShape::Diamond,
            MarkerShape::Cross,
            MarkerShape::Plus,
            MarkerShape::Star,
            MarkerShape::Hexagon,
            MarkerShape::HorizontalBar,
            MarkerShape::VerticalBar,
        ] {
            assert_roundtrip(&shape);
        }
        for fill_mode in [
            MarkerFillMode::Filled,
            MarkerFillMode::Open,
            MarkerFillMode::SemiFilledTop,
            MarkerFillMode::SemiFilledBottom,
            MarkerFillMode::SemiFilledLeft,
            MarkerFillMode::SemiFilledRight,
        ] {
            assert_roundtrip(&fill_mode);
        }
        assert_roundtrip(&MarkerStyle {
            shape: MarkerShape::Diamond,
            fill_mode: MarkerFillMode::SemiFilledLeft,
            size_pt: 6.25,
            fill_color: rgba,
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.25,
        });
        for dash in [
            LineDashStyle::Solid,
            LineDashStyle::Dashed,
            LineDashStyle::Dotted,
            LineDashStyle::DashDot,
            LineDashStyle::DashDotDot,
            LineDashStyle::LongDash,
        ] {
            assert_roundtrip(&dash);
        }
    }

    #[test]
    fn canvas_preview_filter_simulates_cvd_and_bt709_grayscale() {
        use super::{CanvasPreviewFilter, simulate_preview_filter};

        let orange = Rgba8::rgba(230, 159, 0, 180);
        assert_eq!(simulate_preview_filter(orange, CanvasPreviewFilter::Normal), orange);
        let gray = simulate_preview_filter(orange, CanvasPreviewFilter::GrayscalePrint);
        assert!(gray.r.abs_diff(gray.g) <= 1 && gray.g.abs_diff(gray.b) <= 1);
        assert_eq!(gray.a, 180);

        let deut = simulate_preview_filter(orange, CanvasPreviewFilter::Deuteranopia);
        assert_eq!(deut, simulate_cvd(orange, CvdKind::Deuteranopia, 1.0));
        assert_eq!(CanvasPreviewFilter::from_name("grayscale-print"), Some(CanvasPreviewFilter::GrayscalePrint));
        assert_eq!(CanvasPreviewFilter::from_name("deuteranopia"), Some(CanvasPreviewFilter::Deuteranopia));
        assert_eq!(CanvasPreviewFilter::from_name("invalid"), None);
    }
}
