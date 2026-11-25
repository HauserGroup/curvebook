#![forbid(unsafe_code)]

//! Wave B (`R15.6`) human-path regression tests for `B5` (`R15-GUI-19`), `B6` (`R15-GUI-20`),
//! and `B10` (`R15-GUI-14`, `R15-GUI-25`, `R15-GUI-26`).

use curvebook_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::egui_shell::UiIntent;
use curvebook_ui_egui::egui_shell::app_modals::PreferencesCategory;
use curvebook_ui_egui::grid_view::ImportOriginKind;
use egui::{Key, Modifiers};

use crate::common::{cell_number, commit_shell_intents, make_doc_driver, seed_column_doc};

fn build_3x6_column_doc() -> Result<ProjectDocument, String> {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).map_err(|e| e.to_string())?;
    for (c, title) in ["Col A", "Col B", "Col C"].into_iter().enumerate() {
        col.data_sets[c].title = title.to_owned();
        for r in 0..6 {
            col.data_sets[c].cells[r][0] = Cell::number(((r + 1) * 10 + (c + 1)) as f64);
        }
    }
    let mut doc = ProjectDocument::new("Discontiguous Cut Regression").map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-discontig", "Discontiguous Table", TableArchetype::Column(col)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-discontig"));
    Ok(doc)
}

fn build_two_sheets_and_graph_doc() -> Result<ProjectDocument, String> {
    let (mut doc, _) = seed_column_doc(
        "Import Target Separation Regression",
        "ds-a",
        "Sheet A",
        ColumnDesignHint::UnpairedReplicates,
        &[("Ctrl", &[10.0, 20.0, 30.0]), ("Drug", &[40.0, 50.0, 60.0])],
    )?;
    let col_b = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
    let ds_a = doc.data_sheet(&SheetId::new("ds-a")).ok_or("missing ds-a")?.clone();
    doc.add_data_sheet(DataSheet::new("ds-b", "Sheet B", TableArchetype::Column(col_b)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-1", &ds_a).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(doc)
}

#[test]
fn b5_r15_gui_19_large_2x5000_selection_copies_and_cuts_all_10000_cells_without_truncation() -> Result<(), String> {
    let rows = 5_000usize;
    let a: Vec<f64> = (1..=rows).map(|r| r as f64).collect();
    let b: Vec<f64> = (1..=rows).map(|r| (10_000 + r) as f64).collect();
    let (doc, _) =
        seed_column_doc("Large Clipboard", "ds-large", "Large Table", ColumnDesignHint::UnpairedReplicates, &[("Group A", &a), ("Group B", &b)])?;
    let mut driver = make_doc_driver(doc, 1280.0, 800.0);

    driver.click_widget_id("grid.cell.0.0.0")?;
    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-large")).ok_or("missing ds-large")?.clone();
    driver.state_mut().app.grid.extend_shift_range(&ds, GridCellCoord::new(rows - 1, 1, 0));
    driver.run_settled();

    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    let copied = driver.state().app.interaction.last_clipboard_text.clone().ok_or("missing clipboard text")?;
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!((lines.len(), lines.first().copied(), lines.last().copied()), (rows, Some("1\t10001"), Some("5000\t15000")));

    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-large")).ok_or("missing ds-large")?;
    for (r, c) in [(0, 0), (2_500, 1), (rows - 1, 0), (rows - 1, 1)] {
        assert!(ds.cell(CellCoord::new(r, c, 0)).is_some_and(Cell::is_empty), "({r},{c}) must be cleared");
    }
    Ok(())
}

#[test]
fn b5_r15_gui_19_discontiguous_cut_clears_only_selected_cells_and_leaves_b2_intact() -> Result<(), String> {
    let mut driver = make_doc_driver(build_3x6_column_doc()?, 1280.0, 800.0);
    driver.click_widget_id("grid.cell.0.0.0")?;
    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-discontig")).ok_or("missing ds-discontig")?.clone();
    driver.state_mut().app.grid.toggle_cmd_selection(&ds, GridCellCoord::new(4, 2, 0));
    driver.run_settled();

    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    let copied = driver.state().app.interaction.last_clipboard_text.clone().ok_or("missing clipboard")?;
    assert!(copied.contains("11") && copied.contains("53") && !copied.contains("22"));

    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-discontig")).ok_or("missing ds-discontig")?;
    assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty) && ds.cell(CellCoord::new(4, 2, 0)).is_some_and(Cell::is_empty));
    assert_eq!((cell_number(ds, 1, 1), cell_number(ds, 0, 2), cell_number(ds, 4, 0)), (Some(22.0), Some(13.0), Some(51.0)));
    Ok(())
}

#[test]
fn b6_r15_gui_20_paste_link_explicit_target_and_graph_active_no_self_link() -> Result<(), String> {
    for (active_sheet, target, expect_ok, err_sub) in
        [("ds-a", Some("ds-b"), true, ""), ("graph-1", None, false, ""), ("ds-a", Some("ds-a"), false, "onto itself")]
    {
        let mut doc = build_two_sheets_and_graph_doc()?;
        doc.active_sheet_id = Some(SheetId::new(active_sheet));
        let mut driver = make_doc_driver(doc, 1280.0, 800.0);
        driver.state_mut().intents.push(UiIntent::OpenImportWizard {
            origin: ImportOriginKind::InternalDataSheet,
            is_live_link: true,
            target_sheet_id: target.map(SheetId::new),
        });
        commit_shell_intents(&mut driver)?;
        assert_eq!(driver.state().app.grid.import_wizard.target_sheet_id, target.map(SheetId::new));
        assert_eq!(driver.state().app.grid.import_wizard.source_sheet_id, None);

        driver.click_widget_id("import_paste.source_sheet.ds-a")?;
        driver.click_widget_id("modal.ok")?;
        let ds_a = driver.state().app.document().data_sheet(&SheetId::new("ds-a")).ok_or("missing ds-a")?;
        assert!(ds_a.linked_regions.is_empty());
        if expect_ok {
            assert!(driver.state().last_intent_error.is_none() && !driver.state().app.grid.import_wizard.open);
            let ds_b = driver.state().app.document().data_sheet(&SheetId::new("ds-b")).ok_or("missing ds-b")?;
            assert_eq!((ds_b.linked_regions.len(), cell_number(ds_b, 0, 0)), (1, Some(10.0)));
        } else {
            let err = driver.state().last_intent_error.as_deref().unwrap_or("");
            assert!(driver.state().last_intent_error.is_some() && err.contains(err_sub) && driver.state().app.grid.import_wizard.open);
            assert_eq!(cell_number(ds_a, 0, 0), Some(10.0));
        }
    }
    Ok(())
}

#[test]
fn b10_r15_gui_14_welcome_hub_open_pane_uses_curvebook_extension_wording() -> Result<(), String> {
    let mut driver = make_doc_driver(ProjectDocument::new("Welcome Hub Extension").map_err(|e| e.to_string())?, 1280.0, 800.0);
    driver.state_mut().app.egui_views.launcher_open = true;
    driver.state_mut().views.launcher_open = true;
    driver.run_settled();
    driver.click_widget_id("welcome.action.open_existing")?;

    for wid in ["welcome.open.subtitle", "welcome.preview.pipeline"] {
        let lbl = &driver.frame_meta().find_by_widget_id(wid).ok_or(wid)?.label;
        assert!(lbl.contains(".curvebook") && !lbl.contains(".cbk"), "{wid}: {lbl:?}");
    }
    for entry in &driver.frame_meta().entries {
        assert!(!entry.label.contains(".cbk") && !entry.value.as_ref().and_then(serde_json::Value::as_str).is_some_and(|v| v.contains(".cbk")));
    }
    for locale in ["de", "ja"] {
        assert_eq!(driver.state_mut().app.set_locale(locale), locale);
        driver.run_settled();
        let sub = &driver.frame_meta().find_by_widget_id("welcome.open.subtitle").ok_or("missing subtitle")?.label;
        assert!(sub.contains(".curvebook") && !sub.contains(".cbk"), "{locale}: {sub:?}");
    }
    Ok(())
}

#[test]
fn b10_r15_gui_25_preferences_shortcut_cancel_restores_and_resets_draft_while_ok_applies() -> Result<(), String> {
    let mut driver = make_doc_driver(build_3x6_column_doc()?, 1280.0, 800.0);
    let orig = driver.state().app.shortcuts.active_combo("project.save").ok_or("missing shortcut")?.to_owned();

    driver.state_mut().intents.push(UiIntent::OpenPreferences { category: Some(PreferencesCategory::KeyboardShortcuts) });
    commit_shell_intents(&mut driver)?;
    assert!(driver.frame_meta().find_by_widget_id("edit.tab.prefs.graph_defaults").is_none());

    driver.click_widget_id("prefs.shortcuts.search")?;
    driver.type_text("Save")?;
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Y");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    assert!(driver.state().views.app_modals.preferences.shortcut_conflict_error.is_none());
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some(orig.as_str()));

    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some(orig.as_str()));
    assert!(driver.state().app.egui_views.app_modals.preferences.staged_shortcuts.is_none());
    assert!(driver.state().app.egui_views.app_modals.preferences.shortcut_combo_draft.is_empty());

    driver.state_mut().intents.push(UiIntent::OpenPreferences { category: Some(PreferencesCategory::KeyboardShortcuts) });
    commit_shell_intents(&mut driver)?;
    assert!(driver.state().views.app_modals.preferences.shortcut_combo_draft.is_empty());
    assert_eq!(
        driver.state().views.app_modals.preferences.staged_shortcuts.as_ref().and_then(|s| s.active_combo("project.save")),
        Some(orig.as_str())
    );
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Y");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some("Ctrl+Alt+Y"));
    Ok(())
}
