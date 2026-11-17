#![forbid(unsafe_code)]
//! Smoke tests verifying the `R10` Wave 5 scaffold (`views/` 14-module split, `ViewShardStates`,
//! `dispatch_custom_view_action`, `i18n` shard infrastructure, and `human_wf_common` harness).

mod human_wf_common;

use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, all_keys, missing_translations, try_tr};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics, execute_widget_action};
use curvebook_ui_egui::layout::a11y::{FocusScope, WidgetRole};
use curvebook_ui_egui::layout::{WidgetAction, WidgetId};
use curvebook_ui_egui::views::{ViewShardStates, a11y::validate_interactive_a11y, dispatch_custom_view_action};

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
        let action_out = execute_widget_action(
            &mut app,
            &WidgetAction::Custom { domain: domain.to_owned(), payload: String::from("smoke") },
            ViewportMetrics::default(),
        )?;
        assert!(!action_out.handled);
    }

    let err = dispatch_custom_view_action(&mut app, &mut shards, "nonexistent_domain", "payload");
    assert!(err.is_err(), "unknown custom view action domain must return Err");
    let action_err = execute_widget_action(
        &mut app,
        &WidgetAction::Custom { domain: String::from("nonexistent_domain"), payload: String::from("payload") },
        ViewportMetrics::default(),
    );
    assert!(action_err.is_err(), "execute_widget_action with unknown Custom domain must return Err");

    let valid_meta = curvebook_ui_egui::views::a11y::labeled_meta(WidgetRole::Button, "Run", FocusScope::Workspace, "Enter");
    validate_interactive_a11y(&valid_meta)?;

    assert_eq!(EXTRA_SHARD_CATALOGS.len(), 16);
    assert!(all_keys().count() >= 653);
    assert!(missing_translations().is_empty());
    for &locale in LOCALES {
        assert!(try_tr(locale, "menu.file").is_some(), "missing menu.file for `{locale}`");
    }
    Ok(())
}

#[test]
fn human_wf_common_helpers_drive_ui_with_zero_shortcuts() -> Result<(), String> {
    let app = make_smoke_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let report = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    assert!(report.checked_nodes > 0);

    let palette_btn = WidgetId::from("toolbar.palette");
    let _ = human_wf_common::hover_widget_center(&mut driver, &palette_btn)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &palette_btn)?;
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);

    let query_id = WidgetId::from("palette.query");
    let _ = human_wf_common::click_and_type(&mut driver, &query_id, "ANO")?;
    let _ = human_wf_common::type_into_focused(&mut driver, "VA")?;
    assert_eq!(driver.app().canvas_dialogs.command_palette.query, "ANOVA");

    let _ = human_wf_common::press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    let _ = human_wf_common::press_cmd_shortcut(&mut driver, "k")?;
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
    let _ = human_wf_common::press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;

    let analyze_btn = WidgetId::from("toolbar.analyze");
    let _ = human_wf_common::click_widget_center(&mut driver, &analyze_btn)?;
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));

    let list_id = WidgetId::from("analyze.entries_list");
    let anova_id = WidgetId::from("analyze.entry.stats.one_way_anova");
    human_wf_common::scroll_until_widget_visible(&mut driver, &list_id, &anova_id, -180.0, 6)?;
    let _ = human_wf_common::scroll_widget_wheel(&mut driver, &list_id, 0.0, 0.0)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &anova_id)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    assert_eq!(driver.app().document().result_sheets.len(), 1);

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}
