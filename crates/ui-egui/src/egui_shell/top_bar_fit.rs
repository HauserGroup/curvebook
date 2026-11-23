// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Copyright (c) 2026 CurveBook contributors
// Adapted from upstream MIT OR Apache-2.0 pure-egui helpers; see ATTRIBUTION.md and NOTICE.

//! Deterministic top-bar right-group width fitting solver and rank-ordered toolbar overflow solver
//! (`docs/planning/gui-layout-spec.md` §3.1, `docs/design-language.md` §4).

use crate::menus::ActionRank;

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

/// Measured control candidate supplied to [`fit_ranked_toolbar`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RankedToolbarSlot {
    pub rank: ActionRank,
    pub desired_width_pt: f32,
}

/// Partition of toolbar item indices into visible controls and `"⋯"` overflow items.
#[derive(Clone, Debug, PartialEq)]
pub struct RankedToolbarFit {
    /// Surviving slot indices in original left-to-right order.
    pub visible_indices: Vec<usize>,
    /// Overflowed slot indices in original left-to-right order.
    pub overflow_indices: Vec<usize>,
    /// Total horizontal width consumed by visible controls plus `"⋯"` button (when non-empty).
    pub used_width_pt: f32,
}

/// Deterministic priority-based toolbar overflow solver (`docs/design-language.md` §4).
///
/// Drops [`ActionRank::Overflow`] items into `"⋯"` first (from trailing to leading), then
/// [`ActionRank::Secondary`], keeping [`ActionRank::Primary`] visible until last while preserving
/// original left-to-right slot order and clamping widths at `min_target_w_pt`.
#[must_use]
pub fn fit_ranked_toolbar(
    room_pt: f32,
    gap_pt: f32,
    overflow_btn_w_pt: f32,
    min_target_w_pt: f32,
    max_visible: usize,
    items: &[RankedToolbarSlot],
) -> RankedToolbarFit {
    if items.is_empty() {
        return RankedToolbarFit { visible_indices: Vec::new(), overflow_indices: Vec::new(), used_width_pt: 0.0 };
    }
    let gap = if gap_pt.is_finite() && gap_pt >= 0.0 { gap_pt } else { 0.0 };
    let min_w = if min_target_w_pt.is_finite() && min_target_w_pt > 0.0 { min_target_w_pt } else { 24.0 };
    let ov_w = if overflow_btn_w_pt.is_finite() && overflow_btn_w_pt > 0.0 { overflow_btn_w_pt.max(min_w) } else { min_w };
    let cap = max_visible.max(1);
    let widths: Vec<f32> = items.iter().map(|it| if it.desired_width_pt.is_finite() { it.desired_width_pt.max(min_w) } else { min_w }).collect();

    let mut active: Vec<bool> = vec![true; items.len()];
    let compute_used = |mask: &[bool]| -> (usize, f32) {
        let vis_count = mask.iter().filter(|&&on| on).count();
        let has_ov = vis_count < mask.len();
        let sum_w: f32 = mask.iter().enumerate().filter(|&(_, &on)| on).map(|(i, _)| widths[i]).sum();
        let gaps = if vis_count > 1 { gap * (vis_count - 1) as f32 } else { 0.0 };
        let ov_cost = if has_ov { (if vis_count > 0 { gap } else { 0.0 }) + ov_w } else { 0.0 };
        (vis_count, sum_w + gaps + ov_cost)
    };

    loop {
        let (vis_count, used) = compute_used(&active);
        if vis_count <= 1 || (vis_count <= cap && used <= room_pt) {
            break;
        }
        // Drop lowest retention priority first (`Overflow` -> `Secondary` -> `Primary`), rightmost tie-break.
        let drop_idx = active
            .iter()
            .enumerate()
            .filter(|&(_, &on)| on)
            .min_by_key(|&(idx, _)| (items[idx].rank.retention_priority(), std::cmp::Reverse(idx)))
            .map(|(idx, _)| idx);
        let Some(idx) = drop_idx else {
            break;
        };
        active[idx] = false;
    }

    let (_, used_width_pt) = compute_used(&active);
    let mut visible_indices = Vec::new();
    let mut overflow_indices = Vec::new();
    for (i, &on) in active.iter().enumerate() {
        if on {
            visible_indices.push(i);
        } else {
            overflow_indices.push(i);
        }
    }
    RankedToolbarFit { visible_indices, overflow_indices, used_width_pt }
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

    #[test]
    fn ranked_toolbar_drops_overflow_then_secondary_and_preserves_primary_order() {
        use super::{ActionRank, RankedToolbarSlot, fit_ranked_toolbar};

        let slots = [
            RankedToolbarSlot { rank: ActionRank::Secondary, desired_width_pt: 56.0 }, // 0: + New
            RankedToolbarSlot { rank: ActionRank::Overflow, desired_width_pt: 52.0 },  // 1: Undo
            RankedToolbarSlot { rank: ActionRank::Overflow, desired_width_pt: 52.0 },  // 2: Redo
            RankedToolbarSlot { rank: ActionRank::Primary, desired_width_pt: 74.0 },   // 3: Analyze
            RankedToolbarSlot { rank: ActionRank::Secondary, desired_width_pt: 82.0 }, // 4: New Graph
            RankedToolbarSlot { rank: ActionRank::Overflow, desired_width_pt: 96.0 },  // 5: Convert
        ];

        let wide = fit_ranked_toolbar(600.0, 6.0, 34.0, 28.0, 8, &slots);
        assert_eq!(wide.visible_indices, vec![0, 1, 2, 3, 4, 5]);
        assert!(wide.overflow_indices.is_empty());

        // Medium width: Overflow items (5, 2, 1) drop before Secondary (0, 4) or Primary (3)!
        let medium = fit_ranked_toolbar(265.0, 6.0, 34.0, 28.0, 8, &slots);
        assert_eq!(medium.visible_indices, vec![0, 3, 4], "expected [+ New, Analyze, New Graph] preserved in order");
        assert_eq!(medium.overflow_indices, vec![1, 2, 5]);
        assert!(medium.used_width_pt <= 265.0);

        // Tight width: only Primary (3: Analyze) survives alongside `⋯`!
        let tight = fit_ranked_toolbar(120.0, 6.0, 34.0, 28.0, 8, &slots);
        assert_eq!(tight.visible_indices, vec![3], "Primary action must survive last");
        assert_eq!(tight.overflow_indices, vec![0, 1, 2, 4, 5]);
    }
}
