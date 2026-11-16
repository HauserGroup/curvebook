//! Accessibility metadata, focus scopes, modal `Tab` / `Shift+Tab` focus trapping,
//! AccessKit/ARIA tree projection, and the `R10.20` CI accessibility gate (`R10.16`, `R10.20`).

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{LayoutTree, WidgetAction, WidgetNode, ZLayer};

/// Deterministic root node identifier reserved for [`AccessibleTreeUpdate::root_id`].
pub const ACCESSIBLE_ROOT_NODE_ID: u64 = 1;

/// Active keyboard focus scope governing `Tab` navigation and escape handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FocusScope {
    #[default]
    Workspace,
    ModalDialog,
    PopupMenu,
}

/// Canonical widget role emitted on every retained [`WidgetNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WidgetRole {
    MenuBar,
    MenuItem,
    Button,
    Tab,
    TreeItem,
    GridCell,
    TextBox,
    ComboBox,
    CheckBox,
    RadioButton,
    SpinButton,
    Slider,
    ScrollBar,
    Splitter,
    Canvas,
    DataPoint,
    Dialog,
    Banner,
    Tooltip,
    ContextMenu,
    Disclosure,
    Label,
}

impl WidgetRole {
    /// Canonical lowercase role string projected into `inspect_ui_tree`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MenuBar => "menu_bar",
            Self::MenuItem => "menu_item",
            Self::Button => "button",
            Self::Tab => "tab",
            Self::TreeItem => "tree_item",
            Self::GridCell => "grid_cell",
            Self::TextBox => "text_box",
            Self::ComboBox => "combo_box",
            Self::CheckBox => "check_box",
            Self::RadioButton => "radio_button",
            Self::SpinButton => "spin_button",
            Self::Slider => "slider",
            Self::ScrollBar => "scroll_bar",
            Self::Splitter => "splitter",
            Self::Canvas => "canvas",
            Self::DataPoint => "data_point",
            Self::Dialog => "dialog",
            Self::Banner => "banner",
            Self::Tooltip => "tooltip",
            Self::ContextMenu => "context_menu",
            Self::Disclosure => "disclosure",
            Self::Label => "label",
        }
    }

    /// Whether nodes of this role represent user-interactive controls checked by parity & a11y gates.
    #[must_use]
    pub const fn is_interactive(self) -> bool {
        !matches!(self, Self::Label | Self::Banner | Self::Tooltip)
    }
}

/// Accessibility & keyboard navigation descriptor attached to every [`WidgetNode`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityMeta {
    pub role: WidgetRole,
    pub label: String,
    pub value_text: Option<String>,
    pub value_json: Option<Value>,
    pub shortcut: Option<String>,
    pub keyboard_path: Option<String>,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
}

impl AccessibilityMeta {
    /// Construct minimal required metadata (`role`, `label`, `focus_scope`).
    #[must_use]
    pub fn new(role: WidgetRole, label: impl Into<String>, focus_scope: FocusScope) -> Self {
        Self {
            role,
            label: label.into(),
            value_text: None,
            value_json: None,
            shortcut: None,
            keyboard_path: None,
            focus_scope,
            tab_order: None,
            checked: None,
            selected: None,
            expanded: None,
        }
    }

    /// Attach a canonical keyboard activation path (`R10.20`).
    #[must_use]
    pub fn with_keyboard_path(mut self, path: impl Into<String>) -> Self {
        self.keyboard_path = Some(path.into());
        self
    }

    /// Assign an explicit `Tab` focus order inside `focus_scope`.
    #[must_use]
    pub const fn with_tab_order(mut self, order: u32) -> Self {
        self.tab_order = Some(order);
        self
    }
}

/// Ordered list of enabled, focusable widget nodes within `scope` (trapping `Tab` inside active modal/popup scopes).
#[must_use]
pub fn ordered_focus_targets(nodes: &[WidgetNode], scope: FocusScope) -> Vec<&WidgetNode> {
    let mut focusable: Vec<&WidgetNode> = nodes
        .iter()
        .filter(|n| {
            n.a11y.focus_scope == scope
                && n.state.enablement.enabled
                && n.drawn_in_frame
                && n.bounds.visible_rect_px.width > 0
                && n.bounds.visible_rect_px.height > 0
                && is_tab_focusable_role(n.a11y.role)
                && (n.a11y.tab_order.is_some() || n.action != WidgetAction::NoOp)
        })
        .collect();
    focusable.sort_by_key(|n| (n.a11y.tab_order.unwrap_or(u32::MAX), n.z_layer, n.paint_order));
    focusable
}

/// Advance keyboard focus forwards (`reverse == false`) or backwards (`reverse == true`) inside `scope`.
#[must_use]
pub fn cycle_focus_in_scope(nodes: &[WidgetNode], scope: FocusScope, current_id: Option<&str>, reverse: bool) -> Option<String> {
    let targets = ordered_focus_targets(nodes, scope);
    if targets.is_empty() {
        return None;
    }
    let idx = current_id.and_then(|id| targets.iter().position(|n| n.id.as_str() == id));
    let next_idx = match (idx, reverse) {
        (Some(i), false) => (i + 1) % targets.len(),
        (Some(0), true) => targets.len() - 1,
        (Some(i), true) => i - 1,
        (None, false) => 0,
        (None, true) => targets.len() - 1,
    };
    targets.get(next_idx).map(|n| n.id.0.clone())
}

/// Whether `role` participates in sequential `Tab` / `Shift+Tab` keyboard focus traversal.
#[must_use]
pub const fn is_tab_focusable_role(role: WidgetRole) -> bool {
    matches!(
        role,
        WidgetRole::Button
            | WidgetRole::Tab
            | WidgetRole::TreeItem
            | WidgetRole::TextBox
            | WidgetRole::ComboBox
            | WidgetRole::CheckBox
            | WidgetRole::RadioButton
            | WidgetRole::SpinButton
            | WidgetRole::Slider
            | WidgetRole::Disclosure
            | WidgetRole::MenuItem
    )
}

/// Compute a deterministic, non-zero numeric ID (`>= 2`) for `widget_id` across platforms.
///
/// ID `0` is avoided for AccessKit `NonZeroU64` compatibility and [`ACCESSIBLE_ROOT_NODE_ID`] (`1`)
/// is reserved for the workspace root node.
#[must_use]
pub fn deterministic_node_id(widget_id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in widget_id.as_bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    if hash <= ACCESSIBLE_ROOT_NODE_ID { hash.wrapping_add(2) } else { hash }
}

/// Politeness level for live-region announcements across AccessKit and browser ARIA trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AriaLivePoliteness {
    Off,
    #[default]
    Polite,
    Assertive,
}

impl AriaLivePoliteness {
    /// Canonical WAI-ARIA `aria-live` attribute token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Polite => "polite",
            Self::Assertive => "assertive",
        }
    }
}

/// Semantic assistive-technology action supported by an accessible widget node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessibleActionTag {
    Focus,
    Click,
    Expand,
    Collapse,
    Increment,
    Decrement,
    SetValue,
    Scroll,
}

impl AccessibleActionTag {
    /// Canonical lowercase action tag.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Click => "click",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::Increment => "increment",
            Self::Decrement => "decrement",
            Self::SetValue => "set_value",
            Self::Scroll => "scroll",
        }
    }
}

/// Bidirectional mapping from CurveBook [`WidgetRole`] to WAI-ARIA and `accesskit::Role` tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct AriaRoleMapping {
    pub widget_role: WidgetRole,
    pub aria_role: &'static str,
    pub accesskit_role: &'static str,
    pub html_tag: &'static str,
    pub interactive: bool,
    pub tab_focusable: bool,
}

#[derive(Deserialize)]
struct AriaRoleMappingWire {
    widget_role: WidgetRole,
    #[serde(default)]
    accesskit_role: String,
    #[serde(default)]
    html_tag: String,
}

impl<'de> Deserialize<'de> for AriaRoleMapping {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = AriaRoleMappingWire::deserialize(deserializer)?;
        let mut mapping = Self::for_role(wire.widget_role);
        if wire.widget_role == WidgetRole::TextBox && (wire.accesskit_role == "MultilineTextInput" || wire.html_tag == "textarea") {
            mapping.accesskit_role = "MultilineTextInput";
            mapping.html_tag = "textarea";
        }
        Ok(mapping)
    }
}

impl AriaRoleMapping {
    /// Resolve canonical WAI-ARIA and AccessKit role tags for `role`.
    #[must_use]
    pub const fn for_role(role: WidgetRole) -> Self {
        let (aria_role, accesskit_role, html_tag) = match role {
            WidgetRole::MenuBar => ("menubar", "MenuBar", "div"),
            WidgetRole::MenuItem => ("menuitem", "MenuItem", "button"),
            WidgetRole::Button => ("button", "Button", "button"),
            WidgetRole::Tab => ("tab", "Tab", "button"),
            WidgetRole::TreeItem => ("treeitem", "TreeItem", "div"),
            WidgetRole::GridCell => ("gridcell", "GridCell", "div"),
            WidgetRole::TextBox => ("textbox", "TextInput", "input"),
            WidgetRole::ComboBox => ("combobox", "ComboBox", "button"),
            WidgetRole::CheckBox => ("checkbox", "CheckBox", "input"),
            WidgetRole::RadioButton => ("radio", "RadioButton", "input"),
            WidgetRole::SpinButton => ("spinbutton", "SpinButton", "input"),
            WidgetRole::Slider => ("slider", "Slider", "input"),
            WidgetRole::ScrollBar => ("scrollbar", "ScrollBar", "div"),
            WidgetRole::Splitter => ("separator", "Splitter", "div"),
            WidgetRole::Canvas => ("application", "Canvas", "div"),
            WidgetRole::DataPoint => ("graphics-symbol", "GraphicsSymbol", "div"),
            WidgetRole::Dialog => ("dialog", "Dialog", "div"),
            WidgetRole::Banner => ("status", "Banner", "div"),
            WidgetRole::Tooltip => ("tooltip", "Tooltip", "div"),
            WidgetRole::ContextMenu => ("menu", "Menu", "div"),
            WidgetRole::Disclosure => ("button", "DisclosureTriangle", "button"),
            WidgetRole::Label => ("note", "Label", "span"),
        };
        Self {
            widget_role: role,
            aria_role,
            accesskit_role,
            html_tag,
            interactive: role.is_interactive(),
            tab_focusable: is_tab_focusable_role(role),
        }
    }

    /// Resolve role mapping with node context (e.g., multi-line notes editors map to `MultilineTextInput`).
    #[must_use]
    pub fn for_node(node: &WidgetNode) -> Self {
        let mut mapping = Self::for_role(node.a11y.role);
        if node.a11y.role == WidgetRole::TextBox && node.id.as_str() == "info.notes" {
            mapping.accesskit_role = "MultilineTextInput";
            mapping.html_tag = "textarea";
        }
        mapping
    }
}

/// Logical CSS/point bounding box `(x, y, w, h)` emitted for AccessKit and DOM ARIA projection.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AccessibleBounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl AccessibleBounds {
    /// Construct logical bounds from `[x, y, w, h]`.
    #[must_use]
    pub const fn from_rect_logical(rect: [f64; 4]) -> Self {
        Self { x: rect[0], y: rect[1], w: rect[2], h: rect[3] }
    }
}

/// Physical pixel bounding box `(x, y, w, h)` clipped to the active viewport/scroll container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AccessiblePhysicalBounds {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Single drawn node projected into desktop AccessKit (`accesskit::Node`) and browser ARIA DOM trees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibleNodeUpdate {
    pub numeric_id: u64,
    pub widget_id: String,
    pub parent_id: Option<String>,
    pub parent_numeric_id: Option<u64>,
    pub children_ids: Vec<String>,
    pub children_numeric_ids: Vec<u64>,
    pub role: WidgetRole,
    pub role_mapping: AriaRoleMapping,
    pub label: String,
    pub value_text: Option<String>,
    pub bounds_logical: AccessibleBounds,
    pub bounds_physical: AccessiblePhysicalBounds,
    pub z_layer: ZLayer,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub selected: Option<bool>,
    pub checked: Option<bool>,
    pub expanded: Option<bool>,
    pub focused: bool,
    pub hidden: bool,
    pub modal: bool,
    pub keyboard_path: Option<String>,
    pub shortcut: Option<String>,
    pub live_region: Option<AriaLivePoliteness>,
    pub announcement: Option<String>,
    pub actions: Vec<AccessibleActionTag>,
}

impl AccessibleNodeUpdate {
    /// Deterministic DOM element ID used in `apps/curvebook-web/index.html` hidden ARIA live-tree sync.
    #[must_use]
    pub fn dom_id(&self) -> String {
        format!("cb-a11y-{}", self.numeric_id)
    }

    /// Serialize canonical WAI-ARIA attributes for browser DOM element projection.
    #[must_use]
    pub fn to_aria_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = BTreeMap::new();
        attrs.insert(String::from("role"), String::from(self.role_mapping.aria_role));
        attrs.insert(String::from("aria-label"), self.label.clone());
        attrs.insert(String::from("data-widget-id"), self.widget_id.clone());
        if !self.enabled {
            attrs.insert(String::from("aria-disabled"), String::from("true"));
            if let Some(reason) = &self.disabled_reason {
                attrs.insert(String::from("aria-description"), reason.clone());
            }
        }
        if let Some(val) = &self.value_text {
            attrs.insert(String::from("aria-valuetext"), val.clone());
        }
        if let Some(sel) = self.selected {
            attrs.insert(String::from("aria-selected"), String::from(if sel { "true" } else { "false" }));
        }
        if let Some(chk) = self.checked {
            attrs.insert(String::from("aria-checked"), String::from(if chk { "true" } else { "false" }));
        }
        if let Some(exp) = self.expanded {
            attrs.insert(String::from("aria-expanded"), String::from(if exp { "true" } else { "false" }));
        }
        if self.hidden {
            attrs.insert(String::from("aria-hidden"), String::from("true"));
        }
        if self.modal {
            attrs.insert(String::from("aria-modal"), String::from("true"));
        }
        if let Some(live) = self.live_region {
            attrs.insert(String::from("aria-live"), String::from(live.as_str()));
        }
        if let Some(kpath) = &self.keyboard_path {
            attrs.insert(String::from("data-keyboard-path"), kpath.clone());
        }
        if let Some(shortcut) = &self.shortcut {
            attrs.insert(String::from("aria-keyshortcuts"), shortcut.clone());
        }
        let tabindex = if self.focused || (self.role_mapping.tab_focusable && self.enabled && !self.hidden) { "0" } else { "-1" };
        attrs.insert(String::from("tabindex"), String::from(tabindex));
        attrs
    }

    /// Project this node into a browser-ready [`AriaNodeProjection`] element descriptor.
    #[must_use]
    pub fn to_aria_node(&self) -> AriaNodeProjection {
        AriaNodeProjection {
            dom_id: self.dom_id(),
            numeric_id: self.numeric_id,
            widget_id: self.widget_id.clone(),
            parent_dom_id: self.parent_numeric_id.map(|id| format!("cb-a11y-{id}")),
            children_dom_ids: self.children_numeric_ids.iter().map(|id| format!("cb-a11y-{id}")).collect(),
            tag_name: String::from(self.role_mapping.html_tag),
            role: String::from(self.role_mapping.aria_role),
            label: self.label.clone(),
            value_text: self.value_text.clone(),
            keyboard_path: self.keyboard_path.clone(),
            bounds_css_px: [self.bounds_logical.x, self.bounds_logical.y, self.bounds_logical.w, self.bounds_logical.h],
            attributes: self.to_aria_attributes(),
        }
    }
}

/// Complete deterministic accessibility tree update projected from a finalized [`LayoutTree`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibleTreeUpdate {
    pub root_id: u64,
    pub root_label: String,
    pub root_children_ids: Vec<String>,
    pub root_children_numeric_ids: Vec<u64>,
    pub locale: String,
    pub is_rtl: bool,
    pub active_focus_scope: FocusScope,
    pub focus_id: Option<String>,
    pub focus_numeric_id: u64,
    pub scale_factor: f64,
    pub live_announcement: Option<String>,
    pub live_politeness: AriaLivePoliteness,
    pub nodes: Vec<AccessibleNodeUpdate>,
}

impl AccessibleTreeUpdate {
    /// Lookup an [`AccessibleNodeUpdate`] by exact widget ID.
    #[must_use]
    pub fn node(&self, widget_id: &str) -> Option<&AccessibleNodeUpdate> {
        self.nodes.iter().find(|n| n.widget_id == widget_id)
    }

    /// Lookup an [`AccessibleNodeUpdate`] by its deterministic numeric ID.
    #[must_use]
    pub fn node_by_numeric_id(&self, numeric_id: u64) -> Option<&AccessibleNodeUpdate> {
        self.nodes.iter().find(|n| n.numeric_id == numeric_id)
    }

    /// Convert this update into a browser [`AriaTreeProjection`] for `apps/curvebook-web`.
    #[must_use]
    pub fn to_aria_projection(&self) -> AriaTreeProjection {
        let focused_dom_id = self.focus_id.as_deref().and_then(|wid| self.node(wid)).map(AccessibleNodeUpdate::dom_id);
        AriaTreeProjection {
            root_label: self.root_label.clone(),
            locale: self.locale.clone(),
            dir: String::from(if self.is_rtl { "rtl" } else { "ltr" }),
            active_focus_scope: self.active_focus_scope,
            focused_widget_id: self.focus_id.clone(),
            focused_dom_id,
            live_announcement: self.live_announcement.clone(),
            live_politeness: String::from(self.live_politeness.as_str()),
            nodes: self.nodes.iter().map(AccessibleNodeUpdate::to_aria_node).collect(),
        }
    }

    /// Serialize the browser [`AriaTreeProjection`] into a [`serde_json::Value`].
    #[must_use]
    pub fn to_aria_json_value(&self) -> Value {
        self.to_aria_projection().to_json_value()
    }

    /// Serialize the browser [`AriaTreeProjection`] into compact JSON text.
    ///
    /// # Errors
    /// Returns a [`serde_json::Error`] if JSON serialization fails.
    pub fn to_aria_json_string(&self) -> Result<String, serde_json::Error> {
        self.to_aria_projection().to_json_string()
    }
}

/// Serialized DOM node entry consumed by `apps/curvebook-web/index.html` hidden ARIA sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AriaNodeProjection {
    pub dom_id: String,
    pub numeric_id: u64,
    pub widget_id: String,
    pub parent_dom_id: Option<String>,
    pub children_dom_ids: Vec<String>,
    pub tag_name: String,
    pub role: String,
    pub label: String,
    pub value_text: Option<String>,
    pub keyboard_path: Option<String>,
    pub bounds_css_px: [f64; 4],
    pub attributes: BTreeMap<String, String>,
}

/// Complete browser ARIA live-tree projection emitted by `apps/curvebook-web`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AriaTreeProjection {
    pub root_label: String,
    pub locale: String,
    pub dir: String,
    pub active_focus_scope: FocusScope,
    pub focused_widget_id: Option<String>,
    pub focused_dom_id: Option<String>,
    pub live_announcement: Option<String>,
    pub live_politeness: String,
    pub nodes: Vec<AriaNodeProjection>,
}

impl AriaTreeProjection {
    /// Encode this ARIA tree projection into a [`serde_json::Value`].
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        json!(self)
    }

    /// Encode this ARIA tree projection into compact JSON text.
    ///
    /// # Errors
    /// Returns a [`serde_json::Error`] if serialization fails.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Decode an [`AriaTreeProjection`] from a [`serde_json::Value`].
    ///
    /// # Errors
    /// Returns a descriptive error string when `value` does not match [`AriaTreeProjection`].
    pub fn from_json_value(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|err| format!("invalid AriaTreeProjection JSON: {err}"))
    }

    /// Locate a projected ARIA element by CurveBook `widget_id`.
    #[must_use]
    pub fn find_node(&self, widget_id: &str) -> Option<&AriaNodeProjection> {
        self.nodes.iter().find(|n| n.widget_id == widget_id)
    }
}

/// Project every drawn node of `tree` into a deterministic [`AccessibleTreeUpdate`].
#[must_use]
pub fn project_accessible_tree(tree: &LayoutTree, focus_widget_id: Option<&str>) -> AccessibleTreeUpdate {
    let default_announcement = default_tree_announcement(tree, focus_widget_id);
    let politeness = if default_announcement.is_some() { AriaLivePoliteness::Polite } else { AriaLivePoliteness::Off };
    project_accessible_tree_with_announcement(tree, focus_widget_id, default_announcement.as_deref(), politeness)
}

/// Project every drawn node of `tree` into a deterministic [`AccessibleTreeUpdate`] with explicit live announcement.
#[must_use]
pub fn project_accessible_tree_with_announcement(
    tree: &LayoutTree,
    focus_widget_id: Option<&str>,
    announcement: Option<&str>,
    politeness: AriaLivePoliteness,
) -> AccessibleTreeUpdate {
    let drawn_nodes: Vec<&WidgetNode> = tree.nodes.iter().filter(|n| n.drawn_in_frame).collect();
    let numeric_ids = assign_unique_numeric_ids(&drawn_nodes);

    let effective_focus =
        focus_widget_id.filter(|fid| numeric_ids.contains_key(*fid)).or_else(|| drawn_nodes.iter().find(|n| n.state.focused).map(|n| n.id.as_str()));

    let mut children_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut root_children_ids = Vec::new();
    let mut root_children_numeric_ids = Vec::new();

    for node in &drawn_nodes {
        let wid = node.id.as_str();
        if let Some(pid) = node.parent.as_ref().map(|p| p.as_str()).filter(|pid| *pid != wid && numeric_ids.contains_key(*pid)) {
            children_map.entry(pid.to_owned()).or_default().push(wid.to_owned());
        } else {
            root_children_ids.push(wid.to_owned());
            if let Some(&num) = numeric_ids.get(wid) {
                root_children_numeric_ids.push(num);
            }
        }
    }

    let modal_active = tree.active_focus_scope == FocusScope::ModalDialog;
    let nodes = drawn_nodes.iter().map(|node| build_accessible_node(node, &numeric_ids, &children_map, effective_focus, modal_active)).collect();

    let clean_announcement = announcement.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let focus_numeric_id = effective_focus.and_then(|fid| numeric_ids.get(fid).copied()).unwrap_or(ACCESSIBLE_ROOT_NODE_ID);

    AccessibleTreeUpdate {
        root_id: ACCESSIBLE_ROOT_NODE_ID,
        root_label: String::from("CurveBook Workspace"),
        root_children_ids,
        root_children_numeric_ids,
        locale: tree.locale.clone(),
        is_rtl: tree.is_rtl,
        active_focus_scope: tree.active_focus_scope,
        focus_id: effective_focus.map(str::to_owned),
        focus_numeric_id,
        scale_factor: tree.metrics.clamped_scale_factor(),
        live_announcement: clean_announcement,
        live_politeness: if announcement.is_some_and(|s| !s.trim().is_empty()) { politeness } else { AriaLivePoliteness::Off },
        nodes,
    }
}

fn assign_unique_numeric_ids(drawn_nodes: &[&WidgetNode]) -> BTreeMap<String, u64> {
    let mut used: BTreeSet<u64> = BTreeSet::from([0, ACCESSIBLE_ROOT_NODE_ID]);
    let mut map = BTreeMap::new();
    for node in drawn_nodes {
        let mut candidate = deterministic_node_id(node.id.as_str());
        while !used.insert(candidate) {
            candidate = candidate.saturating_add(1).max(2);
        }
        map.insert(node.id.0.clone(), candidate);
    }
    map
}

fn build_accessible_node(
    node: &WidgetNode,
    numeric_ids: &BTreeMap<String, u64>,
    children_map: &BTreeMap<String, Vec<String>>,
    effective_focus: Option<&str>,
    modal_active: bool,
) -> AccessibleNodeUpdate {
    let wid = node.id.as_str();
    let numeric_id = numeric_ids.get(wid).copied().unwrap_or_else(|| deterministic_node_id(wid));
    let resolved_parent = node.parent.as_ref().map(|p| p.as_str()).filter(|pid| *pid != wid && numeric_ids.contains_key(*pid));
    let parent_numeric_id = resolved_parent.and_then(|pid| numeric_ids.get(pid).copied());
    let children_ids = children_map.get(wid).cloned().unwrap_or_default();
    let children_numeric_ids = children_ids.iter().filter_map(|cid| numeric_ids.get(cid).copied()).collect();

    let role_mapping = AriaRoleMapping::for_node(node);
    let focused = effective_focus == Some(wid) || node.state.focused;
    let hidden = modal_active && node.z_layer < ZLayer::ModalBackdrop;
    let modal = node.a11y.role == WidgetRole::Dialog && node.z_layer == ZLayer::ModalDialog && wid == "modal.dialog";
    let (live_region, announcement) = node_live_region(node);

    AccessibleNodeUpdate {
        numeric_id,
        widget_id: node.id.0.clone(),
        parent_id: resolved_parent.map(str::to_owned),
        parent_numeric_id,
        children_ids,
        children_numeric_ids,
        role: node.a11y.role,
        role_mapping,
        label: node.a11y.label.clone(),
        value_text: node.a11y.value_text.clone(),
        bounds_logical: AccessibleBounds::from_rect_logical(node.bounds.rect_logical),
        bounds_physical: AccessiblePhysicalBounds {
            x: node.bounds.visible_rect_px.x,
            y: node.bounds.visible_rect_px.y,
            w: node.bounds.visible_rect_px.width,
            h: node.bounds.visible_rect_px.height,
        },
        z_layer: node.z_layer,
        focus_scope: node.a11y.focus_scope,
        tab_order: node.a11y.tab_order,
        enabled: node.state.enablement.enabled,
        disabled_reason: node.state.enablement.disabled_reason.clone(),
        selected: node.a11y.selected,
        checked: node.a11y.checked,
        expanded: node.a11y.expanded,
        focused,
        hidden,
        modal,
        keyboard_path: node.a11y.keyboard_path.clone(),
        shortcut: node.a11y.shortcut.clone(),
        live_region,
        announcement,
        actions: derive_node_actions(node),
    }
}

fn node_live_region(node: &WidgetNode) -> (Option<AriaLivePoliteness>, Option<String>) {
    let wid = node.id.as_str();
    if wid == "status.summary" || wid == "overlay.tooltip" || wid == "result.diagnostic_banner" || node.a11y.role == WidgetRole::Banner {
        let msg = if node.a11y.label.trim().is_empty() { None } else { Some(node.a11y.label.clone()) };
        return (Some(AriaLivePoliteness::Polite), msg);
    }
    (None, None)
}

fn derive_node_actions(node: &WidgetNode) -> Vec<AccessibleActionTag> {
    if !node.state.enablement.enabled {
        return Vec::new();
    }
    let mut actions = Vec::new();
    if is_tab_focusable_role(node.a11y.role) || matches!(node.action, WidgetAction::FocusWidget(_)) {
        actions.push(AccessibleActionTag::Focus);
    }
    if node.action != WidgetAction::NoOp && !matches!(node.action, WidgetAction::FocusWidget(_)) {
        actions.push(AccessibleActionTag::Click);
    }
    if matches!(node.a11y.role, WidgetRole::Disclosure | WidgetRole::ComboBox)
        || (node.a11y.role == WidgetRole::TreeItem && node.a11y.expanded.is_some())
    {
        match node.a11y.expanded {
            Some(true) => actions.push(AccessibleActionTag::Collapse),
            Some(false) => actions.push(AccessibleActionTag::Expand),
            None => {
                actions.push(AccessibleActionTag::Expand);
                actions.push(AccessibleActionTag::Collapse);
            }
        }
    }
    if matches!(node.a11y.role, WidgetRole::SpinButton | WidgetRole::Slider) {
        actions.push(AccessibleActionTag::Increment);
        actions.push(AccessibleActionTag::Decrement);
    }
    if matches!(node.a11y.role, WidgetRole::TextBox | WidgetRole::SpinButton | WidgetRole::Slider) {
        actions.push(AccessibleActionTag::SetValue);
    }
    if node.a11y.role == WidgetRole::ScrollBar || node.scroll.is_some() {
        actions.push(AccessibleActionTag::Scroll);
    }
    actions
}

fn default_tree_announcement(tree: &LayoutTree, focus_widget_id: Option<&str>) -> Option<String> {
    if let Some(tip) = tree.node("overlay.tooltip").filter(|n| n.drawn_in_frame && !n.a11y.label.trim().is_empty()) {
        return Some(tip.a11y.label.clone());
    }
    if let Some(fid) = focus_widget_id
        && let Some(node) = tree.node(fid).filter(|n| n.drawn_in_frame)
    {
        return Some(format!("{}: {}", node.a11y.role.as_str(), node.a11y.label));
    }
    tree.node("status.summary").filter(|n| n.drawn_in_frame && !n.a11y.label.trim().is_empty()).map(|n| n.a11y.label.clone())
}

/// Single failed invariant emitted by [`validate_layout_tree_accessibility`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityViolation {
    pub widget_id: String,
    pub check: String,
    pub detail: String,
}

/// Summary report emitted by [`audit_layout_tree_accessibility`] and [`validate_layout_tree_accessibility`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AccessibilityGateReport {
    pub checked_interactive_nodes: usize,
    pub focusable_nodes_in_active_scope: usize,
    pub violations: Vec<AccessibilityViolation>,
}

impl AccessibilityGateReport {
    /// Returns `true` when zero accessibility gate violations were found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Error returned by [`validate_layout_tree_accessibility`] when any accessibility gate invariant fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityGateError {
    pub report: AccessibilityGateReport,
}

impl fmt::Display for AccessibilityGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R10.20 accessibility gate failed with {} violation(s):", self.report.violations.len())?;
        for v in &self.report.violations {
            write!(f, " [{} / {}] {}", v.widget_id, v.check, v.detail)?;
        }
        Ok(())
    }
}

impl Error for AccessibilityGateError {}

impl From<AccessibilityGateError> for String {
    fn from(err: AccessibilityGateError) -> Self {
        err.to_string()
    }
}

/// Run the `R10.20` CI Accessibility Gate on `tree` and return the full structured [`AccessibilityGateReport`].
#[must_use]
pub fn audit_layout_tree_accessibility(tree: &LayoutTree) -> AccessibilityGateReport {
    let mut violations = Vec::new();
    let mut checked_interactive_nodes = 0usize;
    let mut seen_ids = BTreeSet::new();

    for node in &tree.nodes {
        if !seen_ids.insert(node.id.0.as_str()) {
            violations.push(AccessibilityViolation {
                widget_id: node.id.0.clone(),
                check: String::from("unique_widget_id"),
                detail: String::from("duplicate WidgetId detected in LayoutTree"),
            });
        }
        check_node_accessibility(tree, node, &mut checked_interactive_nodes, &mut violations);
    }

    let active_targets = ordered_focus_targets(&tree.nodes, tree.active_focus_scope);
    let focusable_nodes_in_active_scope = active_targets.len();
    let active_target_ids: BTreeSet<&str> = active_targets.iter().map(|n| n.id.as_str()).collect();

    check_active_scope_reachability(tree, &active_target_ids, &mut violations);

    AccessibilityGateReport { checked_interactive_nodes, focusable_nodes_in_active_scope, violations }
}

/// Enforce the `R10.20` CI Accessibility Gate on `tree`.
///
/// Verifies every interactive widget (`node.action != WidgetAction::NoOp && node.drawn_in_frame`)
/// exposes:
/// 1. a semantic interactive [`WidgetRole`] (mapped cleanly to WAI-ARIA & AccessKit),
/// 2. a non-empty accessible `label`, and
/// 3. a non-empty `keyboard_path` + reachable [`FocusScope`],
///
/// failing cleanly on any phantom, mis-scoped, or unlabelled interactive control.
///
/// # Errors
/// Returns [`AccessibilityGateError`] listing every failing control and check.
pub fn validate_layout_tree_accessibility(tree: &LayoutTree) -> Result<AccessibilityGateReport, AccessibilityGateError> {
    let report = audit_layout_tree_accessibility(tree);
    if report.is_clean() { Ok(report) } else { Err(AccessibilityGateError { report }) }
}

fn check_node_accessibility(
    tree: &LayoutTree,
    node: &WidgetNode,
    checked_interactive_nodes: &mut usize,
    violations: &mut Vec<AccessibilityViolation>,
) {
    let is_action_interactive = node.action != WidgetAction::NoOp;
    let is_role_interactive = node.a11y.role.is_interactive() && node.z_layer != ZLayer::ModalBackdrop;
    if !is_action_interactive && !is_role_interactive {
        return;
    }

    if !node.drawn_in_frame || node.bounds.visible_rect_px.width == 0 || node.bounds.visible_rect_px.height == 0 {
        violations.push(AccessibilityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("phantom_control"),
            detail: format!(
                "interactive widget is not visibly drawn (drawn_in_frame={}, visible={}x{})",
                node.drawn_in_frame, node.bounds.visible_rect_px.width, node.bounds.visible_rect_px.height
            ),
        });
        return;
    }

    if is_action_interactive {
        *checked_interactive_nodes += 1;
        if !node.a11y.role.is_interactive() {
            violations.push(AccessibilityViolation {
                widget_id: node.id.0.clone(),
                check: String::from("semantic_role"),
                detail: format!("interactive action {:?} cannot use passive role `{}`", node.action, node.a11y.role.as_str()),
            });
        }
    }

    let role_map = AriaRoleMapping::for_node(node);
    if role_map.aria_role.is_empty() || role_map.accesskit_role.is_empty() {
        violations.push(AccessibilityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("role_mapping"),
            detail: format!("WidgetRole `{}` has empty ARIA or AccessKit mapping", node.a11y.role.as_str()),
        });
    }

    if node.a11y.label.trim().is_empty() {
        violations.push(AccessibilityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("missing_label"),
            detail: format!("interactive `{}` widget requires a non-empty accessible label", node.a11y.role.as_str()),
        });
    }

    if node.a11y.keyboard_path.as_deref().is_none_or(|p| p.trim().is_empty()) {
        violations.push(AccessibilityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("missing_keyboard_path"),
            detail: format!("interactive `{}` widget `{}` requires a non-empty keyboard_path", node.a11y.role.as_str(), node.a11y.label),
        });
    }

    check_node_focus_scope_consistency(tree, node, violations);
}

fn check_node_focus_scope_consistency(tree: &LayoutTree, node: &WidgetNode, violations: &mut Vec<AccessibilityViolation>) {
    if node.a11y.role == WidgetRole::ScrollBar {
        return;
    }
    let scope_valid = match node.z_layer {
        ZLayer::Workspace | ZLayer::FloatingOverlay | ZLayer::SplitHandle => node.a11y.focus_scope == FocusScope::Workspace,
        ZLayer::ModalDialog | ZLayer::ModalDropdown => node.a11y.focus_scope == FocusScope::ModalDialog,
        ZLayer::DropdownMenu => matches!(node.a11y.focus_scope, FocusScope::PopupMenu | FocusScope::ModalDialog),
        ZLayer::ModalBackdrop | ZLayer::ContextMenu | ZLayer::Tooltip => true,
    };
    if !scope_valid {
        violations.push(AccessibilityViolation {
            widget_id: node.id.0.clone(),
            check: String::from("unreachable_focus_scope"),
            detail: format!("widget on {:?} has incompatible focus_scope {:?}", node.z_layer, node.a11y.focus_scope),
        });
    }
    let _ = tree;
}

fn check_active_scope_reachability(tree: &LayoutTree, active_target_ids: &BTreeSet<&str>, violations: &mut Vec<AccessibilityViolation>) {
    for node in &tree.nodes {
        if !node.drawn_in_frame
            || node.action == WidgetAction::NoOp
            || !node.state.enablement.enabled
            || node.a11y.focus_scope != tree.active_focus_scope
            || !is_tab_focusable_role(node.a11y.role)
        {
            continue;
        }
        if !active_target_ids.contains(node.id.as_str()) {
            violations.push(AccessibilityViolation {
                widget_id: node.id.0.clone(),
                check: String::from("unreachable_focus_target"),
                detail: format!("enabled focusable widget is absent from ordered_focus_targets({:?})", tree.active_focus_scope),
            });
        }
    }

    let has_tab_candidates = match tree.active_focus_scope {
        FocusScope::ModalDialog => tree
            .nodes
            .iter()
            .any(|n| n.drawn_in_frame && n.z_layer >= ZLayer::ModalDialog && n.state.enablement.enabled && is_tab_focusable_role(n.a11y.role)),
        FocusScope::PopupMenu => tree.nodes.iter().any(|n| {
            n.drawn_in_frame
                && n.z_layer == ZLayer::DropdownMenu
                && n.a11y.focus_scope == FocusScope::PopupMenu
                && n.state.enablement.enabled
                && is_tab_focusable_role(n.a11y.role)
        }),
        FocusScope::Workspace => tree.nodes.iter().any(|n| {
            n.drawn_in_frame
                && n.z_layer < ZLayer::DropdownMenu
                && n.state.enablement.enabled
                && n.action != WidgetAction::NoOp
                && is_tab_focusable_role(n.a11y.role)
        }),
    };
    if has_tab_candidates && active_target_ids.is_empty() {
        violations.push(AccessibilityViolation {
            widget_id: format!("scope.{:?}", tree.active_focus_scope),
            check: String::from("empty_active_focus_scope"),
            detail: format!("active focus scope {:?} has zero reachable keyboard focus targets", tree.active_focus_scope),
        });
    }
}
