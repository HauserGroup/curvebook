//! Verifies `curvebook-text` feature gating both with and without `bundled-fonts`.

#[test]
fn bundled_fonts_feature_matches_cfg() {
    assert_eq!(curvebook_text::has_bundled_fonts_feature(), cfg!(feature = "bundled-fonts"));
    if !cfg!(feature = "bundled-fonts") {
        assert!(curvebook_text::BUNDLED_FONTS.is_empty());
    }
}
