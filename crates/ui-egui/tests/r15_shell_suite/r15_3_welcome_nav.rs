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

    driver.press_egui_key(Key::B, Modifiers::COMMAND);
    assert!((driver.state().leading_rect_pt.ok_or("restored missing")?.width() - 240.0).abs() <= 4.0);

    assert!(driver.state().app.family_open);
    driver.press_egui_key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!driver.state().app.family_open);
    driver.press_egui_key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(driver.state().app.family_open);

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.results")?;
    assert_eq!(driver.state().views.leading_dock.kind_chip, NavigatorKindChip::Results);
    assert!(
        driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some() && driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_none()
    );
    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.all")?;
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

#[test]
fn aud_08_13_and_09_01_07_27_45_welcome_policy_samples_clone_and_nested_nav() -> BoxRes {
    use crate::helpers::{click_input, run_egui_pass_with_ctx};
    use curvebook_doc::{AnalysisKind, AnalysisSpec, ColumnTable, DataSheet, DescriptiveSpec, ProjectDocument, ResultSheet, VariableSource};
    use curvebook_engine::{ExternalPathPolicy, Session};
    use curvebook_ui_egui::egui_shell::leading_dock::{LeadingDockEguiState, collect_visible_sheet_ids};
    use curvebook_ui_egui::egui_shell::welcome_hub::{
        WelcomeArchetypeChoice, WelcomeHubEguiState, WelcomeLeftMode, build_sample_creation_intent, render_welcome_hub,
    };
    use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
    use curvebook_ui_egui::workspace_nav::{SAMPLE_FIXTURES, instantiate_sample_fixture};

    // 1. AUD-09-01: RejectExternal disables open_file; Workdir blocks traversal; failed project.open keeps launcher open.
    let mut app_rej = UiApp::new();
    app_rej.session_mut().set_path_policy(ExternalPathPolicy::reject_external());
    let tokens = Tokens::from_theme_spec(&app_rej.theme, &app_rej.locale);
    let mut st = WelcomeHubEguiState { left_mode: WelcomeLeftMode::OpenFile, open_file_path_draft: "a.curvebook".into(), ..Default::default() };
    let (ctx0, mut m0, mut i0) = (egui::Context::default(), FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx0, egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        render_welcome_hub(ui, &app_rej, &mut st, &tokens, true, &mut m0, &mut i0);
    })?;
    assert!(
        !m0.find_by_widget_id("welcome.btn.open_file").ok_or("btn")?.enabled
            && !m0.find_by_widget_id("welcome.input.open_path").ok_or("inp")?.enabled
            && i0.is_empty()
    );

    let mut app_wd = UiApp::new();
    app_wd.session_mut().set_path_policy(ExternalPathPolicy::workdir(&std::env::current_dir()?)?);
    let mut st_wd =
        WelcomeHubEguiState { left_mode: WelcomeLeftMode::OpenFile, open_file_path_draft: "../../../../etc/passwd".into(), ..Default::default() };
    let (ctx_wd, mut m_wd, mut i_wd) = (egui::Context::default(), FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_wd, egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        render_welcome_hub(ui, &app_wd, &mut st_wd, &tokens, true, &mut m_wd, &mut i_wd);
    })?;
    let r = m_wd.find_by_widget_id("welcome.btn.open_file").ok_or("btn")?.clipped_rect;
    let (mut m_c, mut i_c) = (FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_wd, egui::vec2(1100.0, 720.0), click_input(r.center()), true, |_, ui| {
        render_welcome_hub(ui, &app_wd, &mut st_wd, &tokens, true, &mut m_c, &mut i_c);
    })?;
    assert!(i_c.is_empty() && st_wd.open_file_error.is_some());

    let mut app_unc = UiApp::new();
    let mut st_unc =
        WelcomeHubEguiState { left_mode: WelcomeLeftMode::OpenFile, open_file_path_draft: "missing-rem-test.curvebook".into(), ..Default::default() };
    let (ctx_u, mut mu0, mut iu0) = (egui::Context::default(), FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_u, egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        render_welcome_hub(ui, &app_unc, &mut st_unc, &tokens, true, &mut mu0, &mut iu0);
    })?;
    let ru = mu0.find_by_widget_id("welcome.btn.open_file").ok_or("btn")?.clipped_rect;
    let (mut mu1, mut iu1) = (FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_u, egui::vec2(1100.0, 720.0), click_input(ru.center()), true, |_, ui| {
        render_welcome_hub(ui, &app_unc, &mut st_unc, &tokens, true, &mut mu1, &mut iu1);
    })?;
    assert!(iu1.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { command_id, .. } if command_id == "project.open")));
    assert!(!iu1.intents().iter().any(|i| matches!(i, UiIntent::SetLauncherOpen { open: false })));
    let _ = iu1.commit_all_if_settled(&mut app_unc, false);
    assert!(app_unc.canvas_dialogs.status_banner.is_some());
    let (mut mu2, mut iu2) = (FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_u, egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        render_welcome_hub(ui, &app_unc, &mut st_unc, &tokens, true, &mut mu2, &mut iu2);
    })?;
    assert!(st_unc.open_file_error.is_some() && !iu2.intents().iter().any(|i| matches!(i, UiIntent::SetLauncherOpen { open: false })));

    // 2. AUD-09-27: Every sample fixture preserves slice_labels, Nested subcolumn_titles, and MV Calculated formulas.
    for fx in SAMPLE_FIXTURES {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        intents.push(build_sample_creation_intent(&app, fx.id, None, false, false)?);
        intents.commit_if_settled(&mut app, false)?;
        let ds = app.document().data_sheets.last().ok_or("sample ds")?;
        match &ds.archetype {
            TableArchetype::PartsOfWhole(t) => assert!(t.slice_labels.iter().any(|s| !s.is_empty())),
            TableArchetype::Nested(t) => assert!(t.groups.iter().any(|g| g.subcolumn_titles.iter().any(|s| !s.is_empty()))),
            TableArchetype::MultipleVariables(t) => {
                if let TableArchetype::MultipleVariables(raw_mv) = &instantiate_sample_fixture(fx.id, SheetId::new("raw"))?.archetype {
                    for (idx, rv) in raw_mv.variables.iter().enumerate() {
                        if let VariableSource::Calculated { formula } = &rv.source {
                            assert_eq!(t.variables.get(idx).map(|v| &v.source), Some(&VariableSource::Calculated { formula: formula.clone() }));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // 3. AUD-09-07 & AUD-09-45: ClonePipeline keep_x omits clear_data; PrecomputedError normalizes y_subcolumn_kind -> 2.
    let (app_xy, _) = crate::helpers::seed_xy_app(&[1.0, 2.0], &[10.0, 20.0]);
    let mut st_cl = WelcomeHubEguiState { left_mode: WelcomeLeftMode::ClonePipeline, ..Default::default() };
    st_cl.schema.clone_source_sheet_id = Some(SheetId::new("xy-1"));
    st_cl.schema.clone_clear_values = true;
    st_cl.schema.clone_keep_x = true;
    let (ctx_cl, mut mcl0, mut icl0) = (egui::Context::default(), FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_cl, egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        render_welcome_hub(ui, &app_xy, &mut st_cl, &tokens, true, &mut mcl0, &mut icl0);
    })?;
    let rc = mcl0.find_by_widget_id("welcome.btn.create").ok_or("create")?.clipped_rect;
    let (mut mcl1, mut icl1) = (FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_cl, egui::vec2(1100.0, 720.0), click_input(rc.center()), true, |_, ui| {
        render_welcome_hub(ui, &app_xy, &mut st_cl, &tokens, true, &mut mcl1, &mut icl1);
    })?;
    let cp = icl1
        .intents()
        .iter()
        .find_map(|i| match i {
            UiIntent::EngineCommand { command_id, params } if command_id == "sheet.clonePipeline" => Some(params),
            _ => None,
        })
        .ok_or("clonePipeline")?;
    assert!(cp.get("clear_data").is_none() && cp["keep_x_values"] == json!(true) && cp["clear_y_values"] == json!(true));

    let mut st_col = WelcomeHubEguiState { left_mode: WelcomeLeftMode::NewTableArchetype(WelcomeArchetypeChoice::Column), ..Default::default() };
    (st_col.schema.col_design_kind, st_col.schema.y_subcolumn_kind) = (2, 5);
    run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        render_welcome_hub(ui, &app_xy, &mut st_col, &tokens, true, &mut FrameInspectMeta::default(), &mut PendingUiIntents::new());
    })?;
    assert_eq!(st_col.schema.y_subcolumn_kind, 2);

    // 4. AUD-08-13: collect_visible_sheet_ids follows NestedResultsUnderData & hides collapsed children.
    let mut doc_n = ProjectDocument::new("Nav")?;
    for id in ["d1", "d2"] {
        doc_n.data_sheets.push(DataSheet::new(id, id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?))?);
    }
    doc_n.result_sheets.push(ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?);
    let mut nav_app = UiApp::from_session(Session::from_document(doc_n));
    nav_app.nav.nesting_mode = NavigatorNestingMode::NestedResultsUnderData;
    nav_app.sync_views();
    let mut ns = LeadingDockEguiState::default();
    assert_eq!(collect_visible_sheet_ids(&nav_app, &ns), vec![SheetId::new("d1"), SheetId::new("r1"), SheetId::new("d2")]);
    ns.nested_collapsed_parents.insert(SheetId::new("d1"));
    assert_eq!(collect_visible_sheet_ids(&nav_app, &ns), vec![SheetId::new("d1"), SheetId::new("d2")]);
    Ok(())
}

#[test]
fn r10_36_collapsed_rail_vector_icons_and_kind_dropdown_themes_hidpi() -> BoxRes {
    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        for scale in [1.0_f64, 1.5_f64] {
            let mut app = create_pipeline_fixture_app()?;
            app.set_theme_mode(mode);
            let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 900.0, scale));

            // Expanded Navigator has folder headers echoing section vector icons and labeled `nav.kind_filter` combobox.
            for folder_wid in ["nav.section.data", "nav.section.info", "nav.section.results"] {
                assert!(driver.frame_meta().find_by_widget_id(folder_wid).is_some(), "missing {folder_wid}");
            }
            driver.click_widget_id("nav.kind_filter")?;
            for item_wid in ["nav.chip.all", "nav.chip.data", "nav.chip.info", "nav.chip.results", "nav.chip.graphs", "nav.chip.layouts"] {
                let entry = driver.frame_meta().find_by_widget_id(item_wid).ok_or_else(|| format!("missing dropdown item `{item_wid}`"))?;
                assert!(entry.clipped_rect.height() >= 24.0, "{item_wid} height < 24pt");
            }
            assert_clean_parity(&driver, &format!("open nav.kind_filter {mode:?}@{scale}x"))?;
            driver.click_widget_id("nav.chip.graphs")?;
            assert!(driver.frame_meta().find_by_widget_id("nav.section.graphs").is_some(), "missing nav.section.graphs");
            crate::common::select_nav_kind_filter(&mut driver, "nav.chip.all")?;

            // Collapsed 36pt rail uses vector icons (`nav.rail.*`, `family.rail.toggle`) cleanly across Light/Dark/HighContrast.
            driver.click_widget_id("nav.collapse_rail")?;
            for rail_wid in
                ["nav.rail.expand", "nav.rail.data", "nav.rail.info", "nav.rail.results", "nav.rail.graphs", "nav.rail.layouts", "family.rail.toggle"]
            {
                let entry = driver.frame_meta().find_by_widget_id(rail_wid).ok_or_else(|| format!("missing rail btn `{rail_wid}`"))?;
                assert!(entry.clipped_rect.width() >= 24.0 && entry.clipped_rect.height() >= 24.0, "{rail_wid} below 24pt");
            }
            assert_clean_parity(&driver, &format!("collapsed vector rail {mode:?}@{scale}x"))?;

            // Clicking rail section vector icon expands Navigator filtered to Results.
            driver.click_widget_id("nav.rail.results")?;
            assert_eq!(driver.state().views.leading_dock.kind_chip, NavigatorKindChip::Results);
            assert!(driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some());
        }
    }
    Ok(())
}

#[test]
fn r20_12_disambiguation_two_line_rows_kind_dropdown_and_multi_select() -> BoxRes {
    let mut app = create_pipeline_fixture_app()?;
    // Create duplicate-titled Data (`Cohort`), duplicate-titled Graphs (`Summary`), plus unambiguous Layout (`Appendix Plate`).
    app.execute_command("sheet.createData", json!({ "sheet_id": "dup-d1", "title": "Cohort", "kind": "xy", "cols": 2, "rows": 3, "auto_graph": true, "graph_sheet_id": "dup-g1", "auto_info": false }))?;
    app.execute_command("sheet.createData", json!({ "sheet_id": "dup-d2", "title": "Cohort", "kind": "column", "cols": 3, "rows": 4, "auto_graph": true, "graph_sheet_id": "dup-g2", "auto_info": false }))?;
    app.execute_command("sheet.rename", json!({ "sheet_id": "dup-g1", "title": "Summary" }))?;
    app.execute_command("sheet.rename", json!({ "sheet_id": "dup-g2", "title": "Summary" }))?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Appendix Plate" }))?;
    app.sync_views();

    let mut driver = make_driver(app, 1440.0, 900.0);

    // Locked Decision #6: two-line `38 pt` on Graphs, Results, and duplicate-titled rows; compact `26 pt` on unambiguous Data, Info, and Layouts.
    let d_unique_h = driver.frame_meta().find_by_widget_id("nav.sheet.data-1").ok_or("missing data-1")?.clipped_rect.height();
    let d_dup_h = driver.frame_meta().find_by_widget_id("nav.sheet.dup-d1").ok_or("missing dup-d1")?.clipped_rect.height();
    assert!((d_unique_h - 26.0).abs() <= 1.0, "expected unambiguous Data row ~26pt, got {d_unique_h}");
    assert!((d_dup_h - 38.0).abs() <= 1.0, "expected duplicate Data row ~38pt, got {d_dup_h}");

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.info")?;
    let info_wid = format!("nav.sheet.{}", driver.state().app.document().info_sheets[0].id.as_str());
    let info_h = driver.frame_meta().find_by_widget_id(&info_wid).ok_or("missing info row")?.clipped_rect.height();
    assert!((info_h - 26.0).abs() <= 1.0, "expected unambiguous Info row ~26pt, got {info_h}");

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.results")?;
    let res_h = driver.frame_meta().find_by_widget_id("nav.sheet.res-1").ok_or("missing res-1")?.clipped_rect.height();
    assert!((res_h - 38.0).abs() <= 1.0, "expected Result row ~38pt, got {res_h}");

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.graphs")?;
    let g_dup_h = driver.frame_meta().find_by_widget_id("nav.sheet.dup-g1").ok_or("missing dup-g1")?.clipped_rect.height();
    assert!((g_dup_h - 38.0).abs() <= 1.0, "expected Graph row ~38pt, got {g_dup_h}");

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.layouts")?;
    let lay_h = driver.frame_meta().find_by_widget_id("nav.sheet.lay-1").ok_or("missing lay-1")?.clipped_rect.height();
    assert!((lay_h - 26.0).abs() <= 1.0, "expected unambiguous Layout row ~26pt, got {lay_h}");

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.all")?;

    // Split View selector disambiguates identical sheet titles cleanly.
    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.data_graph")?;
    driver.click_widget_id("split.secondary_selector")?;
    let g1_sub = driver
        .state()
        .app
        .nav
        .all_entries
        .iter()
        .find(|e| e.sheet_id.as_str() == "dup-g1")
        .and_then(|e| e.subtitle.clone())
        .ok_or("missing dup-g1 subtitle")?;
    let expected_split_item = format!("[Graph] Summary \u{2014} {g1_sub}");
    assert!(
        driver
            .accesskit_tree()
            .ok_or("missing AccessKit tree")?
            .nodes
            .iter()
            .any(|(_, n)| curvebook_ui_egui::egui_shell::driver::accesskit_accessible_name(n) == Some(expected_split_item.as_str())),
        "expected split selector to disambiguate Summary with `{expected_split_item}`"
    );
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.single")?;

    // Search filters by disambiguation subtitle too.
    driver.click_widget_id("nav.search_input")?;
    driver.type_text("From Kinetic")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_none());
    driver.click_widget_id("nav.search_clear")?;

    // HIG multi-select: single click -> Cmd+Click discontinuous toggle -> right-click Compare in Split View -> Shift+Click range -> Duplicate batch.
    driver.click_widget_id("nav.sheet.data-1")?;
    assert_eq!(driver.state().views.leading_dock.selected_sheet_ids.len(), 1);

    let mod_cmd =
        curvebook_ui_egui::interaction::KeyModifiers { meta: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..Default::default() };
    let d2_center = widget_center(&driver, "nav.sheet.data-2")?;
    driver.click_at_button(d2_center, curvebook_ui_egui::interaction::PointerButton::Primary, mod_cmd)?;
    assert_eq!(driver.state().views.leading_dock.selected_sheet_ids.len(), 2);

    // Right-click on multi-selected row shows `Compare in Split View` and triggers split comparison.
    let d2_pos = widget_center(&driver, "nav.sheet.data-2")?;
    driver.click_at_button(d2_pos, curvebook_ui_egui::interaction::PointerButton::Secondary, Default::default())?;
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.compare_split").is_some());
    driver.click_widget_id("nav.ctx.compare_split")?;
    assert!(driver.state().app.split.enabled);
    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.single")?;

    // Shift+Click selects contiguous visible range from `data-1` to `dup-d2` (4 Data sheets) and duplicates atomically.
    driver.click_widget_id("nav.sheet.data-1")?;
    let dup2_center = widget_center(&driver, "nav.sheet.dup-d2")?;
    driver.click_at_button(
        dup2_center,
        curvebook_ui_egui::interaction::PointerButton::Primary,
        curvebook_ui_egui::interaction::KeyModifiers { shift: true, ..Default::default() },
    )?;
    assert_eq!(driver.state().views.leading_dock.selected_sheet_ids.len(), 4);

    let data_before = driver.state().app.document().data_sheets.len();
    let dup2_pos = widget_center(&driver, "nav.sheet.dup-d2")?;
    driver.click_at_button(dup2_pos, curvebook_ui_egui::interaction::PointerButton::Secondary, Default::default())?;
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.compare_split").is_none());
    driver.click_widget_id("nav.ctx.duplicate")?;
    assert_eq!(driver.state().app.document().data_sheets.len(), data_before + 4);
    assert_clean_parity(&driver, "r20_12 multi-select duplicate")?;
    Ok(())
}

#[test]
fn welcome_hub_sample_notes_and_clone_override_replicates_human_path() -> BoxRes {
    use curvebook_doc::{ColumnTable, DataSheet, GroupedTable, NestedTable, ProjectDocument, XyTable};
    use curvebook_engine::Session;
    use curvebook_ui_egui::workspace_nav::sample_fixture_instructional_note;

    // (a) Tutorial sample optional instructional FloatingNote via welcome.toggle.sample_notes.
    let mut driver = make_driver(UiApp::new(), 1440.0, 720.0);

    // Unchecked (default) creates 0 floating notes.
    driver.click_widget_id("welcome.arch.xy")?;
    driver.click_widget_id("welcome.entry_mode.sample")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.toggle.sample_notes").is_some());
    assert!(!driver.state().views.welcome_hub.schema.include_sample_notes);
    driver.click_widget_id("welcome.btn.create")?;
    let ds_plain = driver.state().app.document().data_sheets.last().ok_or("missing sample without notes")?;
    assert!(ds_plain.floating_notes.is_empty(), "unchecked sample_notes must attach 0 notes");

    // Checked attaches the clean-room instructional FloatingNote and undoes atomically.
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.xy")?;
    driver.click_widget_id("welcome.entry_mode.sample")?;
    driver.click_widget_id("welcome.toggle.sample_notes")?;
    assert!(driver.state().views.welcome_hub.schema.include_sample_notes);
    let sheets_before_noted = driver.state().app.document().data_sheets.len();
    driver.click_widget_id("welcome.btn.create")?;
    let ds_noted = driver.state().app.document().data_sheets.last().ok_or("missing sample with notes")?;
    assert_eq!(ds_noted.floating_notes.len(), 1, "checked sample_notes must attach 1 instructional FloatingNote");
    let expected_note = sample_fixture_instructional_note("dose-response-4pl").ok_or("missing fixture note")?;
    assert_eq!(ds_noted.floating_notes[0].body_markdown, expected_note);
    let note_card_wid = format!("floating_note.{}.{}.card", ds_noted.id.as_str(), ds_noted.floating_notes[0].id.as_str());
    assert!(driver.frame_meta().find_by_widget_id(&note_card_wid).is_some(), "instructional FloatingNote card must render");

    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(driver.state().app.document().data_sheets.len(), sheets_before_noted);
    drop(driver);

    // (b) Clone Pipeline override_replicates across Xy, Grouped, Nested, Column (disabled), and pre-summarized Grouped.
    let mut doc = ProjectDocument::new("Clone Replicates Human Path")?;
    doc.data_sheets.extend([
        DataSheet::new("xy-src", "XY Source", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4)?))?,
        DataSheet::new("grp-src", "Grouped Source", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?))?,
        DataSheet::new("nest-src", "Nested Source", TableArchetype::Nested(NestedTable::new(2, 3, 4)?))?,
        DataSheet::new("col-src", "Column Source", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?))?,
        DataSheet::new("grp-sum", "Grouped Summary", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::MeanSdN, 2, 3)?))?,
    ]);
    doc.active_sheet_id = Some(SheetId::new("xy-src"));
    let mut clone_app = UiApp::from_session(Session::from_document(doc));
    clone_app.sync_views();
    let mut driver = make_driver(clone_app, 1440.0, 720.0);

    // 1. XY source: initial replicates = 2, enable override, step to 4, apply & clone.
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.mode.clone")?;
    driver.click_widget_id("welcome.clone.source.xy-src")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    assert_eq!(driver.state().views.welcome_hub.schema.clone_override_replicates_count, 2);
    assert!(driver.frame_meta().find_by_widget_id("welcome.clone.override_replicates").ok_or("override cb")?.enabled);
    driver.click_widget_id("welcome.clone.override_replicates")?;
    driver.click_widget_id("welcome.clone.override_replicates_count.inc")?;
    driver.click_widget_id("welcome.clone.override_replicates_count.inc")?;
    assert_eq!(driver.state().views.welcome_hub.schema.clone_override_replicates_count, 4);
    assert_clean_parity(&driver, "clone options submodal with override replicates")?;
    driver.click_widget_id("welcome.clone_modal.apply")?;
    driver.click_widget_id("welcome.btn.create")?;
    match &driver.state().app.document().data_sheets.last().ok_or("cloned xy")?.archetype {
        TableArchetype::Xy(t) => assert_eq!(t.y_subcolumn_mode, SubcolumnMode::RawReplicates(4)),
        other => return Err(format!("expected cloned XY, got {:?}", other.kind()).into()),
    }

    // 2. Grouped source: switches initial replicates to 2, step to 5, apply & clone.
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.mode.clone")?;
    driver.click_widget_id("welcome.clone.source.grp-src")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    assert_eq!(driver.state().views.welcome_hub.schema.clone_override_replicates_count, 2);
    if !driver.state().views.welcome_hub.schema.clone_override_replicates {
        driver.click_widget_id("welcome.clone.override_replicates")?;
    }
    for _ in 0..3 {
        driver.click_widget_id("welcome.clone.override_replicates_count.inc")?;
    }
    assert_eq!(driver.state().views.welcome_hub.schema.clone_override_replicates_count, 5);
    driver.click_widget_id("welcome.clone_modal.apply")?;
    driver.click_widget_id("welcome.btn.create")?;
    match &driver.state().app.document().data_sheets.last().ok_or("cloned grouped")?.archetype {
        TableArchetype::Grouped(t) => assert_eq!(t.subcolumn_mode, SubcolumnMode::RawReplicates(5)),
        other => return Err(format!("expected cloned Grouped, got {:?}", other.kind()).into()),
    }

    // 3. Nested source: switches initial biological units to 3, step to 4, apply & clone.
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.mode.clone")?;
    driver.click_widget_id("welcome.clone.source.nest-src")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    assert_eq!(driver.state().views.welcome_hub.schema.clone_override_replicates_count, 3);
    if !driver.state().views.welcome_hub.schema.clone_override_replicates {
        driver.click_widget_id("welcome.clone.override_replicates")?;
    }
    driver.click_widget_id("welcome.clone.override_replicates_count.inc")?;
    assert_eq!(driver.state().views.welcome_hub.schema.clone_override_replicates_count, 4);
    driver.click_widget_id("welcome.clone_modal.apply")?;
    driver.click_widget_id("welcome.btn.create")?;
    match &driver.state().app.document().data_sheets.last().ok_or("cloned nested")?.archetype {
        TableArchetype::Nested(t) => assert_eq!(t.groups[0].biological_unit_count(), 4),
        other => return Err(format!("expected cloned Nested, got {:?}", other.kind()).into()),
    }

    // 4. Column source: override_replicates is disabled with clear disabled_reason and forced false.
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.mode.clone")?;
    driver.click_widget_id("welcome.clone.source.col-src")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    let col_ov = driver.frame_meta().find_by_widget_id("welcome.clone.override_replicates").ok_or("col override cb")?;
    assert!(!col_ov.enabled, "Column source must disable override_replicates");
    assert!(col_ov.disabled_reason.as_deref().is_some_and(|r| !r.is_empty()), "Column disabled_reason must be non-empty");
    assert!(!driver.state().views.welcome_hub.schema.clone_override_replicates);
    driver.click_widget_id("welcome.clone_modal.close")?;

    // 5. Pre-summarized Grouped (MeanSdN): disabled in Copy mode with disabled_reason, enabled in Clear mode.
    driver.click_widget_id("welcome.clone.source.grp-sum")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    driver.click_widget_id("welcome.clone.copy_data")?;
    let sum_copy_ov = driver.frame_meta().find_by_widget_id("welcome.clone.override_replicates").ok_or("sum copy override cb")?;
    assert!(!sum_copy_ov.enabled, "pre-summarized table in Copy mode must disable override_replicates");
    assert!(sum_copy_ov.disabled_reason.as_deref().is_some_and(|r| !r.is_empty()), "pre-summarized Copy disabled_reason must be non-empty");
    assert!(!driver.state().views.welcome_hub.schema.clone_override_replicates);

    driver.click_widget_id("welcome.clone.clear_data")?;
    let sum_clear_ov = driver.frame_meta().find_by_widget_id("welcome.clone.override_replicates").ok_or("sum clear override cb")?;
    assert!(sum_clear_ov.enabled, "pre-summarized table in Clear mode must enable override_replicates");
    driver.click_widget_id("welcome.clone.override_replicates")?;
    driver.click_widget_id("welcome.clone.override_replicates_count.inc")?;
    driver.click_widget_id("welcome.clone_modal.apply")?;
    driver.click_widget_id("welcome.btn.create")?;
    match &driver.state().app.document().data_sheets.last().ok_or("cloned pre-summarized")?.archetype {
        TableArchetype::Grouped(t) => assert_eq!(t.subcolumn_mode, SubcolumnMode::RawReplicates(4)),
        other => return Err(format!("expected cloned Grouped, got {:?}", other.kind()).into()),
    }
    Ok(())
}
