//! Import, Paste Special, missing-value, and table conversion dialog view builders (`R10.12`).

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::{LayoutBuilder, LayoutRectPx, WidgetId};

use super::dialogs_graph_misc::build_descriptor_modal_body;

/// Transient view state owned by the import / paste / format dialog shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImportPasteViewState {
    pub active_tab: String,
    pub filter_pattern: String,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `import_paste` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ImportPasteViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the `ImportReshape` / `ConvertArchetype` modal body inside `body_rect`.
pub fn build_import_or_convert_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    build_descriptor_modal_body(builder, parent, body_rect);
}
