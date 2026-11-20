//! View-level accessibility metadata, live-region focus announcements, and high-contrast focus
//! ring styling shared with `egui_shell`.

use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::inspect::inspect_ui_tree;
use crate::interaction::UiEventOutcome;
use crate::theme::{UiThemeMode, UiThemeSpec};

/// ARIA live-region politeness mode attached to accessibility announcements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AriaLivePoliteness {
    #[default]
    Off,
    Polite,
    Assertive,
}

impl AriaLivePoliteness {
    /// Canonical lowercase ARIA live-region token (`"off"`, `"polite"`, or `"assertive"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Polite => "polite",
            Self::Assertive => "assertive",
        }
    }
}

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

/// Dispatch custom UI actions targeting the `a11y` domain.
///
/// # Errors
/// Returns an error string when an explicit `announce:` command supplies empty text or an unknown widget ID.
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
        "clear_announcement" => clear_announcement(state),
        "announce_focus" => announce_widget_focus(app, state, None),
        "toggle_high_contrast_focus" => set_high_contrast_focus(state, !state.high_contrast_focus_ring),
        "high_contrast_focus:on" => set_high_contrast_focus(state, true),
        "high_contrast_focus:off" => set_high_contrast_focus(state, false),
        _ => Ok(UiEventOutcome::default()),
    }
}

fn handle_json_a11y_action(app: &UiApp, state: &mut A11yViewState, val: &Value) -> Result<UiEventOutcome, String> {
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
        "clear_announcement" => clear_announcement(state),
        "announce_focus" => {
            let wid = val.get("widget_id").and_then(Value::as_str);
            announce_widget_focus(app, state, wid)
        }
        "set_high_contrast_focus" => {
            let enabled = val.get("enabled").and_then(Value::as_bool).unwrap_or(!state.high_contrast_focus_ring);
            set_high_contrast_focus(state, enabled)
        }
        _ => Ok(UiEventOutcome::default()),
    }
}

fn clear_announcement(state: &mut A11yViewState) -> Result<UiEventOutcome, String> {
    state.last_announcement = None;
    state.live_politeness = AriaLivePoliteness::Off;
    Ok(UiEventOutcome { handled: true, action: String::from("a11y.clear_announcement"), ..Default::default() })
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
    let snap = inspect_ui_tree(app, None, false);
    let Some(node) = snap.find_widget(fid) else {
        return Err(format!("focused widget `{fid}` not present in UI snapshot"));
    };
    let announcement = if node.label.is_empty() { format!("{fid} ({})", node.role) } else { format!("{} ({})", node.label, node.role) };
    state.last_focused_widget_id = Some(fid.to_owned());
    state.last_announcement = Some(announcement.clone());
    state.live_politeness = AriaLivePoliteness::Polite;
    state.announcement_seq = state.announcement_seq.saturating_add(1);
    Ok(UiEventOutcome {
        handled: true,
        action: format!("a11y.announce_focus:{fid}"),
        command_result: Some(json!({
            "widget_id": fid,
            "announcement": announcement,
            "politeness": state.live_politeness.as_str(),
        })),
        ..Default::default()
    })
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::UiApp;

    use super::{A11yViewState, AriaLivePoliteness, handle_custom_action};

    #[test]
    fn clear_announcement_and_high_contrast_toggle_match_across_text_and_json_spellings() {
        let mut app = UiApp::new();
        let mut state = A11yViewState::default();

        let _ = handle_custom_action(&mut app, &mut state, "announce:Saved").unwrap();
        assert_eq!(state.last_announcement.as_deref(), Some("Saved"));
        assert_eq!(state.live_politeness, AriaLivePoliteness::Polite);

        let cleared_text = handle_custom_action(&mut app, &mut state, "clear_announcement").unwrap();
        assert!(cleared_text.handled);
        assert_eq!(cleared_text.action, "a11y.clear_announcement");
        assert_eq!(state.last_announcement, None);
        assert_eq!(state.live_politeness, AriaLivePoliteness::Off);

        let _ = handle_custom_action(&mut app, &mut state, r#"{"op":"announce","text":"Alert","assertive":true}"#).unwrap();
        assert_eq!(state.live_politeness, AriaLivePoliteness::Assertive);
        let cleared_json = handle_custom_action(&mut app, &mut state, r#"{"op":"clear_announcement"}"#).unwrap();
        assert_eq!(cleared_json, cleared_text);
        assert_eq!(state.last_announcement, None);
        assert_eq!(state.live_politeness, AriaLivePoliteness::Off);

        let toggled_on = handle_custom_action(&mut app, &mut state, "toggle_high_contrast_focus").unwrap();
        assert!(state.high_contrast_focus_ring);
        assert_eq!(toggled_on.action, "a11y.high_contrast_focus");
        assert_eq!(toggled_on.command_result, Some(json!({ "high_contrast_focus_ring": true })));

        let toggled_off = handle_custom_action(&mut app, &mut state, r#"{"op":"set_high_contrast_focus"}"#).unwrap();
        assert!(!state.high_contrast_focus_ring);
        assert_eq!(toggled_off.action, "a11y.high_contrast_focus");
        assert_eq!(toggled_off.command_result, Some(json!({ "high_contrast_focus_ring": false })));
    }
}
