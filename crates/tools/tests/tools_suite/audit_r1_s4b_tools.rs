//! Stage 4B regression tests for `curvebook-tools` (`R1-S11-10`, `R1-S11-11`, `R1-S11-12`).

use curvebook_chart::{PointPt, compile_graph};
use curvebook_doc::{
    BracketLabelMode, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, GraphSpec, GroupedArrangement, GroupedTable,
    ProjectDocument, SheetFolderKind, SheetId, SizePt, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_tools::canvas::{BracketAnchor, hit_test_category_slot, resolve_category_slots};
use curvebook_tools::{CanvasToolMode, CanvasToolState, GridCellCoord, GridToolState, NavigatorDragState};

type TestRes = Result<(), String>;

#[test]
fn r1_s11_10_exclude_selection_uses_componentwise_box_and_multi_subcol_intents() -> TestRes {
    let mut grid = GridToolState::default();
    let sheet_id = SheetId::new("grp-1");

    // Reversed subcolumn selection `(row 0, col 0, sub 1) -> (row 2, col 1, sub 0)`.
    grid.select_single(GridCellCoord::new(0, 0, 1));
    grid.extend_selection(GridCellCoord::new(2, 1, 0));
    let (min_c, max_c) = grid.selection_box_bounds().ok_or_else(|| String::from("expected box bounds"))?;
    if min_c != GridCellCoord::new(0, 0, 0) || max_c != GridCellCoord::new(2, 1, 1) {
        return Err(format!("unexpected box bounds: min={min_c:?}, max={max_c:?}"));
    }

    let intent = grid.emit_exclude_selection_intent(&sheet_id, true).ok_or_else(|| String::from("expected single exclude intent"))?;
    if intent.command_id != "table.excludeCells" {
        return Err(format!("wrong command_id: {}", intent.command_id));
    }
    let start = &intent.params["range"]["start"];
    let end = &intent.params["range"]["end"];
    if start["row"].as_u64() != Some(0)
        || start["col"].as_u64() != Some(0)
        || start["subcol"].as_u64() != Some(0)
        || end["row"].as_u64() != Some(2)
        || end["col"].as_u64() != Some(1)
        || end["subcol"].as_u64() != Some(1)
    {
        return Err(format!("unexpected exclude range: {}", intent.params));
    }

    // Uneven cross-group visual span decomposes without dropping or over-including subcolumns.
    let grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 3, 2).map_err(|e| format!("{e}"))?;
    let ds = DataSheet::new("grp-1", "Grouped 3x3", TableArchetype::Grouped(grp)).map_err(|e| format!("{e}"))?;
    let mut grid2 = GridToolState::default();
    grid2.select_single(GridCellCoord::new(0, 0, 2));
    grid2.extend_selection(GridCellCoord::new(0, 2, 0));
    let intents = grid2.emit_exclude_selection_intents_for_sheet(&ds, true);
    if intents.len() != 3 {
        return Err(format!("expected 3 per-column intents, got {}: {intents:?}", intents.len()));
    }
    let expected_ranges = [(0u64, 2u64, 2u64), (1, 0, 2), (2, 0, 0)];
    for (intent, (col, s0, s1)) in intents.iter().zip(expected_ranges) {
        let s = &intent.params["range"]["start"];
        let e = &intent.params["range"]["end"];
        if s["col"].as_u64() != Some(col) || e["col"].as_u64() != Some(col) || s["subcol"].as_u64() != Some(s0) || e["subcol"].as_u64() != Some(s1) {
            return Err(format!("wrong decomposed subcol intent: {}", intent.params));
        }
    }
    Ok(())
}

#[test]
fn r1_s11_11_navigator_drag_preserves_sheet_id_and_ignores_cross_folder_hover() -> TestRes {
    let mut drag = NavigatorDragState::default();
    drag.begin_drag(SheetFolderKind::Data, "ds-alpha", 0);

    // Hovering a sheet in Graphs must not overwrite Data hover target.
    drag.update_hover_in_folder(SheetFolderKind::Graphs, 4);
    if drag.hover_target_index.is_some() {
        return Err(format!("cross-folder hover mutated index: {:?}", drag.hover_target_index));
    }

    // Hovering back on index 0 still commits `(folder, sheet_id, from_index, to_index)` so live
    // reconciliation detects if mid-drag edits shifted `ds-alpha` away from slot 0.
    drag.update_hover_in_folder(SheetFolderKind::Data, 0);
    let committed = drag.commit_drop_with_sheet().ok_or_else(|| String::from("expected commit_drop_with_sheet result"))?;
    if committed != (SheetFolderKind::Data, String::from("ds-alpha"), 0, 0) {
        return Err(format!("unexpected committed drag tuple: {committed:?}"));
    }
    Ok(())
}

#[test]
fn r1_s11_12_category_slots_resolve_real_coordinates_and_reject_misses() -> TestRes {
    // Column table on a wide canvas where slots are ~200pt wide rather than 48pt.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).map_err(|e| format!("{e}"))?;
    for c in 0..3 {
        col.data_sets[c].title = format!("Group {c}");
    }
    let mut ds = DataSheet::new("col-1", "Groups", TableArchetype::Column(col)).map_err(|e| format!("{e}"))?;
    for c in 0..3 {
        for r in 0..4 {
            ds.set_cell(CellCoord::new(r, c, 0), Cell::number((c + 1) as f64 * 10.0 + r as f64)).map_err(|e| format!("{e}"))?;
        }
    }
    let mut spec = GraphSpec::default_for_data_sheet(&ds);
    spec.canvas_size_pt = SizePt::new(720.0, 420.0);
    let gs = GraphSheet {
        id: SheetId::new("gr-col"),
        title: String::from("Column"),
        initialized: true,
        spec,
        floating_notes: Vec::new(),
        color_tag: None,
    };

    let mut doc = ProjectDocument::new("Bracket Slots").map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    doc.add_graph_sheet(gs.clone()).map_err(|e| format!("{e}"))?;

    let scene = compile_graph(&gs, &doc).map_err(|e| format!("compile column graph: {e}"))?;
    let slots = resolve_category_slots(&scene, &gs, &doc);
    if slots.len() != 3 {
        return Err(format!("expected 3 column slots, got {}", slots.len()));
    }
    if slots[0].center_pt <= 48.0 || slots[2].center_pt <= 240.0 {
        return Err(format!("expected wide plot centers beyond 48pt stride: {slots:?}"));
    }

    let mid_y = scene.plot_rect_pt.y + scene.plot_rect_pt.height * 0.5;
    let mut canvas = CanvasToolState { mode: CanvasToolMode::Bracket, ..CanvasToolState::default() };
    canvas.begin_drag(PointPt::new(slots[0].center_pt, mid_y));
    let pair = canvas
        .resolve_bracket_drag_pair(&scene, &gs, &doc, PointPt::new(slots[2].center_pt, mid_y))
        .ok_or_else(|| String::from("expected column slot drag pair (0, 2)"))?;
    if pair != (0, 2) {
        return Err(format!("expected (0, 2), got {pair:?}"));
    }

    // Same-group drag must return None instead of fabricating `(a, a + 1)`.
    canvas.begin_drag(PointPt::new(slots[1].center_pt - 5.0, mid_y));
    if let Some(bad) = canvas.resolve_bracket_drag_pair(&scene, &gs, &doc, PointPt::new(slots[1].center_pt + 5.0, mid_y)) {
        return Err(format!("same-group drag fabricated {bad:?}"));
    }

    // Off-plot vertical or horizontal drags must return None instead of `x / 48.0`.
    canvas.begin_drag(PointPt::new(slots[0].center_pt, scene.plot_rect_pt.y - 20.0));
    if let Some(bad) = canvas.resolve_bracket_drag_pair(&scene, &gs, &doc, PointPt::new(slots[2].center_pt, mid_y)) {
        return Err(format!("above-plot drag fabricated {bad:?}"));
    }
    if hit_test_category_slot(&slots, &scene, PointPt::new(scene.plot_rect_pt.right() + 15.0, mid_y)).is_some() {
        return Err(String::from("off-right hit should return None"));
    }

    // Two-click fallback without resolved `group_index` must return None.
    if canvas
        .click_bracket(
            &gs.id,
            BracketAnchor { pt: PointPt::new(144.0, 90.0), group_index: None, data_value: None, result_sheet_id: None },
            BracketLabelMode::SummaryStars,
        )
        .is_some()
        || canvas.pending_bracket_start.is_some()
    {
        return Err(String::from("click_bracket armed without group_index"));
    }

    // Raw end_drag without scene/graph context must return None in Bracket mode.
    canvas.begin_drag(PointPt::new(48.0, 90.0));
    if canvas.end_drag(&gs.id, PointPt::new(192.0, 90.0)).is_some() {
        return Err(String::from("raw end_drag guessed brackets without compiled scene"));
    }

    // Grouped Interleaved and Separated graphs resolve sub-group slot indices accurately.
    let grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).map_err(|e| format!("{e}"))?;
    let mut grp_ds = DataSheet::new("grp-1", "TwoFactor", TableArchetype::Grouped(grp)).map_err(|e| format!("{e}"))?;
    for c in 0..2 {
        for r in 0..2 {
            grp_ds.set_cell(CellCoord::new(r, c, 0), Cell::number((r * 10 + c + 1) as f64)).map_err(|e| format!("{e}"))?;
            grp_ds.set_cell(CellCoord::new(r, c, 1), Cell::number((r * 10 + c + 2) as f64)).map_err(|e| format!("{e}"))?;
        }
    }
    let mut grp_doc = ProjectDocument::new("Grouped Slots").map_err(|e| format!("{e}"))?;
    grp_doc.add_data_sheet(grp_ds.clone()).map_err(|e| format!("{e}"))?;

    for (arrangement, expected_last) in [(GroupedArrangement::Interleaved, 3usize), (GroupedArrangement::Separated, 3usize)] {
        let mut gspec = GraphSpec::default_for_data_sheet(&grp_ds);
        gspec.grouped_arrangement = arrangement;
        let gsheet = GraphSheet {
            id: SheetId::new(format!("gr-{arrangement:?}")),
            title: String::from("Grouped"),
            initialized: true,
            spec: gspec,
            floating_notes: Vec::new(),
            color_tag: None,
        };
        grp_doc.add_graph_sheet(gsheet.clone()).map_err(|e| format!("{e}"))?;
        let gscene = compile_graph(&gsheet, &grp_doc).map_err(|e| format!("compile grouped: {e}"))?;
        let gslots = resolve_category_slots(&gscene, &gsheet, &grp_doc);
        if gslots.len() != 4 {
            return Err(format!("expected 4 grouped slots for {arrangement:?}, got {}", gslots.len()));
        }
        let gy = gscene.plot_rect_pt.y + gscene.plot_rect_pt.height * 0.5;
        canvas.begin_drag(PointPt::new(gslots[0].center_pt, gy));
        let gpair = canvas
            .resolve_bracket_drag_pair(&gscene, &gsheet, &grp_doc, PointPt::new(gslots[expected_last].center_pt, gy))
            .ok_or_else(|| format!("expected grouped drag pair for {arrangement:?}"))?;
        if gpair != (0, 3) {
            return Err(format!("unexpected grouped pair {gpair:?} for {arrangement:?}"));
        }
    }

    // XY graph must have no category slots.
    let xy = XyTable::new(SubcolumnMode::Single, 1, 3).map_err(|e| format!("{e}"))?;
    let xy_ds = DataSheet::new("xy-1", "XY", TableArchetype::Xy(xy)).map_err(|e| format!("{e}"))?;
    let xy_spec = GraphSpec::default_for_data_sheet(&xy_ds);
    let xy_gs = GraphSheet {
        id: SheetId::new("gr-xy"),
        title: String::from("XY"),
        initialized: true,
        spec: xy_spec,
        floating_notes: Vec::new(),
        color_tag: None,
    };
    let mut xy_doc = ProjectDocument::new("XY").map_err(|e| format!("{e}"))?;
    xy_doc.add_data_sheet(xy_ds).map_err(|e| format!("{e}"))?;
    xy_doc.add_graph_sheet(xy_gs.clone()).map_err(|e| format!("{e}"))?;
    let xy_scene = compile_graph(&xy_gs, &xy_doc).map_err(|e| format!("compile xy: {e}"))?;
    if !resolve_category_slots(&xy_scene, &xy_gs, &xy_doc).is_empty() {
        return Err(String::from("XY graph must not produce comparison category slots"));
    }

    Ok(())
}
