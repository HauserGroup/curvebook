//! View-level accessibility metadata, live-region focus announcements, high-contrast focus
//! rings, and `R10.20` desktop/web projection helpers.

use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::interaction::{UiEventOutcome, ViewportMetrics};
use crate::layout::a11y::cycle_focus_in_scope;
use crate::layout::{LayoutBuilder, LayoutRectPx, LayoutTree, WidgetId, WidgetNode, ZLayer};
use crate::theme::{UiThemeMode, UiThemeSpec};

pub use crate::layout::a11y::{
    ACCESSIBLE_ROOT_NODE_ID, AccessibilityGateError, AccessibilityGateReport, AccessibilityMeta, AccessibilityViolation, AccessibleActionTag,
    AccessibleBounds, AccessibleNodeUpdate, AccessiblePhysicalBounds, AccessibleTreeUpdate, AriaLivePoliteness, AriaNodeProjection, AriaRoleMapping,
    AriaTreeProjection, FocusScope, WidgetRole, audit_layout_tree_accessibility, deterministic_node_id, project_accessible_tree,
    project_accessible_tree_with_announcement, validate_layout_tree_accessibility,
};

/// Transient view state owned by the accessibility shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct A11yViewState {
    #[serde(default)]
    pub last_announcement: Option<String>,
    #[serde(default)]
    pub live_politeness: AriaLivePoliteness,
    #[serde(default)]
    pub high_contrast_focus_ring: bool,
    #[serde(default)]
    pub last_focused_widget_id: Option<String>,
    #[serde(default)]
    pub announcement_seq: u64,
}

/// Visual stroke styling for keyboard focus rings across standard and high-contrast themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusRingStyle {
    pub outer_color: Rgba8,
    pub inner_color: Rgba8,
    pub stroke_px: u32,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `a11y` domain.
///
/// Supported payloads:
/// - `"announce:<text>"` / `"announce_assertive:<text>"`
/// - `"clear_announcement"`
/// - `"announce_focus"` / `"announce_focus:<widget_id>"`
/// - `"focus_next"` / `"focus_prev"`
/// - `"toggle_high_contrast_focus"` / `"high_contrast_focus:on"` / `"high_contrast_focus:off"`
/// - JSON object `{ "op": ..., ... }`
///
/// Unrecognized payloads return `Ok(UiEventOutcome::default())` (`handled: false`) for scaffold compatibility.
///
/// # Errors
/// Returns an error string when an explicit `announce:` command supplies empty text or invalid JSON arguments.
pub fn handle_custom_action(app: &mut UiApp, state: &mut A11yViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let trimmed = payload.trim();
    if let Some(msg) = trimmed.strip_prefix("announce_assertive:") {
        return apply_announcement(state, msg, AriaLivePoliteness::Assertive);
    }
    if let Some(msg) = trimmed.strip_prefix("announce:") {
        return apply_announcement(state, msg, AriaLivePoliteness::Polite);
    }
    if let Some(wid) = trimmed.strip_prefix("announce_focus:") {
        return announce_widget_focus(app, state, Some(wid.trim()));
    }
    if trimmed.starts_with('{')
        && let Ok(val) = serde_json::from_str::<Value>(trimmed)
    {
        return handle_json_a11y_action(app, state, &val);
    }

    match trimmed {
        "clear_announcement" => {
            state.last_announcement = None;
            state.live_politeness = AriaLivePoliteness::Off;
            Ok(UiEventOutcome { handled: true, action: String::from("a11y.clear_announcement"), ..Default::default() })
        }
        "announce_focus" => announce_widget_focus(app, state, None),
        "focus_next" => step_focus_and_announce(app, state, false),
        "focus_prev" => step_focus_and_announce(app, state, true),
        "toggle_high_contrast_focus" => {
            state.high_contrast_focus_ring = !state.high_contrast_focus_ring;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("a11y.high_contrast_focus"),
                command_result: Some(json!({ "high_contrast_focus_ring": state.high_contrast_focus_ring })),
                ..Default::default()
            })
        }
        "high_contrast_focus:on" => set_high_contrast_focus(state, true),
        "high_contrast_focus:off" => set_high_contrast_focus(state, false),
        _ => Ok(UiEventOutcome::default()),
    }
}

fn handle_json_a11y_action(app: &mut UiApp, state: &mut A11yViewState, val: &Value) -> Result<UiEventOutcome, String> {
    let Some(op) = val.get("op").and_then(Value::as_str) else {
        return Ok(UiEventOutcome::default());
    };
    match op {
        "announce" => {
            let text = val.get("text").and_then(Value::as_str).unwrap_or("");
            let assertive = val.get("assertive").and_then(Value::as_bool).unwrap_or(false);
            let politeness = if assertive { AriaLivePoliteness::Assertive } else { AriaLivePoliteness::Polite };
            apply_announcement(state, text, politeness)
        }
        "clear_announcement" => {
            state.last_announcement = None;
            state.live_politeness = AriaLivePoliteness::Off;
            Ok(UiEventOutcome { handled: true, action: String::from("a11y.clear_announcement"), ..Default::default() })
        }
        "announce_focus" => {
            let wid = val.get("widget_id").and_then(Value::as_str);
            announce_widget_focus(app, state, wid)
        }
        "focus_next" => step_focus_and_announce(app, state, false),
        "focus_prev" => step_focus_and_announce(app, state, true),
        "set_high_contrast_focus" => {
            let enabled = val.get("enabled").and_then(Value::as_bool).unwrap_or(!state.high_contrast_focus_ring);
            set_high_contrast_focus(state, enabled)
        }
        _ => Ok(UiEventOutcome::default()),
    }
}

fn set_high_contrast_focus(state: &mut A11yViewState, enabled: bool) -> Result<UiEventOutcome, String> {
    state.high_contrast_focus_ring = enabled;
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("a11y.high_contrast_focus"),
        command_result: Some(json!({ "high_contrast_focus_ring": state.high_contrast_focus_ring })),
        ..Default::default()
    })
}

fn apply_announcement(state: &mut A11yViewState, raw: &str, politeness: AriaLivePoliteness) -> Result<UiEventOutcome, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(String::from("accessibility announcement text cannot be empty"));
    }
    state.last_announcement = Some(text.to_owned());
    state.live_politeness = politeness;
    state.announcement_seq = state.announcement_seq.saturating_add(1);
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("a11y.announce"),
        command_result: Some(json!({
            "announcement": state.last_announcement,
            "politeness": politeness.as_str(),
            "seq": state.announcement_seq,
        })),
        ..Default::default()
    })
}

fn announce_widget_focus(app: &UiApp, state: &mut A11yViewState, target_widget_id: Option<&str>) -> Result<UiEventOutcome, String> {
    let target_id = target_widget_id.filter(|s| !s.is_empty()).or(app.focus_widget_id.as_deref());
    let Some(fid) = target_id else {
        return Ok(UiEventOutcome { handled: true, action: String::from("a11y.no_focused_widget"), ..Default::default() });
    };
    let tree = app.layout_tree_cow(ViewportMetrics::default());
    let Some(node) = tree.node(fid) else {
        return Err(format!("focused widget `{fid}` not present in LayoutTree"));
    };
    let announcement = format_focus_announcement(node);
    state.last_focused_widget_id = Some(fid.to_owned());
    state.last_announcement = Some(announcement.clone());
    state.live_politeness = AriaLivePoliteness::Polite;
    state.announcement_seq = state.announcement_seq.saturating_add(1);
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("a11y.announce_focus"),
        command_result: Some(json!({
            "widget_id": fid,
            "announcement": announcement,
            "seq": state.announcement_seq,
        })),
        ..Default::default()
    })
}

fn step_focus_and_announce(app: &mut UiApp, state: &mut A11yViewState, reverse: bool) -> Result<UiEventOutcome, String> {
    let (next_id, announcement, focus_scope) = {
        let tree = app.layout_tree_cow(ViewportMetrics::default());
        let Some(next_id) = cycle_focus_in_scope(&tree.nodes, tree.active_focus_scope, app.focus_widget_id.as_deref(), reverse) else {
            return Ok(UiEventOutcome { handled: true, action: String::from("a11y.focus_empty"), ..Default::default() });
        };
        let announcement = tree.node(&next_id).map_or_else(|| next_id.clone(), format_focus_announcement);
        (next_id, announcement, tree.active_focus_scope)
    };
    app.focus_widget_id = Some(next_id.clone());
    state.last_focused_widget_id = Some(next_id.clone());
    state.last_announcement = Some(announcement.clone());
    state.live_politeness = AriaLivePoliteness::Polite;
    state.announcement_seq = state.announcement_seq.saturating_add(1);
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("a11y.focus_step"),
        command_result: Some(json!({
            "widget_id": next_id,
            "announcement": announcement,
            "focus_scope": focus_scope,
            "seq": state.announcement_seq,
        })),
        ..Default::default()
    })
}

/// Format a deterministic screen-reader announcement describing `node`'s role, label, state, and shortcut.
#[must_use]
pub fn format_focus_announcement(node: &WidgetNode) -> String {
    let mapping = AriaRoleMapping::for_node(node);
    let mut parts = vec![format!("{}: {}", mapping.aria_role, node.a11y.label)];
    if let Some(val) = &node.a11y.value_text
        && !val.trim().is_empty()
    {
        parts.push(format!("value {val}"));
    }
    if let Some(checked) = node.a11y.checked {
        parts.push(String::from(if checked { "checked" } else { "unchecked" }));
    }
    if let Some(selected) = node.a11y.selected
        && selected
    {
        parts.push(String::from("selected"));
    }
    if let Some(expanded) = node.a11y.expanded {
        parts.push(String::from(if expanded { "expanded" } else { "collapsed" }));
    }
    if !node.state.enablement.enabled {
        if let Some(reason) = &node.state.enablement.disabled_reason {
            parts.push(format!("disabled ({reason})"));
        } else {
            parts.push(String::from("disabled"));
        }
    }
    if let Some(shortcut) = node.a11y.shortcut.as_deref().or(node.a11y.keyboard_path.as_deref()) {
        parts.push(format!("key {shortcut}"));
    }
    parts.join(", ")
}

/// Compute WCAG-compliant outer and inner focus ring stroke styling for `theme` and `state`.
#[must_use]
pub fn high_contrast_focus_ring_style(theme: &UiThemeSpec, state: &A11yViewState) -> FocusRingStyle {
    let hc = state.high_contrast_focus_ring || theme.mode == UiThemeMode::HighContrast;
    if !hc {
        let outer = if theme.accent.contrast_ratio(theme.surface_bg) >= 3.0 {
            theme.accent
        } else if theme.surface_bg.relative_luminance() < 0.25 {
            Rgba8::WHITE
        } else {
            Rgba8::BLACK
        };
        let inner = if outer.relative_luminance() < 0.35 { Rgba8::WHITE } else { Rgba8::BLACK };
        return FocusRingStyle { outer_color: outer, inner_color: inner, stroke_px: 2 };
    }

    let dark_surface = theme.surface_bg.relative_luminance() < 0.25;
    let candidate = if dark_surface { Rgba8::rgb(253, 224, 71) } else { Rgba8::rgb(15, 23, 42) };
    let outer = if candidate.contrast_ratio(theme.surface_bg) >= 7.0 {
        candidate
    } else if dark_surface {
        Rgba8::WHITE
    } else {
        Rgba8::BLACK
    };
    let inner = if dark_surface { Rgba8::BLACK } else { Rgba8::WHITE };
    FocusRingStyle { outer_color: outer, inner_color: inner, stroke_px: 3 }
}

/// Paint an accessible focus ring onto `builder` when `focused` is true (dual high-contrast stroke when active).
pub fn paint_accessible_focus_ring(
    builder: &mut LayoutBuilder<'_>,
    z_layer: ZLayer,
    owner: &WidgetId,
    rect: LayoutRectPx,
    focused: bool,
    state: &A11yViewState,
) {
    if !focused || rect.width < 6 || rect.height < 6 {
        return;
    }
    let style = high_contrast_focus_ring_style(&builder.app.theme, state);
    builder.paint_stroke(z_layer, Some(owner), rect, style.outer_color, style.stroke_px);
    if state.high_contrast_focus_ring || builder.app.theme.mode == UiThemeMode::HighContrast {
        let inner_rect = rect.inset(style.stroke_px);
        if inner_rect.width >= 4 && inner_rect.height >= 4 {
            builder.paint_stroke(z_layer, Some(owner), inner_rect, style.inner_color, 1);
        }
    }
}

/// Resolve the active live-region announcement text and politeness from `app` and `tree`.
#[must_use]
pub fn resolve_live_announcement(app: &UiApp, tree: &LayoutTree) -> (Option<String>, AriaLivePoliteness) {
    if let Some(explicit) = app.view_shards.a11y.last_announcement.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return (Some(explicit.to_owned()), app.view_shards.a11y.live_politeness);
    }
    if let Some(banner) = app.canvas_dialogs.status_banner.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return (Some(banner.to_owned()), AriaLivePoliteness::Assertive);
    }
    if let Some(tip) = tree.node("overlay.tooltip").filter(|n| n.drawn_in_frame && !n.a11y.label.trim().is_empty()) {
        return (Some(tip.a11y.label.clone()), AriaLivePoliteness::Polite);
    }
    if let Some(fid) = app.focus_widget_id.as_deref()
        && let Some(node) = tree.node(fid).filter(|n| n.drawn_in_frame)
    {
        return (Some(format_focus_announcement(node)), AriaLivePoliteness::Polite);
    }
    if let Some(summary) = tree.node("status.summary").filter(|n| n.drawn_in_frame && !n.a11y.label.trim().is_empty()) {
        return (Some(summary.a11y.label.clone()), AriaLivePoliteness::Polite);
    }
    (None, AriaLivePoliteness::Off)
}

/// Project `app`'s current retained [`LayoutTree`] into an [`AccessibleTreeUpdate`] for desktop/web hosts.
#[must_use]
pub fn sync_app_accessibility_tree(app: &UiApp, metrics: ViewportMetrics) -> AccessibleTreeUpdate {
    let tree = app.layout_tree_cow(metrics);
    let (announcement, politeness) = resolve_live_announcement(app, &tree);
    project_accessible_tree_with_announcement(&tree, app.focus_widget_id.as_deref(), announcement.as_deref(), politeness)
}

/// Project `app`'s current retained [`LayoutTree`] directly into a browser [`AriaTreeProjection`].
#[must_use]
pub fn project_app_aria_tree(app: &UiApp, metrics: ViewportMetrics) -> AriaTreeProjection {
    sync_app_accessibility_tree(app, metrics).to_aria_projection()
}

/// Construct [`AccessibilityMeta`] for a view node with a required `keyboard_path`.
#[must_use]
pub fn labeled_meta(role: WidgetRole, label: impl Into<String>, focus_scope: FocusScope, keyboard_path: impl Into<String>) -> AccessibilityMeta {
    AccessibilityMeta::new(role, label, focus_scope).with_keyboard_path(keyboard_path)
}

/// Construct [`AccessibilityMeta`] for a selectable item (`selected` set explicitly).
#[must_use]
pub fn selectable_meta(
    role: WidgetRole,
    label: impl Into<String>,
    focus_scope: FocusScope,
    keyboard_path: impl Into<String>,
    selected: bool,
) -> AccessibilityMeta {
    let mut meta = labeled_meta(role, label, focus_scope, keyboard_path);
    meta.selected = Some(selected);
    meta
}

/// Construct [`AccessibilityMeta`] for a collapsible disclosure header (`expanded` set explicitly).
#[must_use]
pub fn disclosure_meta(label: impl Into<String>, focus_scope: FocusScope, keyboard_path: impl Into<String>, expanded: bool) -> AccessibilityMeta {
    let mut meta = labeled_meta(WidgetRole::Disclosure, label, focus_scope, keyboard_path);
    meta.expanded = Some(expanded);
    meta
}

/// Construct [`AccessibilityMeta`] for a virtualized data-grid cell.
#[must_use]
pub fn grid_cell_meta(label: impl Into<String>, selected: bool, value_text: Option<&str>) -> AccessibilityMeta {
    let mut meta = selectable_meta(WidgetRole::GridCell, label, FocusScope::Workspace, "ArrowKeys", selected);
    meta.value_text = value_text.map(str::to_owned);
    meta
}

/// Validate `R10.20` accessibility requirements on an interactive [`AccessibilityMeta`].
///
/// # Errors
/// Returns a descriptive diagnostic when an interactive role lacks a non-empty `label`,
/// a valid ARIA/AccessKit mapping, or a non-empty `keyboard_path`.
pub fn validate_interactive_a11y(meta: &AccessibilityMeta) -> Result<(), String> {
    if !meta.role.is_interactive() {
        return Ok(());
    }
    let mapping = AriaRoleMapping::for_role(meta.role);
    if mapping.aria_role.is_empty() || mapping.accesskit_role.is_empty() {
        return Err(format!("interactive `{}` widget lacks ARIA/AccessKit role mapping", meta.role.as_str()));
    }
    if meta.label.trim().is_empty() {
        return Err(format!("interactive `{}` widget requires a non-empty accessibility label", meta.role.as_str()));
    }
    if meta.keyboard_path.as_deref().is_none_or(|p| p.trim().is_empty()) {
        return Err(format!("interactive `{}` widget `{}` requires a non-empty keyboard_path", meta.role.as_str(), meta.label));
    }
    Ok(())
}
