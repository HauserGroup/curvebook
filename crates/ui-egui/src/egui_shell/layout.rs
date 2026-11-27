//! Responsive `egui_shell` layout solver, viewport tiers, canonical `1.0x` geometry metrics,
//! and workspace dock presets (`docs/planning/gui-layout-spec.md` §2.3, §9.1, & §10).

use serde::{Deserialize, Serialize};

/// Pre-configured dock visibility & width profiles surfaced by Tier 1's Workspace Preset Switcher (`R15.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WorkspacePreset {
    #[default]
    Default,
    DataPrep,
    Modeling,
    Publication,
    Compact,
}

/// Dock configuration implied by applying a [`WorkspacePreset`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresetDockConfig {
    pub navigator_open: bool,
    pub family_open: bool,
    pub inspector_open: bool,
    pub leading_collapsed_rail: bool,
    pub trailing_collapsed_rail: bool,
    pub leading_width_pt: f32,
    pub trailing_width_pt: f32,
}

impl WorkspacePreset {
    /// Ordered list of workspace presets exposed in Tier 1 switcher controls.
    pub const ALL: [Self; 5] = [Self::Default, Self::DataPrep, Self::Modeling, Self::Publication, Self::Compact];

    /// Canonical identifier string for telemetry, intents, and persistence.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::DataPrep => "data_prep",
            Self::Modeling => "modeling",
            Self::Publication => "publication",
            Self::Compact => "compact",
        }
    }

    /// Parse a serialized preset slug case-insensitively.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "default" => Some(Self::Default),
            "data_prep" | "dataprep" | "data prep" => Some(Self::DataPrep),
            "modeling" => Some(Self::Modeling),
            "publication" => Some(Self::Publication),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }

    /// Concise localized-ready display label for Tier 1 pill UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::DataPrep => "Data Prep",
            Self::Modeling => "Modeling",
            Self::Publication => "Publication",
            Self::Compact => "Compact",
        }
    }

    /// Default dock visibility and width profile for this workspace preset.
    #[must_use]
    pub const fn default_dock_config(self) -> PresetDockConfig {
        match self {
            Self::Default => PresetDockConfig {
                navigator_open: true,
                family_open: true,
                inspector_open: true,
                leading_collapsed_rail: false,
                trailing_collapsed_rail: false,
                leading_width_pt: 240.0,
                trailing_width_pt: 280.0,
            },
            Self::DataPrep => PresetDockConfig {
                navigator_open: true,
                family_open: false,
                inspector_open: true,
                leading_collapsed_rail: false,
                trailing_collapsed_rail: true,
                leading_width_pt: 220.0,
                trailing_width_pt: 280.0,
            },
            Self::Modeling => PresetDockConfig {
                navigator_open: true,
                family_open: true,
                inspector_open: true,
                leading_collapsed_rail: false,
                trailing_collapsed_rail: false,
                leading_width_pt: 210.0,
                trailing_width_pt: 320.0,
            },
            Self::Publication => PresetDockConfig {
                navigator_open: true,
                family_open: false,
                inspector_open: true,
                leading_collapsed_rail: true,
                trailing_collapsed_rail: false,
                leading_width_pt: 240.0,
                trailing_width_pt: 300.0,
            },
            Self::Compact => PresetDockConfig {
                navigator_open: true,
                family_open: false,
                inspector_open: true,
                leading_collapsed_rail: true,
                trailing_collapsed_rail: true,
                leading_width_pt: 210.0,
                trailing_width_pt: 260.0,
            },
        }
    }
}

/// Canonical `1.0x` logical-point geometry, spacing, typography, radius, and motion tokens
/// (`docs/planning/gui-layout-spec.md` §9.1 & §10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutMetrics {
    pub app_bar_h: f32,
    pub action_strip_h: f32,
    pub panel_header_h: f32,
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
    pub split_center_preferred_w: f32,
    pub split_vertical_breakpoint_w: f32,
    pub split_vertical_min_h: f32,
    pub grid_row_h: f32,
    pub grid_header_tier_heights: [f32; 3],
    pub min_hit_target_pt: f32,
    pub min_toolbar_target_pt: f32,
    pub scroll_bar_thickness: f32,
    pub scroll_bar_hover_thickness: f32,
    pub font_ui_body_pt: f32,
    pub font_ui_small_pt: f32,
    pub font_grid_numeral_pt: f32,
    pub font_ui_section_pt: f32,
    pub font_ui_title_pt: f32,
    pub font_ui_display_pt: f32,
    pub space_xxs: f32,
    pub space_xs: f32,
    pub space_sm: f32,
    pub space_md: f32,
    pub space_lg: f32,
    pub space_xl: f32,
    pub space_xxl: f32,
    pub radius_sm: u8,
    pub radius_md: u8,
    pub radius_lg: u8,
    pub stroke_thin_pt: f32,
    pub stroke_focus_pt: f32,
    pub max_motion_sec: f32,
    pub min_window_w_pt: f32,
    pub min_window_h_pt: f32,
    pub default_window_w_pt: f32,
    pub default_window_h_pt: f32,
    pub wide_desktop_breakpoint_w: f32,
}

impl LayoutMetrics {
    /// Canonical `1.0x` logical-point metrics from `docs/planning/gui-layout-spec.md` §9.1 & §10.
    pub const CANONICAL: Self = Self {
        app_bar_h: 28.0,
        action_strip_h: 38.0,
        panel_header_h: 32.0,
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
        split_center_preferred_w: 760.0,
        split_vertical_breakpoint_w: 720.0,
        split_vertical_min_h: 520.0,
        grid_row_h: 24.0,
        grid_header_tier_heights: [24.0, 24.0, 24.0],
        min_hit_target_pt: 24.0,
        min_toolbar_target_pt: 28.0,
        scroll_bar_thickness: 10.0,
        scroll_bar_hover_thickness: 12.0,
        font_ui_body_pt: 13.0,
        font_ui_small_pt: 11.5,
        font_grid_numeral_pt: 13.0,
        font_ui_section_pt: 13.0,
        font_ui_title_pt: 15.0,
        font_ui_display_pt: 18.0,
        space_xxs: 2.0,
        space_xs: 4.0,
        space_sm: 6.0,
        space_md: 8.0,
        space_lg: 12.0,
        space_xl: 16.0,
        space_xxl: 24.0,
        radius_sm: 2,
        radius_md: 4,
        radius_lg: 6,
        stroke_thin_pt: 1.0,
        stroke_focus_pt: 2.0,
        max_motion_sec: 0.15,
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

    /// Total top chrome height with in-window menu bar (`66 pt`: `28 pt` menu bar + `38 pt` unified toolbar).
    #[must_use]
    pub const fn top_chrome_height(self) -> f32 {
        self.app_bar_h + self.action_strip_h
    }

    /// Total top chrome height conditioned on whether the host OS renders the menu bar (`R20.9`).
    #[must_use]
    pub const fn top_chrome_height_for_host(self, host_draws_menu_bar: bool) -> f32 {
        if host_draws_menu_bar { self.action_strip_h } else { self.app_bar_h + self.action_strip_h }
    }

    /// Bottom status footer height (`26 pt`).
    #[must_use]
    pub const fn status_bar_height(self) -> f32 {
        self.status_bar_h
    }

    /// Combined 3-tier frozen column-grid header height (`72 pt`).
    #[must_use]
    pub const fn grid_header_total_height(self) -> f32 {
        self.grid_header_tier_heights[0] + self.grid_header_tier_heights[1] + self.grid_header_tier_heights[2]
    }

    /// Solve responsive dock collapse, splitter widths, and Center Stage budget (`gui-layout-spec.md` §2.3).
    #[must_use]
    pub fn solve_shell_layout(self, input: ShellLayoutInput) -> ResponsiveShellLayout {
        self.solve_split_shell_layout(input, false)
    }

    /// Solve responsive dock collapse and Split Comparison usable-width degradation (`760 pt` preferred /
    /// `720 pt` hard horizontal floor / `< 720 × 520 pt` compact single-pane flip; `gui-layout-spec.md` §2.3).
    #[must_use]
    pub fn solve_split_shell_layout(self, input: ShellLayoutInput, split_active: bool) -> ResponsiveShellLayout {
        let w = sanitize_nonneg(input.viewport_w_pt);
        let h = sanitize_nonneg(input.viewport_h_pt);
        let tier = if w >= self.wide_desktop_breakpoint_w {
            ViewportTier::WideDesktop
        } else if w >= self.inspector_collapse_breakpoint_w {
            ViewportTier::StandardLaptop
        } else if w >= self.min_window_w_pt {
            ViewportTier::CompactTablet
        } else {
            ViewportTier::NarrowHeadless
        };

        let tier1_app_bar_h_pt = if input.host_draws_menu_bar { 0.0 } else { self.app_bar_h.min(h * 0.45).max(0.0) };
        let tier2_action_strip_h_pt = self.action_strip_h.min((h - tier1_app_bar_h_pt).max(0.0) * 0.65).max(0.0);
        let top_chrome_h_pt = tier1_app_bar_h_pt + tier2_action_strip_h_pt;
        let status_bar_h_pt = self.status_bar_h.min((h - top_chrome_h_pt).max(0.0)).max(0.0);
        let center_stage_h_pt = (h - top_chrome_h_pt - status_bar_h_pt).max(0.0);

        let resolve_split_orientation = |center_w: f32, center_h: f32| -> SplitOrientation {
            if !split_active {
                if w >= self.split_vertical_breakpoint_w { SplitOrientation::Horizontal } else { SplitOrientation::Vertical }
            } else if center_w >= self.split_vertical_breakpoint_w {
                SplitOrientation::Horizontal
            } else if center_h >= self.split_vertical_min_h {
                SplitOrientation::Vertical
            } else {
                SplitOrientation::CompactSinglePane
            }
        };

        if w < 420.0 {
            return ResponsiveShellLayout {
                tier,
                top_chrome_h_pt,
                tier1_app_bar_h_pt,
                tier2_action_strip_h_pt,
                status_bar_h_pt,
                leading_dock_w_pt: 0.0,
                leading_collapsed_to_rail: false,
                leading_visible: false,
                leading_splitter_w_pt: 0.0,
                family_visible: false,
                trailing_dock_w_pt: 0.0,
                trailing_collapsed_to_rail: false,
                trailing_visible: false,
                trailing_splitter_w_pt: 0.0,
                center_stage_w_pt: w,
                center_stage_h_pt,
                split_orientation: resolve_split_orientation(w, center_stage_h_pt),
                rtl_mirrored: input.is_rtl,
            };
        }

        if input.empty_workspace {
            let show_rails = w >= self.min_window_w_pt;
            let lead_w = if show_rails && input.navigator_open { self.dock_collapsed_rail_w } else { 0.0 };
            let trail_w = if show_rails && input.inspector_open { self.dock_collapsed_rail_w } else { 0.0 };
            let center_w = (w - lead_w - trail_w).max(0.0);
            return ResponsiveShellLayout {
                tier,
                top_chrome_h_pt,
                tier1_app_bar_h_pt,
                tier2_action_strip_h_pt,
                status_bar_h_pt,
                leading_dock_w_pt: lead_w,
                leading_collapsed_to_rail: lead_w > 0.0,
                leading_visible: lead_w > 0.0,
                leading_splitter_w_pt: 0.0,
                family_visible: false,
                trailing_dock_w_pt: trail_w,
                trailing_collapsed_to_rail: trail_w > 0.0,
                trailing_visible: trail_w > 0.0,
                trailing_splitter_w_pt: 0.0,
                center_stage_w_pt: center_w,
                center_stage_h_pt,
                split_orientation: resolve_split_orientation(center_w, center_stage_h_pt),
                rtl_mirrored: input.is_rtl,
            };
        }

        let rail = self.dock_collapsed_rail_w;
        let split = self.splitter_visual_w;
        let mut lead_collapsed = !input.navigator_open || input.leading_user_collapsed || w < self.navigator_collapse_breakpoint_w;
        let mut trail_collapsed = !input.inspector_open || input.trailing_user_collapsed || w < self.inspector_collapse_breakpoint_w;

        let mut lead_w = if lead_collapsed {
            rail
        } else {
            input.leading_width_override_pt.unwrap_or(self.navigator_default_w).clamp(self.navigator_min_w, self.navigator_max_w)
        };
        let mut trail_w = if trail_collapsed {
            rail
        } else {
            input.trailing_width_override_pt.unwrap_or(self.inspector_default_w).clamp(self.inspector_min_w, self.inspector_max_w)
        };
        let mut lead_split = if lead_collapsed { 0.0 } else { split };
        let mut trail_split = if trail_collapsed { 0.0 } else { split };

        let target_center_min = if split_active {
            self.split_center_preferred_w
        } else if !lead_collapsed && !trail_collapsed && w >= self.inspector_collapse_breakpoint_w {
            480.0
        } else {
            self.center_stage_min_w
        };

        let mut center_w = w - lead_w - lead_split - trail_w - trail_split;
        if center_w < target_center_min {
            let deficit = target_center_min - center_w;
            if !trail_collapsed && !lead_collapsed {
                let lead_slack = (lead_w - self.navigator_min_w).max(0.0);
                let trail_slack = (trail_w - self.inspector_min_w).max(0.0);
                let total_slack = lead_slack + trail_slack;
                if total_slack > 0.0 {
                    let take = deficit.min(total_slack);
                    lead_w -= take * (lead_slack / total_slack);
                    trail_w -= take * (trail_slack / total_slack);
                }
            } else if !lead_collapsed {
                let lead_slack = (lead_w - self.navigator_min_w).max(0.0);
                lead_w -= deficit.min(lead_slack);
            } else if !trail_collapsed {
                let trail_slack = (trail_w - self.inspector_min_w).max(0.0);
                trail_w -= deficit.min(trail_slack);
            }
            center_w = w - lead_w - lead_split - trail_w - trail_split;
        }

        let collapse_floor = if split_active { self.split_center_preferred_w } else { self.center_stage_min_w };
        if center_w < collapse_floor && !trail_collapsed {
            trail_collapsed = true;
            trail_w = rail;
            trail_split = 0.0;
            center_w = w - lead_w - lead_split - trail_w - trail_split;
        }
        if center_w < collapse_floor && !lead_collapsed {
            lead_collapsed = true;
            lead_w = rail;
            lead_split = 0.0;
            center_w = w - lead_w - lead_split - trail_w - trail_split;
        }

        let final_center_w = center_w.max(0.0);
        ResponsiveShellLayout {
            tier,
            top_chrome_h_pt,
            tier1_app_bar_h_pt,
            tier2_action_strip_h_pt,
            status_bar_h_pt,
            leading_dock_w_pt: lead_w,
            leading_collapsed_to_rail: lead_collapsed,
            leading_visible: lead_w > 0.0,
            leading_splitter_w_pt: lead_split,
            family_visible: !lead_collapsed && input.family_open,
            trailing_dock_w_pt: trail_w,
            trailing_collapsed_to_rail: trail_collapsed,
            trailing_visible: trail_w > 0.0,
            trailing_splitter_w_pt: trail_split,
            center_stage_w_pt: final_center_w,
            center_stage_h_pt,
            split_orientation: resolve_split_orientation(final_center_w, center_stage_h_pt),
            rtl_mirrored: input.is_rtl,
        }
    }
}

const fn sanitize_nonneg(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 { value } else { 0.0 }
}

/// Responsive viewport width tier (`gui-layout-spec.md` §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ViewportTier {
    WideDesktop,
    #[default]
    StandardLaptop,
    CompactTablet,
    NarrowHeadless,
}

/// Center split orientation derived from settled logical center area (`gui-layout-spec.md` §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SplitOrientation {
    #[default]
    Horizontal,
    Vertical,
    /// `< 720 × 520 pt` single-pane header flip fallback preserving both materialized split IDs.
    CompactSinglePane,
}

/// Pure input parameters evaluated by [`LayoutMetrics::solve_shell_layout`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellLayoutInput {
    pub viewport_w_pt: f32,
    pub viewport_h_pt: f32,
    pub empty_workspace: bool,
    pub navigator_open: bool,
    pub inspector_open: bool,
    pub family_open: bool,
    pub leading_user_collapsed: bool,
    pub trailing_user_collapsed: bool,
    pub leading_width_override_pt: Option<f32>,
    pub trailing_width_override_pt: Option<f32>,
    pub host_draws_menu_bar: bool,
    pub is_rtl: bool,
}

/// Deterministic panel dimensions and collapse state computed once per frame for `egui_shell`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsiveShellLayout {
    pub tier: ViewportTier,
    pub top_chrome_h_pt: f32,
    pub tier1_app_bar_h_pt: f32,
    pub tier2_action_strip_h_pt: f32,
    pub status_bar_h_pt: f32,
    pub leading_dock_w_pt: f32,
    pub leading_collapsed_to_rail: bool,
    pub leading_visible: bool,
    pub leading_splitter_w_pt: f32,
    pub family_visible: bool,
    pub trailing_dock_w_pt: f32,
    pub trailing_collapsed_to_rail: bool,
    pub trailing_visible: bool,
    pub trailing_splitter_w_pt: f32,
    pub center_stage_w_pt: f32,
    pub center_stage_h_pt: f32,
    pub split_orientation: SplitOrientation,
    pub rtl_mirrored: bool,
}

impl Default for LayoutMetrics {
    fn default() -> Self {
        Self::CANONICAL
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutMetrics, ShellLayoutInput, SplitOrientation, ViewportTier, WorkspacePreset};

    #[test]
    fn responsive_shell_layout_enforces_spec_breakpoints() {
        let metrics = LayoutMetrics::CANONICAL;
        let base_in = ShellLayoutInput {
            viewport_w_pt: 1440.0,
            viewport_h_pt: 900.0,
            empty_workspace: false,
            navigator_open: true,
            inspector_open: true,
            family_open: true,
            leading_user_collapsed: false,
            trailing_user_collapsed: false,
            leading_width_override_pt: None,
            trailing_width_override_pt: None,
            host_draws_menu_bar: false,
            is_rtl: false,
        };

        let rtl_wide = metrics.solve_shell_layout(ShellLayoutInput { is_rtl: true, ..base_in });
        assert!(rtl_wide.rtl_mirrored);
        let wide = metrics.solve_shell_layout(base_in);
        assert!(!wide.rtl_mirrored);
        assert_eq!(wide.tier, ViewportTier::WideDesktop);
        assert!(!wide.leading_collapsed_to_rail);
        assert!(!wide.trailing_collapsed_to_rail);
        assert!(wide.center_stage_w_pt >= 900.0);
        assert_eq!(wide.split_orientation, SplitOrientation::Horizontal);

        let laptop = metrics.solve_shell_layout(ShellLayoutInput { viewport_w_pt: 960.0, ..base_in });
        assert_eq!(laptop.tier, ViewportTier::StandardLaptop);
        assert!(!laptop.leading_collapsed_to_rail);
        assert!(!laptop.trailing_collapsed_to_rail);
        assert!(laptop.center_stage_w_pt >= 480.0);

        let tablet_900 = metrics.solve_shell_layout(ShellLayoutInput { viewport_w_pt: 900.0, ..base_in });
        assert_eq!(tablet_900.tier, ViewportTier::CompactTablet);
        assert!(!tablet_900.leading_collapsed_to_rail);
        assert!(tablet_900.trailing_collapsed_to_rail);
        assert_eq!(tablet_900.trailing_dock_w_pt, 36.0);
        assert!(tablet_900.center_stage_w_pt >= 360.0);

        let tablet_640 = metrics.solve_shell_layout(ShellLayoutInput { viewport_w_pt: 640.0, ..base_in });
        assert!(tablet_640.leading_collapsed_to_rail);
        assert!(tablet_640.trailing_collapsed_to_rail);
        assert_eq!(tablet_640.split_orientation, SplitOrientation::Vertical);
        assert!(tablet_640.center_stage_w_pt >= 360.0);

        assert_eq!((wide.tier1_app_bar_h_pt, wide.tier2_action_strip_h_pt, wide.top_chrome_h_pt), (28.0, 38.0, 66.0));
        let host_menu_wide = metrics.solve_shell_layout(ShellLayoutInput { host_draws_menu_bar: true, ..base_in });
        assert_eq!((host_menu_wide.tier1_app_bar_h_pt, host_menu_wide.tier2_action_strip_h_pt, host_menu_wide.top_chrome_h_pt), (0.0, 38.0, 38.0));
        assert_eq!(host_menu_wide.center_stage_h_pt - wide.center_stage_h_pt, 28.0);

        for preset in WorkspacePreset::ALL {
            assert_eq!(WorkspacePreset::parse(preset.as_str()), Some(preset));
        }
    }
}
