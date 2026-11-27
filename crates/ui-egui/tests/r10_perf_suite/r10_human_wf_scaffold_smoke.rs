#![forbid(unsafe_code)]
//! Smoke tests verifying typed `UiIntent` dispatch, `i18n` catalog integrity,
//! and pure-`egui` `human_wf_common` harness (`R15.5`).

use crate::common::{TestRes, seed_column_doc};
use crate::human_wf_common::{EguiWfHarness, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport};
use crate::suite_helpers::new_shell_app;
use curvebook_doc::ColumnDesignHint;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::{PendingUiIntents, UiIntent};
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, all_keys, missing_translations, try_tr};
use curvebook_ui_egui::interaction::KeyModifiers;

fn make_smoke_app() -> Result<UiApp, String> {
    let (doc, _) = seed_column_doc(
        "Smoke",
        "smoke-col",
        "Smoke Assay",
        ColumnDesignHint::UnpairedReplicates,
        &[("Baseline", &[10.0, 11.0, 12.0, 13.0]), ("Treated", &[15.0, 16.0, 17.0, 18.0])],
    )?;
    Ok(new_shell_app(doc))
}

#[test]
fn typed_ui_intents_and_i18n_catalog_smoke() -> TestRes {
    let mut app = make_smoke_app()?;
    let mut intents = PendingUiIntents::new();
    intents.push(UiIntent::SetRowTitlesCollapsed { collapsed: true });
    let batch = intents.commit_all_if_settled(&mut app, false);
    assert!(batch.results.len() == 1 && batch.results[0].is_ok() && app.grid.row_titles_collapsed_override == Some(true));

    assert!(EXTRA_SHARD_CATALOGS.is_empty() && all_keys().count() == 992 && missing_translations().is_empty());
    assert!(LOCALES.iter().all(|loc| try_tr(loc, "menu.file").is_some()));
    Ok(())
}

#[test]
fn human_wf_common_helpers_drive_ui_with_zero_shortcuts() -> TestRes {
    let mut h = EguiWfHarness::from_app(make_smoke_app()?, default_viewport());
    assert_zero_shortcut_compliance(&mut h)?;
    h.driver.hover_widget_id("toolbar.command_palette")?;
    h.click("toolbar.command_palette")?;
    assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    h.click("palette.query")?;
    h.driver.type_text("ANOVA")?;
    assert_eq!(h.driver.state().views.command_palette.query, "ANOVA");
    h.driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    h.driver.press_key("k", KeyModifiers { meta: true, ..Default::default() })?;
    assert_eq!(h.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    h.driver.press_key("Escape", KeyModifiers::default())?;

    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    assert_eq!((h.app().document().result_sheets.len(), &h.app().document().result_sheets[0].id), (1, &rs_id));
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}
