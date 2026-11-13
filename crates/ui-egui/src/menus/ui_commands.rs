//! UI-only commands handled by the front-end shell (`lumencraft-ui-egui`) rather than the headless
//! document engine. Mirrored to `crates/mcp/src/ui_catalog.rs` by `cargo xtask ui-commands`.

/// `(id, label, shortcut, params doc)` for front-end UI commands.
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("ui.navigator.toggle", "Toggle Project Navigator", "Cmd+Shift+N", "{}"),
    ("ui.inspector.toggle", "Toggle Format Inspector", "Cmd+Shift+I", "{}"),
    ("ui.family.toggle", "Toggle Family Strip", "Cmd+Shift+F", "{}"),
    ("ui.palette.open", "Open Command Palette", "Cmd+K", "{}"),
    ("ui.analyze.open", "Open Analyze Dialog", "Cmd+Shift+A", "{\"sheet_id\":\"optional SheetId\"}"),
    ("ui.changeGraphType.open", "Open Change Graph Type Dialog", "Cmd+Shift+G", "{\"sheet_id\":\"optional GraphSheetId\"}"),
    ("ui.clonePipeline.open", "Open Clone Analysis Pipeline Wizard", "", "{\"sheet_id\":\"optional SheetId\"}"),
    ("ui.zoom.fitSheet", "Zoom to Fit Sheet", "Cmd+0", "{}"),
    ("ui.locale.set", "Set UI Locale", "", "{\"locale\":\"en|de|es|fr|ja|ko|pt-BR|zh-CN|zh-TW\"}"),
    ("ui.theme.set", "Set UI Theme Mode", "", "{\"mode\":\"light|dark|high_contrast\"}"),
    ("ui.screenshot", "Capture Workspace Screenshot PNG", "", "{\"width_px\":1024,\"height_px\":640,\"dpi\":144,\"path\":\"optional.png\"}"),
    ("app.about", "About LumenCraft", "", "{}"),
];
