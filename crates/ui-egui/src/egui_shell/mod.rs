//! Native desktop `egui 0.37` shell contracts, canonical layout/color tokens, single-commit
//! intent buffer, AccessKit inspection metadata, and adapted pure-`egui` widget helpers (`R15`).

use curvebook_engine::{CommandOutcome, EngineError, Session};
use curvebook_palette::{CIVIDIS_STOPS, OKABE_ITO, Rgba8, VIRIDIS_STOPS};
use egui::{Color32, CornerRadius, FontId, Rect, Stroke, TextStyle, Visuals};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::control_io::dispatch_ui_command_as_ui;
use crate::i18n;
use crate::layout::ZLayer;
use crate::theme::{UiThemeMode, UiThemeSpec};

pub mod data_grid;
pub mod driver;
pub mod fonts;
pub mod inspect_bridge;
pub mod rasterizer;
pub mod stat_anova_modal;
pub mod top_bar_fit;
pub mod widgets;

/// Persistent pure-UI `egui_shell` view states owned alongside [`UiApp`] (`R15.2+`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EguiShellViewState {
    pub column_grid: data_grid::ColumnGridEguiState,
    pub anova_modal: stat_anova_modal::AnovaModalEguiState,
}

/// Active desktop UI presentation backend (`--ui-backend=legacy|egui`, default `Legacy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiBackend {
    #[default]
    Legacy,
    Egui,
}

impl UiBackend {
    /// Parse a backend selector flag or environment value (`"legacy"` or `"egui"`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "legacy" => Some(Self::Legacy),
            "egui" => Some(Self::Egui),
            _ => None,
        }
    }

    /// Canonical lowercase CLI/environment string identifier for this backend.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Egui => "egui",
        }
    }

    /// Resolve backend selection from optional `CURVEBOOK_UI_BACKEND` env and `--ui-backend` CLI flag.
    ///
    /// CLI flag takes precedence over environment variable; unset inputs resolve to [`Self::Legacy`].
    ///
    /// # Errors
    /// Returns an actionable message when a non-empty value is neither `"legacy"` nor `"egui"`.
    pub fn from_env_and_args(env_val: Option<&str>, cli_val: Option<&str>) -> Result<Self, String> {
        if let Some(cli) = cli_val.map(str::trim).filter(|s| !s.is_empty()) {
            return Self::parse(cli).ok_or_else(|| format!("invalid --ui-backend value `{cli}`; expected `legacy` or `egui`"));
        }
        if let Some(env_raw) = env_val.map(str::trim).filter(|s| !s.is_empty()) {
            return Self::parse(env_raw).ok_or_else(|| format!("invalid CURVEBOOK_UI_BACKEND value `{env_raw}`; expected `legacy` or `egui`"));
        }
        Ok(Self::Legacy)
    }

    /// Resolve backend selection from `CURVEBOOK_UI_BACKEND`, defaulting to [`Self::Legacy`].
    #[must_use]
    pub fn from_process_env() -> Self {
        std::env::var("CURVEBOOK_UI_BACKEND").ok().and_then(|v| Self::parse(&v)).unwrap_or_default()
    }
}

/// Canonical `1.0x` logical-point geometry and typography tokens (`docs/planning/gui-layout-spec.md` §9.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutMetrics {
    pub app_bar_h: f32,
    pub action_strip_h: f32,
    pub status_bar_h: f32,
    pub navigator_default_w: f32,
    pub navigator_min_w: f32,
    pub navigator_max_w: f32,
    pub navigator_collapse_breakpoint_w: f32,
    pub inspector_default_w: f32,
    pub inspector_min_w: f32,
    pub inspector_max_w: f32,
    pub inspector_collapse_breakpoint_w: f32,
    pub dock_collapsed_rail_w: f32,
    pub splitter_visual_w: f32,
    pub splitter_hit_slop_w: f32,
    pub center_stage_min_w: f32,
    pub split_vertical_breakpoint_w: f32,
    pub grid_row_h: f32,
    pub grid_header_tier_heights: [f32; 3],
    pub min_hit_target_pt: f32,
    pub min_toolbar_target_pt: f32,
    pub scroll_bar_thickness: f32,
    pub scroll_bar_hover_thickness: f32,
    pub font_ui_body_pt: f32,
    pub font_ui_small_pt: f32,
    pub font_grid_numeral_pt: f32,
    pub min_window_w_pt: f32,
    pub min_window_h_pt: f32,
    pub default_window_w_pt: f32,
    pub default_window_h_pt: f32,
    pub wide_desktop_breakpoint_w: f32,
}

impl LayoutMetrics {
    /// Canonical `1.0x` logical-point metrics from `docs/planning/gui-layout-spec.md` §9.1.
    pub const CANONICAL: Self = Self {
        app_bar_h: 32.0,
        action_strip_h: 38.0,
        status_bar_h: 26.0,
        navigator_default_w: 240.0,
        navigator_min_w: 180.0,
        navigator_max_w: 380.0,
        navigator_collapse_breakpoint_w: 760.0,
        inspector_default_w: 280.0,
        inspector_min_w: 220.0,
        inspector_max_w: 420.0,
        inspector_collapse_breakpoint_w: 960.0,
        dock_collapsed_rail_w: 36.0,
        splitter_visual_w: 4.0,
        splitter_hit_slop_w: 8.0,
        center_stage_min_w: 360.0,
        split_vertical_breakpoint_w: 720.0,
        grid_row_h: 24.0,
        grid_header_tier_heights: [20.0, 24.0, 20.0],
        min_hit_target_pt: 24.0,
        min_toolbar_target_pt: 28.0,
        scroll_bar_thickness: 10.0,
        scroll_bar_hover_thickness: 12.0,
        font_ui_body_pt: 13.0,
        font_ui_small_pt: 11.5,
        font_grid_numeral_pt: 12.5,
        min_window_w_pt: 640.0,
        min_window_h_pt: 420.0,
        default_window_w_pt: 1280.0,
        default_window_h_pt: 800.0,
        wide_desktop_breakpoint_w: 1440.0,
    };

    /// Return the canonical `1.0x` layout metrics.
    #[must_use]
    pub const fn canonical() -> Self {
        Self::CANONICAL
    }

    /// Snap a logical `pt` value to rounded device pixels under `scale_factor`.
    #[must_use]
    pub fn scaled_px(pt: f32, scale_factor: f32) -> f32 {
        let safe_pt = if pt.is_finite() && pt > 0.0 { pt } else { 0.0 };
        let safe_scale = if scale_factor.is_finite() { scale_factor.clamp(0.5, 8.0) } else { 1.0 };
        (safe_pt * safe_scale).round()
    }

    /// Total top chrome height (`70 pt` normally, `0 pt` in Presentation Mode).
    #[must_use]
    pub const fn top_chrome_height(self, presentation_mode: bool) -> f32 {
        if presentation_mode { 0.0 } else { self.app_bar_h + self.action_strip_h }
    }

    /// Bottom status footer height (`26 pt` normally, `0 pt` in Presentation Mode).
    #[must_use]
    pub const fn status_bar_height(self, presentation_mode: bool) -> f32 {
        if presentation_mode { 0.0 } else { self.status_bar_h }
    }

    /// Combined 3-tier frozen column-grid header height (`64 pt`).
    #[must_use]
    pub const fn grid_header_total_height(self) -> f32 {
        self.grid_header_tier_heights[0] + self.grid_header_tier_heights[1] + self.grid_header_tier_heights[2]
    }
}

impl Default for LayoutMetrics {
    fn default() -> Self {
        Self::CANONICAL
    }
}

/// Convert [`Rgba8`] into an [`egui::Color32`].
#[must_use]
pub const fn rgba8_to_color32(color: Rgba8) -> Color32 {
    Color32::from_rgba_premultiplied(color.r, color.g, color.b, color.a)
}

/// Convert an unpremultiplied/opaque [`egui::Color32`] to [`Rgba8`].
#[must_use]
pub const fn color32_to_rgba8(color: Color32) -> Rgba8 {
    Rgba8::rgba(color.r(), color.g(), color.b(), color.a())
}

/// Canonical 8-color Okabe-Ito categorical series palette (`gui-layout-spec.md` §9.2).
pub const OKABE_ITO_SERIES_PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x00, 0x00, 0x00),
    Color32::from_rgb(0xE6, 0x9F, 0x00),
    Color32::from_rgb(0x56, 0xB4, 0xE9),
    Color32::from_rgb(0x00, 0x9E, 0x73),
    Color32::from_rgb(0xF0, 0xE4, 0x42),
    Color32::from_rgb(0x00, 0x72, 0xB2),
    Color32::from_rgb(0xD5, 0x5E, 0x00),
    Color32::from_rgb(0xCC, 0x79, 0xA7),
];

/// Diverging Blue-White-Vermilion colorblind-safe ramp stops (`-1 -> 0 -> +1`).
pub const BLUE_WHITE_VERMILION_STOPS: [Color32; 3] =
    [Color32::from_rgb(0x00, 0x72, 0xB2), Color32::from_rgb(0xF7, 0xF7, 0xF7), Color32::from_rgb(0xD5, 0x5E, 0x00)];

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

/// Complete WCAG 2.1 AA theme, semantic role, provenance, and geometry tokens for the `egui` shell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub mode: UiThemeMode,
    pub dark: bool,
    pub locale: &'static str,
    pub ui_scale: f32,
    pub metrics: LayoutMetrics,
    pub app_bar: Color32,
    pub panel_bg: Color32,
    pub surface_bg: Color32,
    pub canvas_mat: Color32,
    pub header_bg: Color32,
    pub border: Color32,
    pub divider: Color32,
    pub hover_bg: Color32,
    pub active_bg: Color32,
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
    pub paper_white: Color32,
    pub scrim: Color32,
    pub role_num: Color32,
    pub role_cat: Color32,
    pub role_lbl: Color32,
    pub role_fx: Color32,
    pub link_blue: Color32,
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
    #[must_use]
    pub fn from_theme_spec(spec: &UiThemeSpec, locale: &str) -> Self {
        let dark = !matches!(spec.mode, UiThemeMode::Light);
        let ui_scale = if spec.ui_scale.is_finite() { spec.ui_scale.clamp(0.75, 3.0) } else { 1.0 };
        let canonical_locale = i18n::normalize_locale(locale);
        let panel_bg = rgba8_to_color32(spec.panel_bg);
        let surface_bg = rgba8_to_color32(spec.surface_bg);
        let text_primary = rgba8_to_color32(spec.text_primary);
        let text_muted = rgba8_to_color32(spec.text_muted);
        let accent = rgba8_to_color32(spec.accent);
        let border = rgba8_to_color32(spec.border);

        match spec.mode {
            UiThemeMode::Light => Self {
                mode: spec.mode,
                dark,
                locale: canonical_locale,
                ui_scale,
                metrics: LayoutMetrics::CANONICAL,
                app_bar: Color32::from_rgb(234, 237, 242),
                panel_bg,
                surface_bg,
                canvas_mat: Color32::from_rgb(237, 240, 245),
                header_bg: Color32::from_rgb(232, 236, 242),
                border,
                divider: Color32::from_rgb(214, 219, 228),
                hover_bg: Color32::from_rgb(228, 234, 244),
                active_bg: Color32::from_rgb(212, 224, 245),
                focus_ring: Color32::from_rgb(29, 78, 216),
                input_bg: Color32::WHITE,
                input_border: Color32::from_rgb(132, 140, 154),
                button_bg: Color32::from_rgb(232, 236, 242),
                button_border: Color32::from_rgb(120, 129, 145),
                text_primary,
                text_strong: Color32::from_rgb(15, 19, 26),
                text_muted,
                text_disabled: Color32::from_rgb(125, 134, 148),
                icon: Color32::from_rgb(38, 45, 58),
                accent,
                accent_strong: Color32::from_rgb(24, 64, 184),
                paper_white: rgba8_to_color32(spec.paper_white),
                scrim: Color32::from_rgba_unmultiplied(15, 23, 42, 128),
                role_num: Color32::from_rgb(29, 78, 216),
                role_cat: Color32::from_rgb(180, 62, 0),
                role_lbl: Color32::from_rgb(71, 85, 105),
                role_fx: Color32::from_rgb(4, 120, 87),
                link_blue: rgba8_to_color32(spec.link_blue),
                link_green: rgba8_to_color32(spec.link_green),
                link_red: rgba8_to_color32(spec.link_red),
                excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
            },
            UiThemeMode::Dark => Self {
                mode: spec.mode,
                dark,
                locale: canonical_locale,
                ui_scale,
                metrics: LayoutMetrics::CANONICAL,
                app_bar: Color32::from_rgb(20, 23, 29),
                panel_bg,
                surface_bg,
                canvas_mat: Color32::from_rgb(21, 24, 30),
                header_bg: Color32::from_rgb(38, 43, 53),
                border,
                divider: Color32::from_rgb(56, 63, 76),
                hover_bg: Color32::from_rgb(45, 52, 66),
                active_bg: Color32::from_rgb(52, 66, 92),
                focus_ring: Color32::from_rgb(96, 165, 250),
                input_bg: Color32::from_rgb(20, 23, 29),
                input_border: border,
                button_bg: Color32::from_rgb(43, 49, 60),
                button_border: Color32::from_rgb(112, 123, 140),
                text_primary,
                text_strong: Color32::from_rgb(248, 250, 252),
                text_muted,
                text_disabled: Color32::from_rgb(128, 138, 152),
                icon: Color32::from_rgb(212, 220, 232),
                accent,
                accent_strong: Color32::from_rgb(37, 99, 235),
                paper_white: rgba8_to_color32(spec.paper_white),
                scrim: Color32::from_rgba_unmultiplied(15, 23, 42, 128),
                role_num: Color32::from_rgb(96, 165, 250),
                role_cat: Color32::from_rgb(251, 146, 60),
                role_lbl: Color32::from_rgb(148, 163, 184),
                role_fx: Color32::from_rgb(52, 211, 153),
                link_blue: rgba8_to_color32(spec.link_blue),
                link_green: rgba8_to_color32(spec.link_green),
                link_red: rgba8_to_color32(spec.link_red),
                excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
            },
            UiThemeMode::HighContrast => Self {
                mode: spec.mode,
                dark,
                locale: canonical_locale,
                ui_scale,
                metrics: LayoutMetrics::CANONICAL,
                app_bar: Color32::BLACK,
                panel_bg,
                surface_bg,
                canvas_mat: Color32::from_rgb(8, 10, 14),
                header_bg: Color32::from_rgb(22, 26, 34),
                border,
                divider: Color32::from_rgb(160, 170, 188),
                hover_bg: Color32::from_rgb(32, 38, 52),
                active_bg: Color32::from_rgb(38, 60, 96),
                focus_ring: Color32::from_rgb(125, 211, 252),
                input_bg: Color32::BLACK,
                input_border: border,
                button_bg: Color32::from_rgb(24, 28, 36),
                button_border: Color32::from_rgb(210, 218, 230),
                text_primary,
                text_strong: Color32::WHITE,
                text_muted,
                text_disabled: Color32::from_rgb(165, 174, 188),
                icon: Color32::WHITE,
                accent,
                accent_strong: Color32::from_rgb(14, 116, 144),
                paper_white: rgba8_to_color32(spec.paper_white),
                scrim: Color32::from_rgba_unmultiplied(15, 23, 42, 128),
                role_num: Color32::from_rgb(125, 211, 252),
                role_cat: Color32::from_rgb(253, 186, 116),
                role_lbl: Color32::from_rgb(203, 213, 225),
                role_fx: Color32::from_rgb(110, 231, 183),
                link_blue: rgba8_to_color32(spec.link_blue),
                link_green: rgba8_to_color32(spec.link_green),
                link_red: rgba8_to_color32(spec.link_red),
                excluded_cell_text: rgba8_to_color32(spec.excluded_cell_text),
            },
        }
    }

    /// Retrieve active [`Tokens`] cached in `ctx`, defaulting to [`UiThemeMode::Light`].
    #[must_use]
    pub fn get(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<Self>(egui::Id::NULL)).unwrap_or_default()
    }

    /// Build [`egui::Visuals`] aligned with these WCAG 2.1 AA theme tokens.
    #[must_use]
    pub fn to_visuals(&self) -> Visuals {
        let mut visuals = if self.dark { Visuals::dark() } else { Visuals::light() };
        visuals.dark_mode = self.dark;
        visuals.override_text_color = Some(self.text_primary);
        visuals.weak_text_color = Some(self.text_muted);
        visuals.panel_fill = self.panel_bg;
        visuals.window_fill = self.surface_bg;
        visuals.extreme_bg_color = self.input_bg;
        visuals.faint_bg_color = self.header_bg;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(6);
        visuals.selection.bg_fill = self.active_bg;
        visuals.selection.stroke = Stroke::new(2.0, self.focus_ring);
        visuals.widgets.noninteractive.bg_fill = self.panel_bg;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text_primary);
        visuals.widgets.inactive.bg_fill = self.button_bg;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.button_border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.text_primary);
        visuals.widgets.hovered.bg_fill = self.hover_bg;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.focus_ring);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.text_strong);
        visuals.widgets.active.bg_fill = self.active_bg;
        visuals.widgets.active.bg_stroke = Stroke::new(2.0, self.focus_ring);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.text_strong);
        visuals.widgets.open.bg_fill = self.hover_bg;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, self.text_strong);
        visuals
    }

    /// Install these tokens, visuals, canonical typography sizes, non-overlapping scrollbar gutter,
    /// and `ui_scale` once per frame on `ctx`.
    pub fn apply_to_ctx(&self, ctx: &egui::Context) {
        let egui_theme = if self.dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, *self));
        ctx.set_theme(egui_theme);
        ctx.set_zoom_factor(self.ui_scale.clamp(0.75, 3.0));
        ctx.set_visuals(self.to_visuals());
        ctx.all_styles_mut(|style| {
            style.text_styles.insert(TextStyle::Body, FontId::proportional(self.metrics.font_ui_body_pt));
            style.text_styles.insert(TextStyle::Button, FontId::proportional(self.metrics.font_ui_body_pt));
            style.text_styles.insert(TextStyle::Small, FontId::proportional(self.metrics.font_ui_small_pt));
            style.text_styles.insert(TextStyle::Monospace, FontId::monospace(self.metrics.font_grid_numeral_pt));
            style.text_styles.insert(TextStyle::Heading, FontId::proportional(15.0));

            style.spacing.scroll.floating = false;
            style.spacing.scroll.bar_width = self.metrics.scroll_bar_thickness;
            style.spacing.scroll.bar_inner_margin = 2.0;
            style.spacing.scroll.bar_outer_margin = 2.0;
            style.spacing.interact_size.y = self.metrics.min_hit_target_pt;
        });
    }

    /// Verify all primary text and interactive UI boundary pairs satisfy WCAG 2.1 AA contrast.
    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        let text_pairs = [
            (self.text_primary, self.surface_bg),
            (self.text_primary, self.panel_bg),
            (self.text_primary, self.app_bar),
            (self.text_primary, self.header_bg),
            (self.text_muted, self.surface_bg),
            (self.text_muted, self.panel_bg),
            (self.text_muted, self.app_bar),
        ];
        let ui_pairs = [
            (self.border, self.surface_bg),
            (self.border, self.panel_bg),
            (self.input_border, self.surface_bg),
            (self.button_border, self.panel_bg),
            (self.focus_ring, self.surface_bg),
            (self.role_num, self.surface_bg),
            (self.role_cat, self.surface_bg),
            (self.role_lbl, self.surface_bg),
            (self.role_fx, self.surface_bg),
        ];
        text_pairs.into_iter().all(|(fg, bg)| color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg)) >= 4.5)
            && ui_pairs.into_iter().all(|(fg, bg)| color32_to_rgba8(fg).contrast_ratio(color32_to_rgba8(bg)) >= 3.0)
    }
}

/// User intent captured during an `egui` pass and deferred until `!ctx.will_discard()` settles.
#[derive(Debug, Clone, PartialEq)]
pub enum UiIntent {
    EngineCommand { command_id: String, params: serde_json::Value },
    UiCommand { command_id: String, params: serde_json::Value },
    SetTheme(UiThemeMode),
    SetLocale(String),
    SetUiScale(f32),
    SelectGridCell { coord: curvebook_tools::GridCellCoord },
    ExtendGridSelection { coord: curvebook_tools::GridCellCoord },
    SetRowTitlesCollapsed { collapsed: bool },
    DialogWidgetClick { widget_id: String },
    DialogButtonAction { action: String },
    CustomViewAction { domain: String, payload: String },
}

/// Outcome produced when committing a buffered [`UiIntent`].
#[derive(Debug, Clone, PartialEq)]
pub enum UiIntentOutcome {
    Engine { command_id: String, outcome: CommandOutcome },
    Ui { command_id: String, result: Option<serde_json::Value> },
    ShellUpdated,
}

/// Single-commit FIFO queue buffering GUI intents across discardable `egui` layout passes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingUiIntents {
    queue: Vec<UiIntent>,
}

impl PendingUiIntents {
    /// Construct an empty intent queue.
    #[must_use]
    pub const fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// Discard all buffered intents without executing them.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Number of queued intents awaiting frame settlement.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns `true` when no intents are buffered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Inspect queued intents in FIFO order.
    #[must_use]
    pub fn intents(&self) -> &[UiIntent] {
        &self.queue
    }

    /// Enqueue a typed [`UiIntent`].
    pub fn push(&mut self, intent: UiIntent) {
        self.queue.push(intent);
    }

    /// Enqueue an engine command to execute once via `Caller::Ui` after layout settles.
    pub fn push_engine(&mut self, command_id: impl Into<String>, params: serde_json::Value) {
        self.queue.push(UiIntent::EngineCommand { command_id: command_id.into(), params });
    }

    /// Enqueue a front-end `ui.*` / `app.*` shell command to execute once after layout settles.
    pub fn push_ui(&mut self, command_id: impl Into<String>, params: serde_json::Value) {
        self.queue.push(UiIntent::UiCommand { command_id: command_id.into(), params });
    }

    /// Enqueue single-cell selection sync for `coord` without mutating `Session`.
    pub fn push_select_grid_cell(&mut self, coord: curvebook_tools::GridCellCoord) {
        self.queue.push(UiIntent::SelectGridCell { coord });
    }

    /// Enqueue O(1) rectangular selection extension to `coord` (`Shift+Click` / drag).
    pub fn push_extend_grid_selection(&mut self, coord: curvebook_tools::GridCellCoord) {
        self.queue.push(UiIntent::ExtendGridSelection { coord });
    }

    /// Enqueue Row Titles column collapse/expand state update.
    pub fn push_set_row_titles_collapsed(&mut self, collapsed: bool) {
        self.queue.push(UiIntent::SetRowTitlesCollapsed { collapsed });
    }

    /// Enqueue a dialog control click (`edit.assume_gaussian`, `edit.post_hoc.tukey`, etc.).
    pub fn push_dialog_widget_click(&mut self, widget_id: impl Into<String>) {
        self.queue.push(UiIntent::DialogWidgetClick { widget_id: widget_id.into() });
    }

    /// Enqueue a dialog footer button action (`"ok"`, `"cancel"`, `"apply"`, `"create"`).
    pub fn push_dialog_button(&mut self, action: impl Into<String>) {
        self.queue.push(UiIntent::DialogButtonAction { action: action.into() });
    }

    /// Enqueue a view-shard custom action (`"stat_params"`, `"grid_core"`, etc.).
    pub fn push_custom_view_action(&mut self, domain: impl Into<String>, payload: impl Into<String>) {
        self.queue.push(UiIntent::CustomViewAction { domain: domain.into(), payload: payload.into() });
    }

    /// Commit buffered intents once via `Session::execute_as_ui` (`Caller::Ui`) when `!will_discard`.
    ///
    /// When `will_discard` is `true`, clears the buffer without executing any mutations so
    /// multi-pass `ctx.request_discard()` layouts never duplicate undo steps or journal entries.
    ///
    /// # Errors
    /// Returns [`EngineError`] if any queued engine or UI command fails validation or preconditions.
    pub fn commit_if_settled(&mut self, app: &mut UiApp, will_discard: bool) -> Result<Vec<UiIntentOutcome>, EngineError> {
        let pending = std::mem::take(&mut self.queue);
        if will_discard || pending.is_empty() {
            return Ok(Vec::new());
        }
        let mut outcomes = Vec::with_capacity(pending.len());
        for intent in pending {
            outcomes.push(apply_single_ui_intent(app, intent)?);
        }
        app.sync_views();
        Ok(outcomes)
    }

    /// Commit only [`UiIntent::EngineCommand`] items directly on `session` (`Caller::Ui`) when `!will_discard`.
    ///
    /// # Errors
    /// Returns [`EngineError`] if any queued command fails.
    pub fn commit_engine_intents_on_session(&mut self, session: &mut Session, will_discard: bool) -> Result<Vec<CommandOutcome>, EngineError> {
        let pending = std::mem::take(&mut self.queue);
        if will_discard || pending.is_empty() {
            return Ok(Vec::new());
        }
        let mut outcomes = Vec::new();
        for intent in pending {
            if let UiIntent::EngineCommand { command_id, params } = intent {
                outcomes.push(session.execute_as_ui(&command_id, params)?);
            }
        }
        Ok(outcomes)
    }
}

fn apply_single_ui_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::EngineCommand { command_id, params } => {
            let outcome = app.execute_session_ui_cmd(&command_id, params)?;
            Ok(UiIntentOutcome::Engine { command_id, outcome })
        }
        UiIntent::UiCommand { command_id, params } => {
            let result = dispatch_ui_command_as_ui(app, &command_id, &params)?;
            Ok(UiIntentOutcome::Ui { command_id, result })
        }
        UiIntent::SetTheme(mode) => {
            let prev_scale = app.theme.ui_scale;
            app.set_theme_mode(mode);
            app.theme.ui_scale = prev_scale;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetLocale(locale) => {
            let _ = app.set_locale(&locale);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetUiScale(scale) => {
            let clamped = if scale.is_finite() { scale.clamp(0.75, 3.0) } else { 1.0 };
            app.theme.ui_scale = clamped;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SelectGridCell { coord } => {
            app.grid.select_single_sync(coord);
            app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ExtendGridSelection { coord } => {
            let mut grid = std::mem::take(&mut app.grid);
            if let Some(active_id) = app.document().active_sheet_id.as_ref()
                && let Some(ds) = app.document().data_sheet(active_id)
            {
                grid.extend_shift_range(ds, coord);
            }
            app.grid = grid;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetRowTitlesCollapsed { collapsed } => {
            app.grid.row_titles_collapsed_override = Some(collapsed);
            app.view_shards.grid_core.row_titles_collapsed = collapsed;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::DialogWidgetClick { widget_id } => {
            let result = crate::gui_actions::apply_dialog_widget_click(app, &widget_id)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.widget:{widget_id}"), result })
        }
        UiIntent::DialogButtonAction { action } => {
            let result = crate::gui_actions::execute_dialog_button_action(app, &action)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.button:{action}"), result: Some(result) })
        }
        UiIntent::CustomViewAction { domain, payload } => {
            let mut shards = std::mem::take(&mut app.view_shards);
            let res = crate::views::dispatch_custom_view_action(app, &mut shards, &domain, &payload);
            app.view_shards = shards;
            let outcome = res.map_err(|msg| EngineError::with_code(curvebook_doc::ErrorCode::Conflict, msg))?;
            Ok(UiIntentOutcome::Ui {
                command_id: outcome.command_id.unwrap_or_else(|| format!("{domain}:{payload}")),
                result: outcome.command_result,
            })
        }
    }
}

/// Metadata recorded for a single rendered `egui` widget during a settled frame pass.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInspectNodeMeta {
    pub egui_id: egui::Id,
    pub accesskit_id: egui::accesskit::NodeId,
    pub widget_id: String,
    pub role: String,
    pub label: String,
    pub command_id: Option<String>,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    /// Interaction rectangle clipped against the active UI clip rect (logical `pt`).
    pub clipped_rect: Rect,
    pub z_layer: ZLayer,
    pub ellipsized: bool,
    pub full_label: Option<String>,
}

/// Parameter bundle for [`FrameInspectMeta::record_response`].
#[derive(Debug, Clone)]
pub struct FrameInspectEntryInput<'a> {
    pub widget_id: &'a str,
    pub role: &'a str,
    pub label: &'a str,
    pub command_id: Option<&'a str>,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub z_layer: ZLayer,
    pub ellipsized: bool,
    pub full_label: Option<&'a str>,
}

/// Settled-pass (`!ctx.will_discard()`) side-table mapping `egui`/AccessKit nodes to CurveBook
/// `widget_id`, `command_id`, `disabled_reason`, clipped interaction `Rect`, and ellipsis metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInspectMeta {
    pub pass_nr: u64,
    pub scale_factor: f32,
    pub viewport_rect: Rect,
    pub entries: Vec<FrameInspectNodeMeta>,
}

impl Default for FrameInspectMeta {
    fn default() -> Self {
        Self { pass_nr: 0, scale_factor: 1.0, viewport_rect: Rect::NOTHING, entries: Vec::new() }
    }
}

impl FrameInspectMeta {
    /// Construct an empty settled-frame side-table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset side-table entries for a fresh frame pass.
    pub fn begin_pass(&mut self, ctx: &egui::Context) {
        self.pass_nr = ctx.cumulative_pass_nr();
        self.scale_factor = ctx.pixels_per_point();
        self.viewport_rect = ctx.content_rect();
        self.entries.clear();
    }

    /// Clear all recorded entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of recorded widget entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when no widget entries are recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a pre-built [`FrameInspectNodeMeta`] entry.
    pub fn record(&mut self, entry: FrameInspectNodeMeta) {
        self.entries.push(entry);
    }

    /// Record inspection metadata from an `egui::Response`, clipping its interaction rect to `ui.clip_rect()`.
    pub fn record_response(&mut self, ui: &egui::Ui, resp: &egui::Response, input: FrameInspectEntryInput<'_>) {
        let clipped_rect = resp.interact_rect.intersect(ui.clip_rect());
        self.entries.push(FrameInspectNodeMeta {
            egui_id: resp.id,
            accesskit_id: resp.id.accesskit_id(),
            widget_id: input.widget_id.to_owned(),
            role: input.role.to_owned(),
            label: input.label.to_owned(),
            command_id: input.command_id.map(str::to_owned),
            value: input.value,
            enabled: input.enabled,
            disabled_reason: input.disabled_reason.map(str::to_owned),
            clipped_rect,
            z_layer: input.z_layer,
            ellipsized: input.ellipsized,
            full_label: input.full_label.map(str::to_owned),
        });
    }

    /// Look up the first recorded entry matching `widget_id`.
    #[must_use]
    pub fn find_by_widget_id(&self, widget_id: &str) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.widget_id == widget_id)
    }

    /// Look up the first recorded entry matching `egui_id`.
    #[must_use]
    pub fn find_by_egui_id(&self, egui_id: egui::Id) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.egui_id == egui_id)
    }

    /// Look up the first recorded entry matching `accesskit_id`.
    #[must_use]
    pub fn find_by_accesskit_id(&self, accesskit_id: egui::accesskit::NodeId) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.accesskit_id == accesskit_id)
    }
}

/// Geometry and DPI synchronization epoch used to prevent stale pre-resize `ui.screenshot` captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct GeometryStamp {
    pub frame_seq: u64,
    pub pass_nr: u64,
    pub physical_width_px: u32,
    pub physical_height_px: u32,
    /// Scale factor quantized in basis points (`round(pixels_per_point * 10_000.0)`).
    pub scale_bps: u32,
    pub settled: bool,
}

impl GeometryStamp {
    /// Quantize a `pixels_per_point` float into integer basis points (`10_000` == `1.0x`).
    #[must_use]
    pub fn quantize_scale(pixels_per_point: f32) -> u32 {
        let clamped = if pixels_per_point.is_finite() { pixels_per_point.clamp(0.5, 8.0) } else { 1.0 };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bps = (f64::from(clamped) * 10_000.0).round() as u32;
        bps
    }

    /// Construct a [`GeometryStamp`] from a settled/unsettled `egui::Context` frame pass.
    #[must_use]
    pub fn from_context(frame_seq: u64, ctx: &egui::Context, physical_size_px: [u32; 2], settled: bool) -> Self {
        Self {
            frame_seq,
            pass_nr: ctx.cumulative_pass_nr(),
            physical_width_px: physical_size_px[0],
            physical_height_px: physical_size_px[1],
            scale_bps: Self::quantize_scale(ctx.pixels_per_point()),
            settled,
        }
    }

    /// Return `true` when this stamp is settled (`!ctx.will_discard()`) and matches the requested
    /// framebuffer pixel dimensions and scale factor.
    #[must_use]
    pub fn matches_target(&self, width_px: u32, height_px: u32, pixels_per_point: f32) -> bool {
        self.settled
            && self.physical_width_px == width_px
            && self.physical_height_px == height_px
            && self.scale_bps == Self::quantize_scale(pixels_per_point)
    }

    /// Recover the floating-point `pixels_per_point` scale factor from [`Self::scale_bps`].
    #[must_use]
    pub fn scale_factor(self) -> f32 {
        (f64::from(self.scale_bps.max(5_000)) / 10_000.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrameInspectEntryInput, FrameInspectMeta, GeometryStamp, LayoutMetrics, PendingUiIntents, ScientificPaletteTokens, Tokens, UiBackend,
        color32_to_rgba8,
    };
    use crate::UiApp;
    use crate::layout::ZLayer;
    use crate::theme::UiThemeMode;
    use serde_json::json;

    #[test]
    fn backend_switch_parses_env_and_cli_with_legacy_default() {
        assert_eq!(UiBackend::from_env_and_args(None, None), Ok(UiBackend::Legacy));
        assert_eq!(UiBackend::from_env_and_args(Some("egui"), None), Ok(UiBackend::Egui));
        assert_eq!(UiBackend::from_env_and_args(Some("egui"), Some("legacy")), Ok(UiBackend::Legacy));
        assert_eq!(UiBackend::from_env_and_args(Some("legacy"), Some("EGUI")), Ok(UiBackend::Egui));
        assert!(UiBackend::from_env_and_args(None, Some("unknown")).is_err());
    }

    #[test]
    fn canonical_tokens_meet_wcag_aa_and_preserve_scientific_palettes() {
        for mode in UiThemeMode::ALL {
            let tokens = Tokens::for_mode(mode);
            assert!(tokens.meets_wcag_aa(), "WCAG 2.1 AA failed for {mode:?}");
        }
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height(false), 70.0);
        assert_eq!(LayoutMetrics::CANONICAL.top_chrome_height(true), 0.0);
        assert_eq!(LayoutMetrics::CANONICAL.status_bar_height(false), 26.0);
        assert_eq!(LayoutMetrics::CANONICAL.grid_header_total_height(), 64.0);
        assert_eq!(ScientificPaletteTokens::okabe_ito_series().len(), 8);
        assert_eq!(ScientificPaletteTokens::viridis_stops().len(), 17);
        assert_eq!(ScientificPaletteTokens::cividis_stops().len(), 17);
        assert_eq!(ScientificPaletteTokens::blue_white_vermilion_stops().len(), 3);
        let first = color32_to_rgba8(ScientificPaletteTokens::okabe_ito_series()[0]);
        assert_eq!(first, curvebook_palette::Rgba8::BLACK);
    }

    #[test]
    fn pending_intents_discard_on_multi_pass_and_commit_once_when_settled() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        intents.push_engine("sheet.createData", json!({"sheet_id": "d-1", "title": "Discard Test", "kind": "column", "auto_graph": false}));
        let discarded = intents.commit_if_settled(&mut app, true)?;
        assert!(discarded.is_empty());
        assert!(intents.is_empty());
        assert_eq!(app.document().total_sheet_count(), 0);

        intents.push_engine("sheet.createData", json!({"sheet_id": "d-1", "title": "Committed Test", "kind": "column", "auto_graph": false}));
        let committed = intents.commit_if_settled(&mut app, false)?;
        assert_eq!(committed.len(), 1);
        assert_eq!(app.document().total_sheet_count(), 1);
        assert_eq!(app.session().undo_depth(), 1);
        Ok(())
    }

    #[test]
    fn frame_inspect_meta_clips_rects_and_geometry_stamp_validates_target() {
        let ctx = egui::Context::default();
        let mut meta = FrameInspectMeta::new();
        let mut captured_id = None;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
            |ui| {
                meta.begin_pass(ui.ctx());
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(120.0, 28.0), egui::Sense::click());
                captured_id = Some(resp.id);
                meta.record_response(
                    ui,
                    &resp,
                    FrameInspectEntryInput {
                        widget_id: "toolbar.analyze",
                        role: "Button",
                        label: "Analyze...",
                        command_id: Some("ui.analyze.open"),
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                assert!(rect.width() >= 28.0);
            },
        );
        output.textures_delta.clear();
        let hit = meta.find_by_widget_id("toolbar.analyze").expect("entry recorded");
        assert_eq!(hit.command_id.as_deref(), Some("ui.analyze.open"));
        assert_eq!(Some(hit.egui_id), captured_id);
        assert!(hit.clipped_rect.width() > 0.0);

        let stale = GeometryStamp::from_context(1, &ctx, [640, 420], false);
        assert!(!stale.matches_target(640, 420, 1.0));
        let settled = GeometryStamp::from_context(2, &ctx, [1280, 840], true);
        assert!(settled.matches_target(1280, 840, 1.0));
        assert!(!settled.matches_target(640, 420, 1.0));
    }

    #[test]
    fn pending_intents_route_grid_and_dialog_actions_once_settled() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        intents.push_select_grid_cell(curvebook_tools::GridCellCoord::new(2, 1, 0));
        intents.push_set_row_titles_collapsed(true);
        let outcomes = intents.commit_if_settled(&mut app, false)?;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(2, 1, 0)));
        assert_eq!(app.focus_widget_id.as_deref(), Some("grid.cell.2.1.0"));
        assert_eq!(app.grid.row_titles_collapsed_override, Some(true));
        Ok(())
    }
}
