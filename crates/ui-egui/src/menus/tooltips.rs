//! Menu and toolbar tooltips resolved from engine [`CommandSpec`](curvebook_engine::cmd::CommandSpec) and front-end [`UiCommandSpec`](crate::menus::ui_commands::UiCommandSpec).

use curvebook_engine::cmd;

use crate::UiApp;
use crate::i18n;
use crate::menus::ui_commands::{ActionRank, UI_COMMANDS};

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
        let disp_shortcut = super::shortcuts::format_shortcut_for_platform(&self.shortcut);
        let mut header =
            if disp_shortcut.is_empty() { String::from(self.localized_label) } else { format!("{} ({disp_shortcut})", self.localized_label) };
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

/// Declarative desktop toolbar button wired to an i18n label key, command ID, and default [`ActionRank`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ToolbarButtonSpec {
    pub id: &'static str,
    pub label_key: &'static str,
    pub command_id: &'static str,
    pub rank: ActionRank,
}

/// Primary desktop toolbar actions exposing rich `summary` + `beginner` tooltips.
pub const TOOLBAR_BUTTONS: &[ToolbarButtonSpec] = &[
    ToolbarButtonSpec { id: "toolbar.new", label_key: "menu.file.new_project", command_id: "project.new", rank: ActionRank::Secondary },
    ToolbarButtonSpec { id: "toolbar.open", label_key: "menu.file.open", command_id: "project.open", rank: ActionRank::Secondary },
    ToolbarButtonSpec { id: "toolbar.save", label_key: "menu.file.save", command_id: "project.save", rank: ActionRank::Secondary },
    ToolbarButtonSpec { id: "toolbar.undo", label_key: "menu.edit.undo", command_id: "edit.undo", rank: ActionRank::Overflow },
    ToolbarButtonSpec { id: "toolbar.redo", label_key: "menu.edit.redo", command_id: "edit.redo", rank: ActionRank::Overflow },
    ToolbarButtonSpec { id: "toolbar.analyze", label_key: "btn.analyze", command_id: "ui.analyze.open", rank: ActionRank::Primary },
    ToolbarButtonSpec { id: "toolbar.axes", label_key: "inspector.format_axes", command_id: "graph.setAxes", rank: ActionRank::Secondary },
    ToolbarButtonSpec { id: "toolbar.palette", label_key: "menu.view.command_palette", command_id: "ui.palette.open", rank: ActionRank::Secondary },
    ToolbarButtonSpec { id: "toolbar.export_svg", label_key: "menu.file.export_svg", command_id: "export.svg", rank: ActionRank::Secondary },
];

/// Resolve the default [`ActionRank`] for any UI (`UI_COMMANDS`) or engine (`cmd::registry()`) command ID.
#[must_use]
pub fn default_command_action_rank(command_id: &str) -> ActionRank {
    if let Some(ui_spec) = UI_COMMANDS.iter().find(|s| s.id == command_id) {
        return ui_spec.rank;
    }
    match command_id {
        "analysis.run" | "fit.run" | "stats.run" | "transform.run" => ActionRank::Primary,
        "sheet.createData" | "graph.create" | "layout.create" | "info.setConstant" | "project.open" | "project.save" | "export.svg"
        | "export.pdf" | "export.png" => ActionRank::Secondary,
        _ => ActionRank::Overflow,
    }
}

/// Map a compact top-bar button caption rendered by the compositor to its underlying command ID.
#[must_use]
pub fn toolbar_caption_command_id(caption: &str) -> Option<&'static str> {
    match caption {
        "Analyze" => Some("ui.analyze.open"),
        "Axes" => Some("graph.setAxes"),
        "Commands" | "Cmd+K" => Some("ui.palette.open"),
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

pub(crate) fn ui_command_menu_info(id: &str) -> (&'static str, &'static str) {
    match id {
        "app.about" => ("menu.help", "Help/About"),
        "app.feedback" => ("menu.help", "Help/Send Feedback..."),
        "ui.navigator.toggle" => ("menu.view", "View/Navigator"),
        "ui.inspector.toggle" => ("menu.view", "View/Inspector"),
        "ui.family.toggle" => ("menu.view", "View/Family Pane"),
        "ui.palette.open" => ("menu.view", "View/Command Palette..."),
        "ui.zoom.fitSheet" => ("menu.view", "View/Zoom to Fit"),
        "ui.gallery.toggle" => ("menu.view", "View/Gallery View"),
        "ui.rulers.toggle" => ("menu.view", "View/Rulers"),
        "ui.pane.cycle" => ("menu.view", "View/Cycle Focus Pane"),
        "ui.shortcut.set" => ("menu.edit", "Edit/Preferences/Keyboard Shortcuts..."),
        "ui.shortcut.reset" => ("menu.edit", "Edit/Preferences/Reset Shortcuts"),
        "ui.locale.set" => ("menu.view", "View/Language"),
        "ui.theme.set" => ("menu.view", "View/Theme"),
        "ui.previewFilter.set" => ("menu.view", "View/Colorblind Preview"),
        "ui.split.toggle" => ("menu.view", "View/Split Workspace"),
        "ui.screenshot" => ("menu.view", "View/Capture Screenshot"),
        "ui.event.dispatch" => ("menu.view", "Tools/Dispatch UI Event"),
        "ui.powerWizard.open" => ("menu.analyze", "Analyze/Power and Sample Size..."),
        "ui.analyze.open" => ("menu.analyze", "Analyze/Analyze..."),
        "ui.editParams.open" => ("menu.analyze", "Analyze/Change Analysis Parameters..."),
        "ui.result.copyMethodsReport" => ("menu.analyze", "Analyze/Copy Methods Report"),
        "ui.result.applyRemediation" => ("menu.analyze", "Analyze/Apply Assumption Remediation"),
        "ui.result.inspectDiagnostics" => ("menu.analyze", "Analyze/Inspect Result Diagnostic Tabs"),
        "ui.result.appendClusterColumn" => ("menu.analyze", "Analyze/Append Cluster Column to Source Table"),
        "ui.formatPoints.open" => ("menu.graph", "Format/Format Selected Points..."),
        "ui.graph.axes.open" => ("menu.graph", "Format/Format Axes..."),
        "ui.graph.insertModelEquation" => ("menu.graph", "Draw/Insert Model Equation..."),
        "ui.graph.bubblePlot.open" => ("menu.graph", "Graph/Bubble Plot Channels..."),
        "ui.graph.format.open" => ("menu.graph", "Format/Format Graph..."),
        "ui.graph.pairwise.open" => ("menu.graph", "Format/Pairwise Comparison Brackets..."),
        "ui.applyStyleFrom.open" => ("menu.graph", "Graph/Apply Style From..."),
        "ui.layout.composer.open" => ("menu.graph", "Layout/Page Layout Composer..."),
        "ui.changeGraphType.open" => ("menu.graph", "Graph/Change Graph Type..."),
        "ui.canvas.applyPreset" => ("menu.graph", "Graph/Apply Canvas Preset"),
        "ui.canvas.preflight" => ("menu.graph", "Graph/Preflight Check"),
        "ui.brush.point" => ("menu.graph", "Graph/Brush Point"),
        "ui.print.open" => ("menu.file", "File/Print..."),
        "ui.export.figure.open" => ("menu.file", "File/Export Figure..."),
        "ui.import.reshape.open" => ("menu.file", "File/Import Reshape..."),
        "ui.clonePipeline.open" => ("menu.data", "Data/Clone Pipeline..."),
        "ui.table.convertArchetype.open" => ("menu.data", "Data/Convert Table Archetype..."),
        "ui.table.randomizeGroups" => ("menu.data", "Data/Randomize Groups Dialog..."),
        "ui.table.randomSampleRows" => ("menu.data", "Data/Random Sample Rows Dialog..."),
        "ui.table.simulateDistributions" => ("menu.insert", "Insert/Simulate Distributions Dialog..."),
        "ui.table.openFormulaBar" => ("menu.data", "Data/Open Formula Bar..."),
        "ui.grid.excludeWithReason" => ("menu.edit", "Edit/Exclude with Reason..."),
        "ui.view.rowTitles.toggle" => ("menu.view", "View/Row Titles"),
        "ui.gridOverlay.toggle" => ("menu.view", "View/Canvas Grid"),
        "ui.graph.create.open" => ("menu.insert", "Insert/New Graph Dialog..."),
        "ui.layout.equalizeScaling.open" => ("menu.graph", "Arrange/Equalize Scaling..."),
        "ui.embeddedTable.format.open" => ("menu.graph", "Graph/Format Embedded Table..."),
        "ui.image.format.open" => ("menu.graph", "Graph/Format Image..."),
        _ => ("menu.view", "UI"),
    }
}

pub(crate) fn ui_command_enabled_for_doc(doc: &curvebook_doc::ProjectDocument, id: &str) -> bool {
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
        "ui.formatPoints.open"
        | "ui.changeGraphType.open"
        | "ui.graph.axes.open"
        | "ui.graph.insertModelEquation"
        | "ui.graph.bubblePlot.open"
        | "ui.graph.format.open"
        | "ui.graph.pairwise.open"
        | "ui.applyStyleFrom.open" => !doc.graph_sheets.is_empty(),
        "ui.editParams.open" | "ui.result.copyMethodsReport" | "ui.result.applyRemediation" | "ui.result.inspectDiagnostics" => {
            !doc.result_sheets.is_empty()
        }
        "ui.result.appendClusterColumn" => doc
            .result_sheets
            .iter()
            .any(|rs| matches!(rs.analysis.kind, curvebook_doc::AnalysisKind::Multivariate(curvebook_doc::MultivariateSpec::Clustering { .. }))),
        "ui.table.openFormulaBar" => doc.data_sheets.iter().any(|ds| matches!(ds.archetype, curvebook_doc::TableArchetype::MultipleVariables(_))),
        "ui.table.convertArchetype.open"
        | "ui.table.randomSampleRows"
        | "ui.grid.excludeWithReason"
        | "ui.graph.create.open"
        | "ui.brush.point"
        | "ui.view.rowTitles.toggle" => !doc.data_sheets.is_empty(),
        "ui.zoom.fitSheet" => doc.total_sheet_count() > 0,
        "ui.layout.composer.open" | "ui.layout.equalizeScaling.open" => !doc.layout_sheets.is_empty(),
        "ui.embeddedTable.format.open" => {
            doc.graph_sheets
                .iter()
                .any(|g| g.spec.annotations.iter().any(|a| matches!(a, curvebook_doc::GraphAnnotation::EmbeddedResultTable { .. })))
                || doc.layout_sheets.iter().any(|l| l.spec.items.iter().any(|it| matches!(it, curvebook_doc::LayoutItem::LiveTable { .. })))
        }
        "ui.image.format.open" => {
            doc.graph_sheets.iter().any(|g| g.spec.annotations.iter().any(|a| matches!(a, curvebook_doc::GraphAnnotation::EmbeddedImage { .. })))
                || doc.layout_sheets.iter().any(|l| l.spec.items.iter().any(|it| matches!(it, curvebook_doc::LayoutItem::EmbeddedImage { .. })))
        }
        "ui.canvas.applyPreset" | "ui.canvas.preflight" | "ui.export.figure.open" => !doc.graph_sheets.is_empty() || !doc.layout_sheets.is_empty(),
        _ => true,
    }
}

fn ui_command_enabled(app: &UiApp, id: &str) -> bool {
    ui_command_enabled_for_doc(app.document(), id)
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

/// Collect all menu sections with localized section titles and per-item [`CommandTooltip`] metadata
/// derived directly from the curated 8-menu hierarchy ([`super::menu_bar::curated_menu_bar`]).
#[must_use]
pub fn menu_bar_tooltips(app: &UiApp) -> Vec<MenuSectionTooltips> {
    super::menu_bar::curated_menu_bar(app)
        .into_iter()
        .map(|sec| MenuSectionTooltips { menu_key: sec.menu_key, title: sec.title, items: sec.into_flat_actions() })
        .collect()
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
        for cap in ["Analyze", "Axes", "Commands", "Cmd+K"] {
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
        let (frame, w, h) = app.render_egui_shell_screenshot_png(
            crate::RenderTargetSpec { width_px: 480, height_px: 320, dpi: 96 },
            crate::inspect::ScreenshotRegionTarget::FullWindow,
        )?;
        assert!(w == 480 && h == 320 && frame.len() > 8);

        // Curated 8-menu bar exposes non-duplicate actions with non-empty summary & beginner guidance.
        let sections = menu_bar_tooltips(&app);
        assert_eq!(sections.len(), 8);
        let mut seen = HashSet::new();
        for sec in &sections {
            assert!(!sec.title.is_empty());
            for item in &sec.items {
                assert!(!item.summary.is_empty(), "empty summary for {}", item.command_id);
                assert!(!item.beginner.is_empty(), "empty beginner for {}", item.command_id);
                assert!(seen.insert(item.command_id), "duplicate menu command {}", item.command_id);
            }
        }
        assert!(seen.len() >= 60 && seen.len() <= 96, "expected curated menu size, got {}", seen.len());
        Ok(())
    }
}
