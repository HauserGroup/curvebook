#![forbid(unsafe_code)]

//! R15.3 Integration & Pixel-Parity Tests: 3-Pane Welcome & Workspace Hub + Leading Dock
//! (`Navigator` & Scoped `Family` Lineage Tree).

use curvebook_doc::{ColumnDesignHint, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TableKind, VariableRole, XColumnMode};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver};
use curvebook_ui_egui::egui_shell::leading_dock::NavigatorKindChip;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::{ActiveWorkspacePane, NavigatorNestingMode};
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use egui::{Key, Modifiers};
use serde_json::json;

use crate::common::{ShellTestState, assert_clean_parity, make_shell_driver, widget_center};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

fn make_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_shell_driver(app, ViewportMetrics::new(w, h, 1.0))
}

fn create_pipeline_fixture_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "data-1", "title": "Kinetic Dose Curve", "kind": "xy", "cols": 2, "rows": 5, "auto_graph": true, "auto_info": true }),
    )?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "data-1", "values": [1.0, 2.0, 3.0, 4.0, 5.0] }))?;
    let d1_edits: Vec<_> = [(0, [10.0, 25.0, 48.0, 76.0, 92.0]), (1, [12.0, 22.0, 41.0, 69.0, 88.0])]
        .into_iter()
        .flat_map(|(col, vals)| vals.into_iter().enumerate().map(move |(row, value)| json!({ "row": row, "col": col, "value": value })))
        .collect();
    app.execute_command("table.setCells", json!({ "sheet_id": "data-1", "edits": d1_edits }))?;

    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "data-2", "title": "Validation Cohort", "kind": "xy", "cols": 2, "rows": 4, "auto_graph": false, "auto_info": false }),
    )?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "data-2", "values": [1.0, 2.0, 3.0, 4.0] }))?;
    let d2_edits: Vec<_> =
        [14.0, 28.0, 44.0, 68.0].into_iter().enumerate().map(|(row, value)| json!({ "row": row, "col": 0, "value": value })).collect();
    app.execute_command("table.setCells", json!({ "sheet_id": "data-2", "edits": d2_edits }))?;

    app.execute_command("analysis.run", json!({ "source_sheet_id": "data-1", "sheet_id": "res-1", "kind": "linear_regression" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "graph-2", "data_sheet_id": "data-2", "kind": "xy", "title": "Cohort Plot" }))?;
    app.execute_command("layout.create", json!({ "sheet_id": "layout-1", "title": "Figure 1" }))?;
    if let Some(first_graph_id) = app.document().graph_sheets.first().map(|g| g.id.as_str().to_owned()) {
        app.execute_command("layout.addPanel", json!({ "sheet_id": "layout-1", "graph_sheet_id": first_graph_id }))?;
        app.execute_command(
            "layout.addPanel",
            json!({ "sheet_id": "layout-1", "graph_sheet_id": "graph-2", "bounds_pt": [260.0, 20.0, 200.0, 140.0] }),
        )?;
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "data-1", "mode": "navigator" }))?;
    app.sync_views();
    Ok(app)
}

#[test]
fn welcome_hub_empty_project_rules_and_home_reopen_preserve_workspace() -> BoxRes {
    let empty_app = UiApp::new();
    assert_eq!(empty_app.document().total_sheet_count(), 0);

    let mut driver = make_driver(empty_app, 1440.0, 600.0);
    assert_clean_parity(&driver, "empty welcome 1440x600")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.btn.create").is_some());
    assert!(driver.frame_meta().find_by_widget_id("welcome.action.cancel").is_none());
    assert!(driver.state().leading_rect_pt.ok_or("leading dock missing")?.width() <= 38.0);

    driver.click_widget_id("welcome.btn.create")?;
    let sheets_before_home = driver.state().app.document().total_sheet_count();
    assert_eq!(sheets_before_home, 3);

    driver.click_widget_id("toolbar.home")?;
    assert!(driver.state().views.launcher_open && driver.frame_meta().find_by_widget_id("welcome.action.cancel").is_some());
    assert_clean_parity(&driver, "reopened welcome overlay")?;

    driver.click_widget_id("welcome.action.cancel")?;
    assert!(!driver.state().views.launcher_open);
    assert_eq!(driver.state().app.document().total_sheet_count(), sheets_before_home);
    assert!(driver.frame_meta().find_by_widget_id("welcome.btn.create").is_none());
    Ok(())
}

#[test]
fn welcome_hub_creates_all_eight_archetypes_with_conditional_schemas_single_commit() -> BoxRes {
    let mut driver = make_driver(UiApp::new(), 1440.0, 600.0);

    let title_center = widget_center(&driver, "welcome.input.title")?;
    driver.click_at(title_center)?;
    driver.type_text("PreCommitTitle")?;
    assert_eq!((driver.state().app.document().total_sheet_count(), driver.state().app.session().undo_depth()), (0, 0));

    let cases: &[(&str, &[&str], &[&str], TableKind)] = &[
        (
            "welcome.arch.xy",
            &["welcome.section.x_column", "welcome.section.y_format"],
            &["welcome.x_mode.numbers_error", "welcome.subcol.raw", "welcome.stepper.replicates.inc"],
            TableKind::Xy,
        ),
        ("welcome.arch.column", &["welcome.section.col_design"], &["welcome.design.paired"], TableKind::Column),
        ("welcome.arch.grouped", &[], &["welcome.subcol.mean_sem_n"], TableKind::Grouped),
        ("welcome.arch.contingency", &[], &[], TableKind::Contingency),
        (
            "welcome.arch.survival",
            &["welcome.survival.event_code.inc"],
            &["welcome.survival.event_code.inc", "welcome.survival.time_date"],
            TableKind::Survival,
        ),
        ("welcome.arch.parts_of_whole", &[], &[], TableKind::PartsOfWhole),
        ("welcome.arch.multiple_variables", &[], &["welcome.mv.role.0.Categorical", "welcome.mv.role.1.Label"], TableKind::MultipleVariables),
        ("welcome.arch.nested", &[], &["welcome.nested.depth.inc"], TableKind::Nested),
    ];

    for (idx, &(arch_wid, expect_present, clicks, expected_kind)) in cases.iter().enumerate() {
        if idx > 0 {
            driver.click_widget_id("toolbar.home")?;
        }
        driver.click_widget_id(arch_wid)?;
        for &wid in expect_present {
            assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "{arch_wid}: missing `{wid}`");
        }
        for &wid in clicks {
            driver.click_widget_id(wid)?;
        }
        let undo_before = driver.state().app.session().undo_depth();
        driver.click_widget_id("welcome.btn.create")?;
        if idx == 0 {
            assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
        }
        let ds = driver.state().app.document().data_sheets.last().ok_or("missing created sheet")?;
        assert_eq!(ds.archetype.kind(), expected_kind);
        match &ds.archetype {
            TableArchetype::Xy(t) => {
                assert_eq!(t.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN });
                assert_eq!(t.y_subcolumn_mode, SubcolumnMode::RawReplicates(4));
            }
            TableArchetype::Column(t) => assert_eq!(t.design_hint, ColumnDesignHint::PairedRepeatedMeasures),
            TableArchetype::MultipleVariables(t) => {
                assert_eq!((t.variables[0].role, t.variables[1].role), (VariableRole::Categorical, VariableRole::Label));
            }
            TableArchetype::Nested(t) => assert_eq!(t.groups[0].subcolumn_titles.len(), 5),
            _ => {}
        }
    }

    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.xy")?;
    driver.click_widget_id("welcome.entry_mode.sample")?;
    driver.click_widget_id("welcome.btn.create")?;
    match &driver.state().app.document().data_sheets.last().ok_or("missing sample sheet")?.archetype {
        TableArchetype::Xy(t) => assert!(!t.x_cells.is_empty() && t.x_cells[0][0].active_number().is_some()),
        other => return Err(format!("expected XY sample, got {:?}", other.kind()).into()),
    }
    Ok(())
}

#[test]
fn welcome_hub_starter_templates_clone_options_and_open_file() -> BoxRes {
    let mut driver = make_driver(UiApp::new(), 1440.0, 600.0);

    driver.click_widget_id("welcome.mode.template")?;
    driver.click_widget_id("welcome.template.1")?;
    driver.click_widget_id("welcome.btn.create")?;
    assert_eq!(
        (
            driver.state().app.document().data_sheets.len(),
            driver.state().app.document().graph_sheets.len(),
            driver.state().app.document().info_sheets.len()
        ),
        (1, 1, 1)
    );

    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.action.open_existing")?;
    let prev_req = driver.state().views.welcome_hub.open_file_request_count;
    driver.click_widget_id("welcome.btn.open_file")?;
    assert_eq!(driver.state().views.welcome_hub.open_file_request_count, prev_req + 1);
    driver.click_widget_id("welcome.action.cancel")?;
    drop(driver);

    let mut driver = make_driver(create_pipeline_fixture_app()?, 1440.0, 600.0);
    let (data_before, res_before) = (driver.state().app.document().data_sheets.len(), driver.state().app.document().result_sheets.len());

    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.mode.clone")?;
    driver.click_widget_id("welcome.clone.source.data-1")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.clone_modal.heading").is_some());
    assert_clean_parity(&driver, "clone options submodal")?;

    driver.click_widget_id("welcome.clone.clear_data")?;
    driver.click_widget_id("welcome.clone.rerun_analyses")?;
    driver.click_widget_id("welcome.clone_modal.apply")?;
    assert!(!driver.state().views.welcome_hub.clone_options_open);

    driver.click_widget_id("welcome.btn.create")?;
    let doc = driver.state().app.document();
    assert_eq!((doc.data_sheets.len(), doc.result_sheets.len()), (data_before + 1, res_before + 1));
    if let TableArchetype::Xy(t) = &doc.data_sheets.last().ok_or("cloned data missing")?.archetype {
        assert!(t.data_sets[0].cell(0, 0).is_none_or(|c| c.value.is_empty()));
    }
    assert_eq!(doc.result_sheets.last().ok_or("cloned result missing")?.status, ResultStatus::Dirty);
    Ok(())
}

#[test]
fn leading_dock_rail_collapse_filters_nested_toggle_and_inline_new() -> BoxRes {
    let mut driver = make_driver(create_pipeline_fixture_app()?, 1440.0, 600.0);
    assert_clean_parity(&driver, "expanded leading dock")?;
    assert!((driver.state().leading_rect_pt.ok_or("dock missing")?.width() - 240.0).abs() <= 4.0);

    driver.click_widget_id("nav.collapse_rail")?;
    assert!((driver.state().leading_rect_pt.ok_or("rail missing")?.width() - 36.0).abs() <= 2.0);
    assert_clean_parity(&driver, "collapsed 36pt rail")?;

    driver.press_egui_key(Key::N, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!((driver.state().leading_rect_pt.ok_or("restored missing")?.width() - 240.0).abs() <= 4.0);

    assert!(driver.state().app.family_open);
    driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!driver.state().app.family_open);
    driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(driver.state().app.family_open);

    driver.click_widget_id("nav.chip.results")?;
    assert_eq!(driver.state().views.leading_dock.kind_chip, NavigatorKindChip::Results);
    assert!(
        driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some() && driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_none()
    );
    driver.click_widget_id("nav.chip.all")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_some());

    driver.click_at(widget_center(&driver, "nav.search_input")?)?;
    driver.type_text("Validation")?;
    assert!(
        driver.frame_meta().find_by_widget_id("nav.sheet.data-2").is_some() && driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_none()
    );
    driver.click_widget_id("nav.search_clear")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_some());

    driver.click_widget_id("nav.view_mode.toggle")?;
    assert_eq!(driver.state().app.nav.nesting_mode, NavigatorNestingMode::NestedResultsUnderData);
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some());
    driver.click_widget_id("nav.view_mode.toggle")?;
    assert_eq!(driver.state().app.nav.nesting_mode, NavigatorNestingMode::SeparateFolders);

    let data_before = driver.state().app.document().data_sheets.len();
    driver.click_widget_id("nav.new.data")?;
    assert_eq!(driver.state().app.document().data_sheets.len(), data_before + 1);
    Ok(())
}

#[test]
fn leading_dock_f2_inline_rename_drag_reorder_and_result_to_data_retarget() -> BoxRes {
    let mut driver = make_driver(create_pipeline_fixture_app()?, 1440.0, 600.0);

    driver.click_widget_id("nav.sheet.data-1")?;
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::Navigator);
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    assert_eq!(driver.state().views.leading_dock.renaming_sheet_id, Some(SheetId::from("data-1")));

    for _ in 0..24 {
        driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    }
    driver.type_text("Kinetic Updated")?;
    assert_ne!(driver.state().app.document().data_sheets[0].title, "Kinetic Updated");

    let undo_before = driver.state().app.session().undo_depth();
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    assert_eq!(driver.state().app.document().data_sheets[0].title, "Kinetic Updated");

    driver.press_egui_key(Key::F2, Modifiers::NONE);
    driver.type_text("DiscardedDraft")?;
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.document().data_sheets[0].title, "Kinetic Updated");

    let (from, to) = (widget_center(&driver, "nav.sheet.data-1")?, widget_center(&driver, "nav.sheet.data-2")?);
    driver.drag(from, to)?;
    assert_eq!(
        (driver.state().app.document().data_sheets[0].id.as_str(), driver.state().app.document().data_sheets[1].id.as_str()),
        ("data-2", "data-1")
    );

    driver.click_widget_id("nav.section.info")?;
    let (res_from, ds2_to) = (widget_center(&driver, "nav.sheet.res-1")?, widget_center(&driver, "nav.sheet.data-2")?);
    driver.drag(res_from, ds2_to)?;
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ChangeDataAnalyzed { ref result_sheet_id } if result_sheet_id.as_str() == "res-1")
    );
    assert_eq!(driver.state().app.canvas_dialogs.change_data.selected_target_id.as_ref().map(|id| id.as_str()), Some("data-2"));
    Ok(())
}

#[test]
fn family_lineage_active_chain_navigation_and_hover_preview_450ms() -> BoxRes {
    let mut driver = make_driver(create_pipeline_fixture_app()?, 1440.0, 600.0);

    for wid in ["nav.splitter", "family.node.data-1", "family.node.res-1"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing `{wid}`");
    }
    driver.click_widget_id("family.node.res-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id, Some(SheetId::from("res-1")));

    let node_pos = widget_center(&driver, "family.node.res-1")?;
    driver.dispatch_events(vec![egui::Event::PointerMoved(node_pos)]);
    for _ in 0..10 {
        driver.dispatch_events(Vec::new());
    }
    assert!(driver.frame_meta().find_by_widget_id("family.hover_preview").is_none());

    driver.hover_widget_id("family.node.res-1")?;
    assert!(driver.frame_meta().find_by_widget_id("family.hover_preview").is_some());
    Ok(())
}

#[test]
fn responsive_viewports_and_all_14_beta_locales_zero_bleed_and_rasterizer() -> BoxRes {
    for &locale in &BETA_LOCALE_CODES {
        let mut empty_app = UiApp::new();
        empty_app.locale = locale.to_owned();
        let driver = make_driver(empty_app, 640.0, 420.0);
        assert_clean_parity(&driver, &format!("welcome 640x420 locale={locale}"))?;
        assert!(driver.state().leading_rect_pt.ok_or("640x420 rail missing")?.width() <= 38.0);

        let mut pop_app = create_pipeline_fixture_app()?;
        pop_app.locale = locale.to_owned();
        assert_clean_parity(&make_driver(pop_app, 1440.0, 600.0), &format!("workspace 1440x600 locale={locale}"))?;
    }

    let long_title = "Very Long Experimental Condition Title For Ellipsis Verification";
    let mut long_app = create_pipeline_fixture_app()?;
    long_app.execute_command("sheet.rename", json!({ "sheet_id": "data-1", "title": long_title }))?;
    long_app.sync_views();
    make_driver(long_app, 1440.0, 600.0).verify_ellipsis_tooltip_and_accesskit("nav.sheet.data-1", long_title)?;

    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        let mut app = create_pipeline_fixture_app()?;
        app.set_theme_mode(mode);
        let (png, w, h) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1440, height_px: 600, dpi: 96 }, ScreenshotRegionTarget::FullWindow)?;
        assert!((w, h) == (1440, 600) && !png.is_empty());
    }
    Ok(())
}

#[test]
fn welcome_hub_grouped_direct_create_uses_valid_replicates_mode() -> BoxRes {
    let mut driver = make_driver(UiApp::new(), 1440.0, 600.0);
    driver.click_widget_id("welcome.arch.grouped")?;
    driver.click_widget_id("welcome.btn.create")?;
    let ds = driver.state().app.document().data_sheets.last().ok_or("expected created Grouped DataSheet")?;
    assert!(matches!(ds.archetype, TableArchetype::Grouped(_)));
    assert_clean_parity(&driver, "grouped direct create")?;
    Ok(())
}
