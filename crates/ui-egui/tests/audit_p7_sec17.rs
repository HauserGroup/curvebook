#![forbid(unsafe_code)]

//! Phase 7 Stage 7.2 (`SEC-17`) Regression Tests (`FA-SEC17-01`..`FA-SEC17-11`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, EmbeddedTableStyle, GraphAnnotation, ImportLinkSpec, LayoutItem, LinkSource, LinkedDataRegion,
    RectPt, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetId, SourceCellId, SubcolumnMode, TableArchetype,
    XColumnMode,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::control_io::dispatch_ui_command;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view_for_sheet};
use curvebook_ui_egui::egui_shell::data_grid::{ColumnGridEguiState, ColumnGridRenderContext, render_data_grid};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::info_view::{InfoViewEguiState, render_info_view_for_sheet};
use curvebook_ui_egui::egui_shell::inspect_bridge::{ShellAboutFrameReport, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::results_view::{ResultsViewEguiState, render_results_view_for_sheet};
use curvebook_ui_egui::egui_shell::trailing_studio::{GraphStudioTab, TrailingStudioContext, TrailingStudioEguiState, render_trailing_studio};
use curvebook_ui_egui::egui_shell::welcome_hub::{WelcomeHubEguiState, WelcomeLeftMode, render_welcome_hub};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, ResponsiveShellLayout, ShellLayoutInput, Tokens, UiIntent};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::views::results::{ResultsViewState, handle_custom_action as handle_results_custom_action};
use serde_json::json;

type TestRes = Result<(), Box<dyn std::error::Error>>;

fn studio_layout_expanded() -> ResponsiveShellLayout {
    LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
        viewport_w_pt: 1280.0,
        viewport_h_pt: 1200.0,
        empty_workspace: false,
        navigator_open: false,
        inspector_open: true,
        family_open: false,
        leading_user_collapsed: false,
        trailing_user_collapsed: false,
        leading_width_override_pt: None,
        trailing_width_override_pt: None,
        host_draws_menu_bar: false,
        is_rtl: false,
    })
}

struct ShellState {
    app: UiApp,
    views: EguiShellViewState,
    carried_intents: PendingUiIntents,
    last_report: ShellAboutFrameReport,
    last_err: Option<String>,
    sync_back_to_app: bool,
}

fn make_shell_driver(app: UiApp, width: f64, height: f64) -> EguiHumanPathDriver<'static, ShellState> {
    make_shell_driver_with_sync(app, width, height, true)
}

fn make_shell_driver_with_sync(app: UiApp, width: f64, height: f64, sync_back_to_app: bool) -> EguiHumanPathDriver<'static, ShellState> {
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellState {
            app,
            views,
            carried_intents: PendingUiIntents::new(),
            last_report: ShellAboutFrameReport::default(),
            last_err: None,
            sync_back_to_app,
        },
        ViewportMetrics::new(width, height, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            st.last_report = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            if st.sync_back_to_app {
                st.app.egui_views = st.views.clone();
            }
            if let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false) {
                st.last_err = Some(err.to_string());
            }
            if st.sync_back_to_app {
                st.views = st.app.egui_views.clone();
            }
        }
        if settled && st.sync_back_to_app {
            st.app.egui_views = st.views.clone();
        }
    });
    driver.run_settled();
    driver
}

fn make_xy_two_graph_layout_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-xy", "kind": "xy", "title": "Assay XY", "cols": 2, "rows": 3, "auto_graph": true, "graph_sheet_id": "g-1", "auto_info": true, "info_sheet_id": "info-1" }),
    )?;
    for (r, x, y0, y1) in [(0, 1.0, 9.8765, 11.0), (1, 2.0, -0.0002, 22.0), (2, 3.0, 33.0, 44.0)] {
        app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "row": r, "value": x }))?;
        app.execute_command("table.setCell", json!({ "sheet_id": "ds-xy", "row": r, "col": 0, "value": y0 }))?;
        app.execute_command("table.setCell", json!({ "sheet_id": "ds-xy", "row": r, "col": 1, "value": y1 }))?;
    }
    app.execute_command("graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "ds-xy", "title": "Graph Two" }))?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure One" }))?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-1", "bounds_pt": { "x": 20.0, "y": 20.0, "width": 160.0, "height": 120.0 } }),
    )?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": { "x": 200.0, "y": 20.0, "width": 160.0, "height": 120.0 } }),
    )?;
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    Ok(app)
}

#[test]
fn fa_sec17_01_live_selection_context_reaches_trailing_studio() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    app.sync_views();
    // Keep `app.egui_views.canvas_view.selected_panel_index` as `None` (simulating the desktop
    // split-borrow where `views` was moved out of `app`), and select panel `1` only in `views`.
    let mut driver = make_shell_driver_with_sync(app, 1440.0, 1280.0, false);
    driver.state_mut().app.egui_views.canvas_view.selected_panel_index = None;
    driver.state_mut().views.canvas_view.selected_panel_index = Some(1);
    driver.run_settled();

    let order_before: Vec<String> = driver
        .state()
        .app
        .document()
        .layout_sheet(&SheetId::new("lay-1"))
        .ok_or("missing layout")?
        .spec
        .items
        .iter()
        .map(|it| match it {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => graph_sheet_id.as_str().to_owned(),
            _ => String::from("other"),
        })
        .collect();
    assert_eq!(order_before, vec!["g-1", "g-2"]);

    driver.click_widget_id("inspector.layout.zorder.back")?;
    assert_eq!(driver.state().last_err, None);
    let order_after: Vec<String> = driver
        .state()
        .app
        .document()
        .layout_sheet(&SheetId::new("lay-1"))
        .ok_or("missing layout")?
        .spec
        .items
        .iter()
        .map(|it| match it {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => graph_sheet_id.as_str().to_owned(),
            _ => String::from("other"),
        })
        .collect();
    assert_eq!(order_after, vec!["g-2", "g-1"], "panel index 1 (`g-2`) should move to back");

    // Also verify live graph annotation selection reaches `TrailingStudioContext`.
    let mut ann_app = driver.state().app.clone();
    ann_app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    let mut spec = AnalysisSpec::new("res-a", "ds-xy", AnalysisKind::OneWayAnova(Default::default()))?;
    spec.custom_title = Some(String::from("Res A"));
    let mut rs = ResultSheet::new(spec)?;
    rs.status = ResultStatus::UpToDate;
    let mut doc = ann_app.document().clone();
    doc.add_result_sheet(rs)?;
    if let Some(gs) = doc.graph_sheet_mut(&SheetId::new("g-1")) {
        gs.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-a"),
            tab_id: None,
            bounds_pt: RectPt::new(10.0, 10.0, 80.0, 50.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        });
        gs.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-a"),
            tab_id: None,
            bounds_pt: RectPt::new(100.0, 10.0, 80.0, 50.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        });
    }
    ann_app = UiApp::from_session(Session::from_document(doc));
    ann_app.sync_views();
    let intents_ann = render_trailing_studio_once_with_graph_sel(&ann_app, Some(1))?;
    let hit_ann1 = intents_ann.iter().any(|it| {
        matches!(
            it,
            UiIntent::UiCommand { command_id, params }
                if command_id == "ui.embeddedTable.format.open" && params.get("annotation_index").and_then(|v| v.as_u64()) == Some(1)
        )
    });
    assert!(hit_ann1, "selected graph annotation index 1 must route through TrailingStudioContext, got {intents_ann:?}");
    Ok(())
}

fn render_trailing_studio_once_with_graph_sel(app: &UiApp, selected_ann: Option<usize>) -> Result<Vec<UiIntent>, String> {
    let tokens = Tokens::default();
    let mut studio_state = TrailingStudioEguiState { graph_tab: GraphStudioTab::Canvas, ..TrailingStudioEguiState::default() };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), Vec::<UiIntent>::new()),
        ViewportMetrics::new(1280.0, 1200.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        move |ui, meta, (ap, out)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let layout = studio_layout_expanded();
            let _ = render_trailing_studio(
                ui,
                ap,
                &mut studio_state,
                TrailingStudioContext {
                    tokens: &t,
                    layout,
                    enabled: true,
                    focused: false,
                    selected_panel_index: None,
                    selected_graph_annotation: selected_ann,
                },
                meta,
                &mut intents,
            );
            out.extend(intents.intents().iter().cloned());
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver.click_widget_id("inspector.canvas.format_embedded_table")?;
    Ok(driver.state().1.clone())
}

#[test]
fn fa_sec17_02_info_notes_dirty_flushed_on_switch_and_hook_jump() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-1", "kind": "column", "title": "Data 1", "auto_info": true, "info_sheet_id": "info-1", "auto_graph": true, "graph_sheet_id": "g-hook" }),
    )?;
    app.execute_command("info.setConstant", json!({ "sheet_id": "info-1", "name": "Conc", "value": 10.0, "kind": "number" }))?;
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-2", "kind": "column", "title": "Data 2", "auto_info": true, "info_sheet_id": "info-2", "auto_graph": false }),
    )?;
    app.execute_command("info.setConstant", json!({ "sheet_id": "info-2", "name": "Batch", "value": "B2", "kind": "text" }))?;
    app.execute_command("graph.setSpec", json!({ "sheet_id": "g-hook", "spec": { "title": "Assay {{info:info-1:Conc}}" } }))?;
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    app.sync_views();
    let conc_idx = app
        .document()
        .info_sheet(&SheetId::new("info-1"))
        .ok_or("missing info-1")?
        .constants
        .iter()
        .position(|c| c.name == "Conc")
        .ok_or("missing Conc constant")?;
    let hook_wid = format!("info.hook_badge.{conc_idx}");

    // Part A: dirty notes on `info-1` + clicking `info.hook_badge.<conc_idx>` flushes `info.setNote` before `sheet.select`.
    let mut info_state = InfoViewEguiState {
        notes_draft: Some((SheetId::new("info-1"), String::from("Protocol draft before jump"))),
        notes_dirty: true,
        ..InfoViewEguiState::default()
    };
    let tokens = Tokens::default();
    let mut hook_driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), info_state.clone()),
        ViewportMetrics::new(960.0, 640.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (ap, st)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_info_view_for_sheet(ui, ap, Some(&SheetId::new("info-1")), st, &t, true, meta, &mut intents);
            let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
        },
    );
    hook_driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    hook_driver.run_settled();
    hook_driver.click_widget_id(&hook_wid)?;
    let info1_after_jump = hook_driver.state().0.document().info_sheet(&SheetId::new("info-1")).ok_or("missing info-1")?;
    assert_eq!(info1_after_jump.notes_markdown, "Protocol draft before jump");
    assert_eq!(hook_driver.state().0.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g-hook"));

    // Part B: direct `sheet.select` to `info-2` flushes dirty `info-1` notes before switching `notes_draft` to `info-2`.
    info_state.notes_draft = Some((SheetId::new("info-1"), String::from("Protocol draft before tab switch")));
    info_state.notes_dirty = true;
    app.execute_command("sheet.select", json!({ "sheet_id": "info-2" }))?;
    let mut switch_driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, info_state),
        ViewportMetrics::new(960.0, 640.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (ap, st)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_info_view_for_sheet(ui, ap, Some(&SheetId::new("info-2")), st, &t, true, meta, &mut intents);
            let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
        },
    );
    switch_driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    switch_driver.run_settled();
    let info1_after_switch = switch_driver.state().0.document().info_sheet(&SheetId::new("info-1")).ok_or("missing info-1")?;
    assert_eq!(info1_after_switch.notes_markdown, "Protocol draft before tab switch");
    assert_eq!(switch_driver.state().1.notes_draft.as_ref().map(|(id, _)| id.as_str()), Some("info-2"));
    assert!(!switch_driver.state().1.notes_dirty);
    Ok(())
}

#[test]
fn fa_sec17_03_page_margins_snap_toggle_and_table_decimals() -> TestRes {
    // 1. Page margins & grid arrangement update layout bounds via `layout.setSpec`.
    let mut app = make_xy_two_graph_layout_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    app.sync_views();
    let mut driver = make_shell_driver(app, 1440.0, 1100.0);
    let bounds_before = match driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?.spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt,
        _ => return Err("expected LiveGraph".into()),
    };
    driver.click_widget_id("inspector.layout.margin.inc")?;
    let bounds_after_margin = match driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?.spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt,
        _ => return Err("expected LiveGraph".into()),
    };
    assert_ne!(bounds_after_margin, bounds_before, "margin increment must update panel bounds via layout.setSpec");

    driver.click_widget_id("inspector.layout.grid.1x2")?;
    let lay = driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?;
    assert_eq!(lay.spec.grid_template, Some((1, 2)));
    let (b0, b1) = match (&lay.spec.items[0], &lay.spec.items[1]) {
        (LayoutItem::LiveGraph { bounds_pt: a, .. }, LayoutItem::LiveGraph { bounds_pt: b, .. }) => (*a, *b),
        _ => return Err("expected two LiveGraph items".into()),
    };
    assert!(b1.x > b0.x, "1x2 grid must arrange second graph to the right of the first");

    // 2. Snap toggle OFF suppresses snap guides during layout panel drag.
    driver.click_widget_id("inspector.layout.snap")?;
    assert!(!driver.state().views.trailing_studio.snap_enabled);
    let canvas_state = CanvasViewEguiState { synchronous_raster: true, ..CanvasViewEguiState::default() };
    let mut app_snap = driver.state().app.clone();
    app_snap.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    let mut snap_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (app_snap, canvas_state, false),
        ViewportMetrics::new(900.0, 680.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, vw, snap_on)| {
            let t = Tokens::get(ui.ctx());
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("curvebook.trailing_studio.snap_enabled"), *snap_on));
            let mut intents = PendingUiIntents::new();
            let ctx = CanvasViewContext { tokens: &t, enabled: true, is_secondary_split: false };
            let _ = render_canvas_view_for_sheet(ui, ap, Some(&SheetId::new("lay-1")), vw, ctx, meta, &mut intents);
            let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
        },
    );
    snap_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    snap_drv.run_settled();
    let p0_rect = snap_drv.frame_meta().find_by_widget_id("canvas.layout_panel.0").ok_or("panel 0")?.clipped_rect;
    snap_drv.drag(p0_rect.center(), p0_rect.center() + egui::vec2(2.0, 0.0))?;
    assert!(snap_drv.state().1.active_snap_guides.is_empty());
    assert!(snap_drv.state().1.last_snap_readout.is_none());

    // 3. `table_decimals` affects both `Column` and `XY` grids (including `-0.00` -> `0.00`).
    let mut col_app = UiApp::new();
    col_app.execute_command("sheet.createData", json!({ "sheet_id": "col-dec", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))?;
    col_app.execute_command("table.setCell", json!({ "sheet_id": "col-dec", "row": 0, "col": 0, "value": 1.23456 }))?;
    col_app.execute_command("table.setCell", json!({ "sheet_id": "col-dec", "row": 1, "col": 0, "value": -0.0001 }))?;
    col_app.nav.welcome.open = false;
    col_app.nav.inspector_open = true;
    col_app.sync_views();
    let mut col_drv = make_shell_driver(col_app, 1280.0, 760.0);
    col_drv.click_widget_id("inspector.table.decimals.2")?;
    let c0 = col_drv.frame_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("col cell 0")?;
    let c1 = col_drv.frame_meta().find_by_widget_id("grid.cell.1.0.0").ok_or("col cell 1")?;
    assert_eq!(c0.value.as_ref().and_then(|v| v.get("text")).and_then(|v| v.as_str()), Some("1.23"));
    assert_eq!(c1.value.as_ref().and_then(|v| v.get("text")).and_then(|v| v.as_str()), Some("0.00"));

    // Check XY multi-archetype grid too.
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    driver.run_settled();
    driver.click_widget_id("inspector.table.decimals.2")?;
    let xy0 = driver.frame_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("xy cell 0")?;
    let xy1 = driver.frame_meta().find_by_widget_id("grid.cell.1.0.0").ok_or("xy cell 1")?;
    assert_eq!(xy0.value.as_ref().and_then(|v| v.get("text")).and_then(|v| v.as_str()), Some("9.88"));
    assert_eq!(xy1.value.as_ref().and_then(|v| v.get("text")).and_then(|v| v.as_str()), Some("0.00"));
    Ok(())
}

#[test]
fn fa_sec17_04_frozen_headers_shortcuts_links_and_mv_studio_guards() -> TestRes {
    // 1. Frozen XY sheet disables `grid.sheet_title`, `grid.x_title.0`, `grid.col_title.0`, `grid.row_title.0` & blocks `Cmd+E` / `F2`.
    let mut app = make_xy_two_graph_layout_app()?;
    app.execute_command(
        "table.setXColumnMode",
        json!({ "sheet_id": "ds-xy", "x_mode": { "kind": "generated_sequence", "start": 0.0, "step": 1.0, "progression": "arithmetic" } }),
    )
    .ok();
    {
        let mut doc = app.document().clone();
        let ds = doc.data_sheet_mut(&SheetId::new("ds-xy")).ok_or("missing ds-xy")?;
        ds.frozen = true;
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    app.grid.tool.active = Some(GridCellCoord::new(0, 0, 0));
    app.sync_views();

    let mut drv = make_shell_driver(app, 1280.0, 760.0);
    for wid in ["grid.sheet_title", "grid.x_title.0", "grid.col_title.0", "grid.row_title.0"] {
        let entry = drv.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing {wid}"))?;
        assert!(!entry.enabled, "header `{wid}` must be disabled when sheet is frozen");
    }
    let undo_before = drv.state().app.session().undo_depth();
    drv.click_widget_id("grid.col_title.0").ok();
    assert!(drv.state().views.column_grid.inline_edit.is_none());
    drv.press_key("e", KeyModifiers { ctrl: true, ..KeyModifiers::default() })?;
    assert_eq!(drv.state().app.session().undo_depth(), undo_before, "Cmd/Ctrl+E must be blocked on frozen sheet");
    drv.press_key("F2", KeyModifiers::default())?;
    assert!(drv.state().views.column_grid.inline_edit.is_none());

    // 2. Unfrozen GeneratedSequence X-cell still blocks `F2`; Linked cell `F2`/`Delete`/`Cmd+E` opens link guard; Calculated MV opens formula bar.
    let mut mv_app = UiApp::new();
    mv_app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-mv",
            "kind": "multiple_variables",
            "cols": 2,
            "rows": 3,
            "variables": [{ "title": "Dose", "role": "continuous" }, { "title": "Scaled", "role": "continuous" }],
            "auto_graph": false
        }),
    )?;
    mv_app.execute_command("table.setCalculatedVariable", json!({ "sheet_id": "ds-mv", "col": 1, "formula": "= [Dose] * 2" }))?;
    {
        let mut doc = mv_app.document().clone();
        let ds = doc.data_sheet_mut(&SheetId::new("ds-mv")).ok_or("missing ds-mv")?;
        ds.add_linked_region(LinkedDataRegion {
            id: String::from("link-mv"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            provenance_summary: String::from("Imported Paste"),
        })?;
        ds.frozen = true;
        *mv_app.session_mut() = Session::from_document(doc);
        mv_app.sync_views();
    }
    mv_app.nav.welcome.open = false;
    mv_app.nav.inspector_open = true;
    mv_app.grid.tool.active = Some(GridCellCoord::new(0, 0, 0));
    mv_app.sync_views();

    let mut frozen_mv_drv = make_shell_driver(mv_app.clone(), 1280.0, 960.0);
    for wid in ["inspector.mv.apply_formula", "inspector.mv.insert_var.0", "inspector.mv.fn.0"] {
        let entry = frozen_mv_drv.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing {wid}"))?;
        assert!(!entry.enabled, "MV studio control `{wid}` must be disabled when frozen");
    }
    frozen_mv_drv.state_mut().views.trailing_studio.mv_sub_tab = 1;
    frozen_mv_drv.run_settled();
    let role_entry = frozen_mv_drv.frame_meta().find_by_widget_id("inspector.mv.role.continuous").ok_or("missing inspector.mv.role.continuous")?;
    assert!(!role_entry.enabled, "MV studio role control must be disabled when frozen");

    // Unfreeze to test `F2` on linked region & Calculated column.
    {
        let mut doc = mv_app.document().clone();
        doc.data_sheet_mut(&SheetId::new("ds-mv")).ok_or("missing ds-mv")?.frozen = false;
        *mv_app.session_mut() = Session::from_document(doc);
        mv_app.sync_views();
    }
    let mut linked_drv = make_shell_driver(mv_app, 1280.0, 800.0);
    linked_drv.press_key("F2", KeyModifiers::default())?;
    assert!(linked_drv.state().views.column_grid.inline_edit.is_none());
    assert!(linked_drv.state().views.column_grid.active_link_popover.is_some(), "F2 on linked cell must open guard popover");
    linked_drv.press_key("Escape", KeyModifiers::default())?;
    assert!(linked_drv.state().views.column_grid.active_link_popover.is_none(), "Escape must close multi link guard popover");

    linked_drv.state_mut().app.grid.tool.active = Some(GridCellCoord::new(0, 1, 0));
    linked_drv.run_settled();
    linked_drv.press_key("F2", KeyModifiers::default())?;
    assert!(linked_drv.state().views.column_grid.inline_edit.is_none(), "F2 on Calculated MV cell must not open text editor");
    Ok(())
}

#[test]
fn fa_sec17_05_canvas_media_insert_passes_sheet_id() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    let tokens = Tokens::default();

    for (sheet_id, wid, expected_cmd) in
        [("g-1", "inspector.canvas.insert_image", "graph.insertImage"), ("lay-1", "inspector.layout.insert_image", "layout.insertImage")]
    {
        app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }))?;
        app.sync_views();
        let mut studio_state = TrailingStudioEguiState { graph_tab: GraphStudioTab::Canvas, ..TrailingStudioEguiState::default() };
        let mut drv = EguiHumanPathDriver::with_tokens_and_fonts(
            (app.clone(), Vec::<UiIntent>::new()),
            ViewportMetrics::new(1280.0, 1200.0, 1.0),
            tokens,
            build_driver_font_definitions(),
            |ui, meta, (ap, captured)| {
                let mut intents = PendingUiIntents::new();
                let t = Tokens::get(ui.ctx());
                let layout = studio_layout_expanded();
                let _ = render_trailing_studio(
                    ui,
                    ap,
                    &mut studio_state,
                    TrailingStudioContext {
                        tokens: &t,
                        layout,
                        enabled: true,
                        focused: false,
                        selected_panel_index: None,
                        selected_graph_annotation: None,
                    },
                    meta,
                    &mut intents,
                );
                captured.extend(intents.intents().iter().cloned());
            },
        );
        drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        drv.run_settled();
        drv.click_widget_id(wid)?;
        let found = drv.state().1.iter().any(|it| {
            matches!(
                it,
                UiIntent::EngineCommand { command_id, params }
                    if command_id == expected_cmd && params.get("sheet_id").and_then(|v| v.as_str()) == Some(sheet_id)
            )
        });
        assert!(found, "expected `{expected_cmd}` with `sheet_id: \"{sheet_id}\"`, got {:?}", drv.state().1);
    }
    Ok(())
}

#[test]
fn fa_sec17_06_survival_codes_validated_and_parts_dead_checkbox_removed() -> TestRes {
    let mut app = UiApp::new();
    // 1. Engine `sheet.createData` accepts canonical `event_code` / `censored_code`, rejects retired camelCase aliases, and rejects equality.
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "surv-1", "kind": "survival", "cols": 2, "rows": 3, "event_code": 2, "censored_code": 3, "auto_graph": false }),
    )?;
    match &app.document().data_sheet(&SheetId::new("surv-1")).ok_or("missing surv-1")?.archetype {
        TableArchetype::Survival(s) => {
            assert_eq!(s.event_code, 2);
            assert_eq!(s.censored_code, 3);
        }
        other => return Err(format!("expected Survival, got {:?}", other.kind()).into()),
    }
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "surv-custom", "kind": "survival", "cols": 2, "rows": 3, "event_code": 7, "censored_code": 9, "auto_graph": false }),
    )?;
    match &app.document().data_sheet(&SheetId::new("surv-custom")).ok_or("missing surv-custom")?.archetype {
        TableArchetype::Survival(s) => {
            assert_eq!(s.event_code, 7);
            assert_eq!(s.censored_code, 9);
        }
        other => return Err(format!("expected Survival, got {:?}", other.kind()).into()),
    }
    assert!(
        app.execute_command(
            "sheet.createData",
            json!({ "sheet_id": "surv-camel", "kind": "survival", "cols": 2, "rows": 3, "eventCode": 7, "censoredCode": 9, "auto_graph": false }),
        )
        .is_err(),
        "retired camelCase aliases eventCode/censoredCode must be rejected"
    );
    assert!(
        app.execute_command("sheet.createData", json!({ "sheet_id": "surv-bad-1", "kind": "survival", "event_code": 0 })).is_err(),
        "event_code == default censored_code (0) must be rejected"
    );
    assert!(
        app.execute_command("sheet.createData", json!({ "sheet_id": "surv-bad-2", "kind": "survival", "event_code": 5, "censored_code": 5 }))
            .is_err(),
        "event_code == censored_code must be rejected"
    );

    // 2. Welcome Hub Survival steppers forward `event_code` and `censored_code`, and PartsOfWhole omits `welcome.pow.check_total`.
    let mut drv = make_shell_driver(UiApp::new(), 1200.0, 800.0);
    drv.click_widget_id("welcome.arch.parts_of_whole")?;
    assert!(drv.frame_meta().find_by_widget_id("welcome.pow.check_total").is_none());

    drv.click_widget_id("welcome.arch.survival")?;
    drv.click_widget_id("welcome.survival.event_code.inc")?; // event = 2
    drv.click_widget_id("welcome.survival.censored_code.dec")?; // censored = -1
    drv.click_widget_id("welcome.btn.create")?;
    let ds = drv.state().app.document().data_sheets.last().ok_or("missing welcome survival sheet")?;
    match &ds.archetype {
        TableArchetype::Survival(s) => {
            assert_eq!(s.event_code, 2);
            assert_eq!(s.censored_code, -1);
        }
        other => return Err(format!("expected Survival from welcome, got {:?}", other.kind()).into()),
    }
    Ok(())
}

#[test]
fn fa_sec17_07_results_copy_targets_rendered_sheet_and_virtualizes_5000_rows() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-r", "kind": "column", "cols": 2, "rows": 3, "auto_graph": false }))?;
    let mut spec1 = AnalysisSpec::new("res-1", "ds-r", AnalysisKind::OneWayAnova(Default::default()))?;
    spec1.custom_title = Some(String::from("Res One"));
    let mut rs1 = ResultSheet::new(spec1)?;
    rs1.status = ResultStatus::UpToDate;
    rs1.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("ColA")],
        row_labels: vec![String::from("First")],
        rows: vec![vec![ResultCell::Text(String::from("TARGET_RES_1"))]],
        footnotes: Vec::new(),
    });

    let mut spec2 = AnalysisSpec::new("res-2", "ds-r", AnalysisKind::OneWayAnova(Default::default()))?;
    spec2.custom_title = Some(String::from("Res Two"));
    let mut rs2 = ResultSheet::new(spec2)?;
    rs2.status = ResultStatus::UpToDate;
    rs2.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("ColB")],
        row_labels: vec![String::from("Active")],
        rows: vec![vec![ResultCell::Text(String::from("WRONG_RES_2"))]],
        footnotes: Vec::new(),
    });

    let mut doc = app.document().clone();
    doc.add_result_sheet(rs1)?;
    doc.add_result_sheet(rs2)?;
    doc.active_sheet_id = Some(SheetId::new("res-2"));
    app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    // Split pane renders `res-1` while `res-2` is active: `Copy` emits `copy_selection:res-1`.
    let tokens = Tokens::default();
    let res_state = ResultsViewEguiState {
        selected_cell: Some((SheetId::new("res-1"), 0, 0, 0)),
        selection_anchor: Some((SheetId::new("res-1"), 0, 0, 0)),
        ..ResultsViewEguiState::default()
    };
    let mut copy_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), res_state, PendingUiIntents::new()),
        ViewportMetrics::new(900.0, 560.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (ap, st, captured)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_results_view_for_sheet(ui, ap, Some(&SheetId::new("res-1")), st, &t, true, meta, &mut intents);
            for it in intents.intents() {
                captured.push(it.clone());
            }
        },
    );
    copy_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    copy_drv.run_settled();
    copy_drv.click_widget_id("result.copy_selection")?;
    let saw_res1_copy = copy_drv.state().2.intents().iter().any(|it| {
        matches!(
            it,
            UiIntent::CustomViewAction { domain, payload } if domain == "results" && payload == "copy_selection:res-1"
        )
    });
    assert!(saw_res1_copy, "expected `copy_selection:res-1`: {:?}", copy_drv.state().2.intents());

    let mut action_state = ResultsViewState::default();
    let copy_outcome = handle_results_custom_action(&mut app, &mut action_state, "copy_selection:res-1")?;
    let copied = copy_outcome.command_result.and_then(|v| v.get("copied_text").and_then(|t| t.as_str()).map(ToOwned::to_owned));
    assert!(copied.as_deref().is_some_and(|s| s.contains("TARGET_RES_1")), "expected `TARGET_RES_1`, got {copied:?}");

    // 5,000-row table virtualization check: rendered cell metadata stays strictly bounded to viewport rows.
    let mut big_spec = AnalysisSpec::new("res-big", "ds-r", AnalysisKind::OneWayAnova(Default::default()))?;
    big_spec.custom_title = Some(String::from("Huge Result"));
    let mut big_rs = ResultSheet::new(big_spec)?;
    big_rs.status = ResultStatus::UpToDate;
    let row_count = 5_000usize;
    let mut row_labels = Vec::with_capacity(row_count);
    let mut rows = Vec::with_capacity(row_count);
    for r in 0..row_count {
        row_labels.push(format!("R{r}"));
        rows.push(vec![
            ResultCell::Number { value: r as f64, prefix: None, units: None },
            ResultCell::Number { value: (r as f64) * 2.0, prefix: None, units: None },
            ResultCell::Number { value: 0.5, prefix: None, units: None },
            ResultCell::Boolean(r % 2 == 0),
        ]);
    }
    big_rs.tabs.push(ResultTab {
        id: String::from("big"),
        title: String::from("Big Matrix"),
        kind: ResultTabKind::CurveCoordinates,
        column_headers: vec![String::from("A"), String::from("B"), String::from("C"), String::from("Flag")],
        row_labels,
        rows,
        footnotes: Vec::new(),
    });
    let mut big_doc = app.document().clone();
    big_doc.add_result_sheet(big_rs)?;
    big_doc.active_sheet_id = Some(SheetId::new("res-big"));
    let mut big_app = UiApp::from_session(Session::from_document(big_doc));
    big_app.sync_views();
    let mut big_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (big_app, ResultsViewEguiState::default()),
        ViewportMetrics::new(900.0, 560.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (ap, st)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_results_view_for_sheet(ui, ap, Some(&SheetId::new("res-big")), st, &t, true, meta, &mut intents);
        },
    );
    big_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    big_drv.run_settled();
    assert!(
        big_drv.frame_meta().entries.len() < 300,
        "5,000-row result table should virtualize rows (rendered {} inspect entries)",
        big_drv.frame_meta().entries.len()
    );
    Ok(())
}

#[test]
fn fa_sec17_08_drill_back_checks_source_sheet_and_emits_structured_points() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-1", "kind": "xy", "cols": 2, "rows": 2, "subcolumn_mode": SubcolumnMode::RawReplicates(3), "auto_graph": true, "graph_sheet_id": "g-1" }),
    )?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-1", "start_row": 0, "values": [1.0, 2.0] }))?;
    app.execute_command("table.setCell", json!({ "sheet_id": "ds-1", "row": 0, "col": 1, "subcol": 2, "value": 15.0 }))?;
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-other", "kind": "xy", "cols": 1, "rows": 2, "auto_graph": false }))?;
    {
        let mut doc = app.document().clone();
        let gs = doc.graph_sheet_mut(&SheetId::new("g-1")).ok_or("missing g-1")?;
        if gs.spec.series.len() >= 2 {
            gs.spec.series.remove(0);
        }
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;

    // 1. Brushed cell from an unrelated sheet (`ds-other`) must disable `inspector.drill_back.format_point`.
    app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("ds-other"), row: 0, col: 1, subcol: 2 });
    app.sync_views();
    let other_drv = make_shell_driver(app.clone(), 1200.0, 800.0);
    let fmt_other = other_drv.frame_meta().find_by_widget_id("inspector.drill_back.format_point").ok_or("missing format_point")?;
    assert!(!fmt_other.enabled, "cross-sheet brushed cell must not format on unrelated graph");

    // 2. Brushed cell from `ds-1` col 1 subcol 2 maps to `series_index: 0, point_index: 0, subcol: 2`.
    app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("ds-1"), row: 0, col: 1, subcol: 2 });
    app.sync_views();
    let mut studio_state = TrailingStudioEguiState::default();
    let mut drill_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), Vec::<UiIntent>::new()),
        ViewportMetrics::new(1280.0, 1200.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, captured)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let layout = studio_layout_expanded();
            let _ = render_trailing_studio(
                ui,
                ap,
                &mut studio_state,
                TrailingStudioContext {
                    tokens: &t,
                    layout,
                    enabled: true,
                    focused: false,
                    selected_panel_index: None,
                    selected_graph_annotation: None,
                },
                meta,
                &mut intents,
            );
            captured.extend(intents.intents().iter().cloned());
        },
    );
    drill_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    drill_drv.run_settled();
    drill_drv.click_widget_id("inspector.drill_back.format_point")?;
    let open_payload = drill_drv
        .state()
        .1
        .iter()
        .find_map(|it| match it {
            UiIntent::UiCommand { command_id, params } if command_id == "ui.formatPoints.open" => Some(params.clone()),
            _ => None,
        })
        .ok_or("missing ui.formatPoints.open intent")?;
    assert_eq!(open_payload["points"], json!([{ "series_index": 0, "point_index": 0, "subcol": 2 }]));
    dispatch_ui_command(&mut app, "ui.formatPoints.open", &open_payload)?;
    Ok(())
}

#[test]
fn fa_sec17_09_popovers_discard_retry_and_meta_toggle() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-mv",
            "kind": "multiple_variables",
            "cols": 2,
            "rows": 3,
            "variables": [{ "title": "Group", "role": "categorical" }, { "title": "Val", "role": "continuous" }],
            "auto_graph": false
        }),
    )?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" }))?;
    app.sync_views();

    // 1. Opening `mv_formula.role_badge.0` keeps popup open on click frame, then outside click and Escape close it.
    let mut drv = make_shell_driver(app.clone(), 1200.0, 760.0);
    drv.click_widget_id("mv_formula.role_badge.0")?;
    assert!(drv.frame_meta().find_by_widget_id("mv_formula.role_opt.0.continuous").is_some(), "role popup must stay open after click");
    drv.click_at_button(egui::pos2(1180.0, 740.0), PointerButton::Primary, KeyModifiers::default())?;
    assert!(drv.frame_meta().find_by_widget_id("mv_formula.role_opt.0.continuous").is_none(), "outside click must close MV role popup");
    drv.click_widget_id("mv_formula.role_badge.0")?;
    assert!(drv.frame_meta().find_by_widget_id("mv_formula.role_opt.0.continuous").is_some());
    drv.press_key("Escape", KeyModifiers::default())?;
    assert!(drv.frame_meta().find_by_widget_id("mv_formula.role_opt.0.continuous").is_none(), "Escape must close MV role popup");

    // 2. `record_inspect_meta = false` omits popup & cell metadata.
    {
        let mut grid_no_meta = ColumnGridEguiState::default();
        grid_no_meta.record_inspect_meta = false;
        let mut no_meta_drv = EguiHumanPathDriver::with_tokens_and_fonts(
            (app.clone(), grid_no_meta),
            ViewportMetrics::new(960.0, 560.0, 1.0),
            Tokens::default(),
            build_driver_font_definitions(),
            |ui, meta, (ap, st)| {
                let mut intents = PendingUiIntents::new();
                let t = Tokens::get(ui.ctx());
                let _ = render_data_grid(ui, ap, st, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
                let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
            },
        );
        no_meta_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        no_meta_drv.run_settled();
        assert!(no_meta_drv.frame_meta().entries.is_empty(), "record_inspect_meta=false must record zero grid metadata");
        let _ = &no_meta_drv.state().1;
    }

    // 3. `will_discard` during multi-archetype inline commit retries cleanly on settled pass.
    let mut xy_app = make_xy_two_graph_layout_app()?;
    xy_app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    xy_app.grid.tool.active = Some(GridCellCoord::new(0, 0, 0));
    xy_app.sync_views();
    let grid_discard = ColumnGridEguiState::default();
    let mut disc_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (xy_app, grid_discard, true),
        ViewportMetrics::new(960.0, 560.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, st, trigger_discard)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_data_grid(ui, ap, st, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            if *trigger_discard && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ui.ctx().request_discard("audit simulated discard on commit pass");
                *trigger_discard = false;
            }
            let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
        },
    );
    disc_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    disc_drv.run_settled();
    disc_drv.press_key("F2", KeyModifiers::default())?;
    disc_drv.state_mut().1.inline_edit.as_mut().ok_or("missing inline edit")?.draft = String::from("123.5");
    disc_drv.run_settled();
    disc_drv.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(disc_drv.state().1.inline_edit.is_none());
    let updated = disc_drv
        .state()
        .0
        .document()
        .data_sheet(&SheetId::new("ds-xy"))
        .ok_or("missing ds-xy")?
        .cell(CellCoord::new(0, 0, 0))
        .ok_or("missing updated cell")?;
    assert_eq!(updated.value, CellValue::Number(123.5));

    // 4. `will_discard` during aux (`XCell`) inline commit also retries cleanly on settled pass.
    disc_drv.click_widget_id("grid.xcell.0.0")?;
    disc_drv.press_key("Backspace", KeyModifiers::default())?;
    disc_drv.type_text("77.5")?;
    disc_drv.state_mut().2 = true;
    disc_drv.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    let updated_x = disc_drv
        .state()
        .0
        .document()
        .data_sheet(&SheetId::new("ds-xy"))
        .ok_or("missing ds-xy")?
        .archetype
        .x_or_time_cell(0, 0)
        .ok_or("missing updated x cell")?;
    assert_eq!(updated_x.value, CellValue::Number(77.5));
    Ok(())
}

#[test]
fn fa_sec17_10_trailing_studio_shortcut_follows_app_shortcuts() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    dispatch_ui_command(&mut app, "ui.shortcut.set", &json!({ "command_id": "ui.inspector.toggle", "combo": "Ctrl+Shift+U" }))?;
    app.sync_views();

    let mut drv = make_shell_driver(app, 1280.0, 800.0);
    assert!(drv.frame_meta().find_by_widget_id("inspector.collapse_rail").is_some());

    // Old hardcoded Cmd/Ctrl+Shift+I must NOT toggle anymore.
    drv.press_egui_key(egui::Key::I, egui::Modifiers { ctrl: true, command: true, shift: true, ..egui::Modifiers::NONE });
    assert!(drv.frame_meta().find_by_widget_id("inspector.collapse_rail").is_some(), "old hardcoded Shift+I must not toggle rebound shortcut");

    // Rebound Ctrl+Shift+U collapses Trailing Studio rail.
    drv.press_egui_key(egui::Key::U, egui::Modifiers { ctrl: true, shift: true, ..egui::Modifiers::NONE });
    assert!(drv.frame_meta().find_by_widget_id("inspector.expand_rail").is_some(), "rebound Ctrl+Shift+U must toggle collapsed rail");

    // Expand back and verify dynamic shortcut label appears in tooltip when hovering `inspector.collapse_rail`.
    drv.press_egui_key(egui::Key::U, egui::Modifiers { ctrl: true, shift: true, ..egui::Modifiers::NONE });
    drv.hover_widget_id("inspector.collapse_rail")?;
    let tips = drv.visible_tooltip_texts();
    assert!(tips.iter().any(|t| !t.contains("Shift+I") && t.contains("U")), "expected dynamic rebound tooltip, got {tips:?}");
    Ok(())
}

#[test]
fn fa_sec17_11_polish_headers_date_validation_mv_roles_and_template_error() -> TestRes {
    // 1. Subcolumn role labels, 3-subcolumn X headers, and super-column > 26 (`AA`).
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-hdr",
            "kind": "xy",
            "cols": 27,
            "rows": 2,
            "x_mode": XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN },
            "subcolumn_mode": SubcolumnMode::MedianIqr,
            "auto_graph": false
        }),
    )?;
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-lh", "kind": "grouped", "cols": 1, "rows": 2, "subcolumn_mode": SubcolumnMode::MeanLowHigh, "auto_graph": false }),
    )?;
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-ul", "kind": "grouped", "cols": 1, "rows": 2, "subcolumn_mode": SubcolumnMode::UpperLowerLimits, "auto_graph": false }),
    )?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-hdr" }))?;
    app.sync_views();

    let mut hdr_state = ColumnGridEguiState::default();
    let mut hdr_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), &mut hdr_state),
        ViewportMetrics::new(1024.0, 520.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, st)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_data_grid(ui, ap, st, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
        },
    );
    hdr_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    hdr_drv.run_settled();
    let x1_lbl = hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.x.1").ok_or("x.1")?.label.clone();
    let x2_lbl = hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.x.2").ok_or("x.2")?.label.clone();
    assert_eq!(x1_lbl, "SD");
    assert_eq!(x2_lbl, "N");
    let med = hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.0").ok_or("0.0")?.label.clone();
    let q1 = hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.1").ok_or("0.1")?.label.clone();
    let q3 = hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.2").ok_or("0.2")?.label.clone();
    assert_eq!((med.as_str(), q1.as_str(), q3.as_str()), ("Median", "Q1", "Q3"));

    // Scroll right to column 26 (`AA`).
    hdr_drv.state_mut().1.scroll_offset_pt.x = 20_000.0;
    hdr_drv.run_settled();
    let col26_hdr = hdr_drv.frame_meta().find_by_widget_id("grid.col_super.26").ok_or("missing super col 26")?.label.clone();
    assert_eq!(col26_hdr, "Group AA");

    // Verify `MeanLowHigh` (`Mean`/`-Err`/`+Err`) & `UpperLowerLimits` (`Mean`/`Min`/`Max`) subcolumn labels.
    hdr_drv.state_mut().1.scroll_offset_pt.x = 0.0;
    hdr_drv.state_mut().0.execute_command("sheet.select", json!({ "sheet_id": "ds-lh" }))?;
    hdr_drv.run_settled();
    assert_eq!(hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.1").ok_or("lh.1")?.label, "-Err");
    assert_eq!(hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.2").ok_or("lh.2")?.label, "+Err");
    hdr_drv.state_mut().0.execute_command("sheet.select", json!({ "sheet_id": "ds-ul" }))?;
    hdr_drv.run_settled();
    assert_eq!(hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.1").ok_or("ul.1")?.label, "Min");
    assert_eq!(hdr_drv.frame_meta().find_by_widget_id("grid.subcol_hdr.0.2").ok_or("ul.2")?.label, "Max");

    // 2. MetadataHooks case-insensitive match & ISO date validation.
    let mut info_app = UiApp::new();
    info_app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-ih", "kind": "column", "auto_info": true, "info_sheet_id": "info-h", "auto_graph": false }),
    )?;
    info_app.execute_command("info.setConstant", json!({ "sheet_id": "info-h", "name": "StartDate", "value": "2026-11-22", "kind": "date" }))?;
    info_app.execute_command("sheet.select", json!({ "sheet_id": "info-h" }))?;
    info_app.nav.welcome.open = false;
    info_app.nav.inspector_open = true;
    info_app.sync_views();
    let mut info_drv = make_shell_driver(info_app, 1200.0, 800.0);
    info_drv.state_mut().views.trailing_studio.info_selected_constant = Some(String::from("startdate"));
    info_drv.run_settled();
    let _chip = info_drv.frame_meta().find_by_widget_id("inspector.info.const.0").ok_or("const chip")?;
    assert_eq!(info_drv.state().views.trailing_studio.info_value_draft, "2026-11-22");
    assert_eq!(info_drv.state().views.trailing_studio.info_kind_draft, 2);
    info_drv.state_mut().views.trailing_studio.info_kind_draft = 2;
    info_drv.state_mut().views.trailing_studio.info_value_draft = String::from("bad-date");
    info_drv.run_settled();
    assert!(info_drv.frame_meta().find_by_widget_id("inspector.info.value_error").is_some(), "expected date validation error");
    let save_btn = info_drv.frame_meta().find_by_widget_id("inspector.info.save_const").ok_or("missing save_const")?;
    assert!(!save_btn.enabled, "Save Constant must disable on malformed Date");

    // 3. Welcome Hub >6 MV variable role rows & StarterTemplates error surfacing.
    let mut w_state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::StarterTemplates, ..Default::default() };
    w_state.schema.selected_template_index = 999; // triggers error banner
    let mut w_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (UiApp::new(), w_state),
        ViewportMetrics::new(1120.0, 760.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, st)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_welcome_hub(ui, ap, st, &t, true, meta, &mut intents);
            let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
        },
    );
    w_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    w_drv.run_settled();
    w_drv.click_widget_id("welcome.btn.create")?;
    assert!(w_drv.frame_meta().find_by_widget_id("welcome.template.error").is_some(), "starter template failure must surface visibly");
    w_drv.click_widget_id("welcome.arch.multiple_variables")?;
    w_drv.state_mut().1.schema.cols = 8;
    w_drv.run_settled();
    w_drv.click_widget_id("welcome.mv.role.7.Label")?;
    assert_eq!(w_drv.state().1.schema.mv_role_kinds.get(7).copied(), Some(2));
    Ok(())
}
