#![forbid(unsafe_code)]
//! Smoke tests verifying the view shard dispatch infrastructure, `i18n` shard infrastructure,
//! and pure-`egui` `human_wf_common` harness (`R15.5`).

mod human_wf_common;

use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, all_keys, missing_translations, try_tr};
use curvebook_ui_egui::interaction::KeyModifiers;
use curvebook_ui_egui::views::{ViewShardStates, dispatch_custom_view_action};

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
fn view_shards_and_custom_action_dispatch_smoke() -> Result<(), String> {
    let mut app = make_smoke_app()?;
    let mut shards = ViewShardStates::default();

    let domains = [
        "a11y",
        "welcome",
        "nav_notes",
        "toolbar_menus",
        "grid_core",
        "mv_formula",
        "results",
        "analyze_picker",
        "stat_params",
        "fit_params",
        "power_wizard",
        "dialogs_graph_misc",
        "import_paste",
        "inspector",
    ];
    for domain in domains {
        let out = dispatch_custom_view_action(&mut app, &mut shards, domain, "smoke")?;
        assert!(!out.handled);
    }

    let err = dispatch_custom_view_action(&mut app, &mut shards, "nonexistent_domain", "payload");
    assert!(err.is_err(), "unknown custom view action domain must return Err");

    assert!(EXTRA_SHARD_CATALOGS.is_empty());
    assert_eq!(all_keys().count(), 708);
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
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, human_wf_common::default_viewport());

    human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;
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

    human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;
    human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}
