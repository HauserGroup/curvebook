//! Unified physical/logical hit-testing and scroll-container resolution over [`super::LayoutTree`].
//!
//! Enforces strict z-order pointer capture: when [`super::ZLayer::ModalBackdrop`] covers `(px, py)`,
//! lower layers (`Workspace`, `FloatingOverlay`, `SplitHandle`, `DropdownMenu`) are completely
//! unreachable so background click-through is impossible (`R10.16`).

use super::a11y::WidgetRole;
use super::scroll::ScrollMeta;
use super::{LayoutTree, WidgetAction, WidgetNode, ZLayer};

/// Hit-test physical framebuffer coordinates `(px, py)` against finalized z-ordered widget bounds.
#[must_use]
pub fn hit_test_physical(tree: &LayoutTree, px: i32, py: i32) -> Option<&WidgetNode> {
    for node in tree.nodes.iter().rev() {
        if !node.drawn_in_frame {
            continue;
        }
        if node.z_layer != ZLayer::ModalBackdrop && node.a11y.role == WidgetRole::Label && node.action == WidgetAction::NoOp && node.scroll.is_none()
        {
            continue;
        }
        if node.bounds.visible_rect_px.contains_px(px, py) {
            // `ModalBackdrop` captures all pointers falling outside `ModalDialog` / `ModalDropdown` / `Tooltip`,
            // guaranteeing zero click-through to workspace chrome behind an open modal.
            return Some(node);
        }
    }
    None
}

/// Convert logical pointer coordinates `(lx, ly)` using `tree.metrics` and hit-test [`LayoutTree`].
#[must_use]
pub fn hit_test_logical(tree: &LayoutTree, lx: f64, ly: f64) -> Option<&WidgetNode> {
    let [phys_x, phys_y] = tree.metrics.logical_to_physical_px([lx, ly]);
    #[allow(clippy::cast_possible_truncation)]
    let px = phys_x.round() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let py = phys_y.round() as i32;
    hit_test_physical(tree, px, py)
}

/// Find the topmost visible [`ScrollMeta`] container under `(px, py)` without crossing `ModalBackdrop`.
#[must_use]
pub fn find_scroll_container_at(tree: &LayoutTree, px: i32, py: i32) -> Option<&ScrollMeta> {
    for node in tree.nodes.iter().rev() {
        if node.z_layer == ZLayer::ModalBackdrop && node.bounds.visible_rect_px.contains_px(px, py) {
            return None;
        }
        if !node.drawn_in_frame || !node.bounds.visible_rect_px.contains_px(px, py) {
            continue;
        }
        if let Some(scroll) = &node.scroll {
            return Some(scroll);
        }
    }
    None
}
