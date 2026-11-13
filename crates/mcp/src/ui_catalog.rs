//! Desktop UI command catalogue for headless MCP discovery (`ui: true`).
//! Generated from `crates/ui-egui/src/menus/ui_commands.rs` by `cargo xtask ui-commands`.
//! Do not edit by hand; re-run the xtask when that table changes.

/// `(id, label, shortcut, params doc)` — same shape as the desktop `UI_COMMANDS` table.
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("ui.navigator.toggle", "Toggle Project Navigator", "Cmd+Shift+N", "{}"),
    ("ui.inspector.toggle", "Toggle Format Inspector", "Cmd+Shift+I", "{}"),
    ("ui.zoom.fitSheet", "Zoom to Fit Sheet", "Cmd+0", "{}"),
    ("app.about", "About LumenCraft", "", "{}"),
];
