//! Native `egui` human-path driver (`EguiHumanPathDriver`), IME preedit/commit checks,
//! AccessKit/`Role::ScrollBar` parity verification, and 14-locale ellipsis verification (`R15.1`).
#![cfg(any(test, feature = "test-support"))]

use std::ops::Range;
use std::time::Duration;

use egui::accesskit::{Node as AccessKitNode, NodeId as AccessKitNodeId, Role as AccessKitRole, TreeUpdate as AccessKitTreeUpdate};
use egui::epaint::{ClippedShape, Shape};
use egui::output::IMEOutput;
use egui::{
    Context, Event, FontDefinitions, FullOutput, ImeEvent, Key, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton as EguiPointerButton,
    Pos2, RawInput, Rect, Response, TouchPhase, Ui, Vec2, ViewportId, WidgetInfo,
};

pub use super::inspect_bridge::{parse_egui_key, to_egui_modifiers, to_egui_pointer_button};
use super::{ActionRank, FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, GeometryStamp, LayoutMetrics, Tokens};
use crate::inspect::{ParityReport, ParityViolation, ZLayer};
use crate::interaction::{KeyModifiers, PointerButton, ViewportMetrics};

/// All 14 Beta-active non-RTL BCP-47 locale identifiers (`docs/planning/gui-layout-spec.md` §1.1, §9.4).
pub const BETA_LOCALE_CODES: [&str; 14] = ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr"];

const MAX_SETTLE_FRAMES: usize = 12;
const BLEED_TOLERANCE_PT: f32 = 1.5;
const SCROLLBAR_OVERLAP_TOLERANCE_PT: f32 = 0.5;

/// Vertical clipping policy applied when checking rendered text runs for unclipped bleed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBleedPolicy {
    /// Require all four edges of non-ellipsized text to stay within both `clip_rect` and the viewport.
    #[default]
    StrictAllEdges,
    /// Require horizontal containment within `clip_rect` while permitting vertical clipping inside scroll containers.
    AllowVerticalScrollClip,
}

/// Rendered text run extracted from a settled `egui` frame's [`ClippedShape`] output.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedTextRun {
    pub full_text: String,
    pub ellipsized: bool,
    pub text_rect: Rect,
    pub clip_rect: Rect,
}

/// Observed ellipsis and accessibility metadata for a rendered label or widget.
#[derive(Debug, Clone, PartialEq)]
pub struct EllipsisRecord {
    pub widget_id: Option<String>,
    pub accesskit_id: Option<AccessKitNodeId>,
    pub full_text: String,
    pub ellipsized: bool,
    pub text_rect: Rect,
    pub clip_rect: Rect,
    pub accesskit_name: Option<String>,
    pub accesskit_description: Option<String>,
}

/// Input configuration for [`render_tracked_label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EllipsisLabelInput<'a> {
    pub widget_id: &'a str,
    pub text: &'a str,
    pub truncate: bool,
    pub z_layer: ZLayer,
}

/// Rendered outcome of [`render_tracked_label`], including live `galley.elided` status.
#[derive(Debug, Clone)]
pub struct EllipsisLabelOutput {
    pub response: Response,
    pub ellipsized: bool,
    pub full_label: String,
}

fn non_empty_str(opt: Option<&str>) -> Option<&str> {
    opt.filter(|s| !s.trim().is_empty())
}

/// Extract the primary accessible name exposed by an AccessKit node (handling `Role::Label` `value()` vs `label()`).
#[must_use]
pub fn accesskit_accessible_name(node: &AccessKitNode) -> Option<&str> {
    if matches!(node.role(), AccessKitRole::Label) {
        non_empty_str(node.value()).or_else(|| non_empty_str(node.label()))
    } else {
        non_empty_str(node.label()).or_else(|| non_empty_str(node.placeholder())).or_else(|| non_empty_str(node.value()))
    }
}

/// Build [`egui::FontDefinitions`] from host OS fonts, `CURVEBOOK_FONTS_DIR`, and CurveBook's
/// deterministic SFNT sans + symbol fallback via [`super::fonts::build_egui_font_definitions`].
#[must_use]
pub fn build_driver_font_definitions() -> FontDefinitions {
    super::fonts::build_egui_font_definitions().definitions
}

/// Render an `egui::Label` with optional truncation (`…`), hover tooltip for full un-truncated
/// text, AccessKit label exposure, and [`FrameInspectMeta`] registration including `ellipsized`.
pub fn render_tracked_label(ui: &mut Ui, meta: &mut FrameInspectMeta, input: EllipsisLabelInput<'_>) -> EllipsisLabelOutput {
    let mut label = egui::Label::new(input.text).selectable(false).show_tooltip_when_elided(true);
    if input.truncate {
        label = label.truncate();
    }
    let (galley_pos, galley, mut response) = label.layout_in_ui(ui);
    let ellipsized = galley.elided;
    let full_label = galley.text().to_owned();
    let is_enabled = ui.is_enabled();
    let accessible = full_label.clone();
    response.widget_info(move || WidgetInfo::labeled(egui::Role::Label, is_enabled, accessible.clone()));

    if ui.is_rect_visible(response.rect) {
        if ellipsized {
            response = response.on_hover_text(full_label.as_str());
        }
        let color = ui.style().visuals.text_color();
        ui.painter().add(egui::epaint::TextShape::new(galley_pos, galley, color));
    }

    meta.record_response(
        ui,
        &response,
        FrameInspectEntryInput {
            widget_id: input.widget_id,
            role: "Label",
            label: &full_label,
            command_id: None,
            value: Some(serde_json::json!(full_label)),
            enabled: is_enabled,
            disabled_reason: None,
            z_layer: input.z_layer,
            ellipsized,
            full_label: ellipsized.then_some(full_label.as_str()),
        },
    );

    EllipsisLabelOutput { response, ellipsized, full_label }
}

/// Extract all painted [`RenderedTextRun`] entries recursively from settled [`ClippedShape`]s.
#[must_use]
pub fn extract_rendered_text_runs(shapes: &[ClippedShape]) -> Vec<RenderedTextRun> {
    let mut out = Vec::new();
    for clipped in shapes {
        collect_text_runs_from_shape(clipped.clip_rect, &clipped.shape, &mut out);
    }
    out
}

fn collect_text_runs_from_shape(clip_rect: Rect, shape: &Shape, out: &mut Vec<RenderedTextRun>) {
    match shape {
        Shape::Text(text_shape) => {
            let text_rect = text_shape.galley.rect.translate(text_shape.pos.to_vec2());
            out.push(RenderedTextRun { full_text: text_shape.galley.text().to_owned(), ellipsized: text_shape.galley.elided, text_rect, clip_rect });
        }
        Shape::Vec(children) => {
            for child in children {
                collect_text_runs_from_shape(clip_rect, child, out);
            }
        }
        Shape::Noop
        | Shape::Circle(_)
        | Shape::Ellipse(_)
        | Shape::LineSegment { .. }
        | Shape::Path(_)
        | Shape::Band(_)
        | Shape::Rect(_)
        | Shape::Mesh(_)
        | Shape::QuadraticBezier(_)
        | Shape::CubicBezier(_)
        | Shape::Callback(_) => {}
    }
}

/// Collect visible tooltip label strings emitted under `Role::Tooltip` in an AccessKit [`AccessKitTreeUpdate`].
#[must_use]
pub fn collect_tooltip_texts_from_accesskit(nodes: &[(AccessKitNodeId, AccessKitNode)]) -> Vec<String> {
    let mut texts = Vec::new();
    for (_, node) in nodes {
        if node.role() != AccessKitRole::Tooltip || node.is_hidden() {
            continue;
        }
        collect_subtree_texts(node, nodes, &mut texts);
    }
    texts
}

fn collect_subtree_texts(node: &AccessKitNode, all_nodes: &[(AccessKitNodeId, AccessKitNode)], out: &mut Vec<String>) {
    if let Some(name) = accesskit_accessible_name(node).map(str::trim).filter(|s| !s.is_empty()) {
        out.push(name.to_owned());
    }
    for child_id in node.children() {
        if let Some((_, child)) = all_nodes.iter().find(|(id, _)| *id == *child_id)
            && !child.is_hidden()
        {
            collect_subtree_texts(child, all_nodes, out);
        }
    }
}

/// Build [`EllipsisRecord`] items cross-referencing painted text runs, [`FrameInspectMeta`], and AccessKit nodes.
#[must_use]
pub fn collect_ellipsis_records(shapes: &[ClippedShape], meta: &FrameInspectMeta, nodes: &[(AccessKitNodeId, AccessKitNode)]) -> Vec<EllipsisRecord> {
    let runs = extract_rendered_text_runs(shapes);
    let mut records = Vec::new();
    for run in runs {
        let matched_meta = meta.entries.iter().find(|entry| {
            entry.clipped_rect.intersects(run.text_rect)
                && (entry.label == run.full_text || entry.full_label.as_deref() == Some(run.full_text.as_str()))
        });
        let matched_ak = matched_meta
            .and_then(|m| nodes.iter().find(|(id, _)| *id == m.accesskit_id))
            .or_else(|| nodes.iter().find(|(_, node)| accesskit_node_matches_run(node, &run)));

        let meta_ellipsized = matched_meta.is_some_and(|m| m.ellipsized);
        records.push(EllipsisRecord {
            widget_id: matched_meta.map(|m| m.widget_id.clone()),
            accesskit_id: matched_ak.map(|(id, _)| *id),
            full_text: run.full_text,
            ellipsized: run.ellipsized || meta_ellipsized,
            text_rect: run.text_rect,
            clip_rect: run.clip_rect,
            accesskit_name: matched_ak.and_then(|(_, n)| accesskit_accessible_name(n).map(str::to_owned)),
            accesskit_description: matched_ak.and_then(|(_, n)| n.description().map(str::to_owned)),
        });
    }
    for entry in &meta.entries {
        if entry.ellipsized && !records.iter().any(|r| r.widget_id.as_deref() == Some(entry.widget_id.as_str())) {
            let matched_ak = nodes.iter().find(|(id, _)| *id == entry.accesskit_id);
            records.push(EllipsisRecord {
                widget_id: Some(entry.widget_id.clone()),
                accesskit_id: Some(entry.accesskit_id),
                full_text: entry.full_label.clone().unwrap_or_else(|| entry.label.clone()),
                ellipsized: true,
                text_rect: entry.clipped_rect,
                clip_rect: entry.clipped_rect,
                accesskit_name: matched_ak.and_then(|(_, n)| accesskit_accessible_name(n).map(str::to_owned)),
                accesskit_description: matched_ak.and_then(|(_, n)| n.description().map(str::to_owned)),
            });
        }
    }
    records
}

fn accesskit_node_matches_run(node: &AccessKitNode, run: &RenderedTextRun) -> bool {
    if node.is_hidden() {
        return false;
    }
    let Some(bounds) = node.bounds() else {
        return false;
    };
    let ak_rect = accesskit_rect_to_egui(bounds);
    ak_rect.intersects(run.text_rect) && accesskit_accessible_name(node) == Some(run.full_text.as_str())
}

const fn accesskit_rect_to_egui(rect: egui::accesskit::Rect) -> Rect {
    #[allow(clippy::cast_possible_truncation)]
    Rect::from_min_max(Pos2::new(rect.x0 as f32, rect.y0 as f32), Pos2::new(rect.x1 as f32, rect.y1 as f32))
}

const fn is_interactive_accesskit_role(role: AccessKitRole) -> bool {
    matches!(
        role,
        AccessKitRole::Button
            | AccessKitRole::DefaultButton
            | AccessKitRole::CheckBox
            | AccessKitRole::RadioButton
            | AccessKitRole::TextInput
            | AccessKitRole::MultilineTextInput
            | AccessKitRole::SearchInput
            | AccessKitRole::ComboBox
            | AccessKitRole::Slider
            | AccessKitRole::SpinButton
            | AccessKitRole::Switch
            | AccessKitRole::Tab
            | AccessKitRole::MenuItem
            | AccessKitRole::MenuListOption
            | AccessKitRole::Link
            | AccessKitRole::TreeItem
            | AccessKitRole::ListBoxOption
            | AccessKitRole::DisclosureTriangle
    )
}

const fn is_scrollbar_overlap_candidate_role(role: AccessKitRole) -> bool {
    is_interactive_accesskit_role(role)
        || matches!(
            role,
            AccessKitRole::Label | AccessKitRole::Cell | AccessKitRole::RowHeader | AccessKitRole::ColumnHeader | AccessKitRole::ListItem
        )
}

/// Visible AccessKit node geometry used to verify [`AccessKitRole::ScrollBar`] non-overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessKitRoleBounds {
    pub id: AccessKitNodeId,
    pub role: AccessKitRole,
    pub label: String,
    pub rect: Rect,
}

/// Assert zero [`AccessKitRole::ScrollBar`] intersections with interactive controls, rows, or labels,
/// isolating foreground modal (`ZLayer::ModalDialog`+) and dropdown/context popup (`ZLayer::DropdownMenu`+)
/// layers from occluded workspace layers behind them.
#[must_use]
pub fn check_accesskit_scrollbar_non_overlap_with_meta(nodes: &[(AccessKitNodeId, AccessKitNode)], meta: &FrameInspectMeta) -> Vec<ParityViolation> {
    let has_modal = meta.entries.iter().any(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown));
    let has_popup = meta.entries.iter().any(|e| matches!(e.z_layer, ZLayer::DropdownMenu | ZLayer::ContextMenu));
    if !has_modal && !has_popup {
        return check_accesskit_scrollbar_non_overlap(nodes);
    }

    let dialog_rects: Vec<Rect> = meta
        .entries
        .iter()
        .filter(|e| matches!(e.z_layer, ZLayer::ModalDialog | ZLayer::ModalDropdown) && e.clipped_rect.is_positive())
        .map(|e| e.clipped_rect)
        .collect();
    let popup_rects: Vec<Rect> = meta
        .entries
        .iter()
        .filter(|e| matches!(e.z_layer, ZLayer::DropdownMenu | ZLayer::ContextMenu) && e.role == "menu" && e.clipped_rect.is_positive())
        .map(|e| e.clipped_rect)
        .collect();

    let node_map: std::collections::HashMap<AccessKitNodeId, &AccessKitNode> = nodes.iter().map(|(id, n)| (*id, n)).collect();
    let collect_subtree = |seed_ids: Vec<AccessKitNodeId>| -> std::collections::HashSet<AccessKitNodeId> {
        let mut set: std::collections::HashSet<AccessKitNodeId> = seed_ids.iter().copied().collect();
        let mut queue = seed_ids;
        while let Some(cur) = queue.pop() {
            if let Some(node) = node_map.get(&cur) {
                for &child in node.children() {
                    if set.insert(child) {
                        queue.push(child);
                    }
                }
            }
        }
        set
    };

    let mut modal_seeds: Vec<AccessKitNodeId> = meta
        .entries
        .iter()
        .filter(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown))
        .map(|e| e.accesskit_id)
        .collect();
    for (id, node) in nodes {
        if node.role() == AccessKitRole::Dialog {
            modal_seeds.push(*id);
        }
    }
    let modal_subtree_ids = collect_subtree(modal_seeds);

    let popup_seeds: Vec<AccessKitNodeId> =
        meta.entries.iter().filter(|e| matches!(e.z_layer, ZLayer::DropdownMenu | ZLayer::ContextMenu)).map(|e| e.accesskit_id).collect();
    let popup_subtree_ids = collect_subtree(popup_seeds);

    let is_popup_edge_scrollbar = |popup: Rect, sb: Rect| -> bool {
        if sb.height() >= sb.width() {
            (popup.right() - sb.right()).abs() <= 20.0 || (sb.left() - popup.left()).abs() <= 20.0
        } else {
            (popup.bottom() - sb.bottom()).abs() <= 20.0
        }
    };

    let mut modal_items = Vec::new();
    let mut popup_items = Vec::new();
    let mut background_items = Vec::new();
    for (id, node) in nodes {
        if node.is_hidden() {
            continue;
        }
        let Some(bounds) = node.bounds() else {
            continue;
        };
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            continue;
        }
        let rect = accesskit_rect_to_egui(bounds);
        let item = AccessKitRoleBounds { id: *id, role: node.role(), label: accesskit_accessible_name(node).unwrap_or("<unnamed>").to_owned(), rect };
        let meta_entry = meta.find_by_accesskit_id(*id);

        let in_popup = popup_subtree_ids.contains(id)
            || meta_entry.is_some_and(|e| matches!(e.z_layer, ZLayer::DropdownMenu | ZLayer::ContextMenu))
            || (meta_entry.is_none()
                && item.role != AccessKitRole::Label
                && !node.is_disabled()
                && popup_rects
                    .iter()
                    .any(|&r| r.expand(4.0).contains_rect(rect) && (item.role != AccessKitRole::ScrollBar || is_popup_edge_scrollbar(r, rect))));

        let in_modal = !in_popup
            && (modal_subtree_ids.contains(id)
                || meta_entry.is_some_and(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown))
                || (meta_entry.is_none()
                    && item.role != AccessKitRole::Label
                    && !node.is_disabled()
                    && dialog_rects
                        .iter()
                        .any(|&r| r.expand(4.0).contains_rect(rect) && (item.role != AccessKitRole::ScrollBar || is_popup_edge_scrollbar(r, rect)))));

        if in_popup {
            popup_items.push(item);
        } else if in_modal {
            modal_items.push(item);
        } else if !dialog_rects.iter().any(|r| r.contains(item.rect.center())) && !popup_rects.iter().any(|r| r.contains(item.rect.center())) {
            background_items.push(item);
        }
    }

    let mut violations = check_role_bounds_scrollbar_non_overlap(&modal_items);
    violations.extend(check_role_bounds_scrollbar_non_overlap(&popup_items));
    violations.extend(check_role_bounds_scrollbar_non_overlap(&background_items));
    violations
}

/// Assert zero [`AccessKitRole::ScrollBar`] intersections with interactive controls, rows, or labels.
#[must_use]
pub fn check_accesskit_scrollbar_non_overlap(nodes: &[(AccessKitNodeId, AccessKitNode)]) -> Vec<ParityViolation> {
    let items: Vec<AccessKitRoleBounds> = nodes
        .iter()
        .filter(|(_, node)| !node.is_hidden())
        .filter_map(|(id, node)| {
            let bounds = node.bounds()?;
            if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                return None;
            }
            Some(AccessKitRoleBounds {
                id: *id,
                role: node.role(),
                label: accesskit_accessible_name(node).unwrap_or("<unnamed>").to_owned(),
                rect: accesskit_rect_to_egui(bounds),
            })
        })
        .collect();
    check_role_bounds_scrollbar_non_overlap(&items)
}

/// Shared [`AccessKitRole::ScrollBar`] non-overlap check operating on extracted logical-point bounds.
#[must_use]
pub fn check_role_bounds_scrollbar_non_overlap(nodes: &[AccessKitRoleBounds]) -> Vec<ParityViolation> {
    let scrollbars: Vec<&AccessKitRoleBounds> =
        nodes.iter().filter(|item| item.role == AccessKitRole::ScrollBar && item.rect.width() > 0.0 && item.rect.height() > 0.0).collect();

    if scrollbars.is_empty() {
        return Vec::new();
    }

    let mut violations = Vec::new();
    for sb in &scrollbars {
        for ctrl in nodes {
            if ctrl.id == sb.id || !is_scrollbar_overlap_candidate_role(ctrl.role) || ctrl.rect.width() <= 0.0 || ctrl.rect.height() <= 0.0 {
                continue;
            }
            if rects_intersect_beyond(sb.rect, ctrl.rect, SCROLLBAR_OVERLAP_TOLERANCE_PT) {
                violations.push(ParityViolation {
                    widget_id: format!("accesskit:{:?}", ctrl.id),
                    check: String::from("scrollbar_overlap"),
                    detail: format!("Role::ScrollBar ({:?}) overlaps {:?} `{}` ({:?})", sb.rect, ctrl.role, ctrl.label, ctrl.rect),
                });
            }
        }
    }
    violations
}

fn rects_intersect_beyond(a: Rect, b: Rect, tol: f32) -> bool {
    let inter = a.intersect(b);
    inter.width() > tol && inter.height() > tol
}

/// Check rendered text runs for unclipped bleed outside their clip rects and viewport bounds.
#[must_use]
pub fn check_rendered_text_bleed(runs: &[RenderedTextRun], viewport_rect: Rect, policy: TextBleedPolicy) -> Vec<ParityViolation> {
    let mut violations = Vec::new();
    for run in runs {
        if run.ellipsized || run.text_rect.width() <= 0.0 || run.text_rect.height() <= 0.0 {
            continue;
        }
        let h_bleed = run.text_rect.left() < run.clip_rect.left() - BLEED_TOLERANCE_PT
            || run.text_rect.right() > run.clip_rect.right() + BLEED_TOLERANCE_PT
            || run.text_rect.left() < viewport_rect.left() - BLEED_TOLERANCE_PT
            || run.text_rect.right() > viewport_rect.right() + BLEED_TOLERANCE_PT;
        let v_bleed = match policy {
            TextBleedPolicy::StrictAllEdges => {
                run.text_rect.top() < run.clip_rect.top() - BLEED_TOLERANCE_PT
                    || run.text_rect.bottom() > run.clip_rect.bottom() + BLEED_TOLERANCE_PT
                    || run.text_rect.top() < viewport_rect.top() - BLEED_TOLERANCE_PT
                    || run.text_rect.bottom() > viewport_rect.bottom() + BLEED_TOLERANCE_PT
            }
            TextBleedPolicy::AllowVerticalScrollClip => false,
        };
        if h_bleed || v_bleed {
            violations.push(ParityViolation {
                widget_id: String::from("text_run"),
                check: String::from("text_unclipped_bleed"),
                detail: format!(
                    "non-ellipsized text `{}` rect {:?} overflows clip {:?} / viewport {:?}",
                    run.full_text, run.text_rect, run.clip_rect, viewport_rect
                ),
            });
        }
    }
    violations
}

/// Verify settled `egui` frame parity across [`FrameInspectMeta`], AccessKit [`AccessKitTreeUpdate`],
/// [`Role::ScrollBar`](AccessKitRole::ScrollBar) non-overlap, and rendered text clip bounds.
#[must_use]
pub fn verify_egui_frame_parity(
    accesskit_update: Option<&AccessKitTreeUpdate>,
    meta: &FrameInspectMeta,
    shapes: &[ClippedShape],
    viewport_rect: Rect,
    policy: TextBleedPolicy,
) -> ParityReport {
    let mut violations = Vec::new();
    let nodes: &[(AccessKitNodeId, AccessKitNode)] = accesskit_update.map_or(&[], |u| u.nodes.as_slice());

    if accesskit_update.is_none() {
        violations.push(ParityViolation {
            widget_id: String::from("accesskit_root"),
            check: String::from("accesskit_enabled"),
            detail: String::from("settled frame emitted no AccessKit TreeUpdate"),
        });
    }

    let checked_nodes = check_frame_inspect_meta(meta, nodes, viewport_rect, &mut violations);
    check_accesskit_interactive_nodes(nodes, viewport_rect, &mut violations);
    violations.extend(check_accesskit_scrollbar_non_overlap_with_meta(nodes, meta));
    violations.extend(check_clickable_partial_non_overlap(meta));
    violations.extend(check_wcag_258_target_size(meta));
    violations.extend(check_single_primary_per_region(meta));

    let text_runs = extract_rendered_text_runs(shapes);
    let checked_text_runs = text_runs.len();
    violations.extend(check_rendered_text_bleed(&text_runs, viewport_rect, policy));

    ParityReport { checked_nodes, checked_text_runs, violations }
}

const INTERACTIVE_OVERLAP_TOL_PT: f32 = 1.0;
const WCAG_258_TARGET_MIN_PT: f32 = 24.0;
const WCAG_258_FLOAT_TOL_PT: f32 = 0.5;

/// Logical workspace region used for per-region control budgets and primary-action hierarchy checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShellRegion {
    TopChrome,
    Navigator,
    CenterStage,
    Inspector,
    StatusFooter,
    DropdownMenu,
    ModalDialog,
}

/// Per-region visible clickable control ceilings (excluding virtualized spreadsheet `gridcell`s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionBudgets {
    pub top_chrome: usize,
    pub navigator: usize,
    pub center_stage: usize,
    pub inspector: usize,
    pub status_footer: usize,
    pub dropdown_menu: usize,
    pub modal_dialog: usize,
}

impl RegionBudgets {
    /// Canonical per-region control-count ceilings anchored to measured workspace and dialog views
    /// (`docs/planning/ui-overlap-and-target-size-audit.md` F5).
    pub const CANONICAL: Self =
        Self { top_chrome: 24, navigator: 36, center_stage: 24, inspector: 36, status_footer: 14, dropdown_menu: 16, modal_dialog: 44 };

    /// Ceiling configured for `region`.
    #[must_use]
    pub const fn limit_for(self, region: ShellRegion) -> usize {
        match region {
            ShellRegion::TopChrome => self.top_chrome,
            ShellRegion::Navigator => self.navigator,
            ShellRegion::CenterStage => self.center_stage,
            ShellRegion::Inspector => self.inspector,
            ShellRegion::StatusFooter => self.status_footer,
            ShellRegion::DropdownMenu => self.dropdown_menu,
            ShellRegion::ModalDialog => self.modal_dialog,
        }
    }
}

impl Default for RegionBudgets {
    fn default() -> Self {
        Self::CANONICAL
    }
}

/// Measured enabled clickable control counts per [`ShellRegion`] on a settled frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegionControlCounts {
    pub top_chrome: usize,
    pub navigator: usize,
    pub center_stage: usize,
    pub inspector: usize,
    pub status_footer: usize,
    pub dropdown_menu: usize,
    pub modal_dialog: usize,
    pub grid_cells: usize,
}

impl RegionControlCounts {
    /// Count recorded for `region`.
    #[must_use]
    pub const fn count_for(self, region: ShellRegion) -> usize {
        match region {
            ShellRegion::TopChrome => self.top_chrome,
            ShellRegion::Navigator => self.navigator,
            ShellRegion::CenterStage => self.center_stage,
            ShellRegion::Inspector => self.inspector,
            ShellRegion::StatusFooter => self.status_footer,
            ShellRegion::DropdownMenu => self.dropdown_menu,
            ShellRegion::ModalDialog => self.modal_dialog,
        }
    }

    /// Total enabled clickable chrome/widget controls across all regions (excluding `grid_cells`).
    #[must_use]
    pub const fn total_chrome_controls(self) -> usize {
        self.top_chrome + self.navigator + self.center_stage + self.inspector + self.status_footer + self.dropdown_menu + self.modal_dialog
    }
}

/// Classify a recorded [`FrameInspectNodeMeta`] entry into its owning [`ShellRegion`].
#[must_use]
pub fn classify_entry_region(entry: &FrameInspectNodeMeta) -> ShellRegion {
    match entry.z_layer {
        ZLayer::ModalDialog | ZLayer::ModalDropdown | ZLayer::ModalBackdrop => return ShellRegion::ModalDialog,
        ZLayer::DropdownMenu | ZLayer::ContextMenu => return ShellRegion::DropdownMenu,
        ZLayer::Workspace | ZLayer::FloatingOverlay | ZLayer::SplitHandle | ZLayer::Tooltip => {}
    }
    let id = entry.widget_id.as_str();
    if id.starts_with("toolbar.") || id.starts_with("menu.") {
        ShellRegion::TopChrome
    } else if id.starts_with("navigator.") || id.starts_with("nav.") || id.starts_with("family.") {
        ShellRegion::Navigator
    } else if id.starts_with("status.") {
        ShellRegion::StatusFooter
    } else if id.starts_with("inspector.") || id.starts_with("studio.") {
        ShellRegion::Inspector
    } else {
        ShellRegion::CenterStage
    }
}

fn is_clickable_meta_role(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "button"
            | "textbox"
            | "textinput"
            | "searchbox"
            | "radio"
            | "radiobutton"
            | "tab"
            | "menuitem"
            | "checkbox"
            | "treeitem"
            | "option"
            | "spinbutton"
            | "combobox"
            | "slider"
            | "link"
            | "gridcell"
            | "splitter"
    )
}

fn is_target_size_exempt_entry(entry: &FrameInspectNodeMeta) -> bool {
    if matches!(entry.role.to_ascii_lowercase().as_str(), "gridcell" | "splitter") {
        return true;
    }
    is_canvas_manipulation_handle(entry.widget_id.as_str())
}

fn is_canvas_manipulation_handle(id: &str) -> bool {
    id.starts_with("canvas.image.handle.") || (id.starts_with("canvas.layout_panel.") && id.contains(".resize_"))
}

fn is_canvas_scene_overlay(entry: &FrameInspectNodeMeta) -> bool {
    entry.z_layer == ZLayer::FloatingOverlay && entry.widget_id.starts_with("canvas.")
}

fn rect_contains_with_tol(outer: Rect, inner: Rect, tol: f32) -> bool {
    inner.left() >= outer.left() - tol
        && inner.right() <= outer.right() + tol
        && inner.top() >= outer.top() - tol
        && inner.bottom() <= outer.bottom() + tol
}

fn is_scroll_edge_clipped_slice(entry: &FrameInspectNodeMeta, min_pt: f32) -> bool {
    let thresh = min_pt - WCAG_258_FLOAT_TOL_PT;
    let unclipped_ok = entry.unclipped_rect.width() >= thresh && entry.unclipped_rect.height() >= thresh;
    let clipped_smaller = entry.clipped_rect.width() + WCAG_258_FLOAT_TOL_PT < entry.unclipped_rect.width()
        || entry.clipped_rect.height() + WCAG_258_FLOAT_TOL_PT < entry.unclipped_rect.height();
    unclipped_ok && clipped_smaller
}

fn circle_intersects_rect(center: Pos2, radius: f32, rect: Rect) -> bool {
    let closest_x = center.x.clamp(rect.left(), rect.right());
    let closest_y = center.y.clamp(rect.top(), rect.bottom());
    let dx = center.x - closest_x;
    let dy = center.y - closest_y;
    dx * dx + dy * dy < radius * radius
}

/// Assert that no two enabled clickable controls on the same [`ZLayer`] partially overlap
/// (`> 1.0 x 1.0 pt`), while allowing nested parent/child containment (`F4.1`).
#[must_use]
pub fn check_clickable_partial_non_overlap(meta: &FrameInspectMeta) -> Vec<ParityViolation> {
    let modal_active = meta.entries.iter().any(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown));
    let active: Vec<&FrameInspectNodeMeta> = meta
        .entries
        .iter()
        .filter(|e| {
            e.enabled
                && e.clipped_rect.is_positive()
                && is_clickable_meta_role(&e.role)
                && (!modal_active || matches!(e.z_layer, ZLayer::ModalDialog | ZLayer::ModalDropdown | ZLayer::ContextMenu))
        })
        .collect();

    let mut violations = Vec::new();
    for i in 0..active.len() {
        let a = active[i];
        for &b in &active[i + 1..] {
            if a.z_layer != b.z_layer || a.egui_id == b.egui_id {
                continue;
            }
            if is_canvas_scene_overlay(a) && is_canvas_scene_overlay(b) {
                continue;
            }
            let ra = a.clipped_rect;
            let rb = b.clipped_rect;
            let inter = ra.intersect(rb);
            if inter.width() <= INTERACTIVE_OVERLAP_TOL_PT || inter.height() <= INTERACTIVE_OVERLAP_TOL_PT {
                continue;
            }
            if rect_contains_with_tol(ra, rb, INTERACTIVE_OVERLAP_TOL_PT) || rect_contains_with_tol(rb, ra, INTERACTIVE_OVERLAP_TOL_PT) {
                continue;
            }
            violations.push(ParityViolation {
                widget_id: a.widget_id.clone(),
                check: String::from("interactive_partial_overlap"),
                detail: format!(
                    "enabled control `{}` ({:?}) partially overlaps `{}` ({:?}) on {:?} by {:.1}x{:.1} pt",
                    a.widget_id,
                    ra,
                    b.widget_id,
                    rb,
                    a.z_layer,
                    inter.width(),
                    inter.height()
                ),
            });
        }
    }
    violations
}

/// Assert WCAG 2.2 SC 2.5.8 Target Size (Minimum) across enabled clickable controls (`F4.2`):
/// every non-exempt control is either `>= 24 x 24 pt` or maintains a `24 pt`-diameter (`12 pt`
/// radius) spacing circle free of neighbouring enabled controls and undersized-target circles.
#[must_use]
pub fn check_wcag_258_target_size(meta: &FrameInspectMeta) -> Vec<ParityViolation> {
    let min_pt = LayoutMetrics::CANONICAL.min_hit_target_pt.max(WCAG_258_TARGET_MIN_PT);
    let thresh = min_pt - WCAG_258_FLOAT_TOL_PT;
    let radius = min_pt * 0.5 - WCAG_258_FLOAT_TOL_PT * 0.5;
    let min_center_dist = min_pt - WCAG_258_FLOAT_TOL_PT;

    let modal_active = meta.entries.iter().any(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown));
    let peers: Vec<&FrameInspectNodeMeta> = meta
        .entries
        .iter()
        .filter(|e| {
            e.enabled
                && e.clipped_rect.is_positive()
                && is_clickable_meta_role(&e.role)
                && (!modal_active || matches!(e.z_layer, ZLayer::ModalDialog | ZLayer::ModalDropdown | ZLayer::ContextMenu))
        })
        .collect();

    let mut violations = Vec::new();
    for (i, &a) in peers.iter().enumerate() {
        if is_target_size_exempt_entry(a) || is_scroll_edge_clipped_slice(a, min_pt) {
            continue;
        }
        let ra = a.clipped_rect;
        if ra.width() >= thresh && ra.height() >= thresh {
            continue;
        }
        let ca = ra.center();
        for (j, &b) in peers.iter().enumerate() {
            if i == j || a.z_layer != b.z_layer || a.egui_id == b.egui_id {
                continue;
            }
            let rb = b.clipped_rect;
            if rect_contains_with_tol(rb, ra, INTERACTIVE_OVERLAP_TOL_PT) || rect_contains_with_tol(ra, rb, INTERACTIVE_OVERLAP_TOL_PT) {
                continue;
            }
            if circle_intersects_rect(ca, radius, rb) {
                violations.push(ParityViolation {
                    widget_id: a.widget_id.clone(),
                    check: String::from("wcag_258_target_size"),
                    detail: format!(
                        "undersized control `{}` ({:.1}x{:.1} pt at {:?}) violates WCAG 2.5.8 24pt spacing against `{}` ({:?})",
                        a.widget_id,
                        ra.width(),
                        ra.height(),
                        ra,
                        b.widget_id,
                        rb
                    ),
                });
                break;
            }
            let b_undersized =
                !is_target_size_exempt_entry(b) && !is_scroll_edge_clipped_slice(b, min_pt) && (rb.width() < thresh || rb.height() < thresh);
            if b_undersized && ca.distance(rb.center()) < min_center_dist {
                violations.push(ParityViolation {
                    widget_id: a.widget_id.clone(),
                    check: String::from("wcag_258_target_size"),
                    detail: format!(
                        "undersized controls `{}` ({:.1}x{:.1} pt) and `{}` ({:.1}x{:.1} pt) have centres {:.1} pt apart (< 24 pt)",
                        a.widget_id,
                        ra.width(),
                        ra.height(),
                        b.widget_id,
                        rb.width(),
                        rb.height(),
                        ca.distance(rb.center())
                    ),
                });
                break;
            }
        }
    }
    violations
}

/// Assert that each [`ShellRegion`] exposes at most one enabled [`ActionRank::Primary`] action (`F5.3`).
#[must_use]
pub fn check_single_primary_per_region(meta: &FrameInspectMeta) -> Vec<ParityViolation> {
    let modal_active = meta.entries.iter().any(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown));
    let mut primaries_by_region: std::collections::BTreeMap<ShellRegion, Vec<&str>> = std::collections::BTreeMap::new();
    for entry in &meta.entries {
        if !entry.enabled || !entry.clipped_rect.is_positive() || entry.action_rank != Some(ActionRank::Primary) {
            continue;
        }
        if modal_active && !matches!(entry.z_layer, ZLayer::ModalDialog | ZLayer::ModalDropdown | ZLayer::ContextMenu) {
            continue;
        }
        let region = classify_entry_region(entry);
        let list = primaries_by_region.entry(region).or_default();
        if !list.contains(&entry.widget_id.as_str()) {
            list.push(entry.widget_id.as_str());
        }
    }

    let mut violations = Vec::new();
    for (region, ids) in primaries_by_region {
        if ids.len() > 1 {
            violations.push(ParityViolation {
                widget_id: String::from(ids[0]),
                check: String::from("single_primary_action"),
                detail: format!("region {region:?} exposes {} Primary actions (expected <= 1): {ids:?}", ids.len()),
            });
        }
    }
    violations
}

/// Count enabled clickable controls by [`ShellRegion`] on `meta` (`F5.3`).
#[must_use]
pub fn count_controls_by_region(meta: &FrameInspectMeta) -> RegionControlCounts {
    let modal_active = meta.entries.iter().any(|e| matches!(e.z_layer, ZLayer::ModalBackdrop | ZLayer::ModalDialog | ZLayer::ModalDropdown));
    let mut counts = RegionControlCounts::default();
    for entry in &meta.entries {
        if !entry.enabled || !entry.clipped_rect.is_positive() || !is_clickable_meta_role(&entry.role) {
            continue;
        }
        if modal_active && !matches!(entry.z_layer, ZLayer::ModalDialog | ZLayer::ModalDropdown | ZLayer::ContextMenu) {
            continue;
        }
        if entry.role.eq_ignore_ascii_case("gridcell") {
            counts.grid_cells += 1;
            continue;
        }
        match classify_entry_region(entry) {
            ShellRegion::TopChrome => counts.top_chrome += 1,
            ShellRegion::Navigator => counts.navigator += 1,
            ShellRegion::CenterStage => counts.center_stage += 1,
            ShellRegion::Inspector => counts.inspector += 1,
            ShellRegion::StatusFooter => counts.status_footer += 1,
            ShellRegion::DropdownMenu => counts.dropdown_menu += 1,
            ShellRegion::ModalDialog => counts.modal_dialog += 1,
        }
    }
    counts
}

/// Verify per-region visible clickable control counts against `budgets` (`F5.3`).
#[must_use]
pub fn check_region_control_budgets(meta: &FrameInspectMeta, budgets: &RegionBudgets) -> Vec<ParityViolation> {
    let counts = count_controls_by_region(meta);
    let mut violations = Vec::new();
    for region in [
        ShellRegion::TopChrome,
        ShellRegion::Navigator,
        ShellRegion::CenterStage,
        ShellRegion::Inspector,
        ShellRegion::StatusFooter,
        ShellRegion::DropdownMenu,
        ShellRegion::ModalDialog,
    ] {
        let actual = counts.count_for(region);
        let limit = budgets.limit_for(region);
        if actual > limit {
            violations.push(ParityViolation {
                widget_id: format!("region:{region:?}"),
                check: String::from("region_control_budget"),
                detail: format!("region {region:?} exposes {actual} enabled clickable controls (budget <= {limit})"),
            });
        }
    }
    violations
}

/// Assert that visible top-level panel headers (`nav.panel_header`, `split.header.primary`,
/// `split.header.secondary`, `inspector.panel_header`) share one horizontal baseline and `32 pt` height (`R20.3`).
#[must_use]
pub fn panel_header_baseline_parity(meta: &FrameInspectMeta) -> Vec<ParityViolation> {
    let ids = ["nav.panel_header", "split.header.primary", "split.header.secondary", "inspector.panel_header"];
    let found: Vec<&FrameInspectNodeMeta> = ids.iter().filter_map(|id| meta.find_by_widget_id(id)).collect();
    if found.len() < 2 {
        return Vec::new();
    }
    let ref_entry = found[0];
    let ref_top = ref_entry.clipped_rect.top();
    let ref_bot = ref_entry.clipped_rect.bottom();
    let mut violations = Vec::new();
    for entry in found.iter().skip(1) {
        let top_diff = (entry.clipped_rect.top() - ref_top).abs();
        let bot_diff = (entry.clipped_rect.bottom() - ref_bot).abs();
        if top_diff > 1.5 || bot_diff > 1.5 {
            violations.push(ParityViolation {
                widget_id: entry.widget_id.clone(),
                check: String::from("panel_header_baseline_parity"),
                detail: format!(
                    "panel header `{}` y=[{:.1}, {:.1}] differs from `{}` y=[{:.1}, {:.1}]",
                    entry.widget_id,
                    entry.clipped_rect.top(),
                    entry.clipped_rect.bottom(),
                    ref_entry.widget_id,
                    ref_top,
                    ref_bot,
                ),
            });
        }
    }
    violations
}

/// Assert three-way split-view targeting invariants (`R20.3`, `gui-layout-spec.md` §2.4):
/// materialized split sheet IDs exist in `doc`, and any pinned Inspector target stays within the active split pair.
#[must_use]
pub fn split_targeting_parity(app: &crate::UiApp) -> Vec<ParityViolation> {
    let mut violations = Vec::new();
    let doc = app.document();
    if !app.split.enabled {
        if app.split.inspector_pinned {
            violations.push(ParityViolation {
                widget_id: String::from("inspector.pin_toggle"),
                check: String::from("split_targeting_parity"),
                detail: String::from("inspector_pinned must be false when Split Comparison is closed"),
            });
        }
        return violations;
    }
    if app.split.primary_sheet_id.as_ref().is_none_or(|id| doc.sheet_section(id).is_none()) {
        violations.push(ParityViolation {
            widget_id: String::from("split.primary_selector"),
            check: String::from("split_targeting_parity"),
            detail: String::from("primary_sheet_id must be materialized and present in document when split is active"),
        });
    }
    if app.split.secondary_sheet_id.as_ref().is_none_or(|id| doc.sheet_section(id).is_none()) {
        violations.push(ParityViolation {
            widget_id: String::from("split.secondary_selector"),
            check: String::from("split_targeting_parity"),
            detail: String::from("secondary_sheet_id must be materialized and present in document when split is active"),
        });
    }
    if app.split.inspector_pinned {
        let valid = app.split.inspector_target_sheet_id.as_ref().is_some_and(|id| doc.sheet_section(id).is_some() && app.split.contains_sheet(id));
        if !valid {
            violations.push(ParityViolation {
                widget_id: String::from("inspector.pin_toggle"),
                check: String::from("split_targeting_parity"),
                detail: String::from("pinned inspector_target_sheet_id must remain inside the active split pair"),
            });
        }
    }
    violations
}

/// Assert that Split Comparison never falls back to retired preview grids (`R20.3`):
/// zero `split.col_preview.*`, `split.focus_in_grid`, or `split.activate_data_sheet` entries may appear.
#[must_use]
pub fn split_unified_datagrid_parity(meta: &FrameInspectMeta) -> Vec<ParityViolation> {
    let mut violations = Vec::new();
    for entry in &meta.entries {
        if entry.widget_id.starts_with("split.col_preview.")
            || entry.widget_id == "split.focus_in_grid"
            || entry.widget_id == "split.activate_data_sheet"
        {
            violations.push(ParityViolation {
                widget_id: entry.widget_id.clone(),
                check: String::from("split_unified_datagrid_parity"),
                detail: format!("retired split preview widget `{}` must not appear; use unified parameterized DataGrid", entry.widget_id),
            });
        }
    }
    violations
}

fn check_frame_inspect_meta(
    meta: &FrameInspectMeta,
    nodes: &[(AccessKitNodeId, AccessKitNode)],
    viewport_rect: Rect,
    violations: &mut Vec<ParityViolation>,
) -> usize {
    let mut checked = 0usize;
    for entry in &meta.entries {
        checked += 1;
        if entry.clipped_rect.width() <= 0.0 || entry.clipped_rect.height() <= 0.0 {
            violations.push(ParityViolation {
                widget_id: entry.widget_id.clone(),
                check: String::from("clipped_rect_nonzero"),
                detail: format!("recorded widget `{}` has empty clipped rect {:?}", entry.widget_id, entry.clipped_rect),
            });
        }
        if !viewport_rect.expand(1.5).intersects(entry.clipped_rect) {
            violations.push(ParityViolation {
                widget_id: entry.widget_id.clone(),
                check: String::from("viewport_containment"),
                detail: format!("widget `{}` clipped rect {:?} lies outside viewport {:?}", entry.widget_id, entry.clipped_rect, viewport_rect),
            });
        }
        let Some((_, ak_node)) = nodes.iter().find(|(id, _)| *id == entry.accesskit_id) else {
            violations.push(ParityViolation {
                widget_id: entry.widget_id.clone(),
                check: String::from("accesskit_node_present"),
                detail: format!("FrameInspectMeta entry `{}` lacks an AccessKit node", entry.widget_id),
            });
            continue;
        };
        if ak_node.is_hidden() {
            violations.push(ParityViolation {
                widget_id: entry.widget_id.clone(),
                check: String::from("accesskit_visible"),
                detail: format!("FrameInspectMeta entry `{}` is marked hidden in AccessKit", entry.widget_id),
            });
        }
        if entry.ellipsized {
            let expected_full = entry.full_label.as_deref().unwrap_or(&entry.label);
            let ak_name = accesskit_accessible_name(ak_node).unwrap_or("");
            if ak_name != expected_full {
                violations.push(ParityViolation {
                    widget_id: entry.widget_id.clone(),
                    check: String::from("ellipsis_accesskit_name"),
                    detail: format!("ellipsized label `{}` exposed `{ak_name}` in AccessKit instead of `{expected_full}`", entry.widget_id),
                });
            }
        }
    }
    checked
}

fn check_accesskit_interactive_nodes(nodes: &[(AccessKitNodeId, AccessKitNode)], viewport_rect: Rect, violations: &mut Vec<ParityViolation>) {
    for (id, node) in nodes {
        if node.is_hidden() || !is_interactive_accesskit_role(node.role()) {
            continue;
        }
        let Some(bounds) = node.bounds() else {
            violations.push(ParityViolation {
                widget_id: format!("accesskit:{id:?}"),
                check: String::from("accesskit_bounds"),
                detail: format!("interactive {:?} node lacks bounding box", node.role()),
            });
            continue;
        };
        let rect = accesskit_rect_to_egui(bounds);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            violations.push(ParityViolation {
                widget_id: format!("accesskit:{id:?}"),
                check: String::from("accesskit_nonzero_bounds"),
                detail: format!("interactive {:?} node has empty bounds {rect:?}", node.role()),
            });
        }
        if !viewport_rect.expand(1.5).intersects(rect) {
            violations.push(ParityViolation {
                widget_id: format!("accesskit:{id:?}"),
                check: String::from("accesskit_viewport_bounds"),
                detail: format!("interactive {:?} node bounds {rect:?} fall outside viewport {viewport_rect:?}", node.role()),
            });
        }
        let has_name = accesskit_accessible_name(node).is_some_and(|s| !s.trim().is_empty()) || !node.labelled_by().is_empty();
        if !has_name {
            violations.push(ParityViolation {
                widget_id: format!("accesskit:{id:?}"),
                check: String::from("accesskit_name"),
                detail: format!("interactive {:?} node at {rect:?} has empty accessible name", node.role()),
            });
        }
    }
}

type DriverUiFn<'a, State> = Box<dyn FnMut(&mut Ui, &mut FrameInspectMeta, &mut State) + 'a>;
type PostFrameHook<'a, State> = Box<dyn FnMut(&mut State, bool) + 'a>;

/// Human-path test and QA driver over real `egui 0.37` frames, raw [`egui::Event`] injection,
/// AccessKit tree snapshots, and [`FrameInspectMeta`].
pub struct EguiHumanPathDriver<'a, State = ()> {
    ctx: Context,
    metrics: ViewportMetrics,
    tokens: Tokens,
    bleed_policy: TextBleedPolicy,
    state: State,
    ui_fn: DriverUiFn<'a, State>,
    post_frame_hook: Option<PostFrameHook<'a, State>>,
    meta: FrameInspectMeta,
    last_accesskit: Option<AccessKitTreeUpdate>,
    last_shapes: Vec<ClippedShape>,
    last_ime_output: Option<IMEOutput>,
    last_immediate_repaint: bool,
    stamp: GeometryStamp,
    frame_seq: u64,
    sim_time_sec: f64,
    pointer_pos: Option<Pos2>,
    last_input_to_paint_ms: f64,
    max_input_to_paint_ms: f64,
}

impl<'a, State> EguiHumanPathDriver<'a, State> {
    /// Construct a new [`EguiHumanPathDriver`] with default [`Tokens`] and immediately settle frame 0.
    #[must_use]
    pub fn new(state: State, metrics: ViewportMetrics, ui_fn: impl FnMut(&mut Ui, &mut FrameInspectMeta, &mut State) + 'a) -> Self {
        Self::with_tokens_and_fonts(state, metrics, Tokens::default(), build_driver_font_definitions(), ui_fn)
    }

    /// Construct a driver with explicit [`Tokens`] (theme, locale, `ui_scale`) and custom [`FontDefinitions`].
    #[must_use]
    pub fn with_tokens_and_fonts(
        state: State,
        metrics: ViewportMetrics,
        tokens: Tokens,
        fonts: FontDefinitions,
        ui_fn: impl FnMut(&mut Ui, &mut FrameInspectMeta, &mut State) + 'a,
    ) -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        super::fonts::install_font_definitions(&ctx, fonts);
        apply_driver_tokens(&ctx, &tokens);

        let mut driver = Self {
            ctx,
            metrics,
            tokens,
            bleed_policy: TextBleedPolicy::StrictAllEdges,
            state,
            ui_fn: Box::new(ui_fn),
            post_frame_hook: None,
            meta: FrameInspectMeta::new(),
            last_accesskit: None,
            last_shapes: Vec::new(),
            last_ime_output: None,
            last_immediate_repaint: false,
            stamp: GeometryStamp::default(),
            frame_seq: 0,
            sim_time_sec: 0.0,
            pointer_pos: None,
            last_input_to_paint_ms: 0.0,
            max_input_to_paint_ms: 0.0,
        };
        driver.run_settled();
        driver
    }

    /// Borrow the underlying [`egui::Context`].
    #[must_use]
    pub const fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// Borrow user state owned by the driver.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// Mutably borrow user state owned by the driver.
    pub const fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Install a callback invoked after each frame pass with `(&mut state, settled)`, allowing
    /// `PendingUiIntents::commit_if_settled` to run outside discardable `ctx.run_ui` layout passes.
    pub fn set_post_frame_hook(&mut self, hook: impl FnMut(&mut State, bool) + 'a) {
        self.post_frame_hook = Some(Box::new(hook));
    }

    /// Current viewport metrics.
    #[must_use]
    pub const fn metrics(&self) -> ViewportMetrics {
        self.metrics
    }

    /// Update viewport dimensions/scale factor and settle a fresh frame.
    pub fn set_viewport(&mut self, metrics: ViewportMetrics) {
        self.metrics = metrics;
        self.run_settled();
    }

    /// Borrow active [`Tokens`].
    #[must_use]
    pub const fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// Update [`Tokens`] (theme, locale, `ui_scale`) and settle a fresh frame.
    pub fn set_tokens(&mut self, tokens: Tokens) {
        self.tokens = tokens;
        apply_driver_tokens(&self.ctx, &self.tokens);
        self.run_settled();
    }

    /// Configure text-bleed checking policy (`StrictAllEdges` vs `AllowVerticalScrollClip`).
    pub const fn set_bleed_policy(&mut self, policy: TextBleedPolicy) {
        self.bleed_policy = policy;
    }

    /// Borrow settled [`FrameInspectMeta`] from the most recent settled frame.
    #[must_use]
    pub const fn frame_meta(&self) -> &FrameInspectMeta {
        &self.meta
    }

    /// Borrow settled AccessKit [`AccessKitTreeUpdate`] from the most recent settled frame.
    #[must_use]
    pub const fn accesskit_tree(&self) -> Option<&AccessKitTreeUpdate> {
        self.last_accesskit.as_ref()
    }

    /// Latest settled [`GeometryStamp`].
    #[must_use]
    pub const fn geometry_stamp(&self) -> GeometryStamp {
        self.stamp
    }

    /// Latest `PlatformOutput::ime` emitted by `egui`.
    #[must_use]
    pub const fn last_ime_output(&self) -> Option<IMEOutput> {
        self.last_ime_output
    }

    /// Last measured input-to-paint latency in milliseconds.
    #[must_use]
    pub const fn last_input_to_paint_ms(&self) -> f64 {
        self.last_input_to_paint_ms
    }

    /// Maximum measured input-to-paint latency in milliseconds.
    #[must_use]
    pub const fn max_input_to_paint_ms(&self) -> f64 {
        self.max_input_to_paint_ms
    }

    /// Logical viewport rectangle derived from [`Self::metrics`].
    #[must_use]
    pub fn viewport_rect(&self) -> Rect {
        #[allow(clippy::cast_possible_truncation)]
        let w = self.metrics.logical_width.max(0.0) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let h = self.metrics.logical_height.max(0.0) as f32;
        Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h))
    }

    /// Return all [`EllipsisRecord`] items observed in the most recent settled frame.
    #[must_use]
    pub fn ellipsis_records(&self) -> Vec<EllipsisRecord> {
        let nodes = self.last_accesskit.as_ref().map_or(&[][..], |u| u.nodes.as_slice());
        collect_ellipsis_records(&self.last_shapes, &self.meta, nodes)
    }

    /// Return whether `widget_id` was recorded as `ellipsized = true` in the settled frame.
    #[must_use]
    pub fn is_widget_ellipsized(&self, widget_id: &str) -> bool {
        self.meta.find_by_widget_id(widget_id).is_some_and(|e| e.ellipsized)
            || self.ellipsis_records().iter().any(|r| r.widget_id.as_deref() == Some(widget_id) && r.ellipsized)
    }

    /// Collect all tooltip label strings currently visible in the AccessKit tree.
    #[must_use]
    pub fn visible_tooltip_texts(&self) -> Vec<String> {
        self.last_accesskit.as_ref().map_or_else(Vec::new, |u| collect_tooltip_texts_from_accesskit(&u.nodes))
    }

    /// Advance the UI without new events until immediate repaint requests settle (up to `MAX_SETTLE_FRAMES` passes).
    pub fn run_settled(&mut self) {
        self.step_events(Vec::new(), 1.0 / 60.0);
        for _ in 0..MAX_SETTLE_FRAMES {
            if !self.last_immediate_repaint {
                break;
            }
            self.step_events(Vec::new(), 1.0 / 60.0);
        }
    }

    /// Run a single `egui` frame with `events`, advancing simulation time by `1/60s`, and settle follow-ups.
    pub fn dispatch_events(&mut self, events: Vec<Event>) {
        let t0 = std::time::Instant::now();
        self.step_events(events, 1.0 / 60.0);
        for _ in 0..MAX_SETTLE_FRAMES {
            if !self.last_immediate_repaint {
                break;
            }
            self.step_events(Vec::new(), 1.0 / 60.0);
        }
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.last_input_to_paint_ms = elapsed_ms;
        if elapsed_ms > self.max_input_to_paint_ms {
            self.max_input_to_paint_ms = elapsed_ms;
        }
    }

    fn step_events(&mut self, events: Vec<Event>, dt_sec: f64) {
        self.sim_time_sec += dt_sec.max(0.0);
        let screen_rect = self.viewport_rect();
        #[allow(clippy::cast_possible_truncation)]
        let ppp = self.metrics.clamped_scale_factor() as f32;

        let mut raw_input = RawInput {
            screen_rect: Some(screen_rect),
            time: Some(self.sim_time_sec),
            predicted_dt: dt_sec as f32,
            focused: true,
            events,
            ..Default::default()
        };
        if let Some(vp) = raw_input.viewports.get_mut(&ViewportId::ROOT) {
            vp.native_pixels_per_point = Some(ppp);
        }

        let mut pass_meta = FrameInspectMeta::new();
        let mut last_discarded = false;
        let output: FullOutput = self.ctx.run_ui(raw_input, |ui| {
            pass_meta.begin_pass(ui.ctx());
            (self.ui_fn)(ui, &mut pass_meta, &mut self.state);
            last_discarded = ui.ctx().will_discard();
        });

        self.frame_seq = self.frame_seq.saturating_add(1);
        self.meta = pass_meta;
        self.last_accesskit = output.platform_output.accesskit_update.clone();
        self.last_ime_output = output.platform_output.ime;
        self.last_immediate_repaint = output.viewport_output.get(&ViewportId::ROOT).is_some_and(|v| v.repaint_delay == Duration::ZERO);
        self.last_shapes = output.shapes.clone();

        let settled = !last_discarded && !output.platform_output.requested_discard();
        let phys_size = [self.metrics.physical_width_px(), self.metrics.physical_height_px()];
        self.stamp = GeometryStamp::from_context(self.frame_seq, &self.ctx, phys_size, settled);
        output.drop_without_applying_deltas();
        if let Some(hook) = self.post_frame_hook.as_mut() {
            hook(&mut self.state, settled);
        }
    }

    /// Move pointer to logical `(pos.x, pos.y)` and dwell long enough for hover tooltips to open.
    ///
    /// # Errors
    /// Returns an error if `pos` is non-finite.
    pub fn hover_at(&mut self, pos: Pos2) -> Result<(), String> {
        validate_finite_pos(pos)?;
        self.pointer_pos = Some(pos);
        self.dispatch_events(vec![Event::PointerMoved(pos)]);
        let delay = f64::from(self.ctx.global_style().interaction.tooltip_delay) + 0.08;
        self.step_events(Vec::new(), delay);
        self.step_events(Vec::new(), 1.0 / 60.0);
        Ok(())
    }

    /// Move pointer onto drawn widget `widget_id` and dwell until hover tooltips open.
    ///
    /// # Errors
    /// Returns an error if `widget_id` is absent or has empty clipped bounds.
    pub fn hover_widget_id(&mut self, widget_id: &str) -> Result<(), String> {
        let center = self.widget_center(widget_id, false)?;
        self.hover_at(center)
    }

    /// Click at logical `pos` using the primary mouse button via raw `PointerMoved` + `PointerButton` events.
    ///
    /// # Errors
    /// Returns an error if `pos` is non-finite.
    pub fn click_at(&mut self, pos: Pos2) -> Result<(), String> {
        self.click_at_button(pos, PointerButton::Primary, KeyModifiers::default())
    }

    /// Click at logical `pos` with explicit `button` and `modifiers`.
    ///
    /// # Errors
    /// Returns an error if `pos` is non-finite.
    pub fn click_at_button(&mut self, pos: Pos2, button: PointerButton, modifiers: KeyModifiers) -> Result<(), String> {
        validate_finite_pos(pos)?;
        self.pointer_pos = Some(pos);
        let egui_btn = to_egui_pointer_button(button);
        let egui_mods = to_egui_modifiers(modifiers);
        self.dispatch_events(vec![Event::ModifiersChanged(egui_mods), Event::PointerMoved(pos)]);
        self.dispatch_events(vec![
            Event::ModifiersChanged(egui_mods),
            Event::PointerButton { pos, button: egui_btn, pressed: true, modifiers: egui_mods },
        ]);
        self.dispatch_events(vec![
            Event::ModifiersChanged(egui_mods),
            Event::PointerButton { pos, button: egui_btn, pressed: false, modifiers: egui_mods },
        ]);
        if !egui_mods.is_none() {
            self.dispatch_events(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        }
        Ok(())
    }

    /// Click drawn widget `widget_id` strictly at its settled [`FrameInspectMeta`] / AccessKit center.
    ///
    /// # Errors
    /// Returns an error if `widget_id` is missing, disabled, hidden in AccessKit, or has empty clipped bounds.
    pub fn click_widget_id(&mut self, widget_id: &str) -> Result<(), String> {
        let center = self.widget_center(widget_id, true)?;
        self.click_at(center)
    }

    /// Drag primary pointer from `from` to `to` in logical coordinates via raw `PointerMoved` + `PointerButton` events.
    ///
    /// # Errors
    /// Returns an error if either coordinate is non-finite.
    pub fn drag(&mut self, from: Pos2, to: Pos2) -> Result<(), String> {
        self.drag_with_modifiers(from, to, KeyModifiers::default())
    }

    /// Drag primary pointer from `from` to `to` while holding `modifiers` (`Shift`, `Ctrl`, etc.).
    ///
    /// # Errors
    /// Returns an error if either coordinate is non-finite.
    pub fn drag_with_modifiers(&mut self, from: Pos2, to: Pos2, modifiers: KeyModifiers) -> Result<(), String> {
        validate_finite_pos(from)?;
        validate_finite_pos(to)?;
        let egui_mods = to_egui_modifiers(modifiers);
        let mid = Pos2::new((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
        self.pointer_pos = Some(to);
        self.dispatch_events(vec![Event::ModifiersChanged(egui_mods), Event::PointerMoved(from)]);
        self.dispatch_events(vec![
            Event::ModifiersChanged(egui_mods),
            Event::PointerButton { pos: from, button: EguiPointerButton::Primary, pressed: true, modifiers: egui_mods },
        ]);
        self.dispatch_events(vec![Event::ModifiersChanged(egui_mods), Event::PointerMoved(mid)]);
        self.dispatch_events(vec![Event::ModifiersChanged(egui_mods), Event::PointerMoved(to)]);
        self.dispatch_events(vec![
            Event::ModifiersChanged(egui_mods),
            Event::PointerButton { pos: to, button: EguiPointerButton::Primary, pressed: false, modifiers: egui_mods },
        ]);
        if !egui_mods.is_none() {
            self.dispatch_events(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        }
        Ok(())
    }

    /// Move pointer to `pos` and emit a raw [`Event::MouseWheel`] scroll delta (in logical points).
    ///
    /// # Errors
    /// Returns an error if `pos` or `delta` contains non-finite coordinates.
    pub fn wheel_at(&mut self, pos: Pos2, delta: Vec2) -> Result<(), String> {
        validate_finite_pos(pos)?;
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(format!("non-finite wheel delta: {delta:?}"));
        }
        self.pointer_pos = Some(pos);
        self.dispatch_events(vec![Event::PointerMoved(pos)]);
        self.dispatch_events(vec![Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta,
            phase: TouchPhase::Move,
            source: MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        }]);
        Ok(())
    }

    /// Emit raw [`Event::Text`] input into the currently focused `egui` widget.
    ///
    /// # Errors
    /// Returns an error when `text` is empty.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Err(String::from("type_text requires non-empty text"));
        }
        self.dispatch_events(vec![Event::Text(text.to_owned())]);
        Ok(())
    }

    /// Press and release key `key` with `modifiers` via raw [`Event::Key`] events.
    ///
    /// # Errors
    /// Returns an error if `key` cannot be mapped to [`egui::Key`].
    pub fn press_key(&mut self, key: &str, modifiers: KeyModifiers) -> Result<(), String> {
        let egui_key = parse_egui_key(key).ok_or_else(|| format!("unknown key `{key}`"))?;
        self.press_egui_key(egui_key, to_egui_modifiers(modifiers));
        Ok(())
    }

    /// Press and release an [`egui::Key`] with [`egui::Modifiers`].
    pub fn press_egui_key(&mut self, key: Key, modifiers: Modifiers) {
        self.dispatch_events(vec![
            Event::ModifiersChanged(modifiers),
            Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers },
            Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers },
            Event::ModifiersChanged(Modifiers::NONE),
        ]);
    }

    /// Emit a raw [`Event::Ime`] [`ImeEvent::Preedit`] into the currently focused `egui` text field.
    pub fn ime_preedit(&mut self, text: &str, active_range_chars: Option<Range<usize>>) {
        self.dispatch_events(vec![Event::Ime(ImeEvent::Preedit { text: text.to_owned(), active_range_chars })]);
    }

    /// Emit a raw [`Event::Ime`] [`ImeEvent::Commit`] into the currently focused `egui` text field.
    pub fn ime_commit(&mut self, text: &str) {
        self.dispatch_events(vec![Event::Ime(ImeEvent::Commit(text.to_owned()))]);
    }

    /// Run settled-frame pixel-to-hit-box, AccessKit, [`AccessKitRole::ScrollBar`] non-overlap,
    /// and text-bleed verification on the latest frame.
    #[must_use]
    pub fn verify_pixel_to_hit_box_parity(&self) -> ParityReport {
        verify_egui_frame_parity(self.last_accesskit.as_ref(), &self.meta, &self.last_shapes, self.viewport_rect(), self.bleed_policy)
    }

    /// Check AccessKit [`AccessKitRole::ScrollBar`] non-overlap on the latest frame.
    #[must_use]
    pub fn check_scrollbar_non_overlap(&self) -> Vec<ParityViolation> {
        self.last_accesskit.as_ref().map_or_else(Vec::new, |u| check_accesskit_scrollbar_non_overlap_with_meta(&u.nodes, &self.meta))
    }

    /// Verify that an ellipsized widget sets `ellipsized == true`, exposes `expected_full_label`
    /// in AccessKit, and surfaces `expected_full_label` in its hover tooltip when hovered.
    ///
    /// # Errors
    /// Returns an error if `widget_id` is not ellipsized or lacks full-label AccessKit/tooltip exposure.
    pub fn verify_ellipsis_tooltip_and_accesskit(&mut self, widget_id: &str, expected_full_label: &str) -> Result<(), String> {
        if !self.is_widget_ellipsized(widget_id) {
            return Err(format!("widget `{widget_id}` was expected to have `ellipsized == true`"));
        }
        let meta_entry = self.meta.find_by_widget_id(widget_id).ok_or_else(|| format!("widget `{widget_id}` missing from FrameInspectMeta"))?;
        let ak_node = self
            .last_accesskit
            .as_ref()
            .and_then(|u| u.nodes.iter().find(|(id, _)| *id == meta_entry.accesskit_id))
            .map(|(_, node)| node)
            .ok_or_else(|| format!("widget `{widget_id}` missing from AccessKit tree"))?;
        let ak_name = accesskit_accessible_name(ak_node).unwrap_or("");
        if ak_name != expected_full_label {
            return Err(format!("AccessKit name for `{widget_id}` was `{ak_name}`, expected `{expected_full_label}`"));
        }
        let ak_desc = ak_node.description().map(str::to_owned);

        self.hover_widget_id(widget_id)?;
        let tooltips = self.visible_tooltip_texts();
        let has_tooltip = tooltips.iter().any(|t| t == expected_full_label) || ak_desc.as_deref() == Some(expected_full_label);
        if !has_tooltip {
            return Err(format!(
                "hovering ellipsized widget `{widget_id}` did not expose `{expected_full_label}` (visible tooltips: {tooltips:?}, description: {ak_desc:?})",
            ));
        }
        Ok(())
    }

    fn widget_center(&self, widget_id: &str, require_enabled: bool) -> Result<Pos2, String> {
        let entry = self.meta.find_by_widget_id(widget_id).ok_or_else(|| format!("widget `{widget_id}` not found in settled FrameInspectMeta"))?;
        if require_enabled && !entry.enabled {
            let reason = entry.disabled_reason.as_deref().unwrap_or("disabled");
            return Err(format!("widget `{widget_id}` is disabled: {reason}"));
        }
        if entry.clipped_rect.width() <= 0.0 || entry.clipped_rect.height() <= 0.0 {
            return Err(format!("widget `{widget_id}` has empty clipped interaction rect {:?}", entry.clipped_rect));
        }
        if let Some(tree) = &self.last_accesskit
            && let Some((_, ak_node)) = tree.nodes.iter().find(|(id, _)| *id == entry.accesskit_id)
        {
            if ak_node.is_hidden() {
                return Err(format!("widget `{widget_id}` is hidden in AccessKit"));
            }
            if require_enabled && ak_node.is_disabled() {
                return Err(format!("widget `{widget_id}` is disabled in AccessKit"));
            }
        }
        Ok(entry.clipped_rect.center())
    }
}

fn validate_finite_pos(pos: Pos2) -> Result<(), String> {
    if pos.x.is_finite() && pos.y.is_finite() { Ok(()) } else { Err(format!("non-finite pointer coordinate: {pos:?}")) }
}

fn apply_driver_tokens(ctx: &Context, tokens: &Tokens) {
    tokens.apply_to_ctx(ctx);
    ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
        style.scroll_animation = egui::style::ScrollAnimation::none();
        style.animation_time = 0.0;
    });
}

#[cfg(test)]
mod tests {
    use super::{
        AccessKitRoleBounds, BETA_LOCALE_CODES, EguiHumanPathDriver, EllipsisLabelInput, build_driver_font_definitions,
        check_role_bounds_scrollbar_non_overlap, parse_egui_key, render_tracked_label,
    };
    use crate::egui_shell::{FrameInspectEntryInput, Tokens};
    use crate::inspect::ZLayer;
    use crate::interaction::{KeyModifiers, ViewportMetrics};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    #[test]
    fn key_parser_and_beta_locale_list_cover_expected_entries() {
        assert_eq!(BETA_LOCALE_CODES.len(), 14);
        assert_eq!(BETA_LOCALE_CODES.as_slice(), crate::i18n::BETA_ACTIVE_LOCALES);
        assert!(BETA_LOCALE_CODES.contains(&"pt-BR"));
        assert!(BETA_LOCALE_CODES.contains(&"zh-CN"));
        assert!(BETA_LOCALE_CODES.contains(&"zh-TW"));
        assert!(BETA_LOCALE_CODES.contains(&"ru"));
        assert!(BETA_LOCALE_CODES.contains(&"tr"));
        assert_eq!(parse_egui_key("Enter"), Some(egui::Key::Enter));
        assert_eq!(parse_egui_key("esc"), Some(egui::Key::Escape));
        assert_eq!(parse_egui_key("ArrowDown"), Some(egui::Key::ArrowDown));
        assert_eq!(parse_egui_key("k"), Some(egui::Key::K));
        assert_eq!(parse_egui_key(""), None);
    }

    #[test]
    fn driver_clicks_widgets_and_checks_kittest_scrollbar_non_overlap() -> Result<(), String> {
        #[derive(Default)]
        struct ShellState {
            clicks: usize,
            text: String,
        }

        let mut driver = EguiHumanPathDriver::new(ShellState::default(), ViewportMetrics::new(640.0, 420.0, 1.0), |ui, meta, state| {
            egui::CentralPanel::default().show(ui, |ui| {
                let btn = ui.button("Apply Fit");
                if btn.clicked() {
                    state.clicks += 1;
                }
                meta.record_response(
                    ui,
                    &btn,
                    FrameInspectEntryInput {
                        widget_id: "btn.apply",
                        role: "Button",
                        label: "Apply Fit",
                        command_id: Some("fit.apply"),
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );

                let edit = ui.add(egui::TextEdit::singleline(&mut state.text).hint_text("Search"));
                meta.record_response(
                    ui,
                    &edit,
                    FrameInspectEntryInput {
                        widget_id: "input.search",
                        role: "TextBox",
                        label: "Search",
                        command_id: None,
                        value: Some(serde_json::json!(state.text)),
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );

                ui.allocate_ui(egui::vec2(80.0, 24.0), |ui| {
                    let _ = render_tracked_label(
                        ui,
                        meta,
                        EllipsisLabelInput {
                            widget_id: "lbl.long",
                            text: "Very long localized experimental condition title",
                            truncate: true,
                            z_layer: ZLayer::Workspace,
                        },
                    );
                });
            });
        });

        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "unexpected parity violations: {:?}", parity.violations);
        driver.click_widget_id("btn.apply")?;
        assert_eq!(driver.state().clicks, 1);

        driver.click_widget_id("input.search")?;
        driver.ime_preedit("にほん", Some(0..3));
        assert_eq!(driver.state().text, "にほん");
        driver.ime_commit("日本");
        assert_eq!(driver.state().text, "日本");
        driver.press_key("Backspace", KeyModifiers::default())?;
        assert_eq!(driver.state().text, "日");

        driver.verify_ellipsis_tooltip_and_accesskit("lbl.long", "Very long localized experimental condition title")?;

        // Also validate via egui_kittest::Harness + ScrollBar non-overlap
        let fonts = build_driver_font_definitions();
        let mut harness = Harness::builder().with_size(egui::vec2(640.0, 420.0)).allow_missing_glyphs().build_ui(|ui| {
            Tokens::default().apply_to_ctx(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for idx in 0..24 {
                        let _ = ui.button(format!("Item {idx}"));
                    }
                });
            });
        });
        harness.ctx.set_fonts(fonts);
        harness.run();
        harness.get_by_label("Item 0").click();
        harness.run();
        let bounds: Vec<AccessKitRoleBounds> = harness
            .root()
            .children_recursive()
            .filter_map(|n| {
                let ak = n.accesskit_node();
                if ak.is_hidden() || ak.bounding_box().is_none() {
                    return None;
                }
                Some(AccessKitRoleBounds {
                    id: ak.locate().0,
                    role: ak.role(),
                    label: ak.label().or_else(|| ak.value()).unwrap_or_else(|| String::from("<unnamed>")),
                    rect: n.rect(),
                })
            })
            .collect();
        let sb_violations = check_role_bounds_scrollbar_non_overlap(&bounds);
        assert!(sb_violations.is_empty(), "scrollbar overlapped controls: {sb_violations:?}");
        Ok(())
    }

    #[test]
    fn overlap_target_size_primary_rank_and_budget_gates_fail_closed_on_violations() {
        use super::{
            ActionRank, FrameInspectMeta, FrameInspectNodeMeta, RegionBudgets, check_clickable_partial_non_overlap, check_region_control_budgets,
            check_single_primary_per_region, check_wcag_258_target_size,
        };
        use egui::{Pos2, Rect, Vec2};

        let mk = |id: &str, role: &str, rect: Rect, rank: Option<ActionRank>| FrameInspectNodeMeta {
            egui_id: egui::Id::unique(id),
            accesskit_id: egui::Id::unique(id).accesskit_id(),
            widget_id: String::from(id),
            role: String::from(role),
            label: String::from(id),
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            unclipped_rect: rect,
            clipped_rect: rect,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
            action_rank: rank,
        };

        // 1. Partial overlap catches two colliding buttons while permitting nested containers.
        let mut meta_ov = FrameInspectMeta::new();
        meta_ov.record(mk("toolbar.a", "button", Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(60.0, 24.0)), Some(ActionRank::Primary)));
        meta_ov.record(mk("toolbar.b", "button", Rect::from_min_size(Pos2::new(88.0, 40.0), Vec2::new(60.0, 24.0)), Some(ActionRank::Primary)));
        assert_eq!(check_clickable_partial_non_overlap(&meta_ov).len(), 1);
        assert_eq!(check_single_primary_per_region(&meta_ov).len(), 1);

        // 2. WCAG 2.5.8 catches undersized stacked 20pt controls with 0pt vertical gap, while
        // passing spaced 20pt controls and scroll-clipped 24pt list slices.
        let mut meta_sz = FrameInspectMeta::new();
        meta_sz.record(mk("grid.super", "button", Rect::from_min_size(Pos2::new(100.0, 40.0), Vec2::new(120.0, 20.0)), None));
        meta_sz.record(mk("grid.title", "button", Rect::from_min_size(Pos2::new(100.0, 60.0), Vec2::new(120.0, 24.0)), None));
        assert_eq!(check_wcag_258_target_size(&meta_sz).len(), 1);

        let mut meta_ok = FrameInspectMeta::new();
        meta_ok.record(mk("status.a", "button", Rect::from_min_size(Pos2::new(10.0, 390.0), Vec2::new(22.0, 20.0)), None));
        meta_ok.record(mk("status.b", "button", Rect::from_min_size(Pos2::new(36.0, 390.0), Vec2::new(22.0, 20.0)), None));
        let mut clipped_row = mk("list.row", "option", Rect::from_min_size(Pos2::new(10.0, 200.0), Vec2::new(200.0, 12.0)), None);
        clipped_row.unclipped_rect = Rect::from_min_size(Pos2::new(10.0, 200.0), Vec2::new(200.0, 28.0));
        meta_ok.record(clipped_row);
        assert!(check_wcag_258_target_size(&meta_ok).is_empty());

        let tight_budget = RegionBudgets { status_footer: 1, ..RegionBudgets::CANONICAL };
        assert_eq!(check_region_control_budgets(&meta_ok, &tight_budget).len(), 1);
    }
}
