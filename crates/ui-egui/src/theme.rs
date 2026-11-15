//! Interface color palette, polarity presets (`Light`, `Dark`, `HighContrast`), WCAG 2.1 AA
//! contrast validation, and font registration hooks for the desktop and browser UI shell.

use lumencraft_palette::Rgba8;
use serde::{Deserialize, Serialize};

/// Minimum WCAG 2.1 AA contrast ratio for normal-sized UI and grid text (`4.5:1`).
pub const WCAG_AA_TEXT_MIN_RATIO: f64 = 4.5;

/// Minimum WCAG 2.1 AA contrast ratio for non-text borders, focus rings, and UI controls (`3.0:1`).
pub const WCAG_AA_UI_MIN_RATIO: f64 = 3.0;

/// Minimum WCAG 2.1 AAA contrast ratio for primary text (`7.0:1`).
pub const WCAG_AAA_TEXT_MIN_RATIO: f64 = 7.0;

/// Surface polarity preset for the LumenCraft application shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiThemeMode {
    #[default]
    Light,
    Dark,
    HighContrast,
}

impl UiThemeMode {
    /// All supported interface polarity presets in display order.
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::HighContrast];

    /// Construct the corresponding [`UiThemeSpec`] token palette for this mode.
    #[must_use]
    pub const fn spec(self) -> UiThemeSpec {
        UiThemeSpec::for_mode(self)
    }

    /// Localization catalog key for this theme mode label.
    #[must_use]
    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::Light => "theme.light",
            Self::Dark => "theme.dark",
            Self::HighContrast => "theme.high_contrast",
        }
    }
}

/// Complete interface chrome, canvas paper, and sheet highlight color specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiThemeSpec {
    pub mode: UiThemeMode,
    /// Chrome/sidebar background color.
    pub bg: Rgba8,
    /// Primary worksheet/card surface background color.
    pub surface: Rgba8,
    /// Alias of [`Self::bg`] retained for sidebar panel callers.
    pub panel_bg: Rgba8,
    /// Alias of [`Self::surface`] retained for worksheet surface callers.
    pub surface_bg: Rgba8,
    pub text_primary: Rgba8,
    pub text_muted: Rgba8,
    pub accent: Rgba8,
    /// Crisp publication figure canvas background (kept white by default across all UI themes).
    pub paper_white: Rgba8,
    /// Subtle structural border meeting WCAG 2.1 AA UI boundary contrast (`>= 3.0:1`).
    pub border: Rgba8,
    pub excluded_cell_text: Rgba8,
    pub link_blue: Rgba8,
    pub link_green: Rgba8,
    pub link_red: Rgba8,
    pub ui_scale: f32,
}

impl UiThemeSpec {
    /// Construct the canonical [`UiThemeSpec`] preset for `mode`.
    #[must_use]
    pub const fn for_mode(mode: UiThemeMode) -> Self {
        match mode {
            UiThemeMode::Light => Self::light(),
            UiThemeMode::Dark => Self::dark(),
            UiThemeMode::HighContrast => Self::high_contrast(),
        }
    }

    /// Light publication studio preset (`WCAG AA` compliant chrome and crisp paper canvas).
    #[must_use]
    pub const fn light() -> Self {
        let bg = Rgba8::rgb(245, 246, 248);
        let surface = Rgba8::WHITE;
        Self {
            mode: UiThemeMode::Light,
            bg,
            surface,
            panel_bg: bg,
            surface_bg: surface,
            text_primary: Rgba8::rgb(24, 28, 36),
            text_muted: Rgba8::rgb(82, 90, 104),
            accent: Rgba8::rgb(29, 78, 216),
            paper_white: Rgba8::WHITE,
            border: Rgba8::rgb(132, 140, 154),
            excluded_cell_text: Rgba8::rgb(29, 78, 216),
            link_blue: Rgba8::rgb(29, 78, 216),
            link_green: Rgba8::rgb(21, 128, 61),
            link_red: Rgba8::rgb(185, 28, 28),
            ui_scale: 1.0,
        }
    }

    /// Dark laboratory workspace preset keeping `paper_white` crisp for publication figures.
    #[must_use]
    pub const fn dark() -> Self {
        let bg = Rgba8::rgb(24, 27, 33);
        let surface = Rgba8::rgb(31, 35, 43);
        Self {
            mode: UiThemeMode::Dark,
            bg,
            surface,
            panel_bg: bg,
            surface_bg: surface,
            text_primary: Rgba8::rgb(236, 240, 245),
            text_muted: Rgba8::rgb(162, 172, 186),
            accent: Rgba8::rgb(96, 165, 250),
            paper_white: Rgba8::WHITE,
            border: Rgba8::rgb(108, 118, 134),
            excluded_cell_text: Rgba8::rgb(147, 197, 253),
            link_blue: Rgba8::rgb(96, 165, 250),
            link_green: Rgba8::rgb(74, 222, 128),
            link_red: Rgba8::rgb(248, 113, 113),
            ui_scale: 1.0,
        }
    }

    /// High-contrast accessibility preset exceeding WCAG AAA primary text contrast (`>= 18:1`)
    /// while keeping `paper_white` crisp white for publication graph previews.
    #[must_use]
    pub const fn high_contrast() -> Self {
        let bg = Rgba8::BLACK;
        let surface = Rgba8::rgb(12, 14, 18);
        Self {
            mode: UiThemeMode::HighContrast,
            bg,
            surface,
            panel_bg: bg,
            surface_bg: surface,
            text_primary: Rgba8::WHITE,
            text_muted: Rgba8::rgb(218, 224, 234),
            accent: Rgba8::rgb(125, 211, 252),
            paper_white: Rgba8::WHITE,
            border: Rgba8::rgb(210, 218, 230),
            excluded_cell_text: Rgba8::rgb(165, 243, 252),
            link_blue: Rgba8::rgb(125, 211, 252),
            link_green: Rgba8::rgb(134, 239, 172),
            link_red: Rgba8::rgb(252, 165, 165),
            ui_scale: 1.0,
        }
    }

    /// Contrast ratio of [`Self::text_primary`] against [`Self::surface`].
    #[must_use]
    pub fn primary_text_contrast(&self) -> f64 {
        wcag_contrast_ratio(self.text_primary, self.surface)
    }

    /// Contrast ratio of [`Self::text_primary`] against [`Self::bg`].
    #[must_use]
    pub fn panel_text_contrast(&self) -> f64 {
        wcag_contrast_ratio(self.text_primary, self.bg)
    }

    /// Contrast ratio of [`Self::text_muted`] against [`Self::surface`].
    #[must_use]
    pub fn muted_text_contrast(&self) -> f64 {
        wcag_contrast_ratio(self.text_muted, self.surface)
    }

    /// Contrast ratio of [`Self::accent`] against [`Self::surface`].
    #[must_use]
    pub fn accent_contrast(&self) -> f64 {
        wcag_contrast_ratio(self.accent, self.surface)
    }

    /// Whether primary text, muted text, and accent colors meet WCAG 2.1 AA (`>= 4.5:1`) on both
    /// [`Self::surface`] and [`Self::bg`].
    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        wcag_contrast_ratio(self.text_primary, self.surface) >= WCAG_AA_TEXT_MIN_RATIO
            && wcag_contrast_ratio(self.text_primary, self.bg) >= WCAG_AA_TEXT_MIN_RATIO
            && wcag_contrast_ratio(self.text_muted, self.surface) >= WCAG_AA_TEXT_MIN_RATIO
            && wcag_contrast_ratio(self.text_muted, self.bg) >= WCAG_AA_TEXT_MIN_RATIO
            && wcag_contrast_ratio(self.accent, self.surface) >= WCAG_AA_TEXT_MIN_RATIO
            && wcag_contrast_ratio(self.accent, self.bg) >= WCAG_AA_TEXT_MIN_RATIO
    }

    /// Whether structural UI borders meet WCAG 2.1 AA graphical contrast (`>= 3.0:1`).
    #[must_use]
    pub fn meets_wcag_aa_borders(&self) -> bool {
        wcag_contrast_ratio(self.border, self.surface) >= WCAG_AA_UI_MIN_RATIO && wcag_contrast_ratio(self.border, self.bg) >= WCAG_AA_UI_MIN_RATIO
    }

    /// Scale a base point size by a finite clamped `ui_scale` (`0.75..=3.0`).
    #[must_use]
    pub fn scaled_pt(&self, base_pt: f32) -> f32 {
        let base = if base_pt.is_finite() && base_pt > 0.0 { base_pt } else { 12.0 };
        let scale = if self.ui_scale.is_finite() { self.ui_scale.clamp(0.75, 3.0) } else { 1.0 };
        base * scale
    }
}

impl Default for UiThemeSpec {
    fn default() -> Self {
        Self::light()
    }
}

/// Compute the WCAG 2.1 contrast ratio (`1.0..=21.0`) between two sRGB colors.
#[must_use]
pub fn wcag_contrast_ratio(foreground: Rgba8, background: Rgba8) -> f64 {
    foreground.contrast_ratio(background)
}

/// Descriptor for an active font face compiled into `lumencraft-text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledFontEntry {
    pub name: &'static str,
    pub byte_len: usize,
}

/// Register bundled interface and scientific fonts from `lumencraft-text`.
///
/// Returns the number of non-empty bundled font faces available (`0` cleanly when compiled with
/// `--no-default-features` or when `LUMENCRAFT_FONTS_DIR` was unset at build time).
#[must_use]
pub fn install_fonts() -> usize {
    if !crate::bundled_fonts_enabled() || !lumencraft_text::has_bundled_fonts_feature() {
        return 0;
    }
    lumencraft_text::BUNDLED_FONTS.iter().filter(|(name, bytes)| !name.is_empty() && !bytes.is_empty()).count()
}

/// Inspect active bundled font face descriptors (empty when `bundled-fonts` is disabled).
#[must_use]
pub fn bundled_font_entries() -> Vec<BundledFontEntry> {
    if !crate::bundled_fonts_enabled() || !lumencraft_text::has_bundled_fonts_feature() {
        return Vec::new();
    }
    lumencraft_text::BUNDLED_FONTS
        .iter()
        .filter(|(name, bytes)| !name.is_empty() && !bytes.is_empty())
        .map(|(name, bytes)| BundledFontEntry { name, byte_len: bytes.len() })
        .collect()
}

/// Font availability and script-support diagnostics without host filesystem probing (native and WASM safe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontDiagnostics {
    pub bundled_count: usize,
    pub system_cjk_candidates: &'static [&'static str],
    pub rtl_supported: bool,
}

/// Inspect bundled font count, system CJK fallback candidates, and RTL script routing for `locale`.
#[must_use]
pub fn font_diagnostics(locale: &str) -> FontDiagnostics {
    FontDiagnostics {
        bundled_count: install_fonts(),
        system_cjk_candidates: lumencraft_text::system_cjk_font_candidates(locale),
        rtl_supported: lumencraft_text::is_rtl_locale(locale),
    }
}

/// In-memory TrueType/OpenType font collection combining embedded fallbacks and optional
/// host system font faces for multi-script glyph shaping and rasterization.
#[derive(Debug, Clone)]
pub struct FontBook {
    faces: Vec<(String, Vec<u8>)>,
}

impl Default for FontBook {
    fn default() -> Self {
        Self::with_embedded_fallback()
    }
}

impl FontBook {
    /// Load bundled fonts (when compiled with `bundled-fonts`) plus the deterministic minimal TTF fallback.
    #[must_use]
    pub fn with_embedded_fallback() -> Self {
        let mut faces = Vec::new();
        if crate::bundled_fonts_enabled() && lumencraft_text::has_bundled_fonts_feature() {
            for &(name, bytes) in lumencraft_text::BUNDLED_FONTS {
                if !name.is_empty() && lumencraft_text::TrueTypeFont::parse(bytes).is_some() {
                    faces.push((name.to_owned(), bytes.to_vec()));
                }
            }
        }
        let minimal = lumencraft_text::build_minimal_truetype_font();
        if lumencraft_text::TrueTypeFont::parse(&minimal).is_some() {
            faces.push((String::from("LumenCraft Minimal Fallback"), minimal));
        }
        Self { faces }
    }

    /// Construct a [`FontBook`] probing standard host OS system TrueType/OpenType font files when
    /// available on native targets, always retaining the embedded fallback faces.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn with_system_and_fallback() -> Self {
        const HOST_CANDIDATE_PATHS: &[&str] = &[
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
            "/System/Library/Fonts/Monaco.ttf",
            "/System/Library/Fonts/Geneva.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ];
        let mut book = Self::with_embedded_fallback();
        let _ = book.load_host_font_paths(HOST_CANDIDATE_PATHS);
        book
    }

    /// Construct a [`FontBook`] on `wasm32` using embedded fallback faces without filesystem probing.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn with_system_and_fallback() -> Self {
        Self::with_embedded_fallback()
    }

    /// Read and register valid SFNT/TrueType fonts from explicit `paths`, prepending them ahead of fallbacks.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_host_font_paths(&mut self, paths: &[&str]) -> usize {
        const MAX_HOST_FONT_BYTES: usize = 64 * 1024 * 1024;
        let mut loaded = 0usize;
        for &path in paths.iter().rev() {
            let Ok(bytes) = lumencraft_platform::fs::read_regular_file_bytes(std::path::Path::new(path), MAX_HOST_FONT_BYTES) else {
                continue;
            };
            if lumencraft_text::TrueTypeFont::parse(&bytes).is_some() {
                self.faces.insert(0, (path.to_owned(), bytes));
                loaded += 1;
            }
        }
        loaded
    }

    /// Names/paths of loaded font faces in priority order.
    #[must_use]
    pub fn face_names(&self) -> Vec<&str> {
        self.faces.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// Parse all loaded faces as zero-copy [`lumencraft_text::TrueTypeFont`] readers.
    #[must_use]
    pub fn parsed_faces(&self) -> Vec<(&str, lumencraft_text::TrueTypeFont<'_>)> {
        self.faces.iter().filter_map(|(name, bytes)| lumencraft_text::TrueTypeFont::parse(bytes).map(|ttf| (name.as_str(), ttf))).collect()
    }

    /// Shape `text` cleanly (applying BiDi visual reordering for RTL locales) and rasterize via
    /// loaded [`lumencraft_text::TrueTypeFont`] glyph outlines + vector fallback into an RGBA pixmap.
    ///
    /// Returns `(ttf_mapped_glyph_count, image)`.
    ///
    /// # Errors
    /// Returns a [`lumencraft_render::RenderError`] if scene rasterization fails for invalid `spec`.
    pub fn rasterize_shaped_text(
        &self,
        text: &str,
        locale: &str,
        font_size_pt: f64,
        spec: lumencraft_render::RenderTargetSpec,
        foreground: Rgba8,
        background: Rgba8,
    ) -> Result<(usize, lumencraft_render::RasterImage), lumencraft_render::RenderError> {
        let is_rtl = lumencraft_text::is_rtl_locale(locale);
        let direction = if is_rtl { lumencraft_text::TextDirection::Rtl } else { lumencraft_text::TextDirection::Ltr };
        let visual = lumencraft_text::reorder_visual_bidi(text, direction);
        let parsed = self.parsed_faces();

        let page_w = f64::from(spec.width_px.max(1)) * 72.0 / f64::from(spec.dpi.max(1));
        let page_h = f64::from(spec.height_px.max(1)) * 72.0 / f64::from(spec.dpi.max(1));
        let mut scene = lumencraft_chart::ChartScene::new(String::from("fontbook-qa"), String::from(locale), page_w, page_h, Some(background));

        let mut pen_x = 12.0;
        let baseline_y = (font_size_pt * 1.4).max(16.0);
        let mut ttf_mapped = 0usize;

        for ch in visual.chars() {
            if ch.is_control() {
                continue;
            }
            if let Some((adv, path)) = first_ttf_glyph_path(&parsed, ch, pen_x, baseline_y, font_size_pt) {
                if !path.commands.is_empty() {
                    scene.primitives.push(lumencraft_chart::ScenePrimitive::Path {
                        path,
                        fill: Some(lumencraft_chart::FillPaint::solid(foreground)),
                        stroke: None,
                        clip: None,
                    });
                    ttf_mapped += 1;
                }
                pen_x += adv.max(font_size_pt * 0.25);
            } else {
                let rich = lumencraft_text::RichText::plain(ch.to_string());
                let metrics = lumencraft_text::measure_text_pt(&rich, font_size_pt);
                scene.primitives.push(lumencraft_chart::ScenePrimitive::Text(lumencraft_chart::PositionedText {
                    rich,
                    anchor_pt: lumencraft_chart::PointPt::new(pen_x, baseline_y),
                    anchor: lumencraft_chart::TextAnchor::Start,
                    font_size_pt,
                    rotation_deg: 0.0,
                    color: foreground,
                }));
                pen_x += metrics.width_pt.max(font_size_pt * 0.45);
            }
        }

        let img = lumencraft_render::render_scene(&scene, spec)?;
        Ok((ttf_mapped, img))
    }
}

fn first_ttf_glyph_path(
    faces: &[(&str, lumencraft_text::TrueTypeFont<'_>)],
    ch: char,
    pen_x: f64,
    baseline_y: f64,
    font_size_pt: f64,
) -> Option<(f64, lumencraft_chart::Path2D)> {
    let (_, ttf) = faces.iter().find(|(_, f)| f.glyph_id(ch).is_some())?;
    let cmds = lumencraft_text::glyph_paths::outline_ttf_glyph(ttf, ch, pen_x, baseline_y, font_size_pt, false)?;
    let adv = ttf.advance_em(ch).unwrap_or(0.55) * font_size_pt;
    Some((adv, glyph_cmds_to_path(&cmds)))
}

fn glyph_cmds_to_path(cmds: &[lumencraft_text::GlyphOutlineCmd]) -> lumencraft_chart::Path2D {
    let mut path = lumencraft_chart::Path2D::new();
    for cmd in cmds {
        match *cmd {
            lumencraft_text::GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                path.commands.push(lumencraft_chart::PathCmd::MoveTo(lumencraft_chart::PointPt::new(x_pt, y_pt)));
            }
            lumencraft_text::GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                path.commands.push(lumencraft_chart::PathCmd::LineTo(lumencraft_chart::PointPt::new(x_pt, y_pt)));
            }
            lumencraft_text::GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                path.commands.push(lumencraft_chart::PathCmd::QuadTo {
                    control: lumencraft_chart::PointPt::new(cx_pt, cy_pt),
                    to: lumencraft_chart::PointPt::new(x_pt, y_pt),
                });
            }
            lumencraft_text::GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                path.commands.push(lumencraft_chart::PathCmd::CubicTo {
                    c1: lumencraft_chart::PointPt::new(c1x_pt, c1y_pt),
                    c2: lumencraft_chart::PointPt::new(c2x_pt, c2y_pt),
                    to: lumencraft_chart::PointPt::new(x_pt, y_pt),
                });
            }
            lumencraft_text::GlyphOutlineCmd::Close => path.commands.push(lumencraft_chart::PathCmd::Close),
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use lumencraft_palette::Rgba8;

    use super::{UiThemeMode, UiThemeSpec, WCAG_AA_TEXT_MIN_RATIO, WCAG_AAA_TEXT_MIN_RATIO, font_diagnostics, install_fonts, wcag_contrast_ratio};

    #[test]
    fn all_theme_presets_meet_wcag_aa_and_keep_paper_white() {
        for mode in UiThemeMode::ALL {
            let spec = UiThemeSpec::for_mode(mode);
            assert_eq!(spec.mode, mode);
            assert_eq!(spec.paper_white, Rgba8::WHITE);
            assert!(spec.primary_text_contrast() >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} primary contrast too low");
            assert!(spec.panel_text_contrast() >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} panel contrast too low");
            assert!(spec.muted_text_contrast() >= WCAG_AA_TEXT_MIN_RATIO, "{mode:?} muted contrast too low");
            assert!(spec.meets_wcag_aa(), "{mode:?} failed AA check");
            assert!(spec.meets_wcag_aa_borders(), "{mode:?} failed border AA check");
        }
        assert!(UiThemeSpec::high_contrast().primary_text_contrast() >= WCAG_AAA_TEXT_MIN_RATIO);
    }

    #[test]
    fn wcag_contrast_and_install_fonts_behave_cleanly() {
        assert!((wcag_contrast_ratio(Rgba8::BLACK, Rgba8::WHITE) - 21.0).abs() < 1e-9);
        assert_eq!(install_fonts(), super::bundled_font_entries().len());
    }

    #[test]
    fn font_diagnostics_reports_locale_cjk_and_rtl_without_fs_probe() {
        let en = font_diagnostics("en");
        assert_eq!(en.bundled_count, install_fonts());
        assert!(!en.rtl_supported);

        let ja = font_diagnostics("ja");
        assert!(!ja.system_cjk_candidates.is_empty());
        assert!(!ja.rtl_supported);

        let zh = font_diagnostics("zh-TW");
        assert!(!zh.system_cjk_candidates.is_empty());
        assert!(!zh.rtl_supported);

        let ar = font_diagnostics("ar");
        assert!(ar.rtl_supported);

        let he = font_diagnostics("he-IL");
        assert!(he.rtl_supported);
    }
}
