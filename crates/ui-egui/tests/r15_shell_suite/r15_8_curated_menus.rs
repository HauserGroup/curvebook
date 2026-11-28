#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use curvebook_engine::cmd;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::i18n::BETA_ACTIVE_LOCALES;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{CuratedMenuNode, UiApp, curated_menu_bar, dispatch_gui_action, menu_bar_tooltips};
use serde_json::json;

use crate::common::{ShellTestState, make_shell_driver_with_state};

fn collect_menu_ids(sections: &[curvebook_ui_egui::CuratedMenuSection]) -> Vec<&'static str> {
    fn visit(nodes: &[CuratedMenuNode], out: &mut Vec<&'static str>) {
        for node in nodes {
            match node {
                CuratedMenuNode::Action(tip) => out.push(tip.command_id),
                CuratedMenuNode::Submenu { children, .. } => visit(children, out),
                CuratedMenuNode::Separator => {}
            }
        }
    }
    let mut ids = Vec::new();
    for sec in sections {
        visit(&sec.nodes, &mut ids);
    }
    ids
}

fn check_menu_container(nodes: &[CuratedMenuNode], locale: &str, path: &str, global_cmds: &mut HashSet<&'static str>) -> Result<(), String> {
    const FORBIDDEN: &[&str] = &["Dialog", "Archetype", "Wizard", "QOL-", "R15"];
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
                if locale == "en"
                    && let Some(bad) = FORBIDDEN.iter().find(|f| tip.label.contains(**f) || tip.localized_label.contains(**f))
                {
                    return Err(format!("internal jargon `{bad}` leaked into English label `{}`", tip.localized_label));
                }
                if tip.localized_label.ends_with("...")
                    || tip.localized_label.ends_with('…')
                    || tip.label.ends_with("...")
                    || tip.label.ends_with('…')
                {
                    return Err(format!(
                        "R20.15 violation: menu item `{}` in `{locale}` retained trailing ellipsis (`{}` / `{}`)",
                        tip.command_id, tip.localized_label, tip.label
                    ));
                }
            }
            CuratedMenuNode::Submenu { id, title, children, .. } => {
                if !container_labels.insert(title.clone()) {
                    return Err(format!("duplicate submenu title `{title}` in `{path}` ({locale})"));
                }
                if locale == "en"
                    && let Some(bad) = FORBIDDEN.iter().find(|f| title.contains(**f))
                {
                    return Err(format!("internal jargon `{bad}` leaked into English submenu `{title}` ({id})"));
                }
                check_menu_container(children, locale, &format!("{path}/{id}"), global_cmds)?;
            }
        }
    }
    Ok(())
}

#[test]
fn r15_8_curated_menu_bar_hygiene_and_parity() -> Result<(), String> {
    const EXPECTED_KEYS: &[&str] = &["menu.file", "menu.edit", "menu.view", "menu.insert", "menu.data", "menu.analyze", "menu.graph", "menu.help"];

    for &loc in BETA_ACTIVE_LOCALES {
        let mut app = UiApp::new();
        app.locale = loc.to_owned();
        let sections = curated_menu_bar(&app);
        assert_eq!(sections.iter().map(|s| s.menu_key).collect::<Vec<_>>(), EXPECTED_KEYS);
        let mut seen_cmds = HashSet::new();
        for sec in &sections {
            let direct_entries = sec.nodes.iter().filter(|n| !matches!(n, CuratedMenuNode::Separator)).count();
            assert!(direct_entries <= 15, "locale `{loc}` section `{}` has {direct_entries} > 15 top-level entries", sec.menu_key);
            check_menu_container(&sec.nodes, loc, sec.menu_key, &mut seen_cmds)?;
        }
        let flat_sections = menu_bar_tooltips(&app);
        assert_eq!(flat_sections.iter().map(|s| s.menu_key).collect::<Vec<_>>(), EXPECTED_KEYS);
        let tooltip_ids: Vec<&str> = flat_sections.iter().flat_map(|s| s.items.iter().map(|it| it.command_id)).collect();
        assert_eq!(tooltip_ids, collect_menu_ids(&sections), "menu_bar_tooltips must mirror curated_menu_bar in locale `{loc}`");
    }

    let mut app = UiApp::new();
    for (cmd_id, params) in [
        ("sheet.createData", json!({ "sheet_id": "ds-menu", "kind": "column", "cols": 2, "rows": 4 })),
        ("table.setCell", json!({ "sheet_id": "ds-menu", "row": 0, "col": 0, "value": 12.0 })),
        ("graph.create", json!({ "sheet_id": "gs-menu", "data_sheet_id": "ds-menu" })),
        ("layout.create", json!({ "sheet_id": "ls-menu", "title": "Menu Layout" })),
    ] {
        app.execute_command(cmd_id, params).map_err(|e| e.to_string())?;
    }
    app.sync_views();

    for (path, expected_cmd) in [
        ("File/Import/Plate / Tidy Long Import", "ui.import.reshape.open"),
        ("File/Import Reshape", "ui.import.reshape.open"),
        ("Data/Convert Table Format", "ui.table.convertArchetype.open"),
    ] {
        let res = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": path })).map_err(|e| e.to_string())?;
        assert_eq!(res["command_id"], expected_cmd);
        app.canvas_dialogs.active_dialog = curvebook_ui_egui::ActiveDialog::None;
    }

    let set_cell_spec = cmd::registry().get("table.setCell").ok_or("missing table.setCell spec")?;
    let curated_id_set: HashSet<&str> = collect_menu_ids(&curated_menu_bar(&app)).into_iter().collect();
    assert!(!curated_id_set.contains(set_cell_spec.id));
    let legacy_engine = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "path": set_cell_spec.menu_path, "params": { "sheet_id": "ds-menu", "row": 1, "col": 0, "value": 9.0 } }),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(legacy_engine["command_id"], "table.setCell");
    app.sync_views();

    assert_in_window_menu_popups(app)?;
    assert_os_adaptive_menu_placement()
}

fn assert_in_window_menu_popups(app: UiApp) -> Result<(), String> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(1440.0, 820.0, 1.0), tokens);
    for (top, submenus) in [
        (
            "menu.file",
            &[
                ("menu.file.import", "menu.ui.import.reshape.open"),
                ("menu.file.export", "menu.ui.export.figure.open"),
                ("menu.file.checkpoints", "menu.project.listCheckpoints"),
            ][..],
        ),
        ("menu.graph", &[("menu.graph.layout", "menu.ui.layout.composer.open")][..]),
    ] {
        driver.click_widget_id(top)?;
        if top == "menu.file" {
            assert!(driver.frame_meta().find_by_widget_id("menu.project.saveTemplate").is_some(), "File menu missing Save as Template");
        }
        for &(sub, child) in submenus {
            driver.hover_widget_id(sub)?;
            assert!(driver.frame_meta().find_by_widget_id(child).is_some(), "{sub} must surface {child}");
            if sub == "menu.file.export" {
                for removed in ["menu.export.pdf", "menu.export.png", "menu.export.svg", "menu.export.tiff"] {
                    assert!(driver.frame_meta().find_by_widget_id(removed).is_none(), "File > Export must not expose `{removed}`");
                }
            }
        }
        driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    }

    driver.click_widget_id("menu.view")?;
    let gallery_entry = driver.frame_meta().find_by_widget_id("menu.ui.gallery.toggle").ok_or("View > Gallery missing")?;
    assert_eq!(gallery_entry.label, "Gallery");
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);

    driver.click_widget_id("menu.data")?;
    let rand_entry = driver.frame_meta().find_by_widget_id("menu.ui.table.randomizeGroups").ok_or("Data > Randomize Groups missing")?;
    let sample_entry = driver.frame_meta().find_by_widget_id("menu.ui.table.randomSampleRows").ok_or("Data > Sample Rows missing")?;
    assert_eq!((rand_entry.label.as_str(), sample_entry.label.as_str()), ("Randomize Groups", "Sample Rows"));
    assert!(rand_entry.enabled && sample_entry.enabled);
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().last_intent_error, None);
    Ok(())
}

fn assert_os_adaptive_menu_placement() -> Result<(), String> {
    // OS-adaptive placement & dot-free "Settings" across all 16 locales.
    for &loc in curvebook_ui_egui::i18n::LOCALES {
        let mut win_app = UiApp::new();
        win_app.locale = loc.to_owned();
        win_app.host_draws_menu_bar = false;
        let win_sections = curated_menu_bar(&win_app);
        let win_ids = collect_menu_ids(&win_sections);
        for required in ["app.quit", "app.preferences", "app.about", "window.close", "ui.zoom.in", "ui.zoom.out", "ui.zoom.fitSheet"] {
            assert!(win_ids.contains(&required), "in-window menu bar ({loc}) missing `{required}`");
        }
        let settings_item = win_sections
            .iter()
            .find(|s| s.menu_key == "menu.edit")
            .and_then(|s| s.flat_actions().into_iter().find(|i| i.command_id == "app.preferences"))
            .ok_or("missing app.preferences")?;
        assert!(
            !settings_item.localized_label.contains("...") && !settings_item.localized_label.contains('…'),
            "locale `{loc}` Settings item retained ellipsis: `{}`",
            settings_item.localized_label
        );
        if loc == "en" {
            assert_eq!(settings_item.localized_label, "Settings");
        }

        let mut mac_app = UiApp::new();
        mac_app.locale = loc.to_owned();
        mac_app.host_draws_menu_bar = true;
        let mac_sections = curated_menu_bar(&mac_app);
        let mac_ids = collect_menu_ids(&mac_sections);
        for omitted in ["app.quit", "app.preferences", "app.about"] {
            assert!(!mac_ids.contains(&omitted), "native host menu bar ({loc}) must omit `{omitted}`");
        }
        assert!(mac_ids.contains(&"window.close"), "native host menu bar ({loc}) must retain File > Close Window");
    }
    Ok(())
}
