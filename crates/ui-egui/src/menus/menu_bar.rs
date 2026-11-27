//! Declarative hierarchical 8-menu bar specification shared across `egui::MenuBar`,
//! native macOS `muda`, and the web chrome adapter (`docs/planning/gui-layout-spec.md` §3.1,
//! `R15-GUI-34`).

use crate::{UiApp, i18n};

use super::tooltips::{CommandTooltip, command_tooltip};

/// Single entry inside a hierarchical curated top-level menu or submenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CuratedMenuNode {
    /// Executable command leaf carrying resolved localized label, shortcut, summary, beginner copy, and enablement.
    Action(CommandTooltip),
    /// Visual divider between semantic action groups.
    Separator,
    /// Nested submenu (`File › Import ▸`, `File › Export ▸`, `File › Checkpoints ▸`, `Graph › Layout ▸`).
    Submenu { id: &'static str, label_key: &'static str, title: String, children: Vec<Self> },
}

/// Top-level curated menu bar section (`File`, `Edit`, `View`, `Insert`, `Data`, `Analyze`, `Graph`, `Help`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratedMenuSection {
    pub menu_key: &'static str,
    pub title: &'static str,
    pub nodes: Vec<CuratedMenuNode>,
}

impl CuratedMenuSection {
    /// Collect all executable [`CommandTooltip`] action leaves recursively in menu order.
    #[must_use]
    pub fn flat_actions(&self) -> Vec<&CommandTooltip> {
        let mut out = Vec::new();
        collect_actions_from_nodes(&self.nodes, &mut out);
        out
    }

    /// Consume and flatten all executable [`CommandTooltip`] action leaves in menu order.
    #[must_use]
    pub fn into_flat_actions(self) -> Vec<CommandTooltip> {
        let mut out = Vec::new();
        drain_actions_from_nodes(self.nodes, &mut out);
        out
    }
}

fn collect_actions_from_nodes<'a>(nodes: &'a [CuratedMenuNode], out: &mut Vec<&'a CommandTooltip>) {
    for node in nodes {
        match node {
            CuratedMenuNode::Action(tip) => out.push(tip),
            CuratedMenuNode::Separator => {}
            CuratedMenuNode::Submenu { children, .. } => collect_actions_from_nodes(children, out),
        }
    }
}

fn drain_actions_from_nodes(nodes: Vec<CuratedMenuNode>, out: &mut Vec<CommandTooltip>) {
    for node in nodes {
        match node {
            CuratedMenuNode::Action(tip) => out.push(tip),
            CuratedMenuNode::Separator => {}
            CuratedMenuNode::Submenu { children, .. } => drain_actions_from_nodes(children, out),
        }
    }
}

/// OS-adaptive menu placement helper (`R20.22`): omit `app.about`, `app.preferences`, and
/// `app.quit` from top-level menus when the host OS menu bar (`CurveBook` app menu on macOS) owns
/// them, while surfacing `File › Quit`, `Edit › Settings`, and `Help › About CurveBook` in
/// in-window Windows/Linux menu bars.
#[must_use]
pub const fn include_app_menu_items_in_top_menus(host_draws_menu_bar: bool) -> bool {
    !host_draws_menu_bar
}

fn push_os_adaptive_app_item(
    nodes: &mut Vec<CuratedMenuNode>,
    app: &UiApp,
    command_id: &'static str,
    label_key: Option<&'static str>,
    with_leading_separator: bool,
) {
    if include_app_menu_items_in_top_menus(app.host_draws_menu_bar)
        && let Some(node) = action_node(app, command_id, label_key)
    {
        if with_leading_separator && !nodes.is_empty() && !matches!(nodes.last(), Some(CuratedMenuNode::Separator)) {
            nodes.push(CuratedMenuNode::Separator);
        }
        nodes.push(node);
    }
}

fn host_routed_tooltip(app: &UiApp, command_id: &'static str) -> Option<CommandTooltip> {
    let has_sheet = app.document().total_sheet_count() > 0;
    match command_id {
        "file.open_unpacked_dir" => Some(CommandTooltip {
            command_id: "file.open_unpacked_dir",
            label: i18n::tr("en", "menu.file.open_unpacked_dir"),
            localized_label: app.tr("menu.file.open_unpacked_dir"),
            shortcut: String::new(),
            summary: "Open an unpacked `.curvebook.d` directory project.",
            beginner: "Browse for a plain-text directory-backed CurveBook project.",
            menu_path: "File/Open Unpacked Folder",
            enabled: true,
            read_only: true,
            destructive: false,
            needs_consent: false,
        }),
        "file.save_as" => Some(CommandTooltip {
            command_id: "file.save_as",
            label: i18n::tr("en", "menu.file.save_as"),
            localized_label: app.tr("menu.file.save_as"),
            shortcut: app.shortcuts.active_combo("file.save_as").unwrap_or("Cmd+Shift+S").to_owned(),
            summary: "Save a copy of the project to a new path or format.",
            beginner: "Choose a new file name or location for the active CurveBook project.",
            menu_path: "File/Save As",
            enabled: true,
            read_only: false,
            destructive: false,
            needs_consent: false,
        }),
        "edit.cut" => Some(CommandTooltip {
            command_id: "edit.cut",
            label: i18n::tr("en", "menu.edit.cut"),
            localized_label: app.tr("menu.edit.cut"),
            shortcut: app.shortcuts.active_combo("edit.cut").unwrap_or("Cmd+X").to_owned(),
            summary: "Copy the selected table cells to the clipboard and clear them.",
            beginner: "Cut the current grid cell selection onto the clipboard.",
            menu_path: "Edit/Cut",
            enabled: has_sheet,
            read_only: false,
            destructive: false,
            needs_consent: false,
        }),
        "edit.copy" => Some(CommandTooltip {
            command_id: "edit.copy",
            label: i18n::tr("en", "menu.edit.copy"),
            localized_label: app.tr("menu.edit.copy"),
            shortcut: app.shortcuts.active_combo("edit.copy").unwrap_or("Cmd+C").to_owned(),
            summary: "Copy the selected cells, result table, or graph/layout vector image.",
            beginner: "Copy the active selection or figure to the system clipboard.",
            menu_path: "Edit/Copy",
            enabled: has_sheet,
            read_only: true,
            destructive: false,
            needs_consent: false,
        }),
        "edit.paste" => Some(CommandTooltip {
            command_id: "edit.paste",
            label: i18n::tr("en", "menu.edit.paste"),
            localized_label: app.tr("menu.edit.paste"),
            shortcut: app.shortcuts.active_combo("edit.paste").unwrap_or("Cmd+V").to_owned(),
            summary: "Paste tabular values from the clipboard into the active sheet.",
            beginner: "Paste clipboard text or TSV rows starting at the selected cell.",
            menu_path: "Edit/Paste",
            enabled: has_sheet,
            read_only: false,
            destructive: false,
            needs_consent: false,
        }),
        "app.preferences" => Some(CommandTooltip {
            command_id: "app.preferences",
            label: i18n::tr("en", "prefs.title"),
            localized_label: app.tr("prefs.title"),
            shortcut: app.shortcuts.active_combo("app.preferences").unwrap_or("Cmd+,").to_owned(),
            summary: "Open application settings (appearance, language, defaults, shortcuts).",
            beginner: "Configure interface theme, language, figure defaults, and keyboard shortcuts.",
            menu_path: "Edit/Settings",
            enabled: true,
            read_only: true,
            destructive: false,
            needs_consent: false,
        }),
        "window.close" => Some(CommandTooltip {
            command_id: "window.close",
            label: i18n::tr("en", "menu.file.close_window"),
            localized_label: app.tr("menu.file.close_window"),
            shortcut: app.shortcuts.active_combo("window.close").unwrap_or("Cmd+W").to_owned(),
            summary: "Close the active project window.",
            beginner: "Closes the active CurveBook window after prompting to save unsaved changes.",
            menu_path: "File/Close Window",
            enabled: true,
            read_only: false,
            destructive: false,
            needs_consent: false,
        }),
        "app.quit" => Some(CommandTooltip {
            command_id: "app.quit",
            label: i18n::tr("en", "menu.file.quit"),
            localized_label: app.tr("menu.file.quit"),
            shortcut: app.shortcuts.active_combo("app.quit").unwrap_or("Cmd+Q").to_owned(),
            summary: "Quit CurveBook.",
            beginner: "Closes all CurveBook windows after prompting to save unsaved changes.",
            menu_path: "File/Quit",
            enabled: true,
            read_only: false,
            destructive: false,
            needs_consent: false,
        }),
        _ => None,
    }
}

fn action_node(app: &UiApp, command_id: &'static str, label_override_key: Option<&'static str>) -> Option<CuratedMenuNode> {
    let mut tip = command_tooltip(app, command_id).or_else(|| host_routed_tooltip(app, command_id))?;
    if let Some(key) = label_override_key {
        tip.label = i18n::tr("en", key);
        tip.localized_label = app.tr(key);
    }
    Some(CuratedMenuNode::Action(tip))
}

fn push_action(nodes: &mut Vec<CuratedMenuNode>, app: &UiApp, command_id: &'static str, label_key: Option<&'static str>) {
    if let Some(node) = action_node(app, command_id, label_key) {
        nodes.push(node);
    }
}

fn submenu_node(id: &'static str, label_key: &'static str, app: &UiApp, children: Vec<CuratedMenuNode>) -> CuratedMenuNode {
    CuratedMenuNode::Submenu { id, label_key, title: app.tr(label_key).to_owned(), children }
}

fn build_file_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "project.new", Some("menu.file.new_project"));
    push_action(&mut nodes, app, "project.open", Some("menu.file.open"));
    push_action(&mut nodes, app, "file.open_unpacked_dir", Some("menu.file.open_unpacked_dir"));
    push_action(&mut nodes, app, "project.merge", Some("menu.file.merge_project"));
    push_action(&mut nodes, app, "project.save", Some("menu.file.save"));
    push_action(&mut nodes, app, "file.save_as", Some("menu.file.save_as"));
    nodes.push(CuratedMenuNode::Separator);

    let mut import_children = Vec::new();
    push_action(&mut import_children, app, "table.importCsv", None);
    push_action(&mut import_children, app, "table.importPlateMap", None);
    push_action(&mut import_children, app, "table.importTidyLong", None);
    push_action(&mut import_children, app, "ui.import.reshape.open", Some("menu.file.import_reshape"));
    push_action(&mut import_children, app, "table.importPzfx", None);
    push_action(&mut import_children, app, "table.importPrism", None);
    push_action(&mut import_children, app, "table.importXlsx", None);
    import_children.push(CuratedMenuNode::Separator);
    push_action(&mut import_children, app, "ui.import.warnings.open", None);
    nodes.push(submenu_node("menu.file.import", "menu.file.import", app, import_children));

    let mut export_children = Vec::new();
    push_action(&mut export_children, app, "ui.export.figure.open", None);
    export_children.push(CuratedMenuNode::Separator);
    push_action(&mut export_children, app, "export.pdf", None);
    push_action(&mut export_children, app, "export.png", None);
    push_action(&mut export_children, app, "export.svg", None);
    push_action(&mut export_children, app, "export.tiff", None);
    export_children.push(CuratedMenuNode::Separator);
    push_action(&mut export_children, app, "ui.export.tableCsv.open", None);
    push_action(&mut export_children, app, "ui.export.allTablesCsv.open", None);
    push_action(&mut export_children, app, "ui.export.resultsCsv.open", None);
    push_action(&mut export_children, app, "analysis.exportJson", None);
    export_children.push(CuratedMenuNode::Separator);
    push_action(&mut export_children, app, "ui.export.pzfx.open", None);
    export_children.push(CuratedMenuNode::Separator);
    push_action(&mut export_children, app, "result.generateReport", Some("menu.file.save_methods_report"));
    nodes.push(submenu_node("menu.file.export", "menu.file.export", app, export_children));

    let mut checkpoint_children = Vec::new();
    push_action(&mut checkpoint_children, app, "project.createCheckpoint", None);
    push_action(&mut checkpoint_children, app, "project.listCheckpoints", None);
    nodes.push(submenu_node("menu.file.checkpoints", "menu.file.checkpoints", app, checkpoint_children));

    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "project.saveTemplate", None);
    push_action(&mut nodes, app, "project.instantiateTemplate", None);
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.print.open", Some("menu.file.print"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "window.close", Some("menu.file.close_window"));
    push_os_adaptive_app_item(&mut nodes, app, "app.quit", Some("menu.file.quit"), false);

    CuratedMenuSection { menu_key: "menu.file", title: app.tr("menu.file"), nodes }
}

fn build_edit_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "edit.undo", Some("menu.edit.undo"));
    push_action(&mut nodes, app, "edit.redo", Some("menu.edit.redo"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "edit.cut", Some("menu.edit.cut"));
    push_action(&mut nodes, app, "edit.copy", Some("menu.edit.copy"));
    push_action(&mut nodes, app, "ui.clipboard.copyForSlides", None);
    push_action(&mut nodes, app, "edit.paste", Some("menu.edit.paste"));
    push_action(&mut nodes, app, "table.pasteLink", Some("menu.edit.paste_link"));
    push_action(&mut nodes, app, "edit.pasteResultTable", None);
    push_action(&mut nodes, app, "edit.pasteImage", None);
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.sheet.find", Some("menu.edit.find_in_sheet"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "table.excludeCells", None);
    push_action(&mut nodes, app, "ui.grid.excludeWithReason", None);
    push_action(&mut nodes, app, "table.exclusionAuditReport", None);
    push_os_adaptive_app_item(&mut nodes, app, "app.preferences", None, true);
    CuratedMenuSection { menu_key: "menu.edit", title: app.tr("menu.edit"), nodes }
}

fn build_view_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "ui.palette.open", Some("menu.view.command_palette"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.navigator.toggle", Some("menu.view.navigator"));
    push_action(&mut nodes, app, "ui.inspector.toggle", Some("menu.view.inspector"));
    push_action(&mut nodes, app, "ui.family.toggle", Some("menu.view.family_pane"));
    push_action(&mut nodes, app, "ui.split.toggle", None);
    push_action(&mut nodes, app, "ui.gallery.toggle", None);
    push_action(&mut nodes, app, "ui.workspacePreset.set", None);
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.rulers.toggle", None);
    push_action(&mut nodes, app, "ui.gridOverlay.toggle", None);
    push_action(&mut nodes, app, "ui.view.rowTitles.toggle", None);
    let mut zoom_children = Vec::new();
    push_action(&mut zoom_children, app, "ui.zoom.in", None);
    push_action(&mut zoom_children, app, "ui.zoom.out", None);
    zoom_children.push(CuratedMenuNode::Separator);
    push_action(&mut zoom_children, app, "ui.zoom.fitSheet", Some("menu.view.zoom_fit"));
    nodes.push(submenu_node("menu.view.zoom", "menu.view.zoom", app, zoom_children));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.theme.set", Some("menu.view.theme"));
    push_action(&mut nodes, app, "ui.previewFilter.set", None);
    push_action(&mut nodes, app, "ui.locale.set", None);
    CuratedMenuSection { menu_key: "menu.view", title: app.tr("menu.view"), nodes }
}

fn build_insert_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "sheet.createData", Some("menu.insert.new_data_table"));
    push_action(&mut nodes, app, "ui.graph.create.open", Some("menu.insert.new_graph"));
    push_action(&mut nodes, app, "layout.create", Some("menu.insert.new_layout"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "graph.addAnnotation", None);
    push_action(&mut nodes, app, "ui.graph.insertModelEquation", None);
    push_action(&mut nodes, app, "graph.insertImage", None);
    push_action(&mut nodes, app, "layout.insertImage", None);
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.table.simulateDistributions", Some("menu.insert.simulate_distributions"));
    CuratedMenuSection { menu_key: "menu.insert", title: app.tr("menu.insert"), nodes }
}

fn build_data_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "ui.table.convertArchetype.open", Some("menu.data.convert_table"));
    push_action(&mut nodes, app, "ui.table.openFormulaBar", None);
    push_action(&mut nodes, app, "ui.table.randomizeGroups", Some("menu.data.randomize_groups"));
    push_action(&mut nodes, app, "ui.table.randomSampleRows", Some("menu.data.random_sample"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "sheet.duplicate", None);
    push_action(&mut nodes, app, "sheet.duplicateFamily", None);
    push_action(&mut nodes, app, "ui.clonePipeline.open", Some("menu.data.clone_pipeline"));
    push_action(&mut nodes, app, "sheet.extractResultTab", None);
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "sheet.freeze", None);
    push_action(&mut nodes, app, "sheet.unfreeze", None);
    push_action(&mut nodes, app, "sheet.delete", None);
    CuratedMenuSection { menu_key: "menu.data", title: app.tr("menu.data"), nodes }
}

fn build_analyze_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "ui.analyze.open", Some("menu.analyze.analyze"));
    push_action(&mut nodes, app, "ui.editParams.open", Some("menu.analyze.change_params"));
    push_action(&mut nodes, app, "analysis.retargetData", Some("menu.analyze.change_data"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "project.recalculate", Some("menu.analyze.run_all"));
    push_action(&mut nodes, app, "ui.powerWizard.open", None);
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.result.inspectDiagnostics", None);
    push_action(&mut nodes, app, "ui.result.copyMethodsReport", None);
    push_action(&mut nodes, app, "ui.result.appendClusterColumn", None);
    CuratedMenuSection { menu_key: "menu.analyze", title: app.tr("menu.analyze"), nodes }
}

fn build_graph_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_action(&mut nodes, app, "ui.changeGraphType.open", Some("menu.graph.change_type"));
    push_action(&mut nodes, app, "ui.graph.format.open", Some("menu.graph.format_graph"));
    push_action(&mut nodes, app, "ui.graph.axes.open", Some("menu.graph.format_axes"));
    push_action(&mut nodes, app, "ui.formatPoints.open", None);
    push_action(&mut nodes, app, "ui.graph.pairwise.open", Some("menu.graph.pairwise_brackets"));
    push_action(&mut nodes, app, "ui.graph.bubblePlot.open", None);
    push_action(&mut nodes, app, "ui.applyStyleFrom.open", Some("menu.graph.apply_style_from"));
    nodes.push(CuratedMenuNode::Separator);
    push_action(&mut nodes, app, "ui.canvas.applyPreset", None);
    push_action(&mut nodes, app, "ui.canvas.preflight", None);
    nodes.push(CuratedMenuNode::Separator);

    let mut layout_children = Vec::new();
    push_action(&mut layout_children, app, "ui.layout.composer.open", None);
    push_action(&mut layout_children, app, "ui.layout.equalizeScaling.open", None);
    push_action(&mut layout_children, app, "layout.alignOrigins", None);
    push_action(&mut layout_children, app, "ui.embeddedTable.format.open", None);
    push_action(&mut layout_children, app, "ui.image.format.open", None);
    nodes.push(submenu_node("menu.graph.layout", "menu.graph.layout", app, layout_children));

    CuratedMenuSection { menu_key: "menu.graph", title: app.tr("menu.graph"), nodes }
}

fn build_help_menu(app: &UiApp) -> CuratedMenuSection {
    let mut nodes = Vec::new();
    push_os_adaptive_app_item(&mut nodes, app, "app.about", Some("menu.help.about"), false);
    push_action(&mut nodes, app, "app.feedback", Some("menu.help.feedback"));
    CuratedMenuSection { menu_key: "menu.help", title: app.tr("menu.help"), nodes }
}

/// Build the single curated, hierarchical 8-menu bar (`File`, `Edit`, `View`, `Insert`,
/// `Data`, `Analyze`, `Graph`, `Help`) shared across all desktop, in-window, and web hosts.
#[must_use]
pub fn curated_menu_bar(app: &UiApp) -> Vec<CuratedMenuSection> {
    vec![
        build_file_menu(app),
        build_edit_menu(app),
        build_view_menu(app),
        build_insert_menu(app),
        build_data_menu(app),
        build_analyze_menu(app),
        build_graph_menu(app),
        build_help_menu(app),
    ]
}
