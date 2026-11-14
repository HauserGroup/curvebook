//! Desktop UI command catalogue for headless MCP discovery (`ui: true`).
//! Generated from `crates/ui-egui/src/menus/ui_commands.rs` by `cargo xtask ui-commands`.
//! Do not edit by hand; re-run the xtask when that table changes.

/// `(id, label, shortcut, params doc)` — same shape as the desktop `UI_COMMANDS` table.
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("ui.navigator.toggle", "Toggle Project Navigator", "Cmd+Shift+N", "{}"),
    ("ui.inspector.toggle", "Toggle Format Inspector", "Cmd+Shift+I", "{}"),
    ("ui.family.toggle", "Toggle Family Strip", "Cmd+Shift+F", "{}"),
    ("ui.palette.open", "Open Command Palette", "Cmd+K", "{}"),
    ("ui.analyze.open", "Open Analyze Dialog", "Cmd+Shift+A", "{\"sheet_id\":\"optional SheetId\"}"),
    ("ui.changeGraphType.open", "Open Change Graph Type Dialog", "Cmd+Shift+G", "{\"sheet_id\":\"optional GraphSheetId\"}"),
    ("ui.clonePipeline.open", "Open Clone Analysis Pipeline Wizard", "", "{\"sheet_id\":\"optional SheetId\"}"),
    ("ui.zoom.fitSheet", "Zoom to Fit Sheet", "Cmd+0", "{}"),
    ("ui.gallery.toggle", "Toggle Portfolio Gallery", "", "{\"folder\":\"optional data|info|results|graphs|layouts\"}"),
    ("ui.rulers.toggle", "Toggle Canvas Physical Rulers", "", "{\"unit\":\"optional pt|in|mm\"}"),
    ("ui.pane.cycle", "Cycle Active Workspace Pane", "F6", "{\"reverse\":false}"),
    ("ui.shortcut.set", "Customize Keyboard Shortcut", "", "{\"command_id\":\"string\",\"shortcut\":\"string\"}"),
    ("ui.shortcut.reset", "Reset Keyboard Shortcut", "", "{\"command_id\":\"optional string\"}"),
    ("ui.locale.set", "Set UI Locale", "", "{\"locale\":\"en|de|es|fr|ja|ko|pt-BR|zh-CN|zh-TW|it|nl|pl|ru|tr|ar|he\"}"),
    ("ui.theme.set", "Set UI Theme Mode", "", "{\"mode\":\"light|dark|high_contrast\"}"),
    ("ui.screenshot", "Capture Workspace Screenshot PNG", "", "{\"width_px\":1024,\"height_px\":640,\"dpi\":144,\"path\":\"optional.png\"}"),
    ("app.about", "About LumenCraft", "", "{}"),
];
