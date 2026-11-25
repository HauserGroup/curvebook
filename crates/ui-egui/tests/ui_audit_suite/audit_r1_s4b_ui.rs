//! Stage 4B regression tests for `curvebook-ui-egui` on native `egui` (`R1-S10-07`, `R1-S11-11`, `R1-S11-12`).

use curvebook_chart::{PointPt, compile_graph};
use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, ProjectDocument, SheetFolderKind, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_render::RenderTargetSpec;
use curvebook_tools::CanvasToolMode;
use curvebook_tools::canvas::resolve_category_slots;
use curvebook_tools::grid::parse_tsv_clipboard;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view};
use curvebook_ui_egui::egui_shell::{PendingUiIntents, Tokens};
use curvebook_ui_egui::{ScreenshotRegionTarget, UiApp, ViewportMetrics, WorkspaceNavState, grid_view::format_cell_display};
use egui::{Event, Modifiers, PointerButton, Pos2};
use serde_json::json;

use crate::common::{TestRes, ensure, seed_column_doc, seed_xy_doc, widget_center};
use crate::helpers::{call_egui_control, exec, run_panel_driver};

fn seed_three_col_doc(title: &str) -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new(title).map_err(|e| e.to_string())?;
    for id in ["data-a", "data-b", "data-c"] {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
        doc.add_data_sheet(DataSheet::new(id, id, TableArchetype::Column(col)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(doc)
}

#[test]
fn r1_s10_07_locale_set_syncs_all_grid_parsing_and_compositor() -> TestRes {
    let (doc, _) = seed_xy_doc("Locale Sync", "ds-1", "Dose", "X", &[1.25], &[("Response", &[2.5, 3.5])])?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;

    let check_seps = |a: &UiApp, sep: char| -> TestRes {
        for (name, got) in [
            ("preset", a.locale_preset.decimal_separator),
            ("grid", a.grid.decimal_separator),
            ("tool", a.grid.tool.decimal_separator),
            ("wizard", a.grid.import_wizard.decimal_separator),
            ("reshape", a.grid.import_reshape.decimal_separator),
        ] {
            ensure(got == sep, format!("{name} separator expected `{sep}`, got `{got}`"))?;
        }
        Ok(())
    };

    let resp_de = call_egui_control(&mut app, &json!({ "id": 1, "op": "ui.locale.set", "params": { "locale": "de-DE" } }))?;
    ensure(resp_de["ok"] == json!(true), format!("de-DE failed: {resp_de}"))?;
    check_seps(&app, ',')?;

    let parsed = parse_tsv_clipboard("3,75", app.grid.tool.decimal_separator, false).map_err(|e| e.to_string())?;
    let val = parsed.first().and_then(|r| r.first()).and_then(Cell::active_number).ok_or("expected numeric from '3,75'")?;
    ensure((val - 3.75).abs() < 1e-9, format!("expected 3.75, got {val}"))?;
    ensure(format_cell_display(&Cell::number(1.25), &app.locale_preset) == "1,25", "de-DE display format mismatch")?;

    let img = app
        .render_egui_shell_rgba(RenderTargetSpec { width_px: 1024, height_px: 640, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
        .map_err(|e| e.to_string())?;
    ensure(!img.rgba.is_empty(), "expected non-empty egui rgba")?;

    let resp_en = call_egui_control(&mut app, &json!({ "id": 2, "op": "ui.locale.set", "params": { "locale": "en-US" } }))?;
    ensure(resp_en["ok"] == json!(true), format!("en-US failed: {resp_en}"))?;
    check_seps(&app, '.')
}

#[test]
fn r1_s11_11_navigator_drag_preserves_sheet_id_under_mid_drag_edits() -> TestRes {
    let mut doc = seed_three_col_doc("Nav Reorder")?;
    let mut nav = WorkspaceNavState::default();
    nav.begin_sheet_drag(SheetFolderKind::Data, "data-a", 0);
    nav.update_sheet_hover_in_folder(SheetFolderKind::Data, 2);
    let extra = DataSheet::new(
        "data-x",
        "Extra",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?),
    )
    .map_err(|e| e.to_string())?;
    doc.data_sheets.insert(0, extra);
    let (cmd, params) = nav.commit_sheet_drop_command_for_doc(&doc).ok_or("expected sheet.reorder")?;
    ensure(cmd == "sheet.reorder" && params["sheet_id"] == "data-a", format!("unexpected reorder: {cmd} {params}"))?;

    nav.begin_sheet_drag(SheetFolderKind::Data, "data-ghost", 0);
    nav.update_sheet_hover_in_folder(SheetFolderKind::Data, 2);
    ensure(nav.commit_sheet_drop_command_for_doc(&doc).is_none(), "deleted sheet drag must cancel")?;

    let mut app = UiApp::from_session(Session::from_document(seed_three_col_doc("Pointer Drag Reorder")?));
    app.nav.welcome.open = false;
    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::default());
    let p_start = widget_center(&h.driver, "nav.sheet.data-a")?;
    let p_drop = widget_center(&h.driver, "nav.sheet.data-c")?;

    h.driver.dispatch_events(vec![
        Event::PointerMoved(p_start),
        Event::PointerButton { pos: p_start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerMoved(Pos2::new(p_start.x, p_start.y + 10.0)),
    ]);
    exec(h.app_mut(), "sheet.reorder", json!({ "sheet_id": "data-b", "index": 0 }))?;
    h.driver.run_settled();
    h.driver.dispatch_events(vec![
        Event::PointerMoved(p_drop),
        Event::PointerButton { pos: p_drop, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);

    let order: Vec<&str> = h.app().document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    ensure(order == vec!["data-b", "data-c", "data-a"], format!("unexpected order: {order:?}"))
}

#[test]
fn r1_s11_12_bracket_pointer_drag_resolves_real_category_slots() -> TestRes {
    let (mut doc, _) = seed_column_doc(
        "Bracket UI Drag",
        "data-col",
        "Cohort",
        ColumnDesignHint::UnpairedReplicates,
        &[("Ctrl", &[10.0, 11.0, 12.0, 13.0]), ("Low", &[20.0, 21.0, 22.0, 23.0]), ("High", &[30.0, 31.0, 32.0, 33.0])],
    )?;
    let (xy_doc, xy_sid) = seed_xy_doc("tmp", "data-xy", "Dose", "X", &[1.0, 2.0, 3.0], &[("Y", &[2.0, 3.0, 4.0])])?;
    doc.add_data_sheet(xy_doc.data_sheet(&xy_sid).ok_or("missing xy")?.clone()).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    exec(
        &mut app,
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet_id": "data-col", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    exec(&mut app, "graph.create", json!({ "sheet_id": "gr-col", "data_sheet_id": "data-col", "kind": "column" }))?;
    exec(&mut app, "graph.create", json!({ "sheet_id": "gr-xy", "data_sheet_id": "data-xy", "kind": "xy" }))?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "gr-col" }))?;
    app.canvas_dialogs.canvas.set_mode(CanvasToolMode::Bracket);

    let drv = run_panel_driver(
        (app.clone(), CanvasViewEguiState::default(), PendingUiIntents::new(), None),
        1280.0,
        800.0,
        |ui, meta, (ap, vw, ints, rect)| {
            let tokens = Tokens::for_mode(ap.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false };
            *rect = render_canvas_view(ui, ap, vw, ctx, meta, ints).paper_rect_pt;
        },
    );
    let paper = drv.state().3.ok_or("expected canvas_view paper_rect")?;

    let gs = app.document().graph_sheet(&SheetId::new("gr-col")).ok_or("missing gr-col")?.clone();
    ensure(gs.spec.kind == GraphKind::Column, format!("unexpected kind: {:?}", gs.spec.kind))?;
    let scene = compile_graph(&gs, app.document()).map_err(|e| e.to_string())?;
    let slots = resolve_category_slots(&scene, &gs, app.document());
    ensure(slots.len() == 3, format!("expected 3 slots, got {}", slots.len()))?;

    let roundtrip = |pt: PointPt| -> PointPt {
        let scr = Pos2::new(
            paper.left() + (pt.x / scene.width_pt.max(1.0)) as f32 * paper.width(),
            paper.top() + (pt.y / scene.height_pt.max(1.0)) as f32 * paper.height(),
        );
        PointPt::new(
            f64::from((scr.x - paper.left()) / paper.width().max(1.0)) * scene.width_pt,
            f64::from((scr.y - paper.top()) / paper.height().max(1.0)) * scene.height_pt,
        )
    };

    let mid_y = scene.plot_rect_pt.y + scene.plot_rect_pt.height * 0.5;
    let (p_slot0, p_slot2) = (roundtrip(PointPt::new(slots[0].center_pt, mid_y)), roundtrip(PointPt::new(slots[2].center_pt, mid_y)));
    let mut canvas = app.canvas_dialogs.canvas.clone();
    canvas.begin_drag(p_slot0);
    let intent = canvas.end_drag_on_graph(&scene, &gs, app.document(), p_slot2).ok_or("expected bracket intent")?;
    ensure(
        intent.command_id == "graph.addComparisonBrackets"
            && intent.params.get("brackets").is_none()
            && intent.params["sheet_id"] == "gr-col"
            && intent.params["result_sheet_id"] == "res-anova"
            && intent.params["group_a"] == 0
            && intent.params["group_b"] == 2,
        format!("unexpected bracket intent: {intent:?}"),
    )?;
    exec(&mut app, &intent.command_id, intent.params)?;
    let updated = app.document().graph_sheet(&SheetId::new("gr-col")).ok_or("missing updated gr-col")?;
    ensure(
        updated.spec.comparisons.len() == 1
            && (updated.spec.comparisons[0].group_a, updated.spec.comparisons[0].group_b) == (0, 2)
            && updated.spec.comparisons[0].result_sheet_id.as_str() == "res-anova",
        "bracket mismatch",
    )?;

    canvas.begin_drag(PointPt::new(slots[0].center_pt, (scene.plot_rect_pt.y - 25.0).max(2.0)));
    ensure(canvas.end_drag_on_graph(&scene, &gs, app.document(), p_slot2).is_none(), "off-plot drag must not emit bracket")?;

    exec(&mut app, "sheet.select", json!({ "sheet_id": "gr-xy" }))?;
    let gs_xy = app.document().graph_sheet(&SheetId::new("gr-xy")).ok_or("missing gr-xy")?;
    let scene_xy = compile_graph(gs_xy, app.document()).map_err(|e| e.to_string())?;
    canvas.begin_drag(PointPt::new(80.0, 120.0));
    ensure(canvas.end_drag_on_graph(&scene_xy, gs_xy, app.document(), PointPt::new(220.0, 120.0)).is_none(), "XY drag must not emit bracket")
}
