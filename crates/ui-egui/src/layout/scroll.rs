//! Persistent scroll containers, wheel delta clamping, and vertical/horizontal scrollbar geometry (`R10.16`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::LayoutRectPx;

/// Scrollable axes supported by a retained [`ScrollMeta`] container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ScrollAxes {
    #[default]
    None,
    Vertical,
    Horizontal,
    Both,
}

impl ScrollAxes {
    #[must_use]
    pub const fn allows_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    #[must_use]
    pub const fn allows_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
}

/// Scroll viewport metrics & active offset stored on scrollable [`super::WidgetNode`] containers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollMeta {
    pub container_id: super::WidgetId,
    pub viewport_px: LayoutRectPx,
    pub content_size_px: [u32; 2],
    pub scroll_offset_px: [f64; 2],
    pub max_scroll_px: [f64; 2],
    pub axes: ScrollAxes,
}

impl ScrollMeta {
    /// Construct clamped [`ScrollMeta`] from viewport, total content dimensions, and raw stored offset.
    #[must_use]
    pub fn new(
        container_id: impl Into<String>,
        viewport_px: LayoutRectPx,
        content_size_px: [u32; 2],
        raw_offset_px: [f64; 2],
        axes: ScrollAxes,
    ) -> Self {
        let max_x = if axes.allows_horizontal() { f64::from(content_size_px[0].saturating_sub(viewport_px.width)) } else { 0.0 };
        let max_y = if axes.allows_vertical() { f64::from(content_size_px[1].saturating_sub(viewport_px.height)) } else { 0.0 };
        let off_x = if raw_offset_px[0].is_finite() { raw_offset_px[0].clamp(0.0, max_x) } else { 0.0 };
        let off_y = if raw_offset_px[1].is_finite() { raw_offset_px[1].clamp(0.0, max_y) } else { 0.0 };
        Self {
            container_id: super::WidgetId(container_id.into()),
            viewport_px,
            content_size_px,
            scroll_offset_px: [off_x, off_y],
            max_scroll_px: [max_x, max_y],
            axes,
        }
    }

    /// Whether vertical content overflows the viewport.
    #[must_use]
    pub fn has_vertical_overflow(&self) -> bool {
        self.axes.allows_vertical() && self.max_scroll_px[1] > 0.5
    }

    /// Whether horizontal content overflows the viewport.
    #[must_use]
    pub fn has_horizontal_overflow(&self) -> bool {
        self.axes.allows_horizontal() && self.max_scroll_px[0] > 0.5
    }
}

/// Persistent container scroll offsets stored on `InteractionControllerState`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScrollStateStore {
    pub offsets: BTreeMap<String, [f64; 2]>,
}

impl ScrollStateStore {
    /// Read current `[x, y]` scroll offset for `container_id`.
    #[must_use]
    pub fn offset(&self, container_id: &str) -> [f64; 2] {
        self.offsets.get(container_id).copied().unwrap_or([0.0, 0.0])
    }

    /// Set exact scroll offset for `container_id`, clamping against `[max_x, max_y]`.
    pub fn set_clamped(&mut self, container_id: &str, offset: [f64; 2], max_scroll: [f64; 2]) -> [f64; 2] {
        let clamped_x = if offset[0].is_finite() { offset[0].clamp(0.0, max_scroll[0].max(0.0)) } else { 0.0 };
        let clamped_y = if offset[1].is_finite() { offset[1].clamp(0.0, max_scroll[1].max(0.0)) } else { 0.0 };
        let clamped = [clamped_x, clamped_y];
        self.offsets.insert(container_id.to_owned(), clamped);
        clamped
    }

    /// Apply wheel scroll delta (`delta_x`, `delta_y`) onto `meta` and store the clamped offset.
    pub fn apply_wheel(&mut self, meta: &ScrollMeta, delta_x: f64, delta_y: f64) -> [f64; 2] {
        let cur = self.offset(meta.container_id.as_str());
        let next_x = if meta.axes.allows_horizontal() && delta_x.is_finite() {
            (cur[0] + delta_x).clamp(0.0, meta.max_scroll_px[0])
        } else {
            cur[0].clamp(0.0, meta.max_scroll_px[0])
        };
        let next_y = if meta.axes.allows_vertical() && delta_y.is_finite() {
            (cur[1] + delta_y).clamp(0.0, meta.max_scroll_px[1])
        } else {
            cur[1].clamp(0.0, meta.max_scroll_px[1])
        };
        let next = [next_x, next_y];
        self.offsets.insert(meta.container_id.0.clone(), next);
        next
    }

    /// Jump scroll position using normalized `[0.0, 1.0]` track click ratio along vertical or horizontal axis.
    pub fn apply_track_ratio(&mut self, meta: &ScrollMeta, vertical: bool, ratio: f64) -> [f64; 2] {
        let r = if ratio.is_finite() { ratio.clamp(0.0, 1.0) } else { 0.0 };
        let mut next = self.offset(meta.container_id.as_str());
        if vertical && meta.axes.allows_vertical() {
            next[1] = (r * meta.max_scroll_px[1]).clamp(0.0, meta.max_scroll_px[1]);
        } else if !vertical && meta.axes.allows_horizontal() {
            next[0] = (r * meta.max_scroll_px[0]).clamp(0.0, meta.max_scroll_px[0]);
        }
        self.offsets.insert(meta.container_id.0.clone(), next);
        next
    }
}

/// Standard scrollbar track thickness in physical pixels (scaled cleanly with DPI).
#[must_use]
pub fn scrollbar_thickness_px(scale_factor: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let w = (10.0 * scale_factor.clamp(0.75, 4.0)).round() as u32;
    w.clamp(8, 28)
}

/// Compute `(track_rect, thumb_rect)` inside `container_rect` when `content_extent > viewport_extent`.
#[must_use]
pub fn compute_scrollbar_thumb(
    track_rect: LayoutRectPx,
    viewport_extent: u32,
    content_extent: u32,
    scroll_offset: f64,
    vertical: bool,
) -> Option<(LayoutRectPx, LayoutRectPx)> {
    if content_extent <= viewport_extent || track_rect.width == 0 || track_rect.height == 0 {
        return None;
    }
    let max_scroll = f64::from(content_extent.saturating_sub(viewport_extent)).max(1.0);
    let frac_visible = (f64::from(viewport_extent) / f64::from(content_extent)).clamp(0.05, 1.0);
    let scroll_t = (scroll_offset / max_scroll).clamp(0.0, 1.0);

    if vertical {
        let track_len = track_rect.height;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let thumb_len = ((f64::from(track_len) * frac_visible).round() as u32).clamp(14.min(track_len), track_len);
        let travel = track_len.saturating_sub(thumb_len);
        #[allow(clippy::cast_possible_truncation)]
        let thumb_off = (f64::from(travel) * scroll_t).round() as i32;
        let thumb = LayoutRectPx::new(track_rect.x + 1, track_rect.y + thumb_off, track_rect.width.saturating_sub(2).max(4), thumb_len);
        Some((track_rect, thumb))
    } else {
        let track_len = track_rect.width;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let thumb_len = ((f64::from(track_len) * frac_visible).round() as u32).clamp(14.min(track_len), track_len);
        let travel = track_len.saturating_sub(thumb_len);
        #[allow(clippy::cast_possible_truncation)]
        let thumb_off = (f64::from(travel) * scroll_t).round() as i32;
        let thumb = LayoutRectPx::new(track_rect.x + thumb_off, track_rect.y + 1, thumb_len, track_rect.height.saturating_sub(2).max(4));
        Some((track_rect, thumb))
    }
}
