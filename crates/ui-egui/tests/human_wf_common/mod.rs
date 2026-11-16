#![allow(dead_code)]
//! Shared zero-unwrap human-path workflow test helpers (`R10.2`, `R10.13`, `R10.20`).
//!
//! All helpers drive [`HumanPathDriver`] strictly through hit-tested widget centers `(x, y)`,
//! physical pointer events, focus-preserving typing, key combos, and wheel scrolling.

use curvebook_doc::ErrorCode;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::dispatch_gui_action;
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, UiEventOutcome, ViewportMetrics};
use curvebook_ui_egui::layout::hit_test::hit_test_logical;
use curvebook_ui_egui::layout::verify::ParityReport;
use curvebook_ui_egui::layout::{LayoutTree, WidgetId, WidgetNode};
use serde_json::json;

/// Default desktop viewport (`1280x800 @ 1.0x`) for human-path workflow tests.
#[must_use]
pub fn default_viewport() -> ViewportMetrics {
    ViewportMetrics::new(1280.0, 800.0, 1.0)
}

/// Construct a [`HumanPathDriver`] with strict `gui_only` and `human_path_only` enabled.
#[must_use]
pub fn new_human_driver(app: UiApp, metrics: ViewportMetrics) -> HumanPathDriver {
    HumanPathDriver::new(app, metrics)
}

fn is_descendant_of(tree: &LayoutTree, candidate: &WidgetNode, ancestor: &WidgetId) -> bool {
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

/// Resolve the logical center `(x, y)` of `widget_id` and verify [`hit_test_logical`] routes to it.
///
/// # Errors
/// Returns an error if the widget is missing, undrawn, disabled, zero-sized, or occluded at its center.
pub fn widget_hit_center(driver: &mut HumanPathDriver, widget_id: &WidgetId) -> Result<[f64; 2], String> {
    let tree = driver.layout_tree();
    let id_str = widget_id.as_str();
    let node = tree.node(id_str).ok_or_else(|| format!("widget `{id_str}` not found in LayoutTree"))?;
    if !node.drawn_in_frame {
        return Err(format!("widget `{id_str}` is not drawn_in_frame"));
    }
    if node.bounds.visible_rect_px.width == 0 || node.bounds.visible_rect_px.height == 0 {
        return Err(format!("widget `{id_str}` has zero visible bounds"));
    }
    if !node.state.enablement.enabled {
        let reason = node.state.enablement.disabled_reason.as_deref().unwrap_or("unavailable");
        return Err(format!("widget `{id_str}` is disabled: {reason}"));
    }
    let [cx, cy] = node.bounds.visible_rect_px.center();
    let [lx, ly] = driver.metrics().physical_to_logical_px([f64::from(cx), f64::from(cy)]);
    let hit = hit_test_logical(&tree, lx, ly).ok_or_else(|| format!("hit_test_logical({lx:.1}, {ly:.1}) for `{id_str}` returned None"))?;
    if hit.id != *widget_id && !is_descendant_of(&tree, hit, widget_id) {
        return Err(format!("hit_test_logical({lx:.1}, {ly:.1}) for `{id_str}` hit `{}` instead", hit.id.as_str()));
    }
    Ok([lx, ly])
}

/// Click `widget_id` strictly at its hit-tested logical center `(x, y)` via `PointerMove` + `PointerDown` + `PointerUp`.
///
/// # Errors
/// Returns an error if hit-testing fails or pointer dispatch fails.
pub fn click_widget_center(driver: &mut HumanPathDriver, widget_id: &WidgetId) -> Result<UiEventOutcome, String> {
    let [lx, ly] = widget_hit_center(driver, widget_id)?;
    driver.click_logical(lx, ly)
}

/// Move pointer to the hit-tested center `(x, y)` of `widget_id` to trigger hover state & tooltips.
///
/// # Errors
/// Returns an error if hit-testing fails or pointer move dispatch fails.
pub fn hover_widget_center(driver: &mut HumanPathDriver, widget_id: &WidgetId) -> Result<UiEventOutcome, String> {
    let [lx, ly] = widget_hit_center(driver, widget_id)?;
    driver.move_pointer(lx, ly)
}

/// Type `text` into the currently focused widget via `TextInput` (requires an already-focused widget).
///
/// # Errors
/// Returns an error if no widget currently holds focus or text dispatch fails.
pub fn type_into_focused(driver: &mut HumanPathDriver, text: &str) -> Result<UiEventOutcome, String> {
    if driver.app().focus_widget_id.as_deref().is_none_or(str::is_empty) {
        return Err(String::from("type_into_focused requires an already-focused widget"));
    }
    driver.type_text(text)
}

/// Click `widget_id` at its hit-tested center `(x, y)` to focus it, then type `text` via `type_text`.
///
/// # Errors
/// Returns an error if clicking or typing fails.
pub fn click_and_type(driver: &mut HumanPathDriver, widget_id: &WidgetId, text: &str) -> Result<UiEventOutcome, String> {
    let _ = click_widget_center(driver, widget_id)?;
    driver.type_text(text)
}

/// Dispatch a raw key press with `modifiers` through the unified keyboard handler.
///
/// # Errors
/// Returns an error if keyboard dispatch fails.
pub fn press_key_combo(driver: &mut HumanPathDriver, key: &str, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
    driver.press_key(key, modifiers)
}

/// Dispatch a `Cmd+<key>` (`meta: true`) keyboard shortcut through the unified keyboard handler.
///
/// # Errors
/// Returns an error if keyboard dispatch fails.
pub fn press_cmd_shortcut(driver: &mut HumanPathDriver, key: &str) -> Result<UiEventOutcome, String> {
    driver.press_key(key, KeyModifiers { meta: true, ..Default::default() })
}

/// Send a scroll wheel event over `widget_id`'s hit-tested center `(x, y)`.
///
/// # Errors
/// Returns an error if `widget_id` is not visibly drawn or wheel dispatch fails.
pub fn scroll_widget_wheel(driver: &mut HumanPathDriver, widget_id: &WidgetId, delta_x: f64, delta_y: f64) -> Result<UiEventOutcome, String> {
    let [lx, ly] = widget_hit_center(driver, widget_id)?;
    driver.wheel_logical(lx, ly, delta_x, delta_y, KeyModifiers::default())
}

/// Scroll `container_id` with `step_y` wheel increments until `target_id` is visibly drawn in `LayoutTree`.
///
/// # Errors
/// Returns an error if `target_id` does not become visible within `max_steps` scroll events.
pub fn scroll_until_widget_visible(
    driver: &mut HumanPathDriver,
    container_id: &WidgetId,
    target_id: &WidgetId,
    step_y: f64,
    max_steps: usize,
) -> Result<(), String> {
    for _ in 0..=max_steps {
        let tree = driver.layout_tree();
        if let Some(node) = tree.node(target_id.as_str())
            && node.drawn_in_frame
            && node.bounds.visible_rect_px.width > 0
            && node.bounds.visible_rect_px.height > 0
        {
            return Ok(());
        }
        let _ = scroll_widget_wheel(driver, container_id, 0.0, step_y)?;
    }
    Err(format!("widget `{}` did not become visible inside `{}` after {max_steps} scroll steps", target_id.as_str(), container_id.as_str()))
}

fn assert_bypass_blocked(label: &str, res: Result<serde_json::Value, curvebook_engine::EngineError>) -> Result<(), String> {
    match res {
        Ok(val) => Err(format!("zero-shortcut compliance failed: `{label}` unexpectedly succeeded with {val}")),
        Err(err) if err.code == ErrorCode::GuiOnlyViolation => Ok(()),
        Err(err) => Err(format!("zero-shortcut compliance failed: `{label}` returned {:?} instead of GuiOnlyViolation", err.code)),
    }
}

/// Verify strict `gui_only` + `human_path_only` guard enforcement (zero semantic bypasses) and
/// run `verify_pixel_to_hit_box_parity` on the active frame.
///
/// # Errors
/// Returns an error if strict guards are disabled, any synthetic bypass succeeds, or parity violations exist.
pub fn assert_zero_shortcut_compliance(driver: &mut HumanPathDriver) -> Result<ParityReport, String> {
    if !driver.app().is_gui_only() {
        return Err(String::from("zero-shortcut compliance failed: UiApp::is_gui_only() is false"));
    }
    if !driver.app().is_human_path_only() {
        return Err(String::from("zero-shortcut compliance failed: UiApp::is_human_path_only() is false"));
    }

    let mut probe = driver.app().clone();
    probe.canvas_dialogs.active_dialog = ActiveDialog::About;
    assert_bypass_blocked(
        "ui.invoke_menu(params)",
        dispatch_gui_action(&mut probe, "ui.invoke_menu", json!({ "command_id": "sheet.createData", "params": { "kind": "column" } })),
    )?;
    assert_bypass_blocked("ui.answer_dialog", dispatch_gui_action(&mut probe, "ui.answer_dialog", json!({ "action": "ok", "fields": {} })))?;
    assert_bypass_blocked(
        "ui.pointer_gesture(widget_id)",
        dispatch_gui_action(&mut probe, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" })),
    )?;
    probe.focus_widget_id = Some(String::from("probe.focus.a"));
    assert_bypass_blocked(
        "ui.type_text(focus_jump)",
        dispatch_gui_action(&mut probe, "ui.type_text", json!({ "widget_id": "probe.focus.b", "text": "x" })),
    )?;

    let report = driver.verify_parity();
    if !report.is_clean() {
        return Err(format!("pixel-to-hit-box parity violations detected: {:?}", report.violations));
    }
    Ok(report)
}

/// Assert that recorded `max_input_to_paint_ms` stays within `budget_ms`.
///
/// # Errors
/// Returns an error when the measured worst-case input-to-paint latency exceeds `budget_ms`.
pub fn assert_input_to_paint_budget(driver: &HumanPathDriver, budget_ms: f64) -> Result<(), String> {
    let max_ms = driver.max_input_to_paint_ms();
    if max_ms > budget_ms {
        return Err(format!("input-to-paint latency budget exceeded: {max_ms:.2} ms > {budget_ms:.2} ms"));
    }
    Ok(())
}
