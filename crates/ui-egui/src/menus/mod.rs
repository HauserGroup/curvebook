//! Menu definitions, customizable shortcuts, and desktop UI command metadata.

pub mod menu_bar;
pub mod shortcuts;
pub mod tooltips;
pub mod ui_commands;

pub use menu_bar::{CuratedMenuNode, CuratedMenuSection, curated_menu_bar};
pub use shortcuts::{
    ShortcutBinding, ShortcutExecution, ShortcutMatch, ShortcutRegistry, ShortcutScope, format_shortcut_for_os, format_shortcut_for_platform,
    normalize_shortcut_combo,
};
pub use tooltips::{
    CommandTooltip, MenuSectionTooltips, TOOLBAR_BUTTONS, ToolbarButtonSpec, ToolbarButtonTooltip, command_tooltip, menu_bar_tooltips,
    toolbar_caption_command_id, toolbar_tooltips,
};
pub use ui_commands::{UI_COMMANDS, UiCommandSpec};

/// Canonical list of desktop host-routed commands intercepted by the native desktop host
/// (`CurveBookNativeApp` / `DesktopHostRuntime`) rather than executed directly in the shell
/// without OS file dialogs, native EMF/figure save dialogs, multi-window orchestration, or
/// OS pasteboard access (`R15-GUI-07`, `R15-GUI-13`, `R15-GUI-23`).
pub const DESKTOP_HOST_ROUTED_COMMANDS: &[&str] = &[
    "project.open",
    "project.save",
    "project.saveAs",
    "file.open",
    "file.open_unpacked_dir",
    "file.save",
    "file.save_as",
    "export.svg",
    "file.export_svg",
    "export.pdf",
    "file.export_pdf",
    "export.png",
    "file.export_png",
    "graph.exportEmf",
    "layout.exportEmf",
    "window.new",
    "file.newWindow",
    "window.close",
    "file.close",
    "file.quit",
    "app.quit",
    "edit.cut",
    "edit.copy",
    "edit.paste",
];

/// Returns `true` when `command_id` must be routed through the native desktop host shell.
#[must_use]
pub fn is_desktop_host_routed_command(command_id: &str) -> bool {
    DESKTOP_HOST_ROUTED_COMMANDS.contains(&command_id)
}
