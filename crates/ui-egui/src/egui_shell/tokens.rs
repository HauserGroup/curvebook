//! Canonical WCAG 2.2 AA/AAA seed-derived theme, semantic status, scientific palette,
//! and typography tokens for `egui_shell` (`docs/design-language.md` §5).

use curvebook_palette::{
    CIVIDIS_STOPS, DIVERGING_BLUE_RED_STOPS, OKABE_ITO, OKABE_ITO_BLACK, OKABE_ITO_BLUE, OKABE_ITO_BLUISH_GREEN, OKABE_ITO_ORANGE,
    OKABE_ITO_REDDISH_PURPLE, OKABE_ITO_SKY_BLUE, OKABE_ITO_VERMILION, OKABE_ITO_YELLOW, Rgba8, VIRIDIS_STOPS,
};
use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Visuals};

use super::layout::LayoutMetrics;
use crate::i18n;
use crate::theme::{DerivedUiPalette, UiThemeMode, UiThemeSpec};

/// Convert [`Rgba8`] into a premultiplied [`egui::Color32`].
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub const fn rgba8_to_color32(color: Rgba8) -> Color32 {
    if color.a == 255 {
        Color32::from_rgb(color.r, color.g, color.b) // design-token-exempt: canonical Rgba8 -> Color32 bridge
    } else if color.a == 0 {
        Color32::TRANSPARENT
    } else {
        let a = color.a as u16;
        let r = ((color.r as u16 * a + 127) / 255) as u8;
        let g = ((color.g as u16 * a + 127) / 255) as u8;
        let b = ((color.b as u16 * a + 127) / 255) as u8;
        Color32::from_rgba_premultiplied(r, g, b, color.a) // design-token-exempt: canonical Rgba8 -> Color32 bridge
    }
}

/// Convert an unpremultiplied/opaque [`egui::Color32`] to [`Rgba8`].
#[must_use]
pub const fn color32_to_rgba8(color: Color32) -> Rgba8 {
    Rgba8::rgba(color.r(), color.g(), color.b(), color.a()) // design-token-exempt: canonical Color32 -> Rgba8 bridge
}

/// Canonical 8-color Okabe-Ito categorical series palette (`gui-layout-spec.md` §9.2).
pub const OKABE_ITO_SERIES_PALETTE: [Color32; 8] = [
    rgba8_to_color32(OKABE_ITO_BLACK),
    rgba8_to_color32(OKABE_ITO_ORANGE),
    rgba8_to_color32(OKABE_ITO_SKY_BLUE),
    rgba8_to_color32(OKABE_ITO_BLUISH_GREEN),
    rgba8_to_color32(OKABE_ITO_YELLOW),
    rgba8_to_color32(OKABE_ITO_BLUE),
    rgba8_to_color32(OKABE_ITO_VERMILION),
    rgba8_to_color32(OKABE_ITO_REDDISH_PURPLE),
];

/// Diverging Blue-White-Vermilion colorblind-safe ramp stops (`-1 -> 0 -> +1`).
pub const BLUE_WHITE_VERMILION_STOPS: [Color32; 3] =
    [rgba8_to_color32(OKABE_ITO_BLUE), rgba8_to_color32(DIVERGING_BLUE_RED_STOPS[8]), rgba8_to_color32(OKABE_ITO_VERMILION)];

/// Scientific categorical and perceptual colormap accessors for `egui` views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScientificPaletteTokens;

impl ScientificPaletteTokens {
    /// Okabe-Ito 8-color series palette (`gui-layout-spec.md` §9.2).
    #[must_use]
    pub const fn okabe_ito_series() -> [Color32; 8] {
        OKABE_ITO_SERIES_PALETTE
    }

    /// Okabe-Ito swatches in `curvebook-palette` default ordering.
    #[must_use]
    pub fn okabe_ito_palette_order() -> Vec<Color32> {
        OKABE_ITO.iter().copied().map(rgba8_to_color32).collect()
    }

    /// 17-stop Viridis perceptual colormap (`t = 0/16 .. 16/16`).
    #[must_use]
    pub fn viridis_stops() -> [Color32; 17] {
        VIRIDIS_STOPS.map(rgba8_to_color32)
    }

    /// 17-stop Cividis CVD-optimized colormap (`t = 0/16 .. 16/16`).
    #[must_use]
    pub fn cividis_stops() -> [Color32; 17] {
        CIVIDIS_STOPS.map(rgba8_to_color32)
    }

    /// Diverging Blue-White-Vermilion stops (`-1 -> 0 -> +1`).
    #[must_use]
    pub const fn blue_white_vermilion_stops() -> [Color32; 3] {
        BLUE_WHITE_VERMILION_STOPS
    }
}

/// Complete WCAG 2.2 AA/AAA seed-derived theme, semantic status, provenance, and geometry tokens for the `egui` shell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub mode: UiThemeMode,
    pub dark: bool,
    pub locale: &'static str,
    pub ui_scale: f32,
    pub metrics: LayoutMetrics,
    pub app_bar: Color32,
    pub panel_bg: Color32,
    pub surface_recessed: Color32,
    pub surface_bg: Color32,
    pub canvas_mat: Color32,
    pub header_bg: Color32,
    pub border: Color32,
    pub divider: Color32,
    pub hover_bg: Color32,
    pub hover_border: Color32,
    pub active_bg: Color32,
    pub selection_unemphasized_bg: Color32,
    pub focus_ring: Color32,
    pub input_bg: Color32,
    pub input_border: Color32,
    pub button_bg: Color32,
    pub button_border: Color32,
    pub text_primary: Color32,
    pub text_strong: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    pub icon: Color32,
    pub accent: Color32,
    pub accent_strong: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub paper_white: Color32,
    pub scrim: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub stale: Color32,
    pub role_num: Color32,
    pub role_cat: Color32,
    pub role_lbl: Color32,
    pub role_fx: Color32,
    /// Structural border/dot color for linked data regions (never rendered as foreground text).
    pub link_blue: Color32,
    /// Clickable text link color (strictly reserved for `ui_kit::link_button`).
    pub text_link: Color32,
    pub link_green: Color32,
    pub link_red: Color32,
    pub excluded_cell_text: Color32,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::for_mode(UiThemeMode::Light)
    }
}

impl Tokens {
    /// Construct canonical [`Tokens`] for `mode` at `1.0x` scale and `"en"` locale.
    #[must_use]
    pub fn for_mode(mode: UiThemeMode) -> Self {
        Self::from_theme_spec(&UiThemeSpec::for_mode(mode), "en")
    }

    /// Construct canonical [`Tokens`] from a [`UiThemeSpec`] and normalized BCP-47 `locale`.
    ///
    /// All surface, state, and semantic tokens are derived deterministically in OKLab/OKLCH from
    /// [`ThemeSeeds`](crate::theme::ThemeSeeds) via [`DerivedUiPalette`], honouring any explicit
    /// field overrides carried on `spec`.
    #[must_use]
    pub fn from_theme_spec(spec: &UiThemeSpec, locale: &str) -> Self {
        let dark = !matches!(spec.mode, UiThemeMode::Light);
        let ui_scale = if spec.ui_scale.is_finite() { spec.ui_scale.clamp(0.75, 3.0) } else { 1.0 };
        let canonical_locale = i18n::normalize_locale(locale);
        let p = DerivedUiPalette::for_mode(spec.mode);
        let border = rgba8_to_color32(spec.border);

        Self {
            mode: spec.mode,
            dark,
            locale: canonical_locale,
            ui_scale,
            metrics: LayoutMetrics::CANONICAL,
            app_bar: rgba8_to_color32(p.app_bar),
            panel_bg: rgba8_to_color32(spec.panel_bg),
            surface_recessed: rgba8_to_color32(p.surface_recessed),
            surface_bg: rgba8_to_color32(spec.surface_bg),
            canvas_mat: rgba8_to_color32(p.canvas_mat),
            header_bg: rgba8_to_color32(p.header_bg),
            border,
            divider: rgba8_to_color32(p.divider),
            hover_bg: rgba8_to_color32(p.hover_bg),
            hover_border: rgba8_to_color32(p.hover_border),
            active_bg: rgba8_to_color32(p.active_bg),
            selection_unemphasized_bg: rgba8_to_color32(p.selection_unemphasized_bg),
            focus_ring: rgba8_to_color32(p.focus_ring),
            input_bg: rgba8_to_color32(p.input_bg),
            input_border: border,
            button_bg: rgba8_to_color32(p.button_bg),
            button_border: rgba8_to_color32(p.button_border),
            text_primary: rgba8_to_color32(spec.text_primary),
            text_strong: rgba8_to_color32(p.text_strong),
            text_muted: rgba8_to_color32(spec.text_muted),
            text_disabled: rgba8_to_color32(p.text_disabled),
            icon: rgba8_to_color32(p.icon),
            accent: rgba8_to_color32(spec.accent),
            accent_strong: rgba8_to_color32(p.accent_strong),
            accent_hover: rgba8_to_color32(p.accent_hover),
            on_accent: rgba8_to_color32(p.on_accent),
            paper_white: rgba8_to_color32(spec.paper_white),
            scrim: rgba8_to_color32(p.scrim),
            error: rgba8_to_color32(spec.error),
            warning: rgba8_to_color32(spec.warning),
            stale: rgba8_to_color32(spec.stale),
            role_num: rgba8_to_color32(p.role_num),
            role_cat: rgba8_to_color32(p.role_cat),
            role_lbl: rgba8_to_color32(p.role_lbl),
            role_fx: rgba8_to_color32(p.role_fx),
            link_blue: rgba8_to_color32(spec.link_blue),
            text_link: rgba8_to_color32(spec.accent),
            link_green: rgba8_to_color32(spec.link_green),
            link_red: rgba8_to_color32(spec.link_red),
            excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
        }
    }

    /// Retrieve active [`Tokens`] cached in `ctx`, defaulting to [`UiThemeMode::Light`].
    #[must_use]
    pub fn get(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<Self>(egui::Id::NULL)).unwrap_or_default()
    }

    /// Build [`egui::Visuals`] aligned with these WCAG 2.2 AA/AAA theme and stroke tokens.
    #[must_use]
    pub fn to_visuals(&self) -> Visuals {
        let mut visuals = if self.dark { Visuals::dark() } else { Visuals::light() };
        let thin = self.metrics.stroke_thin_pt;
        let focus = self.metrics.stroke_focus_pt;
        visuals.dark_mode = self.dark;
        visuals.override_text_color = Some(self.text_primary);
        visuals.weak_text_color = Some(self.text_muted);
        visuals.panel_fill = self.panel_bg;
        visuals.window_fill = self.surface_bg;
        visuals.extreme_bg_color = self.input_bg;
        visuals.faint_bg_color = self.header_bg;
        visuals.window_stroke = Stroke::new(thin, self.border);
        visuals.window_corner_radius = CornerRadius::same(self.metrics.radius_lg);
        visuals.popup_shadow = egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: self.scrim.linear_multiply(0.18) };
        visuals.selection.bg_fill = self.active_bg;
        visuals.selection.stroke = Stroke::new(focus, self.focus_ring);
        let md_radius = CornerRadius::same(self.metrics.radius_md);
        visuals.widgets.noninteractive.corner_radius = md_radius;
        visuals.widgets.noninteractive.bg_fill = self.panel_bg;
        visuals.widgets.noninteractive.weak_bg_fill = self.panel_bg;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(thin, self.border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(thin, self.text_primary);
        visuals.widgets.inactive.corner_radius = md_radius;
        visuals.widgets.inactive.bg_fill = self.button_bg;
        visuals.widgets.inactive.weak_bg_fill = self.button_bg;
        visuals.widgets.inactive.bg_stroke = Stroke::new(thin, self.button_border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(thin, self.text_primary);
        visuals.widgets.hovered.corner_radius = md_radius;
        visuals.widgets.hovered.bg_fill = self.hover_bg;
        visuals.widgets.hovered.weak_bg_fill = self.hover_bg;
        visuals.widgets.hovered.bg_stroke = Stroke::new(thin, self.hover_border);
        visuals.widgets.hovered.fg_stroke = Stroke::new(thin, self.text_strong);
        visuals.widgets.active.corner_radius = md_radius;
        visuals.widgets.active.bg_fill = self.active_bg;
        visuals.widgets.active.weak_bg_fill = self.active_bg;
        visuals.widgets.active.bg_stroke = Stroke::new(focus, self.focus_ring);
        visuals.widgets.active.fg_stroke = Stroke::new(thin, self.text_strong);
        visuals.widgets.open.corner_radius = md_radius;
        visuals.widgets.open.bg_fill = self.hover_bg;
        visuals.widgets.open.weak_bg_fill = self.hover_bg;
        visuals.widgets.open.bg_stroke = Stroke::new(thin, self.border);
        visuals.widgets.open.fg_stroke = Stroke::new(thin, self.text_strong);
        // Solid, high-contrast caret: avoids waking the GPU/CPU at 2 Hz forever when dialogs
        // auto-focus a search box or an inline grid editor stays open idle.
        visuals.text_cursor.blink = false;
        visuals
    }

    /// Apply theme visuals, canonical typography sizes, motion cap (`<= 150 ms`), and non-overlapping scrollbar metrics to `style`.
    pub fn configure_style(&self, style: &mut egui::Style) {
        style.visuals = self.to_visuals();
        style.text_styles.insert(TextStyle::Body, FontId::proportional(self.metrics.font_ui_body_pt));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(self.metrics.font_ui_body_pt));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(self.metrics.font_ui_small_pt));
        style.text_styles.insert(TextStyle::Monospace, FontId::monospace(self.metrics.font_grid_numeral_pt));
        style.text_styles.insert(TextStyle::Heading, FontId::proportional(self.metrics.font_ui_title_pt));
        style.animation_time = style.animation_time.min(self.metrics.max_motion_sec);

        style.spacing.scroll.floating = false;
        style.spacing.scroll.bar_width = self.metrics.scroll_bar_thickness;
        style.spacing.scroll.bar_inner_margin = 2.0;
        style.spacing.scroll.bar_outer_margin = 2.0;
        style.spacing.interact_size.y = self.metrics.min_hit_target_pt;
        style.spacing.tooltip_width = 320.0;
        style.interaction.tooltip_delay = 0.45;
    }

    /// Synchronize the currently executing [`egui::Ui`] style immediately (including pass-0 frames).
    pub fn apply_to_ui(&self, ui: &mut egui::Ui) {
        self.configure_style(ui.style_mut());
    }

    /// Install these tokens, visuals, canonical typography sizes, and non-overlapping scrollbar
    /// gutter once per frame on `ctx` without changing `zoom_factor`.
    pub fn apply_theme_to_ctx(&self, ctx: &egui::Context) {
        let egui_theme = if self.dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, *self));
        ctx.set_theme(egui_theme);
        ctx.set_visuals(self.to_visuals());
        ctx.all_styles_mut(|style| {
            self.configure_style(style);
        });
    }

    /// Install these tokens, visuals, canonical typography sizes, non-overlapping scrollbar gutter,
    /// and `ui_scale` once per frame on `ctx`.
    pub fn apply_to_ctx(&self, ctx: &egui::Context) {
        self.apply_theme_to_ctx(ctx);
        ctx.set_zoom_factor(self.ui_scale.clamp(0.75, 3.0));
    }

    /// Foreground color for accent-emphasized text/glyphs painted on [`Self::active_bg`].
    ///
    /// In light mode this uses [`Self::accent_strong`]; in dark and high-contrast modes
    /// [`Self::accent_strong`] is reserved strictly for solid CTA fills behind [`Self::on_accent`],
    /// so selected surfaces use [`Self::text_strong`] to guarantee WCAG 2.2 AA contrast.
    #[must_use]
    pub const fn selected_accent_text(&self) -> Color32 {
        if self.dark { self.text_strong } else { self.accent_strong }
    }

    /// Return detailed WCAG 2.2 contrast violations across all text-on-background and non-text UI
    /// boundary pairs (`text_disabled` is exempt per WCAG 2.2 SC 1.4.3).
    #[must_use]
    pub fn contrast_violations(&self) -> Vec<String> {
        let mut violations = Vec::new();
        let text_pairs = [
            ("text_primary on surface_bg", self.text_primary, self.surface_bg),
            ("text_primary on surface_recessed", self.text_primary, self.surface_recessed),
            ("text_primary on panel_bg", self.text_primary, self.panel_bg),
            ("text_primary on app_bar", self.text_primary, self.app_bar),
            ("text_primary on header_bg", self.text_primary, self.header_bg),
            ("text_primary on input_bg", self.text_primary, self.input_bg),
            ("text_primary on button_bg", self.text_primary, self.button_bg),
            ("text_primary on hover_bg", self.text_primary, self.hover_bg),
            ("text_primary on active_bg", self.text_primary, self.active_bg),
            ("text_primary on selection_unemphasized_bg", self.text_primary, self.selection_unemphasized_bg),
            ("text_strong on surface_bg", self.text_strong, self.surface_bg),
            ("text_strong on surface_recessed", self.text_strong, self.surface_recessed),
            ("text_strong on panel_bg", self.text_strong, self.panel_bg),
            ("text_strong on hover_bg", self.text_strong, self.hover_bg),
            ("text_strong on active_bg", self.text_strong, self.active_bg),
            ("text_strong on selection_unemphasized_bg", self.text_strong, self.selection_unemphasized_bg),
            ("selected_accent_text on active_bg", self.selected_accent_text(), self.active_bg),
            ("text_muted on surface_bg", self.text_muted, self.surface_bg),
            ("text_muted on surface_recessed", self.text_muted, self.surface_recessed),
            ("text_muted on panel_bg", self.text_muted, self.panel_bg),
            ("text_muted on app_bar", self.text_muted, self.app_bar),
            ("text_muted on header_bg", self.text_muted, self.header_bg),
            ("text_muted on input_bg", self.text_muted, self.input_bg),
            ("text_muted on button_bg", self.text_muted, self.button_bg),
            ("text_muted on hover_bg", self.text_muted, self.hover_bg),
            ("text_muted on active_bg", self.text_muted, self.active_bg),
            ("text_muted on selection_unemphasized_bg", self.text_muted, self.selection_unemphasized_bg),
            ("on_accent on accent_strong", self.on_accent, self.accent_strong),
            ("on_accent on accent_hover", self.on_accent, self.accent_hover),
            ("accent on surface_bg", self.accent, self.surface_bg),
            ("accent on surface_recessed", self.accent, self.surface_recessed),
            ("accent on panel_bg", self.accent, self.panel_bg),
            ("accent on button_bg", self.accent, self.button_bg),
            ("accent on header_bg", self.accent, self.header_bg),
            ("error on surface_bg", self.error, self.surface_bg),
            ("error on panel_bg", self.error, self.panel_bg),
            ("warning on surface_bg", self.warning, self.surface_bg),
            ("warning on panel_bg", self.warning, self.panel_bg),
            ("stale on surface_bg", self.stale, self.surface_bg),
            ("stale on panel_bg", self.stale, self.panel_bg),
            ("role_num on surface_recessed", self.role_num, self.surface_recessed),
            ("text_link on surface_bg", self.text_link, self.surface_bg),
            ("text_link on panel_bg", self.text_link, self.panel_bg),
            ("link_blue on surface_bg", self.link_blue, self.surface_bg),
            ("link_green on surface_bg", self.link_green, self.surface_bg),
            ("link_red on surface_bg", self.link_red, self.surface_bg),
            ("excluded_cell_text on surface_bg", self.excluded_cell_text, self.surface_bg),
            ("excluded_cell_text on panel_bg", self.excluded_cell_text, self.panel_bg),
            ("excluded_cell_text on active_bg", self.excluded_cell_text, self.active_bg),
        ];
        let ui_pairs = [
            ("border on surface_bg", self.border, self.surface_bg),
            ("border on panel_bg", self.border, self.panel_bg),
            ("border on surface_recessed", self.border, self.surface_recessed),
            ("input_border on surface_bg", self.input_border, self.surface_bg),
            ("input_border on input_bg", self.input_border, self.input_bg),
            ("input_border on surface_recessed", self.input_border, self.surface_recessed),
            ("button_border on panel_bg", self.button_border, self.panel_bg),
            ("button_border on button_bg", self.button_border, self.button_bg),
            ("focus_ring on surface_bg", self.focus_ring, self.surface_bg),
            ("focus_ring on panel_bg", self.focus_ring, self.panel_bg),
            ("focus_ring on surface_recessed", self.focus_ring, self.surface_recessed),
            ("hover_border on panel_bg", self.hover_border, self.panel_bg),
            ("hover_border on surface_bg", self.hover_border, self.surface_bg),
            ("hover_border on hover_bg", self.hover_border, self.hover_bg),
            ("accent border on active_bg", self.accent, self.active_bg),
            ("accent border on surface_bg", self.accent, self.surface_bg),
            ("accent border on surface_recessed", self.accent, self.surface_recessed),
            ("accent border on panel_bg", self.accent, self.panel_bg),
            ("accent border on header_bg", self.accent, self.header_bg),
            ("on_accent check/thumb on accent_strong", self.on_accent, self.accent_strong),
            ("on_accent check/thumb on accent_hover", self.on_accent, self.accent_hover),
            ("icon on surface_bg", self.icon, self.surface_bg),
            ("icon on panel_bg", self.icon, self.panel_bg),
            ("icon on surface_recessed", self.icon, self.surface_recessed),
            ("role_num on surface_bg", self.role_num, self.surface_bg),
            ("role_cat on surface_bg", self.role_cat, self.surface_bg),
            ("role_lbl on surface_bg", self.role_lbl, self.surface_bg),
            ("role_fx on surface_bg", self.role_fx, self.surface_bg),
        ];
        for (label, fg, bg) in text_pairs {
            let ratio = color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg));
            if ratio < 4.5 {
                violations.push(format!("{:?} {label}: contrast {ratio:.2}:1 < 4.5:1", self.mode));
            }
        }
        for (label, fg, bg) in ui_pairs {
            let ratio = color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg));
            if ratio < 3.0 {
                violations.push(format!("{:?} {label}: contrast {ratio:.2}:1 < 3.0:1", self.mode));
            }
        }
        let hover_vs_panel = color32_to_rgba8(self.hover_bg).contrast_ratio(color32_to_rgba8(self.panel_bg));
        if hover_vs_panel < 1.30 {
            violations.push(format!("{:?} hover_bg vs panel_bg: contrast {hover_vs_panel:.2}:1 < 1.30:1", self.mode));
        }
        let hover_vs_btn = color32_to_rgba8(self.hover_bg).contrast_ratio(color32_to_rgba8(self.button_bg));
        if hover_vs_btn < 1.25 {
            violations.push(format!("{:?} hover_bg vs button_bg: contrast {hover_vs_btn:.2}:1 < 1.25:1", self.mode));
        }
        violations
    }

    /// Verify all primary text and interactive UI boundary pairs satisfy WCAG 2.2 AA contrast.
    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        self.contrast_violations().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::layout::LayoutMetrics;
    use super::{ScientificPaletteTokens, Tokens, color32_to_rgba8};
    use crate::theme::UiThemeMode;

    #[test]
    fn canonical_tokens_meet_wcag_aa_and_preserve_scientific_palettes() {
        for mode in UiThemeMode::ALL {
            let tokens = Tokens::for_mode(mode);
            assert!(tokens.meets_wcag_aa(), "WCAG 2.1 AA failed for {mode:?}");
            let visuals = tokens.to_visuals();
            assert_eq!(visuals.widgets.inactive.weak_bg_fill, tokens.button_bg, "weak_bg_fill must match button_bg for {mode:?}");
            assert_eq!(visuals.widgets.hovered.weak_bg_fill, tokens.hover_bg);
            assert_eq!(visuals.widgets.active.weak_bg_fill, tokens.active_bg);
            assert_eq!(visuals.widgets.open.weak_bg_fill, tokens.hover_bg);
        }
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height(), 66.0);
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height_for_host(true), 38.0);
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height_for_host(false), 66.0);
        assert_eq!(LayoutMetrics::CANONICAL.status_bar_height(), 26.0);
        assert_eq!(LayoutMetrics::CANONICAL.grid_header_total_height(), 72.0);
        assert_eq!(ScientificPaletteTokens::okabe_ito_series().len(), 8);
        assert_eq!(ScientificPaletteTokens::viridis_stops().len(), 17);
        assert_eq!(ScientificPaletteTokens::cividis_stops().len(), 17);
        assert_eq!(ScientificPaletteTokens::blue_white_vermilion_stops().len(), 3);
        let first = color32_to_rgba8(ScientificPaletteTokens::okabe_ito_series()[0]);
        assert_eq!(first, curvebook_palette::Rgba8::BLACK);
    }

    #[test]
    fn wcag_22_sc_1_4_11_non_text_contrast_across_all_themes() {
        for mode in UiThemeMode::ALL {
            let t = Tokens::for_mode(mode);
            let violations = t.contrast_violations();
            assert!(violations.is_empty(), "WCAG 2.2 SC 1.4.3/1.4.11 violations for {mode:?}: {violations:#?}");

            // Explicitly verify borderless & recessed control state cues (>= 3.0:1).
            let state_cues = [
                ("active tab underline on panel_bg", t.accent, t.panel_bg),
                ("active tab underline on surface_bg", t.accent, t.surface_bg),
                ("checked checkbox/switch outline on surface_bg", t.accent, t.surface_bg),
                ("checked checkbox/switch outline on panel_bg", t.accent, t.panel_bg),
                ("checked checkbox/switch outline on surface_recessed", t.accent, t.surface_recessed),
                ("checkbox/switch tick/thumb on accent_strong", t.on_accent, t.accent_strong),
                ("unchecked checkbox/switch border on surface_recessed", t.input_border, t.surface_recessed),
                ("segmented active border on surface_recessed", t.border, t.surface_recessed),
                ("focus ring on surface_recessed", t.focus_ring, t.surface_recessed),
                ("focus ring on active_bg", t.focus_ring, t.active_bg),
                ("error indicator on panel_bg", t.error, t.panel_bg),
                ("warning indicator on panel_bg", t.warning, t.panel_bg),
            ];
            for (name, fg, bg) in state_cues {
                let ratio = color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg));
                assert!(ratio >= 3.0, "{mode:?} SC 1.4.11 failed for {name}: {ratio:.2}:1 < 3.0:1");
            }
        }
    }

    #[test]
    fn r20_21_configure_style_sets_450ms_tooltip_delay_and_popup_shadow() {
        let t = Tokens::for_mode(UiThemeMode::Light);
        let mut style = egui::Style::default();
        t.configure_style(&mut style);
        assert!((style.interaction.tooltip_delay - 0.45).abs() <= f32::EPSILON);
        assert!((style.spacing.tooltip_width - 320.0).abs() <= f32::EPSILON);
        assert!(style.visuals.popup_shadow.color.a() > 0);
    }
}
