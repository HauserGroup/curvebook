//! `R10.2` Anti-Phantom Parity Gate & `R10.20` Accessibility Completeness Gate (`#248` §4).
//!
//! Asserts that every rendered interactive [`WidgetNode`] matches its rendered pixels, obeys
//! clipping and parent containment, renders non-overlapping/non-clipped text runs, exposes
//! non-empty `WidgetRole` / `label` / `keyboard_path`, and routes raw `(cx, cy)` pointer clicks
//! identically to semantic `WidgetAction`.

use serde::{Deserialize, Serialize};

use super::hit_test::hit_test_physical;
use super::{LayoutRectPx, LayoutTree, WidgetAction, WidgetNode, ZLayer};
use crate::UiApp;
use crate::interaction::KeyModifiers;
use crate::interaction::{PointerButton, UiInputEvent, ViewportMetrics, dispatch_ui_event, execute_widget_action};

/// Single failed invariant detected by [`verify_pixel_to_hit_box_parity`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParityViolation {
    pub widget_id: String,
    pub check: String,
    pub detail: String,
}

/// Result summary emitted by [`verify_pixel_to_hit_box_parity`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParityReport {
    pub checked_nodes: usize,
    pub checked_text_runs: usize,
    pub violations: Vec<ParityViolation>,
}

impl ParityReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Run the 5-part `R10.2` Pixel-to-Hit-Box & `R10.20` A11y gate on the current frame of `app`.
#[must_use]
pub fn verify_pixel_to_hit_box_parity(app: &mut UiApp, metrics: ViewportMetrics) -> ParityReport {
    let tree = app.build_and_cache_layout_tree(metrics);
    verify_tree_invariants(&tree)
}

/// Check invariants directly on a finalized [`LayoutTree`].
#[must_use]
pub fn verify_tree_invariants(tree: &LayoutTree) -> ParityReport {
    let mut violations = Vec::new();
    let mut checked_nodes = 0usize;

    for node in &tree.nodes {
        if !node.a11y.role.is_interactive() || node.z_layer == ZLayer::ModalBackdrop {
            continue;
        }
        checked_nodes += 1;

        if !node.drawn_in_frame {
            violations.push(ParityViolation {
                widget_id: node.id.0.clone(),
                check: String::from("drawn_in_frame"),
                detail: String::from("interactive WidgetNode did not emit any visible PaintOp geometry"),
            });
        }

        check_parent_containment(tree, node, &mut violations);
        check_a11y_fields(node, &mut violations);
        check_center_hit_box(tree, node, &mut violations);
    }

    let checked_text_runs = check_text_runs(tree, &mut violations);
    ParityReport { checked_nodes, checked_text_runs, violations }
}

/// Execute raw pointer click at `(cx, cy)` vs semantic `node.action` on isolated clones and assert equivalence.
///
/// # Errors
/// Returns an error if `widget_id` is absent or either path errors unexpectedly.
pub fn verify_click_action_parity(app: &UiApp, metrics: ViewportMetrics, widget_id: &str) -> Result<bool, String> {
    let mut probe_tree_app = app.clone();
    let tree = probe_tree_app.build_and_cache_layout_tree(metrics);
    let node = tree.node(widget_id).ok_or_else(|| format!("widget `{widget_id}` missing from LayoutTree"))?;

    let [cx, cy] = node.bounds.visible_rect_px.center();
    let [lx, ly] = metrics.physical_to_logical_px([f64::from(cx), f64::from(cy)]);

    let mut raw_app = app.clone();
    raw_app.set_gui_only(true);
    raw_app.set_human_path_only(true);
    dispatch_ui_event(
        &mut raw_app,
        UiInputEvent::PointerDown { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    dispatch_ui_event(
        &mut raw_app,
        UiInputEvent::PointerUp { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;

    let mut semantic_app = app.clone();
    let _ = execute_widget_action(&mut semantic_app, &node.action, metrics)?;

    Ok(raw_app.document() == semantic_app.document()
        && raw_app.canvas_dialogs.active_dialog == semantic_app.canvas_dialogs.active_dialog
        && raw_app.canvas_dialogs.analyze == semantic_app.canvas_dialogs.analyze
        && raw_app.canvas_dialogs.edit_params == semantic_app.canvas_dialogs.edit_params
        && raw_app.document().active_sheet_id == semantic_app.document().active_sheet_id)
}

fn check_parent_containment(tree: &LayoutTree, node: &WidgetNode, violations: &mut Vec<ParityViolation>) {
    let Some(parent_id) = &node.parent else {
        return;
    };
    let Some(parent) = tree.node(parent_id.as_str()) else {
        return;
    };
    let vis = node.bounds.visible_rect_px;
    if vis.width == 0 || vis.height == 0 {
        return;
    }
    let container = if parent.scroll.is_some() { parent.bounds.visible_rect_px } else { parent.bounds.rect_px };
    if vis.x < container.x - 1 || vis.y < container.y - 1 || vis.right() > container.right() + 1 || vis.bottom() > container.bottom() + 1 {
        violations.push(ParityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("parent_containment"),
            detail: format!("visible rect {vis:?} escapes parent `{}` bounds {container:?}", parent_id.as_str()),
        });
    }
}

fn check_a11y_fields(node: &WidgetNode, violations: &mut Vec<ParityViolation>) {
    if node.a11y.label.trim().is_empty() {
        violations.push(ParityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("a11y_label"),
            detail: String::from("interactive WidgetNode must expose a non-empty label"),
        });
    }
    if node.a11y.keyboard_path.as_deref().is_none_or(|p| p.trim().is_empty()) {
        violations.push(ParityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("keyboard_path"),
            detail: String::from("interactive WidgetNode must expose a non-empty keyboard_path (R10.20)"),
        });
    }
}

fn check_center_hit_box(tree: &LayoutTree, node: &WidgetNode, violations: &mut Vec<ParityViolation>) {
    let vis = node.bounds.visible_rect_px;
    if vis.width < 3 || vis.height < 3 {
        return;
    }
    let [cx, cy] = vis.center();
    let Some(hit) = hit_test_physical(tree, cx, cy) else {
        violations.push(ParityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("center_hit_box"),
            detail: format!("hit_test_physical({cx}, {cy}) returned None"),
        });
        return;
    };
    if hit.id == node.id {
        return;
    }
    // Acceptable if a legitimately higher-ZLayer popup/modal or a descendant inside this container sits directly over `(cx, cy)`.
    let occluded = hit.z_layer > node.z_layer || is_descendant_of(tree, hit, &node.id);
    if !occluded {
        violations.push(ParityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("center_hit_box"),
            detail: format!("hit_test_physical({cx}, {cy}) returned `{}` instead of `{}`", hit.id.as_str(), node.id.as_str()),
        });
    }
}

fn is_descendant_of(tree: &LayoutTree, candidate: &WidgetNode, ancestor: &super::WidgetId) -> bool {
    let mut cur = candidate.parent.as_ref();
    let mut hops = 0usize;
    while let Some(pid) = cur
        && hops < 32
    {
        if pid == ancestor {
            return true;
        }
        cur = tree.node(pid.as_str()).and_then(|n| n.parent.as_ref());
        hops += 1;
    }
    false
}

fn check_text_runs(tree: &LayoutTree, violations: &mut Vec<ParityViolation>) -> usize {
    for (idx, run) in tree.text_runs.iter().enumerate() {
        if run.shaped.missing_glyph_count > 0 {
            violations.push(ParityViolation {
                widget_id: run.owner_widget_id.0.clone(),
                check: String::from("text_glyphs"),
                detail: format!("text run `{}` has {} missing glyph(s)", run.text, run.shaped.missing_glyph_count),
            });
        }
        if !run.ellipsized
            && let Some(cid) = run.clip
            && let Some(clip) = tree.clips.get(cid.0)
            && clip.width > 0
            && (f64::from(run.bounds_px.right()) > f64::from(clip.right()) + 2.0)
        {
            violations.push(ParityViolation {
                widget_id: run.owner_widget_id.0.clone(),
                check: String::from("text_unclipped"),
                detail: format!("non-ellipsized text `{}` overflows clip right edge ({} > {})", run.text, run.bounds_px.right(), clip.right()),
            });
        }
        for other in tree.text_runs.iter().skip(idx + 1) {
            if run.z_layer != other.z_layer || run.scroll_container_id != other.scroll_container_id {
                continue;
            }
            if rects_overlap_more_than_1px(run.bounds_px, other.bounds_px) {
                violations.push(ParityViolation {
                    widget_id: run.owner_widget_id.0.clone(),
                    check: String::from("text_overlap"),
                    detail: format!("text `{}` overlaps `{}` on {:?}", run.text, other.text, run.z_layer),
                });
            }
        }
    }
    tree.text_runs.len()
}

fn rects_overlap_more_than_1px(a: LayoutRectPx, b: LayoutRectPx) -> bool {
    let inter = a.intersect(b);
    inter.width > 1 && inter.height > 1
}

/// Check that a raw action on `WidgetNode` is non-NoOp when checking action parity.
#[must_use]
pub const fn action_is_executable(action: &WidgetAction) -> bool {
    !matches!(action, WidgetAction::NoOp)
}
