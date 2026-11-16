//! Accessibility metadata, focus scopes, and modal `Tab` / `Shift+Tab` focus trapping (`R10.16`, `R10.20`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::WidgetNode;

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

/// Ordered list of enabled, focusable widget IDs within `scope` (trapping `Tab` inside active modal/popup scopes).
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
                && n.a11y.tab_order.is_some()
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

const fn is_tab_focusable_role(role: WidgetRole) -> bool {
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
