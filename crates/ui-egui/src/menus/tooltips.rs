//! Menu and toolbar tooltips resolved from engine [`CommandSpec`] and front-end [`UiCommandSpec`].

use lumencraft_engine::cmd;

use crate::UiApp;
use crate::i18n;
use crate::menus::ui_commands::UI_COMMANDS;

/// Rich menu/toolbar hover tooltip metadata surfacing `summary`, `beginner`, status, and effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTooltip {
    pub command_id: &'static str,
    pub label: &'static str,
    pub localized_label: &'static str,
    pub shortcut: String,
    pub summary: &'static str,
    pub beginner: &'static str,
    pub menu_path: &'static str,
    pub enabled: bool,
    pub read_only: bool,
    pub destructive: bool,
    pub needs_consent: bool,
}

impl CommandTooltip {
    /// Format multi-line hover tooltip copy surfacing localized label, shortcut, `summary`, and `beginner`.
    #[must_use]
    pub fn format_tooltip(&self) -> String {
        let mut header =
            if self.shortcut.is_empty() { String::from(self.localized_label) } else { format!("{} ({})", self.localized_label, self.shortcut) };
        if !self.enabled {
            header.push_str(" [unavailable]");
        }
        format!("{header}\n{}\nTip: {}", self.summary, self.beginner)
    }

    /// Format compact single-line status/banner tooltip surfacing `summary` and `beginner`.
    #[must_use]
    pub fn format_inline_tooltip(&self) -> String {
        let mut prefix = String::from(self.localized_label);
        if !self.enabled {
            prefix.push_str(" [unavailable]");
        }
        format!("{prefix}: {} | Tip: {}", self.summary, self.beginner)
    }
}

/// Declarative desktop toolbar button wired to an i18n label key and command ID.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ToolbarButtonSpec {
    pub id: &'static str,
    pub label_key: &'static str,
    pub command_id: &'static str,
}

/// Primary desktop toolbar actions exposing rich `summary` + `beginner` tooltips.
pub const TOOLBAR_BUTTONS: &[ToolbarButtonSpec] = &[
    ToolbarButtonSpec { id: "toolbar.new", label_key: "menu.file.new_project", command_id: "project.new" },
    ToolbarButtonSpec { id: "toolbar.open", label_key: "menu.file.open", command_id: "project.open" },
    ToolbarButtonSpec { id: "toolbar.save", label_key: "menu.file.save", command_id: "project.save" },
    ToolbarButtonSpec { id: "toolbar.undo", label_key: "menu.edit.undo", command_id: "edit.undo" },
    ToolbarButtonSpec { id: "toolbar.redo", label_key: "menu.edit.redo", command_id: "edit.redo" },
    ToolbarButtonSpec { id: "toolbar.analyze", label_key: "btn.analyze", command_id: "ui.analyze.open" },
    ToolbarButtonSpec { id: "toolbar.axes", label_key: "inspector.format_axes", command_id: "graph.setAxes" },
    ToolbarButtonSpec { id: "toolbar.palette", label_key: "menu.view.command_palette", command_id: "ui.palette.open" },
    ToolbarButtonSpec { id: "toolbar.export_svg", label_key: "menu.file.export_svg", command_id: "export.svg" },
];

/// Map a compact top-bar button caption rendered by the compositor to its underlying command ID.
#[must_use]
pub fn toolbar_caption_command_id(caption: &str) -> Option<&'static str> {
    match caption {
        "Analyze" => Some("ui.analyze.open"),
        "Axes" => Some("graph.setAxes"),
        "Cmd+K" => Some("ui.palette.open"),
        _ => None,
    }
}

/// Look up rich tooltip metadata for `command_id` against the current `app` session, shortcuts, and locale.
#[must_use]
pub fn command_tooltip(app: &UiApp, command_id: &str) -> Option<CommandTooltip> {
    if let Some(spec) = cmd::registry().get(command_id) {
        let shortcut = app.shortcuts.active_combo(spec.id).unwrap_or(spec.shortcut).to_owned();
        return Some(CommandTooltip {
            command_id: spec.id,
            label: spec.label,
            localized_label: i18n::command_label(&app.locale, spec.id, spec.label),
            shortcut,
            summary: spec.summary,
            beginner: spec.beginner,
            menu_path: spec.menu_path,
            enabled: (spec.enabled)(&app.session),
            read_only: spec.effects.read_only_hint(),
            destructive: spec.effects.destructive_hint(),
            needs_consent: spec.effects.needs_consent(),
        });
    }
    UI_COMMANDS.iter().find(|spec| spec.id == command_id).map(|spec| {
        let shortcut = app.shortcuts.active_combo(spec.id).unwrap_or(spec.shortcut).to_owned();
        let (_bucket, menu_path) = ui_command_menu_info(spec.id);
        CommandTooltip {
            command_id: spec.id,
            label: spec.label,
            localized_label: i18n::command_label(&app.locale, spec.id, spec.label),
            shortcut,
            summary: spec.summary,
            beginner: spec.beginner,
            menu_path,
            enabled: ui_command_enabled(app, spec.id),
            read_only: true,
            destructive: false,
            needs_consent: false,
        }
    })
}

fn ui_command_menu_info(id: &str) -> (&'static str, &'static str) {
    match id {
        "app.about" => ("menu.help", "Help"),
        "ui.powerWizard.open" => ("menu.analyze", "Analyze/Power and Sample Size..."),
        "ui.analyze.open" => ("menu.analyze", "Analyze/Analyze..."),
        "ui.result.copyMethodsReport" => ("menu.analyze", "Analyze/Copy Methods Report"),
        "ui.result.applyRemediation" => ("menu.analyze", "Analyze/Apply Assumption Remediation"),
        "ui.formatPoints.open" => ("menu.graph", "Format/Format Selected Points..."),
        "ui.changeGraphType.open" => ("menu.graph", "Graph/Change Graph Type..."),
        "ui.canvas.applyPreset" => ("menu.graph", "Graph/Apply Canvas Preset"),
        "ui.canvas.preflight" => ("menu.graph", "Graph/Preflight Check"),
        "ui.brush.point" => ("menu.graph", "Graph/Brush Point"),
        "ui.print.open" => ("menu.file", "File/Print..."),
        "ui.clonePipeline.open" => ("menu.file", "File/Clone Pipeline..."),
        "ui.import.reshape.open" => ("menu.file", "File/Import Reshape..."),
        "ui.table.convertArchetype.open" => ("menu.insert", "Data/Convert Table Archetype..."),
        "ui.grid.excludeWithReason" => ("menu.edit", "Edit/Exclude with Reason..."),
        _ => ("menu.view", "UI"),
    }
}

fn ui_command_enabled(app: &UiApp, id: &str) -> bool {
    let doc = app.document();
    match id {
        "ui.analyze.open" => {
            let active_analyzable = doc.active_sheet_id.as_ref().is_some_and(|sid| {
                doc.data_sheet(sid).is_some()
                    || doc
                        .result_sheet(sid)
                        .is_some_and(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.column_headers.is_empty() && !t.rows.is_empty()))
            });
            active_analyzable || !doc.data_sheets.is_empty()
        }
        "ui.formatPoints.open" | "ui.changeGraphType.open" => !doc.graph_sheets.is_empty(),
        "ui.result.copyMethodsReport" | "ui.result.applyRemediation" => !doc.result_sheets.is_empty(),
        "ui.table.convertArchetype.open" | "ui.grid.excludeWithReason" => !doc.data_sheets.is_empty(),
        "ui.canvas.applyPreset" | "ui.canvas.preflight" => !doc.graph_sheets.is_empty() || !doc.layout_sheets.is_empty(),
        _ => true,
    }
}

/// Resolved localized toolbar button entry paired with its hover [`CommandTooltip`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarButtonTooltip {
    pub button: ToolbarButtonSpec,
    pub caption: &'static str,
    pub tooltip: CommandTooltip,
}

/// Build localized toolbar items and hover tooltips for `app`.
#[must_use]
pub fn toolbar_tooltips(app: &UiApp) -> Vec<ToolbarButtonTooltip> {
    let mut out = Vec::with_capacity(TOOLBAR_BUTTONS.len());
    for &button in TOOLBAR_BUTTONS {
        if let Some(tooltip) = command_tooltip(app, button.command_id) {
            out.push(ToolbarButtonTooltip { button, caption: app.tr(button.label_key), tooltip });
        }
    }
    out
}

/// Top-level menu category paired with resolved [`CommandTooltip`] entries for its commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSectionTooltips {
    pub menu_key: &'static str,
    pub title: &'static str,
    pub items: Vec<CommandTooltip>,
}

fn menu_key_for_path(menu_path: &str) -> &'static str {
    let head = menu_path.split(['/', '>']).next().map_or("", str::trim);
    match head {
        "File" => "menu.file",
        "Edit" => "menu.edit",
        "Insert" => "menu.insert",
        "Analyze" | "Data" | "Change" | "Info" => "menu.analyze",
        "Graph" | "Format" | "Draw" | "Arrange" => "menu.graph",
        "Help" => "menu.help",
        _ => "menu.view",
    }
}

/// Collect all menu sections with localized section titles and per-item [`CommandTooltip`] metadata.
#[must_use]
pub fn menu_bar_tooltips(app: &UiApp) -> Vec<MenuSectionTooltips> {
    let mut sections = vec![
        MenuSectionTooltips { menu_key: "menu.file", title: app.tr("menu.file"), items: Vec::new() },
        MenuSectionTooltips { menu_key: "menu.edit", title: app.tr("menu.edit"), items: Vec::new() },
        MenuSectionTooltips { menu_key: "menu.insert", title: app.tr("menu.insert"), items: Vec::new() },
        MenuSectionTooltips { menu_key: "menu.analyze", title: app.tr("menu.analyze"), items: Vec::new() },
        MenuSectionTooltips { menu_key: "menu.graph", title: app.tr("menu.graph"), items: Vec::new() },
        MenuSectionTooltips { menu_key: "menu.view", title: app.tr("menu.view"), items: Vec::new() },
        MenuSectionTooltips { menu_key: "menu.help", title: app.tr("menu.help"), items: Vec::new() },
    ];

    for spec in cmd::registry().specs() {
        let bucket = menu_key_for_path(spec.menu_path);
        if let (Some(section), Some(tip)) = (sections.iter_mut().find(|s| s.menu_key == bucket), command_tooltip(app, spec.id)) {
            section.items.push(tip);
        }
    }
    for spec in UI_COMMANDS {
        let (bucket, _) = ui_command_menu_info(spec.id);
        if let (Some(section), Some(tip)) = (sections.iter_mut().find(|s| s.menu_key == bucket), command_tooltip(app, spec.id)) {
            section.items.push(tip);
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::{TOOLBAR_BUTTONS, command_tooltip, menu_bar_tooltips, toolbar_caption_command_id, toolbar_tooltips};
    use crate::UiApp;

    #[test]
    fn toolbar_and_menu_tooltips_surface_summary_beginner_and_session_state() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        app.locale = String::from("de");

        let toolbar = toolbar_tooltips(&app);
        assert_eq!(toolbar.len(), TOOLBAR_BUTTONS.len());
        for entry in &toolbar {
            let formatted = entry.tooltip.format_tooltip();
            assert!(formatted.contains(entry.tooltip.summary));
            assert!(formatted.contains(entry.tooltip.beginner));
            assert!(!entry.caption.is_empty());
        }

        // German localized label appears in toolbar tooltip.
        let save_tip = command_tooltip(&app, "project.save").ok_or("missing project.save tooltip")?;
        assert_eq!(save_tip.localized_label, "Projekt speichern");
        assert!(save_tip.format_tooltip().contains("Projekt speichern"));
        assert!(save_tip.format_inline_tooltip().contains(save_tip.beginner));

        // Compositor quick buttons resolve cleanly.
        for cap in ["Analyze", "Axes", "Cmd+K"] {
            let cmd_id = toolbar_caption_command_id(cap).ok_or("missing caption mapping")?;
            let tip = command_tooltip(&app, cmd_id).ok_or("missing command tooltip")?;
            assert!(!tip.summary.is_empty());
            assert!(!tip.beginner.is_empty());
        }

        // Session `(spec.enabled)(session)` reflects state changes on graph/data commands.
        let axes_disabled = command_tooltip(&app, "graph.setAxes").ok_or("missing graph.setAxes tooltip")?;
        assert!(!axes_disabled.enabled);
        assert!(axes_disabled.format_tooltip().contains("[unavailable]"));

        app.execute_command("sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "title": "Assay", "auto_graph": true }))?;
        let axes_enabled = command_tooltip(&app, "graph.setAxes").ok_or("missing graph.setAxes tooltip")?;
        assert!(axes_enabled.enabled);

        app.hover_command_id = Some(String::from("analyze.run"));
        app.canvas_dialogs.open_command_palette();
        let frame = crate::render_workspace_frame_png(&app, crate::RenderTargetSpec { width_px: 480, height_px: 320, dpi: 96 })?;
        assert!(frame.len() > 8);

        // Menu bar covers all 92 engine + 32 UI commands with non-empty summary & beginner guidance.
        let sections = menu_bar_tooltips(&app);
        assert_eq!(sections.len(), 7);
        let mut seen = HashSet::new();
        for sec in &sections {
            assert!(!sec.title.is_empty());
            for item in &sec.items {
                assert!(!item.summary.is_empty(), "empty summary for {}", item.command_id);
                assert!(!item.beginner.is_empty(), "empty beginner for {}", item.command_id);
                assert!(seen.insert(item.command_id), "duplicate menu command {}", item.command_id);
            }
        }
        assert_eq!(seen.len(), 124);
        Ok(())
    }
}
