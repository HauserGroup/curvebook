//! Phase 7 Stage 7.2 (`SEC-17`) Regression Tests (`FA-SEC17-01`..`FA-SEC17-11`).
#![forbid(unsafe_code)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, EmbeddedTableStyle, GraphAnnotation, ImportLinkSpec, LayoutItem, LinkSource, LinkedDataRegion,
    RectPt, ResultCell, ResultSheet, ResultStatus, ResultTabKind, SheetCellRange, SheetId, SourceCellId, SubcolumnMode, TableArchetype, XColumnMode,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::copy_result_table_to_clipboard;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view_for_sheet};
use curvebook_ui_egui::egui_shell::data_grid::{ColumnGridEguiState, ColumnGridRenderContext, render_data_grid};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::info_view::{InfoViewEguiState, render_info_view_for_sheet};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::results_view::{ResultsViewEguiState, render_results_view_for_sheet};
use curvebook_ui_egui::egui_shell::trailing_studio::{GraphStudioTab, TrailingStudioContext, TrailingStudioEguiState, render_trailing_studio};
use curvebook_ui_egui::egui_shell::welcome_hub::{WelcomeHubEguiState, WelcomeLeftMode, render_welcome_hub};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, ResponsiveShellLayout, ShellLayoutInput, Tokens, UiIntent};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use serde_json::json;

use crate::common::{TestRes, widget_rect};
use crate::helpers::{exec, has_widget, make_merge_driver, make_result_tab, run_panel_driver, ui_cmd, widget_enabled, widget_label, widget_value};

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

fn click_trailing_studio(app: &UiApp, graph_tab: GraphStudioTab, selected_ann: Option<usize>, wid: &str) -> Result<Vec<UiIntent>, String> {
    let mut studio = TrailingStudioEguiState { graph_tab, ..Default::default() };
    let mut drv = run_panel_driver((app.clone(), Vec::<UiIntent>::new()), 1280.0, 1200.0, move |ui, meta, (ap, out)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let ctx = TrailingStudioContext {
            tokens: &t,
            layout: studio_layout_expanded(),
            enabled: true,
            focused: false,
            selected_panel_index: None,
            selected_graph_annotation: selected_ann,
        };
        let _ = render_trailing_studio(ui, ap, &mut studio, ctx, meta, &mut intents);
        out.extend(intents.intents().iter().cloned());
    });
    drv.click_widget_id(wid)?;
    Ok(drv.state().1.clone())
}

fn make_xy_two_graph_layout_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-xy", "kind": "xy", "title": "Assay XY", "cols": 2, "rows": 3, "auto_graph": true, "graph_sheet_id": "g-1", "auto_info": true, "info_sheet_id": "info-1" }),
    )?;
    for (r, x, y0, y1) in [(0, 1.0, 9.8765, 11.0), (1, 2.0, -0.0002, 22.0), (2, 3.0, 33.0, 44.0)] {
        exec(&mut app, "table.setXCells", json!({ "sheet_id": "ds-xy", "row": r, "value": x }))?;
        exec(&mut app, "table.setCell", json!({ "sheet_id": "ds-xy", "row": r, "col": 0, "value": y0 }))?;
        exec(&mut app, "table.setCell", json!({ "sheet_id": "ds-xy", "row": r, "col": 1, "value": y1 }))?;
    }
    exec(&mut app, "graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "ds-xy", "title": "Graph Two" }))?;
    exec(&mut app, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure One" }))?;
    for (gid, x) in [("g-1", 20.0), ("g-2", 200.0)] {
        exec(
            &mut app,
            "layout.addPanel",
            json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": gid, "bounds_pt": { "x": x, "y": 20.0, "width": 160.0, "height": 120.0 } }),
        )?;
    }
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    Ok(app)
}

fn layout_graph_ids(app: &UiApp) -> Result<Vec<String>, String> {
    Ok(app
        .document()
        .layout_sheet(&SheetId::new("lay-1"))
        .ok_or("missing lay-1")?
        .spec
        .items
        .iter()
        .map(|it| match it {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => graph_sheet_id.as_str().to_owned(),
            _ => String::from("other"),
        })
        .collect())
}

#[test]
fn fa_sec17_01_live_selection_context_reaches_trailing_studio() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "lay-1" }))?;
    app.sync_views();

    // Simulate desktop split-borrow where `views` is moved out of `app` (app.egui_views stays None).
    struct SplitState {
        app: UiApp,
        views: EguiShellViewState,
        intents: PendingUiIntents,
        last_err: Option<String>,
    }
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        SplitState { app, views, intents: PendingUiIntents::new(), last_err: None },
        ViewportMetrics::new(1440.0, 1280.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass);
            st.intents.merge_pass(pass);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled
            && !st.intents.is_empty()
            && let Err(e) = st.intents.commit_if_settled(&mut st.app, false)
        {
            st.last_err = Some(e.to_string());
        }
    });
    driver.run_settled();
    driver.state_mut().app.egui_views.canvas_view.selected_panel_index = None;
    driver.state_mut().views.canvas_view.selected_panel_index = Some(1);
    driver.run_settled();

    assert_eq!(layout_graph_ids(&driver.state().app)?, vec!["g-1", "g-2"]);
    driver.click_widget_id("inspector.layout.zorder.back")?;
    assert_eq!(driver.state().last_err, None);
    assert_eq!(layout_graph_ids(&driver.state().app)?, vec!["g-2", "g-1"]);

    // Verify live graph annotation selection routes through `TrailingStudioContext`.
    let mut ann_app = driver.state().app.clone();
    exec(&mut ann_app, "sheet.select", json!({ "sheet_id": "g-1" }))?;
    let mut spec = AnalysisSpec::new("res-a", "ds-xy", AnalysisKind::OneWayAnova(Default::default())).map_err(|e| e.to_string())?;
    spec.custom_title = Some(String::from("Res A"));
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.status = ResultStatus::UpToDate;
    let mut doc = ann_app.document().clone();
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    for x in [10.0, 100.0] {
        doc.graph_sheet_mut(&SheetId::new("g-1")).ok_or("missing g-1")?.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-a"),
            tab_id: None,
            bounds_pt: RectPt::new(x, 10.0, 80.0, 50.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        });
    }
    ann_app = UiApp::from_session(Session::from_document(doc));
    ann_app.sync_views();
    let intents = click_trailing_studio(&ann_app, GraphStudioTab::Canvas, Some(1), "inspector.canvas.format_embedded_table")?;
    assert!(intents.iter().any(|it| matches!(it, UiIntent::UiCommand { command_id, params } if command_id == "ui.embeddedTable.format.open" && params["annotation_index"] == 1)));
    Ok(())
}

#[test]
fn fa_sec17_02_info_notes_dirty_flushed_on_switch_and_hook_jump() -> TestRes {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-1", "kind": "column", "title": "Data 1", "auto_info": true, "info_sheet_id": "info-1", "auto_graph": true, "graph_sheet_id": "g-hook" }),
    )?;
    exec(&mut app, "info.setConstant", json!({ "sheet_id": "info-1", "name": "Conc", "value": 10.0, "kind": "number" }))?;
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-2", "kind": "column", "title": "Data 2", "auto_info": true, "info_sheet_id": "info-2", "auto_graph": false }),
    )?;
    exec(&mut app, "info.setConstant", json!({ "sheet_id": "info-2", "name": "Batch", "value": "B2", "kind": "text" }))?;
    exec(&mut app, "graph.setSpec", json!({ "sheet_id": "g-hook", "spec": { "title": "Assay {{info:info-1:Conc}}" } }))?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "info-1" }))?;
    app.sync_views();
    let conc_idx = app
        .document()
        .info_sheet(&SheetId::new("info-1"))
        .ok_or("missing info-1")?
        .constants
        .iter()
        .position(|c| c.name == "Conc")
        .ok_or("missing Conc")?;

    let run_info = |ap: UiApp, st: InfoViewEguiState, sid: &'static str| {
        run_panel_driver((ap, st), 960.0, 640.0, move |ui, meta, (a, s)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_info_view_for_sheet(ui, a, Some(&SheetId::new(sid)), s, &t, true, meta, &mut intents);
            let _ = intents.commit_if_settled(a, ui.ctx().will_discard());
        })
    };

    let mut info_state = InfoViewEguiState {
        notes_draft: Some((SheetId::new("info-1"), String::from("Protocol draft before jump"))),
        notes_dirty: true,
        ..Default::default()
    };
    let mut hook_drv = run_info(app.clone(), info_state.clone(), "info-1");
    hook_drv.click_widget_id(&format!("info.hook_badge.{conc_idx}"))?;
    assert_eq!(hook_drv.state().0.document().info_sheet(&SheetId::new("info-1")).ok_or("info-1")?.notes_markdown, "Protocol draft before jump");
    assert_eq!(hook_drv.state().0.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g-hook"));

    info_state.notes_draft = Some((SheetId::new("info-1"), String::from("Protocol draft before tab switch")));
    info_state.notes_dirty = true;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "info-2" }))?;
    let sw_drv = run_info(app, info_state, "info-2");
    assert_eq!(sw_drv.state().0.document().info_sheet(&SheetId::new("info-1")).ok_or("info-1")?.notes_markdown, "Protocol draft before tab switch");
    assert_eq!(sw_drv.state().1.notes_draft.as_ref().map(|(id, _)| id.as_str()), Some("info-2"));
    assert!(!sw_drv.state().1.notes_dirty);
    Ok(())
}

#[test]
fn fa_sec17_03_page_margins_snap_toggle_and_table_decimals() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "lay-1" }))?;
    app.sync_views();
    let mut driver = make_merge_driver(app, 1440.0, 1100.0);
    let panel_bounds = |ap: &UiApp, i: usize| -> Result<RectPt, String> {
        match ap.document().layout_sheet(&SheetId::new("lay-1")).ok_or("lay-1")?.spec.items[i] {
            LayoutItem::LiveGraph { bounds_pt, .. } => Ok(bounds_pt),
            _ => Err(String::from("expected LiveGraph")),
        }
    };

    let b_before = panel_bounds(&driver.state().app, 0)?;
    driver.click_widget_id("inspector.layout.margin.inc")?;
    assert_ne!(panel_bounds(&driver.state().app, 0)?, b_before);

    driver.click_widget_id("inspector.layout.grid.1x2")?;
    assert_eq!(driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("lay-1")?.spec.grid_template, Some((1, 2)));
    assert!(panel_bounds(&driver.state().app, 1)?.x > panel_bounds(&driver.state().app, 0)?.x);

    // Snap toggle OFF suppresses snap guides during drag.
    driver.click_widget_id("inspector.layout.snap")?;
    assert!(!driver.state().views.trailing_studio.snap_enabled);
    let mut app_snap = driver.state().app.clone();
    exec(&mut app_snap, "sheet.select", json!({ "sheet_id": "lay-1" }))?;
    let mut snap_drv =
        run_panel_driver((app_snap, CanvasViewEguiState { synchronous_raster: true, ..Default::default() }), 900.0, 680.0, |ui, meta, (ap, vw)| {
            let t = Tokens::get(ui.ctx());
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("curvebook.trailing_studio.snap_enabled"), false));
            let mut intents = PendingUiIntents::new();
            let _ = render_canvas_view_for_sheet(
                ui,
                ap,
                Some(&SheetId::new("lay-1")),
                vw,
                CanvasViewContext { tokens: &t, enabled: true, is_secondary_split: false },
                meta,
                &mut intents,
            );
            let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
        });
    let p0 = widget_rect(&snap_drv, "canvas.layout_panel.0")?.center();
    snap_drv.drag(p0, p0 + egui::vec2(2.0, 0.0))?;
    assert!(snap_drv.state().1.active_snap_guides.is_empty() && snap_drv.state().1.last_snap_readout.is_none());

    // `table_decimals` affects both Column and XY grids (including `-0.00` -> `0.00`).
    let mut col_app = UiApp::new();
    exec(&mut col_app, "sheet.createData", json!({ "sheet_id": "col-dec", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))?;
    exec(&mut col_app, "table.setCell", json!({ "sheet_id": "col-dec", "row": 0, "col": 0, "value": 1.23456 }))?;
    exec(&mut col_app, "table.setCell", json!({ "sheet_id": "col-dec", "row": 1, "col": 0, "value": -0.0001 }))?;
    col_app.nav.welcome.open = false;
    col_app.nav.inspector_open = true;
    col_app.sync_views();
    let mut col_drv = make_merge_driver(col_app, 1280.0, 760.0);
    col_drv.click_widget_id("inspector.table.decimals.2")?;
    assert_eq!(widget_value(&col_drv, "grid.cell.0.0.0")?.as_ref().and_then(|v| v["text"].as_str()), Some("1.23"));
    assert_eq!(widget_value(&col_drv, "grid.cell.1.0.0")?.as_ref().and_then(|v| v["text"].as_str()), Some("0.00"));

    exec(&mut driver.state_mut().app, "sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    driver.run_settled();
    driver.click_widget_id("inspector.table.decimals.2")?;
    assert_eq!(widget_value(&driver, "grid.cell.0.0.0")?.as_ref().and_then(|v| v["text"].as_str()), Some("9.88"));
    assert_eq!(widget_value(&driver, "grid.cell.1.0.0")?.as_ref().and_then(|v| v["text"].as_str()), Some("0.00"));
    Ok(())
}

#[test]
fn fa_sec17_04_frozen_headers_shortcuts_links_and_mv_studio_guards() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    let _ = app.execute_command(
        "table.setXColumnMode",
        json!({ "sheet_id": "ds-xy", "x_mode": { "kind": "generated_sequence", "start": 0.0, "step": 1.0, "progression": "arithmetic" } }),
    );
    let mut doc = app.document().clone();
    doc.data_sheet_mut(&SheetId::new("ds-xy")).ok_or("ds-xy")?.frozen = true;
    *app.session_mut() = Session::from_document(doc);
    exec(&mut app, "sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    app.grid.tool.active = Some(GridCellCoord::new(0, 0, 0));
    app.sync_views();

    let mut drv = make_merge_driver(app, 1280.0, 760.0);
    for wid in ["grid.sheet_title", "grid.x_title.0", "grid.col_title.0", "grid.row_title.0"] {
        assert!(!widget_enabled(&drv, wid)?, "`{wid}` must be disabled when frozen");
    }
    let undo_before = drv.state().app.session().undo_depth();
    let _ = drv.click_widget_id("grid.col_title.0");
    assert!(drv.state().views.column_grid.inline_edit.is_none());
    drv.press_key("e", KeyModifiers { ctrl: true, ..Default::default() })?;
    assert_eq!(drv.state().app.session().undo_depth(), undo_before);
    drv.press_key("F2", KeyModifiers::default())?;
    assert!(drv.state().views.column_grid.inline_edit.is_none());

    let mut mv_app = UiApp::new();
    exec(
        &mut mv_app,
        "sheet.createData",
        json!({ "sheet_id": "ds-mv", "kind": "multiple_variables", "cols": 2, "rows": 3, "variables": [{ "title": "Dose", "role": "continuous" }, { "title": "Scaled", "role": "continuous" }], "auto_graph": false }),
    )?;
    exec(&mut mv_app, "table.setCalculatedVariable", json!({ "sheet_id": "ds-mv", "col": 1, "formula": "= [Dose] * 2" }))?;
    let mut mv_doc = mv_app.document().clone();
    let mv_ds = mv_doc.data_sheet_mut(&SheetId::new("ds-mv")).ok_or("ds-mv")?;
    mv_ds
        .add_linked_region(LinkedDataRegion {
            id: String::from("link-mv"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            provenance_summary: String::from("Imported Paste"),
        })
        .map_err(|e| e.to_string())?;
    mv_ds.frozen = true;
    *mv_app.session_mut() = Session::from_document(mv_doc);
    mv_app.nav.welcome.open = false;
    mv_app.nav.inspector_open = true;
    mv_app.grid.tool.active = Some(GridCellCoord::new(0, 0, 0));
    mv_app.sync_views();

    let mut frozen_mv = make_merge_driver(mv_app.clone(), 1280.0, 960.0);
    for wid in ["inspector.mv.apply_formula", "inspector.mv.insert_var.0", "inspector.mv.fn.0"] {
        assert!(!widget_enabled(&frozen_mv, wid)?, "`{wid}` disabled when frozen");
    }
    frozen_mv.state_mut().views.trailing_studio.mv_sub_tab = 1;
    frozen_mv.run_settled();
    assert!(!widget_enabled(&frozen_mv, "inspector.mv.role.continuous")?);

    let mut unfrozen_doc = mv_app.document().clone();
    unfrozen_doc.data_sheet_mut(&SheetId::new("ds-mv")).ok_or("ds-mv")?.frozen = false;
    *mv_app.session_mut() = Session::from_document(unfrozen_doc);
    mv_app.sync_views();
    let mut linked = make_merge_driver(mv_app, 1280.0, 800.0);
    linked.press_key("F2", KeyModifiers::default())?;
    assert!(linked.state().views.column_grid.inline_edit.is_none() && linked.state().views.column_grid.active_link_popover.is_some());
    linked.press_key("Escape", KeyModifiers::default())?;
    assert!(linked.state().views.column_grid.active_link_popover.is_none());

    linked.state_mut().app.grid.tool.active = Some(GridCellCoord::new(0, 1, 0));
    linked.run_settled();
    linked.press_key("F2", KeyModifiers::default())?;
    assert!(linked.state().views.column_grid.inline_edit.is_none());
    Ok(())
}

#[test]
fn fa_sec17_05_canvas_media_insert_passes_sheet_id() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    for (sid, wid, expected_cmd) in
        [("g-1", "inspector.canvas.insert_image", "graph.insertImage"), ("lay-1", "inspector.layout.insert_image", "layout.insertImage")]
    {
        exec(&mut app, "sheet.select", json!({ "sheet_id": sid }))?;
        app.sync_views();
        let intents = click_trailing_studio(&app, GraphStudioTab::Canvas, None, wid)?;
        assert!(
            intents
                .iter()
                .any(|it| matches!(it, UiIntent::EngineCommand { command_id, params } if command_id == expected_cmd && params["sheet_id"] == sid))
        );
    }
    Ok(())
}

#[test]
fn fa_sec17_06_survival_codes_validated_and_parts_dead_checkbox_removed() -> TestRes {
    let mut app = UiApp::new();
    for (sid, ev, cen) in [("surv-1", 2, 3), ("surv-custom", 7, 9)] {
        exec(
            &mut app,
            "sheet.createData",
            json!({ "sheet_id": sid, "kind": "survival", "cols": 2, "rows": 3, "event_code": ev, "censored_code": cen, "auto_graph": false }),
        )?;
        let TableArchetype::Survival(s) = &app.document().data_sheet(&SheetId::new(sid)).ok_or(sid)?.archetype else {
            return Err("expected Survival".into());
        };
        assert_eq!((s.event_code, s.censored_code), (ev, cen));
    }
    for bad in [
        json!({ "sheet_id": "surv-camel", "kind": "survival", "cols": 2, "rows": 3, "eventCode": 7, "censoredCode": 9, "auto_graph": false }),
        json!({ "sheet_id": "surv-bad-1", "kind": "survival", "event_code": 0 }),
        json!({ "sheet_id": "surv-bad-2", "kind": "survival", "event_code": 5, "censored_code": 5 }),
    ] {
        assert!(app.execute_command("sheet.createData", bad).is_err());
    }

    let mut drv = make_merge_driver(UiApp::new(), 1200.0, 800.0);
    drv.click_widget_id("welcome.arch.parts_of_whole")?;
    assert!(!has_widget(&drv, "welcome.pow.check_total"));
    for wid in ["welcome.arch.survival", "welcome.survival.event_code.inc", "welcome.survival.censored_code.dec", "welcome.btn.create"] {
        drv.click_widget_id(wid)?;
    }
    let TableArchetype::Survival(s) = &drv.state().app.document().data_sheets.last().ok_or("missing sheet")?.archetype else {
        return Err("expected Survival".into());
    };
    assert_eq!((s.event_code, s.censored_code), (2, -1));
    Ok(())
}

#[test]
fn fa_sec17_07_results_copy_targets_rendered_sheet_and_virtualizes_5000_rows() -> TestRes {
    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "ds-r", "kind": "column", "cols": 2, "rows": 3, "auto_graph": false }))?;
    let mut doc = app.document().clone();
    for (id, title, hdr, rlbl, cell) in [("res-1", "Res One", "ColA", "First", "TARGET_RES_1"), ("res-2", "Res Two", "ColB", "Active", "WRONG_RES_2")]
    {
        let mut sp = AnalysisSpec::new(id, "ds-r", AnalysisKind::OneWayAnova(Default::default())).map_err(|e| e.to_string())?;
        sp.custom_title = Some(title.into());
        let mut rs = ResultSheet::new(sp).map_err(|e| e.to_string())?;
        rs.status = ResultStatus::UpToDate;
        rs.tabs.push(make_result_tab("main", "Main", ResultTabKind::MainSummary, &[hdr], &[rlbl], vec![vec![ResultCell::Text(cell.into())]]));
        doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    }
    doc.active_sheet_id = Some(SheetId::new("res-2"));
    app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    let res_state = ResultsViewEguiState {
        selected_cell: Some((SheetId::new("res-1"), 0, 0, 0)),
        selection_anchor: Some((SheetId::new("res-1"), 0, 0, 0)),
        ..Default::default()
    };
    let mut copy_drv = run_panel_driver((app.clone(), res_state, PendingUiIntents::new()), 900.0, 560.0, |ui, meta, (ap, st, cap)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let _ = render_results_view_for_sheet(ui, ap, Some(&SheetId::new("res-1")), st, &t, true, meta, &mut intents);
        for it in intents.intents() {
            cap.push(it.clone());
        }
    });
    copy_drv.click_widget_id("result.copy_selection")?;
    assert!(
        copy_drv
            .state()
            .2
            .intents()
            .iter()
            .any(|it| matches!(it, UiIntent::CopyResultTable { sheet_id, force_full: false } if sheet_id.as_str() == "res-1"))
    );
    let copied = copy_result_table_to_clipboard(&mut app, "res-1", false).command_result.and_then(|v| v["copied_text"].as_str().map(str::to_owned));
    assert!(copied.as_deref().is_some_and(|s| s.contains("TARGET_RES_1")));

    // 5,000-row result table virtualization check.
    let mut big_sp = AnalysisSpec::new("res-big", "ds-r", AnalysisKind::OneWayAnova(Default::default())).map_err(|e| e.to_string())?;
    big_sp.custom_title = Some("Huge Result".into());
    let mut big_rs = ResultSheet::new(big_sp).map_err(|e| e.to_string())?;
    big_rs.status = ResultStatus::UpToDate;
    let (row_labels, rows): (Vec<String>, Vec<Vec<ResultCell>>) = (0..5_000usize)
        .map(|r| {
            (
                format!("R{r}"),
                vec![
                    ResultCell::Number { value: r as f64, prefix: None, units: None },
                    ResultCell::Number { value: (r as f64) * 2.0, prefix: None, units: None },
                    ResultCell::Number { value: 0.5, prefix: None, units: None },
                    ResultCell::Boolean(r % 2 == 0),
                ],
            )
        })
        .unzip();
    big_rs.tabs.push(curvebook_doc::ResultTab {
        id: "big".into(),
        title: "Big Matrix".into(),
        kind: ResultTabKind::CurveCoordinates,
        column_headers: vec!["A".into(), "B".into(), "C".into(), "Flag".into()],
        row_labels,
        rows,
        footnotes: Vec::new(),
    });
    let mut big_doc = app.document().clone();
    big_doc.add_result_sheet(big_rs).map_err(|e| e.to_string())?;
    big_doc.active_sheet_id = Some(SheetId::new("res-big"));
    let mut big_app = UiApp::from_session(Session::from_document(big_doc));
    big_app.sync_views();
    let big_drv = run_panel_driver((big_app, ResultsViewEguiState::default()), 900.0, 560.0, |ui, meta, (ap, st)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let _ = render_results_view_for_sheet(ui, ap, Some(&SheetId::new("res-big")), st, &t, true, meta, &mut intents);
    });
    assert!(big_drv.frame_meta().entries.len() < 300, "5000-row table rendered {} entries", big_drv.frame_meta().entries.len());
    Ok(())
}

#[test]
fn fa_sec17_08_drill_back_checks_source_sheet_and_emits_structured_points() -> TestRes {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-1", "kind": "xy", "cols": 2, "rows": 2, "subcolumn_mode": SubcolumnMode::RawReplicates(3), "auto_graph": true, "graph_sheet_id": "g-1" }),
    )?;
    exec(&mut app, "table.setXCells", json!({ "sheet_id": "ds-1", "start_row": 0, "values": [1.0, 2.0] }))?;
    exec(&mut app, "table.setCell", json!({ "sheet_id": "ds-1", "row": 0, "col": 1, "subcol": 2, "value": 15.0 }))?;
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "ds-other", "kind": "xy", "cols": 1, "rows": 2, "auto_graph": false }))?;
    let mut doc = app.document().clone();
    let gs = doc.graph_sheet_mut(&SheetId::new("g-1")).ok_or("g-1")?;
    if gs.spec.series.len() >= 2 {
        gs.spec.series.remove(0);
    }
    *app.session_mut() = Session::from_document(doc);
    exec(&mut app, "sheet.select", json!({ "sheet_id": "g-1" }))?;

    app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("ds-other"), row: 0, col: 1, subcol: 2 });
    app.sync_views();
    assert!(!widget_enabled(&make_merge_driver(app.clone(), 1200.0, 800.0), "inspector.drill_back.format_point")?);

    app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("ds-1"), row: 0, col: 1, subcol: 2 });
    app.sync_views();
    let intents = click_trailing_studio(&app, GraphStudioTab::Data, None, "inspector.drill_back.format_point")?;
    let payload = intents
        .into_iter()
        .find_map(|it| match it {
            UiIntent::UiCommand { command_id, params } if command_id == "ui.formatPoints.open" => Some(params),
            _ => None,
        })
        .ok_or("missing ui.formatPoints.open")?;
    assert_eq!(payload["points"], json!([{ "series_index": 0, "point_index": 0, "subcol": 2 }]));
    ui_cmd(&mut app, "ui.formatPoints.open", payload)?;
    Ok(())
}

#[test]
fn fa_sec17_09_popovers_discard_retry_and_meta_toggle() -> TestRes {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-mv", "kind": "multiple_variables", "cols": 2, "rows": 3, "variables": [{ "title": "Group", "role": "categorical" }, { "title": "Val", "role": "continuous" }], "auto_graph": false }),
    )?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "ds-mv" }))?;
    app.sync_views();

    let mut drv = make_merge_driver(app.clone(), 1200.0, 760.0);
    drv.click_widget_id("mv_formula.role_badge.0")?;
    assert!(has_widget(&drv, "mv_formula.role_opt.0.continuous"));
    drv.click_at_button(egui::pos2(1180.0, 740.0), PointerButton::Primary, KeyModifiers::default())?;
    assert!(!has_widget(&drv, "mv_formula.role_opt.0.continuous"));
    drv.click_widget_id("mv_formula.role_badge.0")?;
    assert!(has_widget(&drv, "mv_formula.role_opt.0.continuous"));
    drv.press_key("Escape", KeyModifiers::default())?;
    assert!(!has_widget(&drv, "mv_formula.role_opt.0.continuous"));

    let mut grid_no_meta = ColumnGridEguiState::default();
    grid_no_meta.record_inspect_meta = false;
    let no_meta = run_panel_driver((app, grid_no_meta), 960.0, 560.0, |ui, meta, (ap, st)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let _ = render_data_grid(ui, ap, st, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
        let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
    });
    assert!(no_meta.frame_meta().entries.is_empty());

    let mut xy_app = make_xy_two_graph_layout_app()?;
    exec(&mut xy_app, "sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    xy_app.grid.tool.active = Some(GridCellCoord::new(0, 0, 0));
    xy_app.sync_views();
    let mut disc = run_panel_driver((xy_app, ColumnGridEguiState::default(), true), 960.0, 560.0, |ui, meta, (ap, st, trig)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let _ = render_data_grid(ui, ap, st, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
        if *trig && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ui.ctx().request_discard("audit simulated discard on commit pass");
            *trig = false;
        }
        let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
    });
    disc.press_key("F2", KeyModifiers::default())?;
    disc.state_mut().1.inline_edit.as_mut().ok_or("missing inline edit")?.draft = String::from("123.5");
    disc.run_settled();
    disc.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(disc.state().1.inline_edit.is_none());
    assert_eq!(
        disc.state().0.document().data_sheet(&SheetId::new("ds-xy")).ok_or("ds-xy")?.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value),
        Some(&CellValue::Number(123.5))
    );

    disc.click_widget_id("grid.xcell.0.0")?;
    disc.press_key("Backspace", KeyModifiers::default())?;
    disc.type_text("77.5")?;
    disc.state_mut().2 = true;
    disc.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        disc.state().0.document().data_sheet(&SheetId::new("ds-xy")).ok_or("ds-xy")?.archetype.x_or_time_cell(0, 0).map(|c| &c.value),
        Some(&CellValue::Number(77.5))
    );
    Ok(())
}

#[test]
fn fa_sec17_10_trailing_studio_shortcut_follows_app_shortcuts() -> TestRes {
    let mut app = make_xy_two_graph_layout_app()?;
    ui_cmd(&mut app, "ui.shortcut.set", json!({ "command_id": "ui.inspector.toggle", "combo": "Ctrl+Shift+U" }))?;
    app.sync_views();

    let mut drv = make_merge_driver(app, 1280.0, 800.0);
    assert!(has_widget(&drv, "inspector.collapse_rail"));
    drv.press_egui_key(egui::Key::I, egui::Modifiers { ctrl: true, command: true, shift: true, ..egui::Modifiers::NONE });
    assert!(has_widget(&drv, "inspector.collapse_rail"));
    let ctrl_shift = egui::Modifiers { ctrl: true, shift: true, ..egui::Modifiers::NONE };
    drv.press_egui_key(egui::Key::U, ctrl_shift);
    assert!(has_widget(&drv, "inspector.expand_rail"));
    drv.press_egui_key(egui::Key::U, ctrl_shift);
    drv.hover_widget_id("inspector.collapse_rail")?;
    assert!(drv.visible_tooltip_texts().iter().any(|t| !t.contains("Shift+I") && t.contains('U')));
    Ok(())
}

#[test]
fn fa_sec17_11_polish_headers_date_validation_mv_roles_and_template_error() -> TestRes {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-hdr", "kind": "xy", "cols": 27, "rows": 2, "x_mode": XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, "subcolumn_mode": SubcolumnMode::MedianIqr, "auto_graph": false }),
    )?;
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-lh", "kind": "grouped", "cols": 1, "rows": 2, "subcolumn_mode": SubcolumnMode::MeanLowHigh, "auto_graph": false }),
    )?;
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-ul", "kind": "grouped", "cols": 1, "rows": 2, "subcolumn_mode": SubcolumnMode::UpperLowerLimits, "auto_graph": false }),
    )?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "ds-hdr" }))?;
    app.sync_views();

    let mut hdr = run_panel_driver((app, ColumnGridEguiState::default()), 1024.0, 520.0, |ui, meta, (ap, st)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let _ = render_data_grid(ui, ap, st, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
    });
    for (wid, expected) in [
        ("grid.subcol_hdr.x.1", "SD"),
        ("grid.subcol_hdr.x.2", "N"),
        ("grid.subcol_hdr.0.0", "Median"),
        ("grid.subcol_hdr.0.1", "Q1"),
        ("grid.subcol_hdr.0.2", "Q3"),
    ] {
        assert_eq!(widget_label(&hdr, wid)?, expected);
    }
    hdr.state_mut().1.scroll_offset_pt.x = 20_000.0;
    hdr.run_settled();
    assert_eq!(widget_label(&hdr, "grid.col_super.26")?, "Group AA");

    hdr.state_mut().1.scroll_offset_pt.x = 0.0;
    for (sid, s1, s2) in [("ds-lh", "-Err", "+Err"), ("ds-ul", "Min", "Max")] {
        exec(&mut hdr.state_mut().0, "sheet.select", json!({ "sheet_id": sid }))?;
        hdr.run_settled();
        assert_eq!((widget_label(&hdr, "grid.subcol_hdr.0.1")?, widget_label(&hdr, "grid.subcol_hdr.0.2")?), (s1.into(), s2.into()));
    }

    let mut info_app = UiApp::new();
    exec(
        &mut info_app,
        "sheet.createData",
        json!({ "sheet_id": "ds-ih", "kind": "column", "auto_info": true, "info_sheet_id": "info-h", "auto_graph": false }),
    )?;
    exec(&mut info_app, "info.setConstant", json!({ "sheet_id": "info-h", "name": "StartDate", "value": "2026-11-22", "kind": "date" }))?;
    exec(&mut info_app, "sheet.select", json!({ "sheet_id": "info-h" }))?;
    info_app.nav.welcome.open = false;
    info_app.nav.inspector_open = true;
    info_app.sync_views();
    let mut info_drv = make_merge_driver(info_app, 1200.0, 800.0);
    info_drv.state_mut().views.trailing_studio.info_selected_constant = Some(String::from("startdate"));
    info_drv.run_settled();
    assert!(has_widget(&info_drv, "inspector.info.const.0"));
    assert_eq!(
        (info_drv.state().views.trailing_studio.info_value_draft.as_str(), info_drv.state().views.trailing_studio.info_kind_draft),
        ("2026-11-22", 2)
    );
    info_drv.state_mut().views.trailing_studio.info_value_draft = String::from("bad-date");
    info_drv.run_settled();
    assert!(has_widget(&info_drv, "inspector.info.value_error") && !widget_enabled(&info_drv, "inspector.info.save_const")?);

    let mut w_state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::StarterTemplates, ..Default::default() };
    w_state.schema.selected_template_index = 999;
    let mut w_drv = run_panel_driver((UiApp::new(), w_state), 1120.0, 760.0, |ui, meta, (ap, st)| {
        let mut intents = PendingUiIntents::new();
        let t = Tokens::get(ui.ctx());
        let _ = render_welcome_hub(ui, ap, st, &t, true, meta, &mut intents);
        let _ = intents.commit_if_settled(ap, ui.ctx().will_discard());
    });
    w_drv.click_widget_id("welcome.btn.create")?;
    assert!(has_widget(&w_drv, "welcome.template.error"));
    w_drv.click_widget_id("welcome.arch.multiple_variables")?;
    w_drv.state_mut().1.schema.cols = 8;
    w_drv.run_settled();
    w_drv.click_widget_id("welcome.mv.role.7.Label")?;
    assert_eq!(w_drv.state().1.schema.mv_role_kinds.get(7).copied(), Some(2));
    Ok(())
}
