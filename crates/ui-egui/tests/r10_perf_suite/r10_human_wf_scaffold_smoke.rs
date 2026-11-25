#![forbid(unsafe_code)]
//! Smoke tests verifying typed `UiIntent` dispatch, `i18n` catalog integrity,
//! and pure-`egui` `human_wf_common` harness (`R15.5`).

use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::{PendingUiIntents, UiIntent};
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, all_keys, missing_translations, try_tr};
use curvebook_ui_egui::interaction::KeyModifiers;

fn make_smoke_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Baseline");
    col.data_sets[1].title = String::from("Treated");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64));
        col.data_sets[1].cells[r][0] = Cell::number(15.0 + (r as f64));
    }
    let sheet = DataSheet::new("smoke-col", "Smoke Assay", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(sheet).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("smoke-col")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn typed_ui_intents_and_i18n_catalog_smoke() -> Result<(), String> {
    let mut app = make_smoke_app()?;
    let mut intents = PendingUiIntents::new();
    intents.push(UiIntent::SetRowTitlesCollapsed { collapsed: true });
    let batch = intents.commit_all_if_settled(&mut app, false);
    assert_eq!(batch.results.len(), 1);
    assert!(batch.results.first().is_some_and(Result::is_ok));
    assert_eq!(app.grid.row_titles_collapsed_override, Some(true));

    assert!(EXTRA_SHARD_CATALOGS.is_empty());
    assert_eq!(all_keys().count(), 791);
    assert!(missing_translations().is_empty());
    for &locale in LOCALES {
        assert!(try_tr(locale, "menu.file").is_some(), "missing menu.file for `{locale}`");
    }
    Ok(())
}

#[test]
fn human_wf_common_helpers_drive_ui_with_zero_shortcuts() -> Result<(), String> {
    let mut app = make_smoke_app()?;
    app.nav.welcome.open = false;
    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(app, crate::human_wf_common::default_viewport());

    crate::human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;
    harness.driver.hover_widget_id("toolbar.command_palette")?;
    harness.click("toolbar.command_palette")?;
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);

    harness.click("palette.query")?;
    harness.driver.type_text("ANOVA")?;
    assert_eq!(harness.driver.state().views.command_palette.query, "ANOVA");

    harness.driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    harness.driver.press_key("k", KeyModifiers { meta: true, ..Default::default() })?;
    assert_eq!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    harness.driver.press_key("Escape", KeyModifiers::default())?;

    let rs_id = harness.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    assert_eq!(harness.app().document().result_sheets.len(), 1);
    assert_eq!(harness.app().document().result_sheets[0].id, rs_id);

    crate::human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;
    crate::human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}
