//! View-level accessibility metadata and `R10.20` verification helpers.

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};

/// Transient view state owned by the accessibility shard.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct A11yViewState {}

/// Dispatch custom `WidgetAction::Custom` events targeting the `a11y` domain.
///
/// # Errors
/// Never returns an error in the Wave 5 scaffold; reserved for future accessibility actions.
pub fn handle_custom_action(_app: &mut UiApp, _state: &mut A11yViewState, _payload: &str) -> Result<UiEventOutcome, String> {
    Ok(UiEventOutcome::default())
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
/// Returns a descriptive diagnostic when an interactive role lacks a non-empty `label` or `keyboard_path`.
pub fn validate_interactive_a11y(meta: &AccessibilityMeta) -> Result<(), String> {
    if !meta.role.is_interactive() {
        return Ok(());
    }
    if meta.label.trim().is_empty() {
        return Err(format!("interactive `{}` widget requires a non-empty accessibility label", meta.role.as_str()));
    }
    if meta.keyboard_path.as_deref().is_none_or(|p| p.trim().is_empty()) {
        return Err(format!("interactive `{}` widget `{}` requires a non-empty keyboard_path", meta.role.as_str(), meta.label));
    }
    Ok(())
}
