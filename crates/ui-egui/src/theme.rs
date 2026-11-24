//! Seed-derived OKLab/OKLCH interface colour palette (`ThemeSeeds`, `DerivedUiPalette`), polarity
//! presets (`Light`, `Dark`, `HighContrast`), WCAG 2.2 AA/AAA contrast validation, and font
//! registration hooks for the desktop and browser UI shell (`docs/design-language.md`).

use curvebook_palette::{Oklch, Rgba8, lerp_oklab, oklab_even_steps};
use serde::{Deserialize, Serialize};

/// Minimum WCAG 2.2 AA contrast ratio for normal-sized UI and grid text (`4.5:1`).
pub const WCAG_AA_TEXT_MIN_RATIO: f64 = 4.5;

/// Minimum WCAG 2.2 AA contrast ratio for non-text borders, focus rings, and UI controls (`3.0:1`).
pub const WCAG_AA_UI_MIN_RATIO: f64 = 3.0;

/// Minimum WCAG 2.2 AAA contrast ratio for primary text (`7.0:1`).
pub const WCAG_AAA_TEXT_MIN_RATIO: f64 = 7.0;

/// Canonical perceptual seeds from which all UI chrome, surface, state, and semantic tokens are
/// generated as deterministic OKLab/OKLCH steps (`docs/design-language.md` §1–§2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThemeSeeds {
    /// Cool-slate neutral seed (`c <= 0.01`) keeping chrome neutral so colour belongs to the data.
    pub neutral: Oklch,
    /// Restrained slate-indigo accent distinct from Okabe-Ito categorical series and Viridis ramps.
    pub accent: Oklch,
    /// Crimson seed for validation errors and broken external/internal data links.
    pub error: Oklch,
    /// Deep amber seed for assumption and clipping warnings.
    pub warning: Oklch,
    /// Olive-ochre seed for out-of-date/stale downstream results.
    pub stale: Oklch,
    /// Emerald hue (`deg`) for live-linked provenance borders and `FX=` formula badges.
    pub provenance_green_hue_deg: f64,
    /// Warm copper hue (`deg`) for categorical (`CAT`) variable-role badges.
    pub role_cat_hue_deg: f64,
}

impl ThemeSeeds {
    /// Canonical seed specification used across `Light`, `Dark`, and `HighContrast`.
    pub const CANONICAL: Self = Self {
        neutral: Oklch { l: 0.50, c: 0.008, h_deg: 255.0, alpha: 1.0 },
        accent: Oklch { l: 0.45, c: 0.130, h_deg: 268.0, alpha: 1.0 },
        error: Oklch { l: 0.46, c: 0.175, h_deg: 26.0, alpha: 1.0 },
        warning: Oklch { l: 0.45, c: 0.115, h_deg: 62.0, alpha: 1.0 },
        stale: Oklch { l: 0.45, c: 0.100, h_deg: 88.0, alpha: 1.0 },
        provenance_green_hue_deg: 152.0,
        role_cat_hue_deg: 46.0,
    };
}

impl Default for ThemeSeeds {
    fn default() -> Self {
        Self::CANONICAL
    }
}

/// Complete derived UI colour table generated in OKLab/OKLCH from [`ThemeSeeds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedUiPalette {
    pub mode: UiThemeMode,
    pub app_bar: Rgba8,
    pub panel_bg: Rgba8,
    pub surface_recessed: Rgba8,
    pub surface_bg: Rgba8,
    pub canvas_mat: Rgba8,
    pub header_bg: Rgba8,
    pub border: Rgba8,
    pub divider: Rgba8,
    pub hover_bg: Rgba8,
    pub hover_border: Rgba8,
    pub active_bg: Rgba8,
    pub selection_unemphasized_bg: Rgba8,
    pub focus_ring: Rgba8,
    pub input_bg: Rgba8,
    pub input_border: Rgba8,
    pub button_bg: Rgba8,
    pub button_border: Rgba8,
    pub text_primary: Rgba8,
    pub text_strong: Rgba8,
    pub text_muted: Rgba8,
    pub text_disabled: Rgba8,
    pub icon: Rgba8,
    /// Foreground accent color for labels, selected borders, focus/active indicators, and icons on `surface_bg`/`panel_bg`.
    pub accent: Rgba8,
    /// Solid primary CTA fill color reserved strictly for fills behind [`Self::on_accent`] (never dark-surface foreground text/borders).
    pub accent_strong: Rgba8,
    pub accent_hover: Rgba8,
    pub on_accent: Rgba8,
    pub paper_white: Rgba8,
    pub scrim: Rgba8,
    pub error: Rgba8,
    pub warning: Rgba8,
    pub stale: Rgba8,
    pub role_num: Rgba8,
    pub role_cat: Rgba8,
    pub role_lbl: Rgba8,
    pub role_fx: Rgba8,
    pub link_blue: Rgba8,
    pub link_green: Rgba8,
    pub link_red: Rgba8,
    pub excluded_cell_text: Rgba8,
}

impl DerivedUiPalette {
    /// Derive the full UI palette for `mode` from [`ThemeSeeds::CANONICAL`].
    #[must_use]
    pub fn for_mode(mode: UiThemeMode) -> Self {
        Self::from_seeds(mode, ThemeSeeds::CANONICAL)
    }

    /// Derive the full UI palette for `mode` as deterministic OKLab/OKLCH steps from `seeds`.
    #[must_use]
    pub fn from_seeds(mode: UiThemeMode, seeds: ThemeSeeds) -> Self {
        let nh = seeds.neutral.h_deg;
        let nc = seeds.neutral.c;
        let ah = seeds.accent.h_deg;
        let ac = seeds.accent.c;

        match mode {
            UiThemeMode::Light => {
                let dark_ink = Oklch::new(0.18, nc * 1.25, nh).to_rgba8_gamut_mapped();
                let n = oklab_even_steps(dark_ink, Rgba8::WHITE, 17);
                let panel_bg = lerp_oklab(n[15], n[16], 0.42);
                let surface_recessed = lerp_oklab(n[14], n[15], 0.35);
                let surface_bg = n[16];
                let button_bg = lerp_oklab(n[15], n[16], 0.15);
                let app_bar = n[15];
                let accent = Oklch::new(0.45, ac, ah).to_rgba8_gamut_mapped();
                let accent_strong = Oklch::new(0.42, ac * 1.05, ah).to_rgba8_gamut_mapped();
                let accent_hover = Oklch::new(0.37, ac * 1.05, ah).to_rgba8_gamut_mapped();
                let accent_wash = Oklch::new(0.83, ac * 0.48, ah).to_rgba8_gamut_mapped();
                let hover_bg = lerp_oklab(n[13], accent_wash, 0.36);
                let active_bg = lerp_oklab(n[12], accent_wash, 0.46);
                let selection_unemphasized_bg = n[14];
                let border = n[8];
                let error = Oklch::new(0.45, seeds.error.c, seeds.error.h_deg).to_rgba8_gamut_mapped();
                let warning = Oklch::new(0.45, seeds.warning.c, seeds.warning.h_deg).to_rgba8_gamut_mapped();
                let stale = Oklch::new(0.45, seeds.stale.c, seeds.stale.h_deg).to_rgba8_gamut_mapped();
                let link_green = Oklch::new(0.46, 0.125, seeds.provenance_green_hue_deg).to_rgba8_gamut_mapped();

                Self {
                    mode,
                    app_bar,
                    panel_bg,
                    surface_recessed,
                    surface_bg,
                    canvas_mat: surface_recessed,
                    header_bg: app_bar,
                    border,
                    divider: n[14],
                    hover_bg,
                    hover_border: accent,
                    active_bg,
                    selection_unemphasized_bg,
                    focus_ring: accent,
                    input_bg: surface_bg,
                    input_border: border,
                    button_bg,
                    button_border: n[7],
                    text_primary: n[1],
                    text_strong: n[0],
                    text_muted: n[3],
                    text_disabled: n[8],
                    icon: n[2],
                    accent,
                    accent_strong,
                    accent_hover,
                    on_accent: Rgba8::WHITE,
                    paper_white: Rgba8::WHITE,
                    scrim: n[0].with_alpha(128),
                    error,
                    warning,
                    stale,
                    role_num: accent,
                    role_cat: Oklch::new(0.48, 0.135, seeds.role_cat_hue_deg).to_rgba8_gamut_mapped(),
                    role_lbl: n[4],
                    role_fx: link_green,
                    link_blue: accent,
                    link_green,
                    link_red: error,
                    excluded_cell_text: accent,
                }
            }
            UiThemeMode::Dark => {
                let dark_base = Oklch::new(0.20, nc * 1.25, nh).to_rgba8_gamut_mapped();
                let light_top = Oklch::new(0.97, nc * 0.50, nh).to_rgba8_gamut_mapped();
                let n = oklab_even_steps(dark_base, light_top, 17);
                let app_bar = n[0];
                let surface_recessed = lerp_oklab(n[0], n[1], 0.30);
                let panel_bg = n[1];
                let surface_bg = n[2];
                let header_bg = n[3];
                let border = n[8];
                let accent = Oklch::new(0.76, ac * 0.92, ah).to_rgba8_gamut_mapped();
                let accent_strong = Oklch::new(0.45, ac * 1.05, ah).to_rgba8_gamut_mapped();
                let accent_hover = Oklch::new(0.40, ac * 1.05, ah).to_rgba8_gamut_mapped();
                let hover_tint = Oklch::new(0.44, ac * 0.55, ah).to_rgba8_gamut_mapped();
                let active_tint = Oklch::new(0.41, ac * 0.65, ah).to_rgba8_gamut_mapped();
                let hover_bg = lerp_oklab(n[5], hover_tint, 0.35);
                let active_bg = lerp_oklab(n[4], active_tint, 0.45);
                let selection_unemphasized_bg = n[4];
                let error = Oklch::new(0.74, seeds.error.c * 0.85, seeds.error.h_deg).to_rgba8_gamut_mapped();
                let warning = Oklch::new(0.78, seeds.warning.c * 1.05, seeds.warning.h_deg).to_rgba8_gamut_mapped();
                let stale = Oklch::new(0.79, seeds.stale.c * 1.05, seeds.stale.h_deg).to_rgba8_gamut_mapped();
                let link_green = Oklch::new(0.79, 0.135, seeds.provenance_green_hue_deg).to_rgba8_gamut_mapped();

                Self {
                    mode,
                    app_bar,
                    panel_bg,
                    surface_recessed,
                    surface_bg,
                    canvas_mat: surface_recessed,
                    header_bg,
                    border,
                    divider: n[4],
                    hover_bg,
                    hover_border: accent,
                    active_bg,
                    selection_unemphasized_bg,
                    focus_ring: accent,
                    input_bg: app_bar,
                    input_border: border,
                    button_bg: header_bg,
                    button_border: n[9],
                    text_primary: n[15],
                    text_strong: n[16],
                    text_muted: n[14],
                    text_disabled: n[8],
                    icon: n[14],
                    accent,
                    accent_strong,
                    accent_hover,
                    on_accent: Rgba8::WHITE,
                    paper_white: Rgba8::WHITE,
                    scrim: n[0].with_alpha(148),
                    error,
                    warning,
                    stale,
                    role_num: accent,
                    role_cat: Oklch::new(0.77, 0.135, seeds.role_cat_hue_deg).to_rgba8_gamut_mapped(),
                    role_lbl: n[12],
                    role_fx: link_green,
                    link_blue: accent,
                    link_green,
                    link_red: error,
                    excluded_cell_text: Oklch::new(0.81, ac * 0.78, ah).to_rgba8_gamut_mapped(),
                }
            }
            UiThemeMode::HighContrast => {
                let n = oklab_even_steps(Rgba8::BLACK, Rgba8::WHITE, 17);
                let panel_bg = n[0];
                let surface_recessed = n[1];
                let surface_bg = lerp_oklab(n[1], n[2], 0.35);
                let border = n[14];
                let accent = Oklch::new(0.85, ac * 0.85, ah).to_rgba8_gamut_mapped();
                let accent_strong = Oklch::new(0.41, ac, ah).to_rgba8_gamut_mapped();
                let accent_hover = Oklch::new(0.35, ac, ah).to_rgba8_gamut_mapped();
                let hover_tint = Oklch::new(0.38, ac * 0.65, ah).to_rgba8_gamut_mapped();
                let active_tint = Oklch::new(0.42, ac * 0.75, ah).to_rgba8_gamut_mapped();
                let hover_bg = lerp_oklab(n[5], hover_tint, 0.40);
                let active_bg = lerp_oklab(n[6], active_tint, 0.50);
                let selection_unemphasized_bg = n[4];
                let error = Oklch::new(0.83, seeds.error.c * 0.75, seeds.error.h_deg).to_rgba8_gamut_mapped();
                let warning = Oklch::new(0.86, seeds.warning.c, seeds.warning.h_deg).to_rgba8_gamut_mapped();
                let stale = Oklch::new(0.87, seeds.stale.c, seeds.stale.h_deg).to_rgba8_gamut_mapped();
                let link_green = Oklch::new(0.86, 0.130, seeds.provenance_green_hue_deg).to_rgba8_gamut_mapped();

                Self {
                    mode,
                    app_bar: panel_bg,
                    panel_bg,
                    surface_recessed,
                    surface_bg,
                    canvas_mat: surface_recessed,
                    header_bg: n[3],
                    border,
                    divider: n[12],
                    hover_bg,
                    hover_border: accent,
                    active_bg,
                    selection_unemphasized_bg,
                    focus_ring: accent,
                    input_bg: panel_bg,
                    input_border: border,
                    button_bg: n[3],
                    button_border: border,
                    text_primary: Rgba8::WHITE,
                    text_strong: Rgba8::WHITE,
                    text_muted: n[15],
                    text_disabled: n[12],
                    icon: Rgba8::WHITE,
                    accent,
                    accent_strong,
                    accent_hover,
                    on_accent: Rgba8::WHITE,
                    paper_white: Rgba8::WHITE,
                    scrim: Rgba8::BLACK.with_alpha(176),
                    error,
                    warning,
                    stale,
                    role_num: accent,
                    role_cat: Oklch::new(0.85, 0.125, seeds.role_cat_hue_deg).to_rgba8_gamut_mapped(),
                    role_lbl: n[14],
                    role_fx: link_green,
                    link_blue: accent,
                    link_green,
                    link_red: error,
                    excluded_cell_text: Oklch::new(0.88, ac * 0.70, ah).to_rgba8_gamut_mapped(),
                }
            }
        }
    }
}

/// Surface polarity preset for the CurveBook application shell.
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
    pub fn spec(self) -> UiThemeSpec {
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

fn default_light_error() -> Rgba8 {
    DerivedUiPalette::for_mode(UiThemeMode::Light).error
}

fn default_light_warning() -> Rgba8 {
    DerivedUiPalette::for_mode(UiThemeMode::Light).warning
}

fn default_light_stale() -> Rgba8 {
    DerivedUiPalette::for_mode(UiThemeMode::Light).stale
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
    /// Subtle structural border meeting WCAG 2.2 AA UI boundary contrast (`>= 3.0:1`).
    pub border: Rgba8,
    pub excluded_cell_text: Rgba8,
    pub link_blue: Rgba8,
    pub link_green: Rgba8,
    pub link_red: Rgba8,
    #[serde(default = "default_light_error")]
    pub error: Rgba8,
    #[serde(default = "default_light_warning")]
    pub warning: Rgba8,
    #[serde(default = "default_light_stale")]
    pub stale: Rgba8,
    pub ui_scale: f32,
}

impl UiThemeSpec {
    /// Construct the canonical [`UiThemeSpec`] preset for `mode` from [`DerivedUiPalette`].
    #[must_use]
    pub fn for_mode(mode: UiThemeMode) -> Self {
        let p = DerivedUiPalette::for_mode(mode);
        Self {
            mode,
            bg: p.panel_bg,
            surface: p.surface_bg,
            panel_bg: p.panel_bg,
            surface_bg: p.surface_bg,
            text_primary: p.text_primary,
            text_muted: p.text_muted,
            accent: p.accent,
            paper_white: p.paper_white,
            border: p.border,
            excluded_cell_text: p.excluded_cell_text,
            link_blue: p.link_blue,
            link_green: p.link_green,
            link_red: p.link_red,
            error: p.error,
            warning: p.warning,
            stale: p.stale,
            ui_scale: 1.0,
        }
    }

    /// Light publication studio preset (`WCAG AA` compliant chrome and crisp paper canvas).
    #[must_use]
    pub fn light() -> Self {
        Self::for_mode(UiThemeMode::Light)
    }

    /// Dark laboratory workspace preset keeping `paper_white` crisp for publication figures.
    #[must_use]
    pub fn dark() -> Self {
        Self::for_mode(UiThemeMode::Dark)
    }

    /// High-contrast accessibility preset exceeding WCAG AAA primary text contrast (`>= 18:1`)
    /// while keeping `paper_white` crisp white for publication graph previews.
    #[must_use]
    pub fn high_contrast() -> Self {
        Self::for_mode(UiThemeMode::HighContrast)
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

    /// Whether primary text, muted text, accent, and semantic status colors meet WCAG 2.2 AA
    /// (`>= 4.5:1`) on both [`Self::surface`] and [`Self::bg`].
    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        [self.text_primary, self.text_muted, self.accent, self.error, self.warning, self.stale, self.link_blue, self.link_green, self.link_red]
            .into_iter()
            .all(|fg| fg.meets_wcag_aa(self.surface) && fg.meets_wcag_aa(self.bg))
    }

    /// Whether primary text, muted text, accent, and semantic status colors meet WCAG 2.2 AAA (`>= 7.0:1`).
    #[must_use]
    pub fn meets_wcag_aaa(&self) -> bool {
        [self.text_primary, self.text_muted, self.accent, self.error, self.warning, self.stale]
            .into_iter()
            .all(|fg| fg.meets_wcag_aaa(self.surface) && fg.meets_wcag_aaa(self.bg))
    }

    /// Whether structural UI borders meet WCAG 2.2 AA graphical contrast (`>= 3.0:1`).
    #[must_use]
    pub fn meets_wcag_aa_borders(&self) -> bool {
        self.border.meets_wcag_aa_large(self.surface) && self.border.meets_wcag_aa_large(self.bg)
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

/// Compute the WCAG 2.2 contrast ratio (`1.0..=21.0`) between two sRGB colors.
#[must_use]
pub fn wcag_contrast_ratio(foreground: Rgba8, background: Rgba8) -> f64 {
    foreground.contrast_ratio(background)
}

/// Descriptor for an active font face compiled into `curvebook-text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledFontEntry {
    pub name: &'static str,
    pub byte_len: usize,
}

/// Register bundled interface and scientific fonts from `curvebook-text`.
///
/// Returns the number of non-empty bundled font faces available (`0` cleanly when compiled with
/// `--no-default-features` or when `CURVEBOOK_FONTS_DIR` was unset at build time).
#[must_use]
pub fn install_fonts() -> usize {
    if !crate::bundled_fonts_enabled() || !curvebook_text::has_bundled_fonts_feature() {
        return 0;
    }
    curvebook_text::BUNDLED_FONTS.iter().filter(|(name, bytes)| !name.is_empty() && !bytes.is_empty()).count()
}

/// Inspect active bundled font face descriptors (empty when `bundled-fonts` is disabled).
#[must_use]
pub fn bundled_font_entries() -> Vec<BundledFontEntry> {
    if !crate::bundled_fonts_enabled() || !curvebook_text::has_bundled_fonts_feature() {
        return Vec::new();
    }
    curvebook_text::BUNDLED_FONTS
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
        system_cjk_candidates: curvebook_text::system_cjk_font_candidates(locale),
        rtl_supported: curvebook_text::is_rtl_locale(locale),
    }
}

#[derive(Debug, Clone)]
struct FontFaceEntry {
    name: String,
    bytes: std::sync::Arc<[u8]>,
    sfnt_offset: usize,
}

fn collect_font_entries(path: &str, bytes: Vec<u8>, out: &mut Vec<FontFaceEntry>) {
    let offsets: Vec<usize> = curvebook_text::TrueTypeFont::parse_collection(&bytes).into_iter().map(|(off, _)| off).collect();
    if offsets.is_empty() {
        return;
    }
    let shared: std::sync::Arc<[u8]> = std::sync::Arc::from(bytes);
    let multi = offsets.len() > 1;
    for (idx, sfnt_offset) in offsets.into_iter().enumerate() {
        let name = if multi && idx > 0 { format!("{path}#{idx}") } else { path.to_owned() };
        out.push(FontFaceEntry { name, bytes: std::sync::Arc::clone(&shared), sfnt_offset });
    }
}

/// In-memory TrueType/OpenType font collection combining embedded fallbacks and optional
/// host system font faces for multi-script glyph shaping and rasterization.
#[derive(Debug, Clone)]
pub struct FontBook {
    faces: Vec<FontFaceEntry>,
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
        if crate::bundled_fonts_enabled() && curvebook_text::has_bundled_fonts_feature() {
            for &(name, bytes) in curvebook_text::BUNDLED_FONTS {
                if !name.is_empty() && curvebook_text::TrueTypeFont::parse(bytes).is_some() {
                    faces.push(FontFaceEntry { name: name.to_owned(), bytes: std::sync::Arc::from(bytes), sfnt_offset: 0 });
                }
            }
        }
        let minimal = curvebook_text::build_minimal_truetype_font();
        if curvebook_text::TrueTypeFont::parse(&minimal).is_some() {
            faces.push(FontFaceEntry { name: String::from("CurveBook Minimal Fallback"), bytes: std::sync::Arc::from(minimal), sfnt_offset: 0 });
        }
        Self { faces }
    }

    /// Construct a [`FontBook`] probing optional `CURVEBOOK_FONTS_DIR` and standard host OS system
    /// TrueType/OpenType font files when available on native targets, always retaining the embedded
    /// fallback faces.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn with_system_and_fallback() -> Self {
        const HOST_CANDIDATE_PATHS: &[&str] = &[
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
            "/System/Library/Fonts/Monaco.ttf",
            "/System/Library/Fonts/Geneva.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSerif-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
            "C:\\Windows\\Fonts\\consola.ttf",
        ];
        let mut book = Self::with_embedded_fallback();
        let _ = book.load_host_font_paths(HOST_CANDIDATE_PATHS);
        let _ = book.load_env_fonts_dir();
        book
    }

    /// Construct a [`FontBook`] on `wasm32` using embedded fallback faces without filesystem probing.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn with_system_and_fallback() -> Self {
        Self::with_embedded_fallback()
    }

    /// Probe `.ttf` / `.otf` files from `CURVEBOOK_FONTS_DIR` when set on native targets.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_env_fonts_dir(&mut self) -> usize {
        let Some(dir_os) = std::env::var_os("CURVEBOOK_FONTS_DIR") else {
            return 0;
        };
        let Ok(entries) = std::fs::read_dir(std::path::Path::new(&dir_os)) else {
            return 0;
        };
        let mut paths = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let is_font = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf") || ext.eq_ignore_ascii_case("ttc"));
            if is_font && let Some(s) = path.to_str() {
                paths.push(s.to_owned());
            }
        }
        paths.sort();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        self.load_host_font_paths(&refs)
    }

    /// Parse and prepend a single SFNT or `.ttc` TrueType collection from raw `bytes`,
    /// returning how many sub-faces were registered.
    pub fn prepend_font_bytes(&mut self, name: &str, bytes: Vec<u8>) -> usize {
        let mut out = Vec::new();
        collect_font_entries(name, bytes, &mut out);
        let count = out.len();
        if count > 0 {
            self.faces.splice(0..0, out);
        }
        count
    }

    /// Read and register valid SFNT/TrueType or `.ttc` collection faces from explicit `paths`,
    /// prepending them ahead of fallbacks.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_host_font_paths(&mut self, paths: &[&str]) -> usize {
        const MAX_HOST_FONT_BYTES: usize = 64 * 1024 * 1024;
        let mut prepended = Vec::new();
        for &path in paths {
            let Ok(bytes) = curvebook_platform::fs::read_regular_file_bytes(std::path::Path::new(path), MAX_HOST_FONT_BYTES) else {
                continue;
            };
            collect_font_entries(path, bytes, &mut prepended);
        }
        let loaded = prepended.len();
        if loaded > 0 {
            self.faces.splice(0..0, prepended);
        }
        loaded
    }

    /// Names/paths of loaded font faces in priority order.
    #[must_use]
    pub fn face_names(&self) -> Vec<&str> {
        self.faces.iter().map(|entry| entry.name.as_str()).collect()
    }

    /// Parse all loaded faces as zero-copy [`curvebook_text::TrueTypeFont`] readers.
    #[must_use]
    pub fn parsed_faces(&self) -> Vec<(&str, curvebook_text::TrueTypeFont<'_>)> {
        self.faces
            .iter()
            .filter_map(|entry| curvebook_text::TrueTypeFont::parse_at(&entry.bytes, entry.sfnt_offset).map(|ttf| (entry.name.as_str(), ttf)))
            .collect()
    }

    /// Parse the face at `index` in priority order (supporting both standalone SFNT and `.ttc` sub-faces).
    #[must_use]
    pub fn parsed_face(&self, index: usize) -> Option<curvebook_text::TrueTypeFont<'_>> {
        self.faces.get(index).and_then(|entry| curvebook_text::TrueTypeFont::parse_at(&entry.bytes, entry.sfnt_offset))
    }

    /// Raw `(face_name, sfnt_bytes)` slices in priority order.
    #[must_use]
    pub fn raw_faces(&self) -> Vec<(&str, &[u8])> {
        self.faces.iter().map(|entry| (entry.name.as_str(), entry.bytes.as_ref())).collect()
    }

    /// Stable FNV-1a fingerprint over loaded face names, byte lengths, and SFNT offsets.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for entry in &self.faces {
            for b in entry.name.as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            for b in (entry.bytes.len() as u64).to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            for b in (entry.sfnt_offset as u64).to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    /// Shape `text` cleanly (applying BiDi visual reordering for RTL locales) and rasterize via
    /// loaded [`curvebook_text::TrueTypeFont`] glyph outlines + vector fallback into an RGBA pixmap.
    ///
    /// Returns `(ttf_mapped_glyph_count, image)`.
    ///
    /// # Errors
    /// Returns a [`curvebook_render::RenderError`] if scene rasterization fails for invalid `spec`.
    pub fn rasterize_shaped_text(
        &self,
        text: &str,
        locale: &str,
        font_size_pt: f64,
        spec: curvebook_render::RenderTargetSpec,
        foreground: Rgba8,
        background: Rgba8,
    ) -> Result<(usize, curvebook_render::RasterImage), curvebook_render::RenderError> {
        let is_rtl = curvebook_text::is_rtl_locale(locale);
        let direction = if is_rtl { curvebook_text::TextDirection::Rtl } else { curvebook_text::TextDirection::Ltr };
        let visual = curvebook_text::reorder_visual_bidi(text, direction);
        let parsed = self.parsed_faces();

        let page_w = f64::from(spec.width_px.max(1)) * 72.0 / f64::from(spec.dpi.max(1));
        let page_h = f64::from(spec.height_px.max(1)) * 72.0 / f64::from(spec.dpi.max(1));
        let mut scene = curvebook_chart::ChartScene::new(String::from("fontbook-qa"), String::from(locale), page_w, page_h, Some(background));

        let mut pen_x = 12.0;
        let baseline_y = (font_size_pt * 1.4).max(16.0);
        let mut ttf_mapped = 0usize;

        for ch in visual.chars() {
            if ch.is_control() {
                continue;
            }
            if let Some((adv, path)) = first_ttf_glyph_path(&parsed, ch, pen_x, baseline_y, font_size_pt) {
                if !path.commands.is_empty() {
                    scene.primitives.push(curvebook_chart::ScenePrimitive::Path {
                        path,
                        fill: Some(curvebook_chart::FillPaint::solid(foreground)),
                        stroke: None,
                        clip: None,
                    });
                    ttf_mapped += 1;
                }
                pen_x += adv.max(font_size_pt * 0.25);
            } else {
                let rich = curvebook_text::RichText::plain(ch.to_string());
                let metrics = curvebook_text::measure_text_pt(&rich, font_size_pt);
                scene.primitives.push(curvebook_chart::ScenePrimitive::Text(curvebook_chart::PositionedText {
                    rich,
                    anchor_pt: curvebook_chart::PointPt::new(pen_x, baseline_y),
                    anchor: curvebook_chart::TextAnchor::Start,
                    font_size_pt,
                    rotation_deg: 0.0,
                    color: foreground,
                }));
                pen_x += metrics.width_pt.max(font_size_pt * 0.45);
            }
        }

        let img = curvebook_render::render_scene(&scene, spec)?;
        Ok((ttf_mapped, img))
    }
}

fn first_ttf_glyph_path(
    faces: &[(&str, curvebook_text::TrueTypeFont<'_>)],
    ch: char,
    pen_x: f64,
    baseline_y: f64,
    font_size_pt: f64,
) -> Option<(f64, curvebook_chart::Path2D)> {
    let (_, ttf) = faces.iter().find(|(_, f)| f.glyph_id(ch).is_some())?;
    let cmds = curvebook_text::glyph_paths::outline_ttf_glyph(ttf, ch, pen_x, baseline_y, font_size_pt, false)?;
    let adv = ttf.advance_em(ch).unwrap_or(0.55) * font_size_pt;
    Some((adv, glyph_cmds_to_path(&cmds)))
}

fn glyph_cmds_to_path(cmds: &[curvebook_text::GlyphOutlineCmd]) -> curvebook_chart::Path2D {
    let mut path = curvebook_chart::Path2D::new();
    for cmd in cmds {
        match *cmd {
            curvebook_text::GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                path.commands.push(curvebook_chart::PathCmd::MoveTo(curvebook_chart::PointPt::new(x_pt, y_pt)));
            }
            curvebook_text::GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                path.commands.push(curvebook_chart::PathCmd::LineTo(curvebook_chart::PointPt::new(x_pt, y_pt)));
            }
            curvebook_text::GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                path.commands.push(curvebook_chart::PathCmd::QuadTo {
                    control: curvebook_chart::PointPt::new(cx_pt, cy_pt),
                    to: curvebook_chart::PointPt::new(x_pt, y_pt),
                });
            }
            curvebook_text::GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                path.commands.push(curvebook_chart::PathCmd::CubicTo {
                    c1: curvebook_chart::PointPt::new(c1x_pt, c1y_pt),
                    c2: curvebook_chart::PointPt::new(c2x_pt, c2y_pt),
                    to: curvebook_chart::PointPt::new(x_pt, y_pt),
                });
            }
            curvebook_text::GlyphOutlineCmd::Close => path.commands.push(curvebook_chart::PathCmd::Close),
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use curvebook_palette::Rgba8;

    use super::{UiThemeMode, UiThemeSpec, WCAG_AA_TEXT_MIN_RATIO, font_diagnostics, install_fonts, wcag_contrast_ratio};

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
        let hc = UiThemeSpec::high_contrast();
        assert!(hc.primary_text_contrast() >= 18.0, "HighContrast primary contrast {} < 18.0", hc.primary_text_contrast());
        assert!(hc.meets_wcag_aaa(), "HighContrast failed AAA check");
    }

    #[test]
    fn derived_palette_keeps_chrome_neutral_and_accent_distinct_from_okabe_ito_series() {
        for mode in UiThemeMode::ALL {
            let p = super::DerivedUiPalette::for_mode(mode);
            for neutral in [p.panel_bg, p.surface_recessed, p.surface_bg, p.app_bar, p.header_bg, p.divider, p.border, p.selection_unemphasized_bg] {
                assert!(neutral.to_oklch().c <= 0.02, "{mode:?} chrome neutral {neutral:?} exceeded low-chroma cap");
            }
        }
        let dark = super::DerivedUiPalette::for_mode(UiThemeMode::Dark);
        assert!(
            dark.surface_recessed.to_oklab().l < dark.panel_bg.to_oklab().l && dark.panel_bg.to_oklab().l < dark.surface_bg.to_oklab().l,
            "Dark Mode higher surface tiers must step lighter in OKLab"
        );
        let light_accent = super::DerivedUiPalette::for_mode(UiThemeMode::Light).accent.to_oklab();
        for &series_color in curvebook_palette::OKABE_ITO {
            let dist = light_accent.distance(series_color.to_oklab());
            assert!(dist >= 0.05, "UI accent {light_accent:?} too close to Okabe-Ito series {series_color:?} (DeltaE_OK={dist:.4})");
        }
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

    #[test]
    fn font_book_loads_ttc_collections_without_panicking_or_duplicating_blobs() {
        let sfnt = curvebook_text::build_minimal_truetype_font();
        let num_tables = u16::from_be_bytes([sfnt[4], sfnt[5]]) as usize;
        let header_len = 12 + 4 * 2; // 'ttcf'(4) + ver(4) + count(4) + 2 offsets(8)
        let face0_off = header_len;
        let face1_off = header_len + sfnt.len();

        let shift_sfnt = |base_off: usize| -> Vec<u8> {
            let mut copy = sfnt.clone();
            for t in 0..num_tables {
                let off_pos = 12 + t * 16 + 8;
                let orig = u32::from_be_bytes([copy[off_pos], copy[off_pos + 1], copy[off_pos + 2], copy[off_pos + 3]]);
                let shifted = orig + (base_off as u32);
                copy[off_pos..off_pos + 4].copy_from_slice(&shifted.to_be_bytes());
            }
            copy
        };

        let mut ttc = Vec::new();
        ttc.extend_from_slice(b"ttcf");
        ttc.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
        ttc.extend_from_slice(&2_u32.to_be_bytes());
        ttc.extend_from_slice(&(face0_off as u32).to_be_bytes());
        ttc.extend_from_slice(&(face1_off as u32).to_be_bytes());
        ttc.extend_from_slice(&shift_sfnt(face0_off));
        ttc.extend_from_slice(&shift_sfnt(face1_off));

        let mut book = super::FontBook::with_embedded_fallback();
        let fp_before = book.fingerprint();
        let loaded = book.prepend_font_bytes("synthetic/NotoSansCJK-Regular.ttc", ttc);
        assert_eq!(loaded, 2);
        assert_eq!(book.face_names()[0], "synthetic/NotoSansCJK-Regular.ttc");
        assert_eq!(book.face_names()[1], "synthetic/NotoSansCJK-Regular.ttc#1");
        assert!(book.parsed_face(0).is_some());
        assert!(book.parsed_face(1).is_some());
        assert!(book.parsed_faces().len() >= 3);
        assert_ne!(book.fingerprint(), fp_before);
    }
}
