//! Thin desktop and browser UI shell for LumenCraft: project navigator, typed data/results
//! spreadsheet grid, statistical parameter dialogs, graph/layout canvas, menus, localization and
//! local JSON-lines control channel.
//!
//! Layer L6 front-end crate: panels inspect engine state and mutate sessions solely by dispatching
//! commands. Governing spec: `docs/ui-parity.md`, `docs/control-protocol.md`, and `docs/development.md`.
#![forbid(unsafe_code)]

pub mod i18n;
pub mod menus;

/// Whether the UI crate was compiled with bundled interface/document fonts enabled.
#[must_use]
pub const fn bundled_fonts_enabled() -> bool {
    cfg!(feature = "bundled-fonts")
}

#[cfg(test)]
mod tests {
    use super::menus::ui_commands::UI_COMMANDS;

    #[test]
    fn ui_commands_table_is_populated() {
        assert!(!UI_COMMANDS.is_empty());
    }
}
