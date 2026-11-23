//! Accessibility view shard state carried by [`crate::views::ViewShardStates`].

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;

/// ARIA live-region politeness mode attached to accessibility announcements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AriaLivePoliteness {
    #[default]
    Off,
    Polite,
    Assertive,
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

/// Dispatch custom UI actions targeting the `a11y` domain.
pub fn handle_custom_action(_app: &mut UiApp, _state: &mut A11yViewState, _payload: &str) -> Result<UiEventOutcome, String> {
    Ok(UiEventOutcome::default())
}
