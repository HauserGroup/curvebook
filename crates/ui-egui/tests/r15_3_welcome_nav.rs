#![forbid(unsafe_code)]

//! R15.3 Integration & Pixel-Parity Tests: 3-Pane Welcome & Workspace Hub + Leading Dock
//! (`Navigator` & Scoped `Family` Lineage Tree).

use curvebook_doc::{ColumnDesignHint, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TableKind, VariableRole, XColumnMode};
use curvebook_ui_egui::compositor::ScreenshotRegionTarget;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::leading_dock::NavigatorKindChip;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::{ActiveWorkspacePane, NavigatorNestingMode};
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

struct ShellDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    family_hover_preview_rect_pt: Option<egui::Rect>,
    leading_dock_rect_pt: Option<egui::Rect>,
    leading_splitter_rect_pt: Option<egui::Rect>,
}

impl ShellDriverState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self {
            app,
            views,
            intents: PendingUiIntents::default(),
            family_hover_preview_rect_pt: None,
            leading_dock_rect_pt: None,
            leading_splitter_rect_pt: None,
        }
    }
}

fn make_shell_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellDriverState> {
    let locale = app.locale.clone();
    let theme = app.theme.clone();
    let tokens = Tokens::from_theme_spec(&theme, &locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellDriverState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
            st.leading_dock_rect_pt = rep.leading_dock_rect_pt;
            st.family_hover_preview_rect_pt = meta.find_by_widget_id("family.hover_preview").map(|e| e.clipped_rect);
            st.leading_splitter_rect_pt = meta.find_by_widget_id("nav.splitter").map(|e| e.clipped_rect);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views.launcher_open = st.app.egui_views.launcher_open;
            st.views.leading_collapsed_rail = st.app.egui_views.leading_collapsed_rail;
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn widget_center(driver: &EguiHumanPathDriver<'_, ShellDriverState>, widget_id: &str) -> Result<Pos2, String> {
    let node = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    if !node.clipped_rect.is_positive() {
        return Err(format!("widget `{widget_id}` has empty clipped rect"));
    }
    Ok(node.clipped_rect.center())
}

fn assert_clean_parity(driver: &EguiHumanPathDriver<'_, ShellDriverState>, label: &str) {
    let report = driver.verify_pixel_to_hit_box_parity();
    assert!(report.is_clean(), "{label} parity violations: {:?}", report.violations);
    let sb = driver.check_scrollbar_non_overlap();
    assert!(sb.is_empty(), "{label} scrollbar overlaps: {:?}", sb);
}

fn new_test_app() -> UiApp {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    app
}

fn create_pipeline_fixture_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut app = new_test_app();
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "data-1",
            "title": "Kinetic Dose Curve",
            "kind": "xy",
            "cols": 2,
            "rows": 5,
            "auto_graph": true,
            "auto_info": true
        }),
    )?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "data-1", "values": [1.0, 2.0, 3.0, 4.0, 5.0] }))?;
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "data-1",
            "edits": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 1, "col": 0, "value": 25.0 },
                { "row": 2, "col": 0, "value": 48.0 },
                { "row": 3, "col": 0, "value": 76.0 },
                { "row": 4, "col": 0, "value": 92.0 },
                { "row": 0, "col": 1, "value": 12.0 },
                { "row": 1, "col": 1, "value": 22.0 },
                { "row": 2, "col": 1, "value": 41.0 },
                { "row": 3, "col": 1, "value": 69.0 },
                { "row": 4, "col": 1, "value": 88.0 }
            ]
        }),
    )?;
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "data-2",
            "title": "Validation Cohort",
            "kind": "xy",
            "cols": 2,
            "rows": 4,
            "auto_graph": false,
            "auto_info": false
        }),
    )?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "data-2", "values": [1.0, 2.0, 3.0, 4.0] }))?;
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "data-2",
            "edits": [
                { "row": 0, "col": 0, "value": 14.0 },
                { "row": 1, "col": 0, "value": 28.0 },
                { "row": 2, "col": 0, "value": 44.0 },
                { "row": 3, "col": 0, "value": 68.0 }
            ]
        }),
    )?;
    app.execute_command(
        "analysis.run",
        json!({
            "source_sheet_id": "data-1",
            "sheet_id": "res-1",
            "kind": "linear_regression"
        }),
    )?;
    app.execute_command(
        "graph.create",
        json!({
            "sheet_id": "graph-2",
            "data_sheet_id": "data-2",
            "kind": "xy",
            "title": "Cohort Plot"
        }),
    )?;
    app.execute_command(
        "layout.create",
        json!({
            "sheet_id": "layout-1",
            "title": "Figure 1"
        }),
    )?;
    if let Some(first_graph_id) = app.document().graph_sheets.first().map(|g| g.id.as_str().to_owned()) {
        app.execute_command(
            "layout.addPanel",
            json!({
                "sheet_id": "layout-1",
                "graph_sheet_id": first_graph_id
            }),
        )?;
        app.execute_command(
            "layout.addPanel",
            json!({
                "sheet_id": "layout-1",
                "graph_sheet_id": "graph-2",
                "bounds_pt": [260.0, 20.0, 200.0, 140.0]
            }),
        )?;
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "data-1", "mode": "navigator" }))?;
    app.sync_views();
    Ok(app)
}

#[test]
fn welcome_hub_empty_project_rules_and_home_reopen_preserve_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let empty_app = new_test_app();
    assert_eq!(empty_app.document().total_sheet_count(), 0);

    let mut driver = make_shell_driver(empty_app, 1440.0, 600.0);
    assert_clean_parity(&driver, "empty welcome 1440x600");

    // Empty workspace: Cancel hidden, Welcome Hub rendered, Leading Dock collapsed to 36 pt rail.
    assert!(driver.frame_meta().find_by_widget_id("welcome.btn.create").is_some());
    assert!(driver.frame_meta().find_by_widget_id("welcome.action.cancel").is_none());
    let lead_rect = driver.state().leading_dock_rect_pt.ok_or("leading dock missing")?;
    assert!(lead_rect.width() <= 38.0, "empty workspace leading dock must collapse to 36pt rail, got {}", lead_rect.width());

    // Create a single Column sheet via Welcome Hub CTA.
    driver.click_widget_id("welcome.btn.create")?;
    assert_eq!(driver.state().app.document().total_sheet_count(), 3); // Data + auto Info + auto Graph
    let sheets_before_home = driver.state().app.document().total_sheet_count();

    // Reopen Welcome Hub from Top Chrome Home button over active workspace.
    driver.click_widget_id("toolbar.home")?;
    assert!(driver.state().views.launcher_open);
    assert!(driver.frame_meta().find_by_widget_id("welcome.action.cancel").is_some());
    assert_clean_parity(&driver, "reopened welcome overlay");

    // Cancel closes overlay and preserves every existing sheet intact.
    driver.click_widget_id("welcome.action.cancel")?;
    assert!(!driver.state().views.launcher_open);
    assert_eq!(driver.state().app.document().total_sheet_count(), sheets_before_home);
    assert!(driver.frame_meta().find_by_widget_id("welcome.btn.create").is_none());
    Ok(())
}

#[test]
fn welcome_hub_creates_all_eight_archetypes_with_conditional_schemas_single_commit() -> Result<(), Box<dyn std::error::Error>> {
    let app = new_test_app();
    let mut driver = make_shell_driver(app, 1440.0, 600.0);

    // Confirm typing into title does NOT mutate document or undo depth before CTA commit.
    let title_center = widget_center(&driver, "welcome.input.title")?;
    driver.click_at(title_center)?;
    driver.type_text("PreCommitTitle")?;
    assert_eq!(driver.state().app.document().total_sheet_count(), 0);
    assert_eq!(driver.state().app.session().undo_depth(), 0);

    // 1. XY with Numbers + X Error & Raw Replicates
    driver.click_widget_id("welcome.arch.xy")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.section.x_column").is_some());
    assert!(driver.frame_meta().find_by_widget_id("welcome.section.y_format").is_some());
    driver.click_widget_id("welcome.x_mode.numbers_error")?;
    driver.click_widget_id("welcome.subcol.raw")?;
    driver.click_widget_id("welcome.stepper.replicates.inc")?; // 3 -> 4 replicates
    let undo_before = driver.state().app.session().undo_depth();
    driver.click_widget_id("welcome.btn.create")?;
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    let xy_ds = driver.state().app.document().data_sheets.last().ok_or("missing xy sheet")?;
    assert_eq!(xy_ds.archetype.kind(), TableKind::Xy);
    match &xy_ds.archetype {
        TableArchetype::Xy(t) => {
            assert_eq!(t.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN });
            assert_eq!(t.y_subcolumn_mode, SubcolumnMode::RawReplicates(4));
        }
        other => return Err(format!("expected XY, got {:?}", other.kind()).into()),
    }

    // 2. Column with Paired design hint
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.column")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.section.col_design").is_some());
    driver.click_widget_id("welcome.design.paired")?;
    driver.click_widget_id("welcome.btn.create")?;
    let col_ds = driver.state().app.document().data_sheets.last().ok_or("missing column sheet")?;
    match &col_ds.archetype {
        TableArchetype::Column(t) => assert_eq!(t.design_hint, ColumnDesignHint::PairedRepeatedMeasures),
        other => return Err(format!("expected Column, got {:?}", other.kind()).into()),
    }

    // 3. Grouped with Mean/SEM/N
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.grouped")?;
    driver.click_widget_id("welcome.subcol.mean_sem_n")?;
    driver.click_widget_id("welcome.btn.create")?;
    let grp_ds = driver.state().app.document().data_sheets.last().ok_or("missing grouped sheet")?;
    assert_eq!(grp_ds.archetype.kind(), TableKind::Grouped);

    // 4. Contingency
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.contingency")?;
    driver.click_widget_id("welcome.btn.create")?;
    let cnt_ds = driver.state().app.document().data_sheets.last().ok_or("missing contingency sheet")?;
    assert_eq!(cnt_ds.archetype.kind(), TableKind::Contingency);

    // 5. Survival with date/event code controls
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.survival")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.survival.event_code.inc").is_some());
    driver.click_widget_id("welcome.survival.event_code.inc")?; // event = 2
    driver.click_widget_id("welcome.survival.time_date")?;
    driver.click_widget_id("welcome.btn.create")?;
    let surv_ds = driver.state().app.document().data_sheets.last().ok_or("missing survival sheet")?;
    assert_eq!(surv_ds.archetype.kind(), TableKind::Survival);

    // 6. PartsOfWhole
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.parts_of_whole")?;
    driver.click_widget_id("welcome.btn.create")?;
    let pow_ds = driver.state().app.document().data_sheets.last().ok_or("missing parts sheet")?;
    assert_eq!(pow_ds.archetype.kind(), TableKind::PartsOfWhole);

    // 7. MultipleVariables with categorical/label variable roles
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.multiple_variables")?;
    driver.click_widget_id("welcome.mv.role.0.Categorical")?;
    driver.click_widget_id("welcome.mv.role.1.Label")?;
    driver.click_widget_id("welcome.btn.create")?;
    let mv_ds = driver.state().app.document().data_sheets.last().ok_or("missing mv sheet")?;
    match &mv_ds.archetype {
        TableArchetype::MultipleVariables(t) => {
            assert_eq!(t.variables[0].role, VariableRole::Categorical);
            assert_eq!(t.variables[1].role, VariableRole::Label);
        }
        other => return Err(format!("expected MultipleVariables, got {:?}", other.kind()).into()),
    }

    // 8. Nested with subcolumn units depth
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.nested")?;
    driver.click_widget_id("welcome.nested.depth.inc")?; // 4 -> 5 units
    driver.click_widget_id("welcome.btn.create")?;
    let nested_ds = driver.state().app.document().data_sheets.last().ok_or("missing nested sheet")?;
    match &nested_ds.archetype {
        TableArchetype::Nested(t) => assert_eq!(t.groups[0].subcolumn_titles.len(), 5),
        other => return Err(format!("expected Nested, got {:?}", other.kind()).into()),
    }

    // Also verify Tutorial Sample creation populates actual cell data.
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.arch.xy")?;
    driver.click_widget_id("welcome.entry_mode.sample")?;
    driver.click_widget_id("welcome.btn.create")?;
    let sample_ds = driver.state().app.document().data_sheets.last().ok_or("missing sample sheet")?;
    match &sample_ds.archetype {
        TableArchetype::Xy(t) => assert!(!t.x_cells.is_empty() && t.x_cells[0][0].active_number().is_some()),
        other => return Err(format!("expected XY sample, got {:?}", other.kind()).into()),
    }

    Ok(())
}

#[test]
fn welcome_hub_starter_templates_clone_options_and_open_file() -> Result<(), Box<dyn std::error::Error>> {
    let app = new_test_app();
    let mut driver = make_shell_driver(app, 1440.0, 600.0);

    // 1. Starter Templates (.cbtemplate)
    driver.click_widget_id("welcome.mode.template")?;
    driver.click_widget_id("welcome.template.1")?; // Paired Before / After
    driver.click_widget_id("welcome.btn.create")?;
    assert_eq!(driver.state().app.document().data_sheets.len(), 1);
    assert_eq!(driver.state().app.document().graph_sheets.len(), 1);
    assert_eq!(driver.state().app.document().info_sheets.len(), 1);

    // 2. Open File hook
    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.action.open_existing")?;
    let prev_req = driver.state().views.welcome_hub.open_file_request_count;
    driver.click_widget_id("welcome.btn.open_file")?;
    assert_eq!(driver.state().views.welcome_hub.open_file_request_count, prev_req + 1);
    driver.click_widget_id("welcome.action.cancel")?;
    drop(driver);

    // 3. Clone Pipeline + Clone Options sub-modal (preserve links, clear values, re-run analyses)
    let pipe_app = create_pipeline_fixture_app()?;
    let mut driver = make_shell_driver(pipe_app, 1440.0, 600.0);
    let data_before = driver.state().app.document().data_sheets.len();
    let res_before = driver.state().app.document().result_sheets.len();

    driver.click_widget_id("toolbar.home")?;
    driver.click_widget_id("welcome.mode.clone")?;
    driver.click_widget_id("welcome.clone.source.data-1")?;
    driver.click_widget_id("welcome.btn.clone_options")?;
    assert!(driver.frame_meta().find_by_widget_id("welcome.clone_modal.heading").is_some());
    assert_clean_parity(&driver, "clone options submodal");

    // Toggle clear values & defer re-run
    driver.click_widget_id("welcome.clone.clear_data")?;
    driver.click_widget_id("welcome.clone.rerun_analyses")?;
    driver.click_widget_id("welcome.clone_modal.apply")?;
    assert!(!driver.state().views.welcome_hub.clone_options_open);

    driver.click_widget_id("welcome.btn.create")?;
    let doc = driver.state().app.document();
    assert_eq!(doc.data_sheets.len(), data_before + 1);
    assert_eq!(doc.result_sheets.len(), res_before + 1);
    let cloned_ds = doc.data_sheets.last().ok_or("cloned data missing")?;
    if let TableArchetype::Xy(t) = &cloned_ds.archetype {
        assert!(t.data_sets[0].cell(0, 0).is_none_or(|c| c.value.is_empty()), "clear_data must clear Y cells");
    }
    let cloned_res = doc.result_sheets.last().ok_or("cloned result missing")?;
    assert_eq!(cloned_res.status, ResultStatus::Dirty, "deferred rerun must keep cloned result Dirty");
    Ok(())
}

#[test]
fn leading_dock_rail_collapse_filters_nested_toggle_and_inline_new() -> Result<(), Box<dyn std::error::Error>> {
    let app = create_pipeline_fixture_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 600.0);
    assert_clean_parity(&driver, "expanded leading dock");

    let dock_w = driver.state().leading_dock_rect_pt.ok_or("dock missing")?.width();
    assert!((dock_w - 240.0).abs() <= 4.0, "expected 240pt leading dock, got {dock_w}");

    // Collapse to 36pt rail via button, then restore via Cmd/Ctrl+Shift+N
    driver.click_widget_id("nav.collapse_rail")?;
    let rail_w = driver.state().leading_dock_rect_pt.ok_or("rail missing")?.width();
    assert!((rail_w - 36.0).abs() <= 2.0, "expected 36pt rail, got {rail_w}");
    assert_clean_parity(&driver, "collapsed 36pt rail");

    driver.press_egui_key(Key::N, Modifiers::COMMAND | Modifiers::SHIFT);
    let restored_w = driver.state().leading_dock_rect_pt.ok_or("restored missing")?.width();
    assert!((restored_w - 240.0).abs() <= 4.0, "expected restored 240pt dock, got {restored_w}");

    // Toggle Family pane with Cmd/Ctrl+Shift+F
    assert!(driver.state().app.family_open);
    driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!driver.state().app.family_open);
    driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(driver.state().app.family_open);

    // Filter chips
    driver.click_widget_id("nav.chip.results")?;
    assert_eq!(driver.state().views.leading_dock.kind_chip, NavigatorKindChip::Results);
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_none());
    driver.click_widget_id("nav.chip.all")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_some());

    // Search box + clear X
    let search_pos = widget_center(&driver, "nav.search_input")?;
    driver.click_at(search_pos)?;
    driver.type_text("Validation")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-2").is_some());
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_none());
    driver.click_widget_id("nav.search_clear")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.data-1").is_some());

    // Toggle 5-Folder vs Nested Data-with-Results mode
    driver.click_widget_id("nav.view_mode.toggle")?;
    assert_eq!(driver.state().app.nav.nesting_mode, NavigatorNestingMode::NestedResultsUnderData);
    assert!(driver.frame_meta().find_by_widget_id("nav.sheet.res-1").is_some());
    driver.click_widget_id("nav.view_mode.toggle")?;
    assert_eq!(driver.state().app.nav.nesting_mode, NavigatorNestingMode::SeparateFolders);

    // Inline + New Data... row
    let data_before = driver.state().app.document().data_sheets.len();
    driver.click_widget_id("nav.new.data")?;
    assert_eq!(driver.state().app.document().data_sheets.len(), data_before + 1);
    Ok(())
}

#[test]
fn leading_dock_f2_inline_rename_drag_reorder_and_result_to_data_retarget() -> Result<(), Box<dyn std::error::Error>> {
    let app = create_pipeline_fixture_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 600.0);

    // Select data-1 and open F2 inline rename
    driver.click_widget_id("nav.sheet.data-1")?;
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::Navigator);
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    assert_eq!(driver.state().views.leading_dock.renaming_sheet_id, Some(SheetId::from("data-1")));

    // Clear existing text with Backspace sequence then type new title
    for _ in 0..24 {
        driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    }
    driver.type_text("Kinetic Updated")?;
    // Typing in buffer must NOT mutate session yet
    assert_ne!(driver.state().app.document().data_sheets[0].title, "Kinetic Updated");

    let undo_before = driver.state().app.session().undo_depth();
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    assert_eq!(driver.state().app.document().data_sheets[0].title, "Kinetic Updated");

    // Verify Escape cancels inline F2 rename without committing
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    driver.type_text("DiscardedDraft")?;
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.document().data_sheets[0].title, "Kinetic Updated");

    // Drag-reorder data-1 onto data-2 within Data section
    let from = widget_center(&driver, "nav.sheet.data-1")?;
    let to = widget_center(&driver, "nav.sheet.data-2")?;
    driver.drag(from, to)?;
    assert_eq!(driver.state().app.document().data_sheets[0].id.as_str(), "data-2");
    assert_eq!(driver.state().app.document().data_sheets[1].id.as_str(), "data-1");

    // Collapse Info folder so both Data and Result rows are simultaneously visible on 600px height.
    driver.click_widget_id("nav.section.info")?;

    // Drag ResultSheet (res-1) onto DataSheet (data-2) -> ChangeDataAnalyzed retarget modal
    let res_from = widget_center(&driver, "nav.sheet.res-1")?;
    let ds2_to = widget_center(&driver, "nav.sheet.data-2")?;
    driver.drag(res_from, ds2_to)?;
    assert!(matches!(
        driver.state().app.canvas_dialogs.active_dialog,
        ActiveDialog::ChangeDataAnalyzed { ref result_sheet_id } if result_sheet_id.as_str() == "res-1"
    ));
    assert_eq!(driver.state().app.canvas_dialogs.change_data.selected_target_id.as_ref().map(|id| id.as_str()), Some("data-2"));

    Ok(())
}

#[test]
fn family_lineage_active_chain_navigation_and_hover_preview_450ms() -> Result<(), Box<dyn std::error::Error>> {
    let app = create_pipeline_fixture_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 600.0);

    // Splitter exists around ~65% split
    assert!(driver.state().leading_splitter_rect_pt.is_some());
    assert!(driver.frame_meta().find_by_widget_id("family.node.data-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("family.node.res-1").is_some());

    // Click Family node res-1 navigates to ResultSheet and updates active-chain highlight
    driver.click_widget_id("family.node.res-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id, Some(SheetId::from("res-1")));

    // Hover over Family node: < 450ms should NOT show preview, >= 450ms MUST show preview
    let node_pos = widget_center(&driver, "family.node.res-1")?;
    driver.dispatch_events(vec![egui::Event::PointerMoved(node_pos)]);
    for _ in 0..10 {
        driver.dispatch_events(Vec::new()); // ~0.18s elapsed < 0.45s
    }
    assert!(driver.state().family_hover_preview_rect_pt.is_none(), "preview card must stay hidden before 450ms");

    driver.hover_widget_id("family.node.res-1")?; // dwells past tooltip_delay (~0.5s >= 0.45s)
    assert!(driver.state().family_hover_preview_rect_pt.is_some(), "preview card must render after >= 450ms hover");
    assert!(driver.frame_meta().find_by_widget_id("family.hover_preview").is_some());
    Ok(())
}

#[test]
fn responsive_viewports_and_all_14_beta_locales_zero_bleed_and_rasterizer() -> Result<(), Box<dyn std::error::Error>> {
    for &locale in &BETA_LOCALE_CODES {
        // 1. 640x420 Welcome Hub + 36pt collapsed Leading Dock rail
        let mut empty_app = new_test_app();
        empty_app.locale = locale.to_owned();
        let driver = make_shell_driver(empty_app, 640.0, 420.0);
        assert_clean_parity(&driver, &format!("welcome 640x420 locale={locale}"));
        let rail_w = driver.state().leading_dock_rect_pt.ok_or("640x420 leading rail missing")?.width();
        assert!(rail_w <= 38.0, "640x420 expected collapsed 36pt rail, got {rail_w}");

        // 2. 1440x600 populated Leading Dock + Family pane
        let mut pop_app = create_pipeline_fixture_app()?;
        pop_app.locale = locale.to_owned();
        let pop_driver = make_shell_driver(pop_app, 1440.0, 600.0);
        assert_clean_parity(&pop_driver, &format!("workspace 1440x600 locale={locale}"));
    }

    // Ellipsized long navigator title exposes full label in tooltip & AccessKit
    let long_title = "Very Long Experimental Condition Title For Ellipsis Verification";
    let mut long_app = create_pipeline_fixture_app()?;
    long_app.execute_command("sheet.rename", json!({ "sheet_id": "data-1", "title": long_title }))?;
    long_app.sync_views();
    let mut long_driver = make_shell_driver(long_app, 1440.0, 600.0);
    long_driver.verify_ellipsis_tooltip_and_accesskit("nav.sheet.data-1", long_title)?;

    // Light / Dark / HighContrast software rasterization non-blank smoke check
    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        let mut app = create_pipeline_fixture_app()?;
        app.set_theme_mode(mode);
        let (png, w, h) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1440, height_px: 600, dpi: 96 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((w, h), (1440, 600));
        assert!(!png.is_empty());
    }
    Ok(())
}
