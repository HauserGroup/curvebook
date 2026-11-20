// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Copyright (c) 2026 CurveBook contributors
// Adapted from upstream MIT OR Apache-2.0 pure-egui helpers; see ATTRIBUTION.md and NOTICE.

//! Deterministic top-bar right-group width fitting solver (`docs/planning/gui-layout-spec.md` §3.1).

/// Square icon button side length in the Tier 1 Application Bar (`28 pt`).
pub const BAR_ICON_PT: f32 = 28.0;

/// Narrowest width the workspace preset switcher shrinks to before secondary controls yield.
pub const MIN_SWITCHER_PT: f32 = 64.0;

/// How the `Cmd/Ctrl+K` command search control renders inside Tier 1 Application Bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchVisibility {
    /// Full search field pill (`Search commands, analyses & sheets (Cmd+K)`).
    Field,
    /// Magnifier icon button alone.
    Icon,
    /// Hidden from the bar (command palette remains reachable via `Cmd/Ctrl+K` and `Help`).
    Hidden,
}

/// Measured widths supplied to [`fit_right_group`], in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RightWidths {
    /// Available trailing bar width after the menu bar, less any fixed status pill width.
    pub room: f32,
    /// Whether a pinned left status badge took precedence over the right group.
    pub has_pinned_badge: bool,
    /// Full desired width of the Workspace Preset Switcher pill.
    pub switcher: f32,
    /// Desired width of the full command-search pill.
    pub field: f32,
    /// Side length of a square icon button (`BAR_ICON_PT`).
    pub icon: f32,
    /// Horizontal gap between adjacent controls (`item_spacing.x`).
    pub gap: f32,
    /// Optional trailing secondary pill width.
    pub trailing_extra: f32,
}

/// Visibility and width allocation computed by [`fit_right_group`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RightGroup {
    pub switcher: bool,
    pub switcher_w: f32,
    pub search: SearchVisibility,
    pub split_toggle: bool,
    pub trailing_extra: bool,
}

/// Deterministic Tier 1 right-group width solver (`gui-layout-spec.md` §3.1, §9.3).
///
/// As available width shrinks, controls degrade cleanly without jitter:
/// optional trailing pill -> split toggle -> search field collapses to
/// icon -> workspace preset switcher narrows to [`MIN_SWITCHER_PT`] -> search icon hides.
#[must_use]
pub fn fit_right_group(w: &RightWidths) -> RightGroup {
    let room = w.room;
    let switcher = !w.has_pinned_badge || room >= MIN_SWITCHER_PT + 8.0;
    let icon = switcher && room >= MIN_SWITCHER_PT + 8.0 + w.gap + w.icon;
    let field = icon && room >= w.switcher + 8.0 + w.gap + w.field;
    let search = if field {
        SearchVisibility::Field
    } else if icon {
        SearchVisibility::Icon
    } else {
        SearchVisibility::Hidden
    };
    let search_w = match search {
        SearchVisibility::Field => w.gap + w.field,
        SearchVisibility::Icon => w.gap + w.icon,
        SearchVisibility::Hidden => 0.0,
    };
    let switcher_w = w.switcher.min(room - 8.0 - search_w).max(MIN_SWITCHER_PT);
    let split_end = switcher_w + 8.0 + search_w + w.gap + w.icon;
    let split_toggle = field && room >= split_end;
    let trailing_extra = split_toggle && room >= split_end + 10.0 + w.gap + w.trailing_extra;
    RightGroup { switcher, switcher_w, search, split_toggle, trailing_extra }
}

#[cfg(test)]
mod tests {
    use super::{BAR_ICON_PT, MIN_SWITCHER_PT, RightGroup, RightWidths, SearchVisibility, fit_right_group};

    fn right_widths(room: f32, has_pinned_badge: bool) -> RightWidths {
        RightWidths { room, has_pinned_badge, switcher: 150.0, field: 170.0, icon: BAR_ICON_PT, gap: 8.0, trailing_extra: 80.0 }
    }

    fn right_stage(g: &RightGroup) -> String {
        let switcher = match g.switcher_w {
            _ if !g.switcher => "no switcher",
            w if w >= 150.0 => "full switcher",
            w if w > MIN_SWITCHER_PT => "narrower switcher",
            _ => "minimal switcher",
        };
        format!("extra {}, split {}, {:?}, {switcher}", g.trailing_extra, g.split_toggle, g.search)
    }

    #[test]
    fn right_group_degrades_in_deterministic_order_without_jitter() {
        let mut stages: Vec<String> = Vec::new();
        for room in (0..=700).rev().map(|r| r as f32) {
            let w = right_widths(room, false);
            let g = fit_right_group(&w);
            let stage = right_stage(&g);
            if stages.last() != Some(&stage) {
                stages.push(stage);
            }
            let search = match g.search {
                SearchVisibility::Field => w.gap + w.field,
                SearchVisibility::Icon => w.gap + w.icon,
                SearchVisibility::Hidden => 0.0,
            };
            let split = if g.split_toggle { w.gap + w.icon } else { 0.0 };
            let extra = if g.trailing_extra { 10.0 + w.gap + w.trailing_extra } else { 0.0 };
            let used = g.switcher_w + 8.0 + search + split + extra;
            assert!(used <= room || g.switcher_w == MIN_SWITCHER_PT && g.search == SearchVisibility::Hidden, "{room}: {g:?} needs {used}");
        }
        assert_eq!(
            stages,
            [
                "extra true, split true, Field, full switcher",
                "extra false, split true, Field, full switcher",
                "extra false, split false, Field, full switcher",
                "extra false, split false, Icon, full switcher",
                "extra false, split false, Icon, narrower switcher",
                "extra false, split false, Icon, minimal switcher",
                "extra false, split false, Hidden, narrower switcher",
                "extra false, split false, Hidden, minimal switcher",
            ]
        );
        let pinned = |room| right_stage(&fit_right_group(&right_widths(room, true)));
        assert_eq!(pinned(700.0), "extra true, split true, Field, full switcher");
        assert_eq!(pinned(72.0), "extra false, split false, Hidden, minimal switcher");
        assert_eq!(pinned(71.0), "extra false, split false, Hidden, no switcher");
    }
}
