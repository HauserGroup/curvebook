//! Stage 4B regression tests for `curvebook-ui-egui` (`R1-S10-07`, `R1-S11-11`, `R1-S11-12`).

use curvebook_chart::{PointPt, compile_graph};
use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, ProjectDocument, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype,
    XyTable,
};
use curvebook_engine::Session;
use curvebook_render::RenderTargetSpec;
use curvebook_tools::CanvasToolMode;
use curvebook_tools::canvas::resolve_category_slots;
use curvebook_tools::grid::parse_tsv_clipboard;
use curvebook_ui_egui::{
    KeyModifiers, PointerButton, ScenePaperMapping, UiApp, UiInputEvent, ViewportMetrics, WorkspaceNavState, WorkspacePixelRect,
    compositor::render_workspace_rgba, grid_view::format_cell_display,
};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn rect_center_logical(rect: WorkspacePixelRect, metrics: ViewportMetrics) -> [f64; 2] {
    let cx = f64::from(rect.x) + f64::from(rect.width) * 0.5;
    let cy = f64::from(rect.y) + f64::from(rect.height) * 0.5;
    metrics.physical_to_logical_px([cx, cy])
}

fn scene_pt_to_logical(paper: &ScenePaperMapping, metrics: ViewportMetrics, target: PointPt) -> Result<[f64; 2], String> {
    let x_a = f64::from(paper.paper.x) + 12.0;
    let y_a = f64::from(paper.paper.y) + 12.0;
    let x_b = f64::from(paper.paper.x) + f64::from(paper.paper.width) - 12.0;
    let y_b = f64::from(paper.paper.y) + f64::from(paper.paper.height) - 12.0;
    let s_a = paper.pixel_to_scene_pt(x_a, y_a).ok_or_else(|| String::from("probe A outside paper"))?;
    let s_b = paper.pixel_to_scene_pt(x_b, y_b).ok_or_else(|| String::from("probe B outside paper"))?;
    let dx_scene = s_b.x - s_a.x;
    let dy_scene = s_b.y - s_a.y;
    if dx_scene.abs() < 1e-6 || dy_scene.abs() < 1e-6 {
        return Err(String::from("degenerate paper mapping"));
    }
    let phys_x = x_a + ((target.x - s_a.x) / dx_scene) * (x_b - x_a);
    let phys_y = y_a + ((target.y - s_a.y) / dy_scene) * (y_b - y_a);
    Ok(metrics.physical_to_logical_px([phys_x, phys_y]))
}

#[test]
fn r1_s10_07_locale_set_syncs_all_grid_parsing_and_compositor() -> TestRes {
    let mut doc = ProjectDocument::new("Locale Sync").map_err(|e| format!("{e}"))?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).map_err(|e| format!("{e}"))?;
    xy.data_sets[0].title = String::from("Response");
    let mut ds = DataSheet::new("ds-1", "Dose", TableArchetype::Xy(xy)).map_err(|e| format!("{e}"))?;
    ds.set_x_or_time_cell(0, 0, Cell::number(1.25)).map_err(|e| format!("{e}"))?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(2.5)).map_err(|e| format!("{e}"))?;
    ds.set_cell(CellCoord::new(1, 0, 0), Cell::number(3.5)).map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;

    let session = Session::from_document(doc);
    let mut app = UiApp::from_session(session);

    // Switch to de-DE via UI control command and verify every grid/wizard state syncs to ','.
    let resp_de_raw = app.dispatch_control_line(&json!({"id": 1, "op": "ui.locale.set", "params": {"locale": "de-DE"}}).to_string());
    let resp_de: Value = serde_json::from_str(&resp_de_raw).map_err(|e| format!("parse locale reply: {e}"))?;
    if resp_de["ok"].as_bool() != Some(true) {
        return Err(format!("ui.locale.set de-DE failed: {resp_de}"));
    }
    if app.locale_preset.decimal_separator != ','
        || app.grid.decimal_separator != ','
        || app.grid.tool.decimal_separator != ','
        || app.grid.import_wizard.decimal_separator != ','
        || app.grid.import_reshape.decimal_separator != ','
    {
        return Err(format!(
            "decimal separators out of sync after de-DE: preset={}, grid={}, tool={}, wizard={}, reshape={}",
            app.locale_preset.decimal_separator,
            app.grid.decimal_separator,
            app.grid.tool.decimal_separator,
            app.grid.import_wizard.decimal_separator,
            app.grid.import_reshape.decimal_separator
        ));
    }

    // Parsing TSV / clipboard using GridToolState.decimal_separator parses '3,75' as 3.75.
    let parsed_rows = parse_tsv_clipboard("3,75", app.grid.tool.decimal_separator, false).map_err(|e| format!("parse_tsv_clipboard failed: {e}"))?;
    let parsed_val = parsed_rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| cell.active_number())
        .ok_or_else(|| String::from("expected numeric cell from '3,75'"))?;
    if (parsed_val - 3.75).abs() > 1e-9 {
        return Err(format!("expected 3.75, got {parsed_val}"));
    }
    let formatted_de = format_cell_display(&Cell::number(1.25), &app.locale_preset);
    if formatted_de != "1,25" {
        return Err(format!("expected '1,25', got '{formatted_de}'"));
    }

    // Compositor renders cleanly after de-DE switch.
    let img = render_workspace_rgba(&app, RenderTargetSpec { width_px: 1024, height_px: 640, dpi: 96 })
        .map_err(|e| format!("compositor render failed: {e}"))?;
    if img.rgba.is_empty() {
        return Err(String::from("expected non-empty compositor rgba"));
    }

    // Switch back to en-US and verify revert across all components.
    let resp_en_raw = app.dispatch_control_line(&json!({"id": 2, "op": "ui.locale.set", "params": {"locale": "en-US"}}).to_string());
    let resp_en: Value = serde_json::from_str(&resp_en_raw).map_err(|e| format!("parse locale reply: {e}"))?;
    if resp_en["ok"].as_bool() != Some(true) {
        return Err(format!("ui.locale.set en-US failed: {resp_en}"));
    }
    for (name, sep) in [
        ("preset", app.locale_preset.decimal_separator),
        ("grid", app.grid.decimal_separator),
        ("tool", app.grid.tool.decimal_separator),
        ("wizard", app.grid.import_wizard.decimal_separator),
        ("reshape", app.grid.import_reshape.decimal_separator),
    ] {
        if sep != '.' {
            return Err(format!("{name} failed to revert to '.', got '{sep}'"));
        }
    }
    Ok(())
}

#[test]
fn r1_s11_11_navigator_drag_preserves_sheet_id_under_mid_drag_edits() -> TestRes {
    let mut doc = ProjectDocument::new("Nav Reorder").map_err(|e| format!("{e}"))?;
    for id in ["data-a", "data-b", "data-c"] {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| format!("{e}"))?;
        let ds = DataSheet::new(id, id, TableArchetype::Column(col)).map_err(|e| format!("{e}"))?;
        doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    }

    // 1) Unit-level WorkspaceNavState reconciliation after a mid-drag insertion/reorder.
    let mut nav = WorkspaceNavState::default();
    nav.begin_sheet_drag(SheetFolderKind::Data, "data-a", 0);
    nav.update_sheet_hover_in_folder(SheetFolderKind::Data, 2);
    // Simulate mid-drag insertion before data-a so data-a is now at index 1 while data-x occupies 0.
    let extra_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| format!("{e}"))?;
    doc.data_sheets.insert(0, DataSheet::new("data-x", "Extra", TableArchetype::Column(extra_col)).map_err(|e| format!("{e}"))?);
    let (cmd, params) = nav.commit_sheet_drop_command_for_doc(&doc).ok_or_else(|| String::from("expected sheet.reorder command"))?;
    if cmd != "sheet.reorder" || params["sheet_id"].as_str() != Some("data-a") {
        return Err(format!("expected sheet.reorder for data-a, got {cmd} {params}"));
    }

    // Dragged sheet deleted mid-drag must cancel cleanly rather than moving an unrelated sheet at index 0.
    nav.begin_sheet_drag(SheetFolderKind::Data, "data-ghost", 0);
    nav.update_sheet_hover_in_folder(SheetFolderKind::Data, 2);
    if let Some(unexpected) = nav.commit_sheet_drop_command_for_doc(&doc) {
        return Err(format!("deleted sheet drag emitted reorder: {unexpected:?}"));
    }

    // 2) Full pointer-event drag on UiApp where `sheet.reorder` occurs between PointerDown and PointerUp.
    let mut doc2 = ProjectDocument::new("Pointer Drag Reorder").map_err(|e| format!("{e}"))?;
    for id in ["data-a", "data-b", "data-c"] {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| format!("{e}"))?;
        let ds = DataSheet::new(id, id, TableArchetype::Column(col)).map_err(|e| format!("{e}"))?;
        doc2.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    }
    let mut app = UiApp::from_session(Session::from_document(doc2));
    let metrics = ViewportMetrics::default();
    let map0 = app.layout_map(metrics);
    let entry_a = map0
        .navigator_entries
        .iter()
        .find(|e| e.folder == SheetFolderKind::Data && e.sheet_id.as_str() == "data-a")
        .ok_or_else(|| String::from("missing data-a navigator entry"))?;
    let entry_c = map0
        .navigator_entries
        .iter()
        .find(|e| e.folder == SheetFolderKind::Data && e.sheet_id.as_str() == "data-c")
        .ok_or_else(|| String::from("missing data-c navigator entry"))?;
    let p_start = rect_center_logical(entry_a.rect, metrics);
    let p_drop = rect_center_logical(entry_c.rect, metrics);

    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: p_start[0], y: p_start[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;

    // Mid-drag external reorder moves data-b to index 0 (`[data-b, data-a, data-c]`).
    app.execute_command("sheet.reorder", json!({ "sheet_id": "data-b", "index": 0 })).map_err(|e| format!("mid-drag reorder failed: {e}"))?;

    app.dispatch_ui_event(UiInputEvent::PointerMove { x: p_drop[0], y: p_drop[1], modifiers: KeyModifiers::default() }, metrics)?;
    let drop_out = app.dispatch_ui_event(
        UiInputEvent::PointerUp { x: p_drop[0], y: p_drop[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    if drop_out.command_id.as_deref() != Some("sheet.reorder") {
        return Err(format!("expected sheet.reorder on drop, got {drop_out:?}"));
    }

    let order: Vec<&str> = app.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    if order != vec!["data-b", "data-c", "data-a"] {
        return Err(format!("expected dragged data-a to move to end ([data-b, data-c, data-a]), got {order:?}"));
    }
    Ok(())
}

#[test]
fn r1_s11_12_bracket_pointer_drag_resolves_real_category_slots() -> TestRes {
    let mut doc = ProjectDocument::new("Bracket UI Drag").map_err(|e| format!("{e}"))?;
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).map_err(|e| format!("{e}"))?;
    for (idx, title) in ["Ctrl", "Low", "High"].into_iter().enumerate() {
        col.data_sets[idx].title = String::from(title);
    }
    let mut ds = DataSheet::new("data-col", "Cohort", TableArchetype::Column(col)).map_err(|e| format!("{e}"))?;
    for idx in 0..3 {
        for r in 0..4 {
            ds.set_cell(CellCoord::new(r, idx, 0), Cell::number((idx + 1) as f64 * 10.0 + r as f64)).map_err(|e| format!("{e}"))?;
        }
    }
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;

    // Also add an XY sheet + graph to confirm bracket drag ignores non-categorical plots.
    let xy = XyTable::new(SubcolumnMode::Single, 1, 3).map_err(|e| format!("{e}"))?;
    let mut ds_xy = DataSheet::new("data-xy", "Dose", TableArchetype::Xy(xy)).map_err(|e| format!("{e}"))?;
    for r in 0..3 {
        ds_xy.set_x_or_time_cell(r, 0, Cell::number((r + 1) as f64)).map_err(|e| format!("{e}"))?;
        ds_xy.set_cell(CellCoord::new(r, 0, 0), Cell::number((r + 2) as f64)).map_err(|e| format!("{e}"))?;
    }
    doc.add_data_sheet(ds_xy).map_err(|e| format!("{e}"))?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet": "data-col", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )
    .map_err(|e| format!("analysis.run failed: {e}"))?;
    app.execute_command("graph.create", json!({ "sheet_id": "gr-col", "data_sheet_id": "data-col", "kind": "column" }))
        .map_err(|e| format!("graph.create col failed: {e}"))?;
    app.execute_command("graph.create", json!({ "sheet_id": "gr-xy", "data_sheet_id": "data-xy", "kind": "xy" }))
        .map_err(|e| format!("graph.create xy failed: {e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": "gr-col" })).map_err(|e| format!("sheet.select gr-col failed: {e}"))?;
    app.canvas_dialogs.canvas.set_mode(CanvasToolMode::Bracket);

    let gs = app.document().graph_sheet(&SheetId::new("gr-col")).ok_or_else(|| String::from("missing gr-col"))?.clone();
    if gs.spec.kind != GraphKind::Column {
        return Err(format!("unexpected graph kind: {:?}", gs.spec.kind));
    }
    let scene = compile_graph(&gs, app.document()).map_err(|e| format!("compile_graph: {e}"))?;
    let slots = resolve_category_slots(&scene, &gs, app.document());
    if slots.len() != 3 {
        return Err(format!("expected 3 category slots, got {}", slots.len()));
    }

    let metrics = ViewportMetrics::default();
    let map = app.layout_map(metrics);
    let paper = map.scene_paper.as_ref().ok_or_else(|| String::from("expected scene_paper mapping"))?;
    let mid_y = scene.plot_rect_pt.y + scene.plot_rect_pt.height * 0.5;
    let p_slot0 = scene_pt_to_logical(paper, metrics, PointPt::new(slots[0].center_pt, mid_y))?;
    let p_slot2 = scene_pt_to_logical(paper, metrics, PointPt::new(slots[2].center_pt, mid_y))?;

    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: p_slot0[0], y: p_slot0[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    let up_out = app.dispatch_ui_event(
        UiInputEvent::PointerUp { x: p_slot2[0], y: p_slot2[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    if up_out.command_id.as_deref() != Some("graph.addComparisonBrackets") {
        return Err(format!("expected graph.addComparisonBrackets, got {up_out:?}"));
    }

    let updated = app.document().graph_sheet(&SheetId::new("gr-col")).ok_or_else(|| String::from("missing updated gr-col"))?;
    if updated.spec.comparisons.len() != 1 {
        return Err(format!("expected 1 bracket, got {}", updated.spec.comparisons.len()));
    }
    let br = &updated.spec.comparisons[0];
    if (br.group_a, br.group_b) != (0, 2) || br.result_sheet_id.as_str() != "res-anova" {
        return Err(format!("unexpected bracket spec: {br:?}"));
    }

    // Off-plot vertical drag on Column graph must not create a bracket.
    let off_y = scene_pt_to_logical(paper, metrics, PointPt::new(slots[0].center_pt, (scene.plot_rect_pt.y - 25.0).max(2.0)))?;
    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: off_y[0], y: off_y[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    let off_up = app.dispatch_ui_event(
        UiInputEvent::PointerUp { x: p_slot2[0], y: p_slot2[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    if off_up.command_id.is_some() {
        return Err(format!("off-plot bracket drag should not emit command, got {off_up:?}"));
    }

    // Switching to XY graph and dragging across the plot must not fabricate comparison brackets.
    app.execute_command("sheet.select", json!({ "sheet_id": "gr-xy" })).map_err(|e| format!("sheet.select gr-xy failed: {e}"))?;
    let xy_map = app.layout_map(metrics);
    let xy_paper = xy_map.scene_paper.as_ref().ok_or_else(|| String::from("expected xy scene_paper"))?;
    let xy_a = scene_pt_to_logical(xy_paper, metrics, PointPt::new(80.0, 120.0))?;
    let xy_b = scene_pt_to_logical(xy_paper, metrics, PointPt::new(220.0, 120.0))?;
    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: xy_a[0], y: xy_a[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    let xy_up = app.dispatch_ui_event(
        UiInputEvent::PointerUp { x: xy_b[0], y: xy_b[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    if xy_up.command_id.is_some() {
        return Err(format!("XY drag should not emit bracket command, got {xy_up:?}"));
    }

    Ok(())
}
