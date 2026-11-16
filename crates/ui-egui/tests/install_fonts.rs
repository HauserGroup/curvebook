//! Verifies `curvebook-ui-egui` compiles and passes font setup checks with or without `bundled-fonts`.

#[test]
fn bundled_fonts_flag_matches_cfg() {
    assert_eq!(curvebook_ui_egui::bundled_fonts_enabled(), cfg!(feature = "bundled-fonts"));
}
