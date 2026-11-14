//! Menu definitions, customizable shortcuts, and desktop UI command metadata.

pub mod shortcuts;
pub mod ui_commands;

pub use shortcuts::{ShortcutBinding, ShortcutExecution, ShortcutMatch, ShortcutRegistry, normalize_shortcut_combo};
pub use ui_commands::UI_COMMANDS;
