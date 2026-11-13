//! UI-only commands handled by the front-end shell (`lumencraft-ui-egui`) rather than the headless
//! document engine. Mirrored to `crates/mcp/src/ui_catalog.rs` by `cargo xtask ui-commands`.

/// `(id, label, shortcut, params doc)` for front-end UI commands.
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("ui.navigator.toggle", "Toggle Project Navigator", "Cmd+Shift+N", "{}"),
    ("ui.inspector.toggle", "Toggle Format Inspector", "Cmd+Shift+I", "{}"),
    ("ui.zoom.fitSheet", "Zoom to Fit Sheet", "Cmd+0", "{}"),
    ("app.about", "About LumenCraft", "", "{}"),
];
