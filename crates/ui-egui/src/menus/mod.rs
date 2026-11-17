//! Menu definitions, customizable shortcuts, and desktop UI command metadata.

pub mod shortcuts;
pub mod tooltips;
pub mod ui_commands;

pub use shortcuts::{ShortcutBinding, ShortcutExecution, ShortcutMatch, ShortcutRegistry, ShortcutScope, normalize_shortcut_combo};
pub use tooltips::{
    CommandTooltip, MenuSectionTooltips, TOOLBAR_BUTTONS, ToolbarButtonSpec, ToolbarButtonTooltip, command_tooltip, menu_bar_tooltips,
    toolbar_caption_command_id, toolbar_tooltips,
};
pub use ui_commands::{STAGED_UI_COMMANDS, UI_COMMANDS, UiCommandSpec};
