#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use curvebook_engine::cmd;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::i18n::BETA_ACTIVE_LOCALES;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{CuratedMenuNode, UiApp, curated_menu_bar, dispatch_gui_action, menu_bar_tooltips};
use serde_json::json;

struct CuratedMenuShellState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_err: Option<String>,
}

fn make_driver(app: UiApp) -> EguiHumanPathDriver<'static, CuratedMenuShellState> {
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        CuratedMenuShellState { app, views, intents: PendingUiIntents::new(), last_err: None },
        ViewportMetrics::new(1440.0, 820.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.intents.merge_pass(pass_intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_err = Some(err.to_string());
            } else {
                st.last_err = None;
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn collect_menu_ids(sections: &[curvebook_ui_egui::CuratedMenuSection]) -> Vec<&'static str> {
    let mut ids = Vec::new();
    fn visit(nodes: &[CuratedMenuNode], out: &mut Vec<&'static str>) {
        for node in nodes {
            match node {
                CuratedMenuNode::Action(tip) => out.push(tip.command_id),
                CuratedMenuNode::Submenu { children, .. } => visit(children, out),
                CuratedMenuNode::Separator => {}
            }
        }
    }
    for sec in sections {
        visit(&sec.nodes, &mut ids);
    }
    ids
}

fn check_menu_container(nodes: &[CuratedMenuNode], locale: &str, path: &str, global_cmds: &mut HashSet<&'static str>) -> Result<(), String> {
    let mut container_labels = HashSet::new();
    for node in nodes {
        match node {
            CuratedMenuNode::Separator => {}
            CuratedMenuNode::Action(tip) => {
                if !global_cmds.insert(tip.command_id) {
                    return Err(format!("duplicate command_id `{}` in curated menu bar ({locale}) at `{path}`", tip.command_id));
                }
                if !container_labels.insert(tip.localized_label.to_owned()) {
                    return Err(format!("duplicate label `{}` in `{path}` ({locale})", tip.localized_label));
                }
                if locale == "en" {
                    for forbidden in ["Dialog", "Archetype", "Wizard", "QOL-", "R15"] {
                        if tip.label.contains(forbidden) || tip.localized_label.contains(forbidden) {
                            return Err(format!("internal jargon `{forbidden}` leaked into English label `{}`", tip.localized_label));
                        }
                    }
                }
            }
            CuratedMenuNode::Submenu { id, title, children, .. } => {
                if !container_labels.insert(title.clone()) {
                    return Err(format!("duplicate submenu title `{title}` in `{path}` ({locale})"));
                }
                if locale == "en" {
                    for forbidden in ["Dialog", "Archetype", "Wizard", "QOL-", "R15"] {
                        if title.contains(forbidden) {
                            return Err(format!("internal jargon `{forbidden}` leaked into English submenu `{title}` ({id})"));
                        }
                    }
                }
                check_menu_container(children, locale, &format!("{path}/{id}"), global_cmds)?;
            }
        }
    }
    Ok(())
}

#[test]
fn r15_8_curated_menu_bar_hygiene_and_parity() -> Result<(), String> {
    let expected_keys = ["menu.file", "menu.edit", "menu.view", "menu.insert", "menu.data", "menu.analyze", "menu.graph", "menu.help"];

    for &loc in BETA_ACTIVE_LOCALES {
        let mut app = UiApp::new();
        app.locale = loc.to_owned();
        let sections = curated_menu_bar(&app);
        let keys: Vec<&str> = sections.iter().map(|s| s.menu_key).collect();
        assert_eq!(keys, expected_keys, "locale `{loc}` must keep canonical 8-menu order");

        let mut seen_cmds = HashSet::new();
        for sec in &sections {
            let direct_entries = sec.nodes.iter().filter(|n| !matches!(n, CuratedMenuNode::Separator)).count();
            assert!(direct_entries <= 15, "locale `{loc}` section `{}` has {direct_entries} > 15 top-level entries", sec.menu_key);
            check_menu_container(&sec.nodes, loc, sec.menu_key, &mut seen_cmds)?;
        }

        let flat_sections = menu_bar_tooltips(&app);
        let flat_keys: Vec<&str> = flat_sections.iter().map(|s| s.menu_key).collect();
        assert_eq!(flat_keys, expected_keys);
        let curated_ids = collect_menu_ids(&sections);
        let tooltip_ids: Vec<&str> = flat_sections.iter().flat_map(|s| s.items.iter().map(|it| it.command_id)).collect();
        assert_eq!(tooltip_ids, curated_ids, "menu_bar_tooltips must mirror curated_menu_bar in locale `{loc}`");
    }

    // Exercise nested submenus in the live egui shell and legacy `ui.invoke_menu` fallback.
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-menu", "kind": "column", "cols": 2, "rows": 4 })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCell", json!({ "sheet_id": "ds-menu", "row": 0, "col": 0, "value": 12.0 })).map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "gs-menu", "data_sheet_id": "ds-menu" })).map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "ls-menu", "title": "Menu Layout" })).map_err(|e| e.to_string())?;
    app.sync_views();

    // Verify `ui.invoke_menu` resolves nested curated paths, flat paths, and non-curated command paths.
    let nested =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": "File/Import/Plate / Tidy Long Import..." })).map_err(|e| e.to_string())?;
    assert_eq!(nested["command_id"], "ui.import.reshape.open");
    app.canvas_dialogs.active_dialog = curvebook_ui_egui::ActiveDialog::None;

    let flat = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": "File/Import Reshape..." })).map_err(|e| e.to_string())?;
    assert_eq!(flat["command_id"], "ui.import.reshape.open");
    app.canvas_dialogs.active_dialog = curvebook_ui_egui::ActiveDialog::None;

    let legacy_ui =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": "Data/Convert Table Archetype..." })).map_err(|e| e.to_string())?;
    assert_eq!(legacy_ui["command_id"], "ui.table.convertArchetype.open");
    app.canvas_dialogs.active_dialog = curvebook_ui_egui::ActiveDialog::None;

    let set_cell_spec = cmd::registry().get("table.setCell").ok_or("missing table.setCell spec")?;
    let curated_id_set: HashSet<&str> = collect_menu_ids(&curated_menu_bar(&app)).into_iter().collect();
    assert!(!curated_id_set.contains(set_cell_spec.id), "expected table.setCell to test non-curated registry fallback");
    let legacy_engine = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "path": set_cell_spec.menu_path, "params": { "sheet_id": "ds-menu", "row": 1, "col": 0, "value": 9.0 } }),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(legacy_engine["command_id"], "table.setCell");
    app.sync_views();

    // Hover nested submenus in EguiHumanPathDriver and verify submenu children appear.
    let mut driver = make_driver(app);

    driver.click_widget_id("menu.file")?;
    driver.hover_widget_id("menu.file.import")?;
    assert!(driver.frame_meta().find_by_widget_id("menu.ui.import.reshape.open").is_some(), "File > Import must surface menu.ui.import.reshape.open");

    driver.hover_widget_id("menu.file.export")?;
    assert!(driver.frame_meta().find_by_widget_id("menu.export.svg").is_some(), "File > Export must surface menu.export.svg");

    driver.hover_widget_id("menu.file.checkpoints")?;
    assert!(driver.frame_meta().find_by_widget_id("menu.project.listCheckpoints").is_some(), "File > Checkpoints must surface listCheckpoints");

    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    driver.click_widget_id("menu.graph")?;
    driver.hover_widget_id("menu.graph.layout")?;
    assert!(
        driver.frame_meta().find_by_widget_id("menu.ui.layout.composer.open").is_some(),
        "Graph > Layout must surface menu.ui.layout.composer.open"
    );
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().last_err, None);

    Ok(())
}
