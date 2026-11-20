#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    CartesianOrientation, DataSheet, ErrorCode, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapRowLabelMode, MAX_SHEETS_PER_PROJECT,
    ProjectDocument, ReplicateLineMode, SheetId, SurvivalDisplayMetric, TableArchetype, TickDirection, TickLabelFormat,
};
use curvebook_engine::{CommandOutcome, Session};
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_render::{RenderTargetSpec, encode_png, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use serde_json::{Value, json};

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn seed_column_doc() -> ProjectDocument {
    let mut session = Session::from_document(ProjectDocument::default());
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d-col", "title": "Groups", "archetype": "column", "cols": 3, "rows": 4 }));
    ok_cmd(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "d-col",
            "cells": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 1, "col": 0, "value": 12.0 },
                { "row": 2, "col": 0, "value": 11.0 },
                { "row": 3, "col": 0, "value": 13.0 },
                { "row": 0, "col": 1, "value": 24.0 },
                { "row": 1, "col": 1, "value": 26.0 },
                { "row": 2, "col": 1, "value": 25.0 },
                { "row": 3, "col": 1, "value": 27.0 },
                { "row": 0, "col": 2, "value": 40.0 },
                { "row": 1, "col": 2, "value": 44.0 },
                { "row": 2, "col": 2, "value": 42.0 },
                { "row": 3, "col": 2, "value": 46.0 }
            ]
        }),
    );
    session.document().clone()
}

#[test]
fn graph_create_one_per_data_set_y_title_atomic_cap_and_undo() {
    let mut session = Session::from_document(seed_column_doc());

    let res = ok_cmd(
        &mut session,
        "graph.create",
        json!({
            "data_sheet_id": "d-col",
            "one_per_data_set": true,
            "y_axis_title": "Normalized Signal",
            "kind": "column"
        }),
    );
    assert_eq!(res.payload["created_count"], 3);
    let ids = res.payload["sheet_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 3);
    for (idx, v) in ids.iter().enumerate() {
        let gid = SheetId::new(v.as_str().unwrap());
        let gs = session.document().graph_sheet(&gid).unwrap();
        assert_eq!(gs.spec.series.len(), 1);
        assert_eq!(gs.spec.series[0].data_set_index, idx);
        assert_eq!(gs.spec.y_axis.title, "Normalized Signal");
    }

    // Single-step undo removes all 3 created graphs; single-step redo restores all 3.
    let count_before_undo = session.document().graph_sheets.len();
    assert_eq!(count_before_undo, 3);
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert_eq!(session.document().graph_sheets.len(), 0);
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(session.document().graph_sheets.len(), 3);

    // Upfront MAX_SHEETS_PER_PROJECT check prevents partial insertion.
    let mut capped_doc = seed_column_doc();
    let existing = capped_doc.total_sheet_count();
    let filler_needed = MAX_SHEETS_PER_PROJECT - existing - 2; // room for 2, but one_per_data_set wants 3
    for i in 0..filler_needed {
        let tbl = curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
        let ds = DataSheet::new(SheetId::new(format!("fill-{i}")), "Fill", TableArchetype::Column(tbl)).unwrap();
        capped_doc.add_data_sheet(ds).unwrap();
    }
    let mut capped_sess = Session::from_document(capped_doc);
    let before = capped_sess.document().total_sheet_count();
    let err = capped_sess
        .execute("graph.create", json!({ "data_sheet_id": "d-col", "one_per_data_set": true }))
        .expect_err("must reject exceeding MAX_SHEETS_PER_PROJECT atomically");
    assert_eq!(err.code, ErrorCode::ResourceLimit);
    assert_eq!(capped_sess.document().total_sheet_count(), before, "no partial graph sheets should be added on cap error");
}

#[test]
fn graph_apply_style_from_batch_skips_donor_atomic_and_single_undo() {
    let mut session = Session::from_document(seed_column_doc());
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-donor", "data_sheet_id": "d-col", "kind": "column" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-t1", "data_sheet_id": "d-col", "kind": "column" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-t2", "data_sheet_id": "d-col", "kind": "column" }));

    ok_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g-donor", "paper_fill": "#F2F4F8" }));
    ok_cmd(&mut session, "graph.setAxes", json!({ "sheet_id": "g-donor", "axis": "y", "tick_direction": "inside", "show_tick_labels": false }));

    // Atomic rollback if any target in batch is missing
    let err = session
        .execute("graph.applyStyleFrom", json!({ "donor_sheet_id": "g-donor", "target_sheet_ids": ["g-t1", "missing-target"] }))
        .expect_err("missing target must fail before mutating g-t1");
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(session.document().graph_sheet(&SheetId::new("g-t1")).unwrap().spec.y_axis.show_tick_labels);

    // Batch clone skipping donor in target list + single undo
    let out = ok_cmd(&mut session, "graph.applyStyleFrom", json!({ "donor_sheet_id": "g-donor", "target_sheet_ids": ["g-donor", "g-t1", "g-t2"] }));
    assert_eq!(out.payload["skipped_sheet_ids"], json!(["g-donor"]));
    assert_eq!(out.payload["applied_sheet_ids"], json!(["g-t1", "g-t2"]));
    assert!(!session.document().graph_sheet(&SheetId::new("g-t1")).unwrap().spec.y_axis.show_tick_labels);
    assert!(!session.document().graph_sheet(&SheetId::new("g-t2")).unwrap().spec.y_axis.show_tick_labels);

    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(session.document().graph_sheet(&SheetId::new("g-t1")).unwrap().spec.y_axis.show_tick_labels);
    assert!(session.document().graph_sheet(&SheetId::new("g-t2")).unwrap().spec.y_axis.show_tick_labels);
}

#[test]
fn graph_set_spec_axes_and_series_cover_all_r13_fields() {
    let mut session = Session::from_document(seed_column_doc());
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d-xy", "title": "XY", "archetype": "xy", "cols": 2, "rows": 3, "replicates": 2 }));
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d-surv", "title": "Surv", "archetype": "survival", "cols": 2, "rows": 3 }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "d-xy", "kind": "xy" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-surv", "data_sheet_id": "d-surv", "kind": "survival" }));

    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g-xy",
            "paper_fill": null,
            "orientation": "horizontal",
            "forest_show_pooled_diamond": false,
            "heatmap_labels": { "row_labels": "row_numbers", "column_labels": "column_letters", "column_angle": "vertical_90" }
        }),
    );
    let g_xy = session.document().graph_sheet(&SheetId::new("g-xy")).unwrap();
    assert_eq!(g_xy.spec.paper_fill, None);
    assert_eq!(g_xy.spec.orientation, CartesianOrientation::Horizontal);
    assert!(!g_xy.spec.forest_show_pooled_diamond);
    assert_eq!(g_xy.spec.heatmap_labels.row_labels, HeatmapRowLabelMode::RowNumbers);
    assert_eq!(g_xy.spec.heatmap_labels.column_labels, HeatmapColumnLabelMode::ColumnLetters);
    assert_eq!(g_xy.spec.heatmap_labels.column_angle, HeatmapColumnLabelAngle::Vertical90);

    ok_cmd(
        &mut session,
        "graph.setAxes",
        json!({
            "sheet_id": "g-xy",
            "x_axis": { "tick_direction": "both", "show_tick_labels": false, "tick_format": "absolute_decimal" }
        }),
    );
    let g_xy = session.document().graph_sheet(&SheetId::new("g-xy")).unwrap();
    assert_eq!(g_xy.spec.x_axis.tick_direction, TickDirection::Both);
    assert!(!g_xy.spec.x_axis.show_tick_labels);
    assert_eq!(g_xy.spec.x_axis.tick_format, TickLabelFormat::AbsoluteDecimal);

    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "all_series": true,
            "offset": { "x_data": 0.25, "x_pt": 6.0, "y_pt": -4.0 },
            "bar_width_pt": 16.0,
            "show_raw_data": false,
            "replicate_lines": "individual_replicate_trajectories",
            "alpha": 0.5
        }),
    );
    let g_xy = session.document().graph_sheet(&SheetId::new("g-xy")).unwrap();
    for s in &g_xy.spec.series {
        assert!((s.offset.x_data - 0.25).abs() < 1e-9);
        assert!((s.offset.x_pt - 6.0).abs() < 1e-9);
        assert!((s.offset.y_pt - (-4.0)).abs() < 1e-9);
        assert_eq!(s.bar_width_override_pt, Some(16.0));
        assert!(!s.show_raw_data);
        if let curvebook_doc::GraphicalDataObject::ScatterAndLine { replicate_lines, line, .. } = s.gdo {
            assert_eq!(replicate_lines, ReplicateLineMode::IndividualReplicateTrajectories);
            assert_eq!(line.unwrap().color.a, 128);
        }
    }

    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g-surv",
            "all_series": true,
            "display_metric": "percent_cumulative_event",
            "staircase": false,
            "error_display": "sem"
        }),
    );
    let g_surv = session.document().graph_sheet(&SheetId::new("g-surv")).unwrap();
    if let curvebook_doc::GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, .. } = g_surv.spec.series[0].gdo {
        assert_eq!(display_metric, SurvivalDisplayMetric::PercentCumulativeEvent);
        assert!(!staircase);
        assert_eq!(error_display, Some(curvebook_doc::ErrorBarMetric::Sem));
    } else {
        panic!("expected KaplanMeierStep GDO");
    }
}

#[test]
fn transform_run_create_graph_reactive_undo_and_atomic_cap() {
    let mut session = Session::from_document(seed_column_doc());
    let out = ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "res-norm",
            "source_sheet_id": "d-col",
            "kind": "normalize",
            "create_graph": true,
            "graph_sheet_id": "g-norm"
        }),
    );
    assert_eq!(out.payload["graph_sheet_id"], "g-norm");
    let gs = session.document().graph_sheet(&SheetId::new("g-norm")).unwrap();
    assert_eq!(gs.spec.primary_source_sheet_id.as_str(), "res-norm");
    assert!(compile_graph(gs, session.document()).is_ok());

    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(session.document().result_sheet(&SheetId::new("res-norm")).is_none());
    assert!(session.document().graph_sheet(&SheetId::new("g-norm")).is_none());
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert!(session.document().graph_sheet(&SheetId::new("g-norm")).is_some());

    // Atomic TooManySheets when space remains for ResultSheet (1) but NOT ResultSheet + GraphSheet (2)
    let mut capped_doc = seed_column_doc();
    let filler_needed = MAX_SHEETS_PER_PROJECT - capped_doc.total_sheet_count() - 1;
    for i in 0..filler_needed {
        let tbl = curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
        let ds = DataSheet::new(SheetId::new(format!("f-{i}")), "Fill", TableArchetype::Column(tbl)).unwrap();
        capped_doc.add_data_sheet(ds).unwrap();
    }
    let mut capped = Session::from_document(capped_doc);
    let count_before = capped.document().total_sheet_count();
    let err = capped
        .execute("transform.run", json!({ "sheet_id": "res-over", "source_sheet_id": "d-col", "kind": "normalize", "create_graph": true }))
        .expect_err("must reject atomically when +2 sheets exceeds cap");
    assert_eq!(err.code, ErrorCode::ResourceLimit);
    assert_eq!(capped.document().total_sheet_count(), count_before);
}

#[test]
fn edit_paste_result_table_renders_across_png_svg_pdf_and_updates_on_upstream_edit() {
    let mut session = Session::from_document(seed_column_doc());
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-anova", "source_sheet_id": "d-col", "kind": "one_way_anova" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "d-col", "kind": "column" }));
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-tbl", "preset": "double_column" }));

    // Paste transposed multi-column result table onto Graph and standard onto Layout
    ok_cmd(
        &mut session,
        "edit.pasteResultTable",
        json!({
            "source_sheet_id": "res-anova",
            "target_sheet_id": "g-col",
            "bounds_pt": [60.0, 30.0, 260.0, 120.0],
            "transpose": true,
            "style": { "bold_headers": true, "outer_border_width_pt": 1.0, "border_color": "#202020" }
        }),
    );
    ok_cmd(
        &mut session,
        "edit.pasteResultTable",
        json!({
            "source_sheet_id": "res-anova",
            "target_sheet_id": "lay-tbl",
            "bounds_pt": [20.0, 20.0, 320.0, 140.0],
            "panel_id": "p-anova-tbl"
        }),
    );

    // Export both GraphScene & LayoutScene across PNG, SVG, and %PDF-1.7
    let g_scene = compile_graph(session.document().graph_sheet(&SheetId::new("g-col")).unwrap(), session.document()).unwrap();
    let l_scene = compile_layout(session.document().layout_sheet(&SheetId::new("lay-tbl")).unwrap(), session.document()).unwrap();
    for scene in [&g_scene, &l_scene] {
        let raster = render_scene(scene, RenderTargetSpec { width_px: 360, height_px: 220, dpi: 96 }).expect("render rgba");
        let png = encode_png(&raster).expect("encode png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let svg = render_svg(scene, &SvgExportOptions::default()).expect("render svg");
        validate_svg_structure(&svg).expect("valid svg");
        assert!(svg.contains("<text"));

        let pdf = render_pdf(scene, &PdfExportOptions::default()).expect("render pdf");
        assert!(pdf.starts_with(b"%PDF-1.7"));
    }

    // Mutate upstream cell strongly to shift ANOVA F / R squared, verify live SVG changes accordingly
    let svg_before = render_svg(&g_scene, &SvgExportOptions::default()).unwrap();
    ok_cmd(&mut session, "table.setCells", json!({ "sheet_id": "d-col", "cells": [{ "row": 0, "col": 2, "value": 999.0 }] }));
    let g_scene_after = compile_graph(session.document().graph_sheet(&SheetId::new("g-col")).unwrap(), session.document()).unwrap();
    let svg_after = render_svg(&g_scene_after, &SvgExportOptions::default()).unwrap();
    assert_ne!(svg_before, svg_after, "pasted result table must refresh on upstream data edit");

    // Undo / Redo edit.pasteResultTable
    ok_cmd(&mut session, "edit.undo", json!({})); // undoes setCells
    ok_cmd(&mut session, "edit.undo", json!({})); // undoes layout pasteResultTable
    assert!(session.document().layout_sheet(&SheetId::new("lay-tbl")).unwrap().spec.items.is_empty());
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(session.document().layout_sheet(&SheetId::new("lay-tbl")).unwrap().spec.items.len(), 1);
}

#[test]
fn layout_equalize_scaling_and_reorder_panel_with_undo_redo() {
    let mut session = Session::from_document(seed_column_doc());
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "d-col", "kind": "column" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "d-col", "kind": "column" }));
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-eq", "preset": "double_column" }));
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-eq", "panel_id": "p1", "kind": "graph", "graph_sheet_id": "g-1", "bounds_pt": [10.0, 10.0, 240.0, 160.0] }),
    );
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-eq", "panel_id": "p2", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": [260.0, 10.0, 140.0, 95.0] }),
    );
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "kind": "text", "markdown": "FrontLabel", "bounds_pt": [30.0, 30.0, 90.0, 24.0] }),
    );

    // EqualizeScaling across MatchSmallest, MatchLargest, ExplicitPercent (within <= 0.25 pt inner plot width parity)
    for eq_params in [
        json!({ "sheet_id": "lay-eq", "mode": "match_smallest" }),
        json!({ "sheet_id": "lay-eq", "mode": "match_largest" }),
        json!({ "sheet_id": "lay-eq", "mode": "explicit_percent", "percent": 60.0 }),
    ] {
        let out = ok_cmd(&mut session, "layout.equalizeScaling", eq_params);
        let panels = out.payload["panels"].as_array().unwrap();
        let w0 = panels[0]["rendered_plot_width_pt"].as_f64().unwrap();
        let w1 = panels[1]["rendered_plot_width_pt"].as_f64().unwrap();
        assert!((w0 - w1).abs() <= 0.25, "rendered plot widths diverged: {w0} vs {w1}");
        // Also verify against independent scene compilation scale calculation
        let lay = session.document().layout_sheet(&SheetId::new("lay-eq")).unwrap();
        let sub0 = compile_graph(session.document().graph_sheet(&SheetId::new("g-1")).unwrap(), session.document()).unwrap();
        let sub1 = compile_graph(session.document().graph_sheet(&SheetId::new("g-2")).unwrap(), session.document()).unwrap();
        let [
            curvebook_doc::LayoutItem::LiveGraph { bounds_pt: b0, scale_percent: sp0, .. },
            curvebook_doc::LayoutItem::LiveGraph { bounds_pt: b1, scale_percent: sp1, .. },
            _,
        ] = &lay.spec.items[..]
        else {
            panic!("expected 3 layout items");
        };
        let eff0 = (b0.width / sub0.width_pt).min(b0.height / sub0.height_pt) * (sp0 / 100.0);
        let eff1 = (b1.width / sub1.width_pt).min(b1.height / sub1.height_pt) * (sp1 / 100.0);
        assert!((sub0.plot_rect_pt.width * eff0 - sub1.plot_rect_pt.width * eff1).abs() <= 0.25);
    }

    // ReorderPanel: verify vector z-order & compiled paint order
    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "action": "send_to_back" }));
    let scene_back = compile_layout(session.document().layout_sheet(&SheetId::new("lay-eq")).unwrap(), session.document()).unwrap();
    let idx_callout_back =
        scene_back.primitives.iter().position(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "FrontLabel")).unwrap();
    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_index": 0, "action": "bring_to_front" }));
    let scene_front = compile_layout(session.document().layout_sheet(&SheetId::new("lay-eq")).unwrap(), session.document()).unwrap();
    let idx_callout_front =
        scene_front.primitives.iter().position(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "FrontLabel")).unwrap();
    assert!(idx_callout_front > idx_callout_back, "bring_to_front must paint panel later than send_to_back");

    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "action": "send_backward" }));
    assert_eq!(
        session
            .document()
            .layout_sheet(&SheetId::new("lay-eq"))
            .unwrap()
            .spec
            .items
            .iter()
            .position(|it| matches!(it, curvebook_doc::LayoutItem::TextCallout { id, .. } if id == "p3-callout")),
        Some(1)
    );
    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "action": "bring_forward" }));
    assert_eq!(
        session
            .document()
            .layout_sheet(&SheetId::new("lay-eq"))
            .unwrap()
            .spec
            .items
            .iter()
            .position(|it| matches!(it, curvebook_doc::LayoutItem::TextCallout { id, .. } if id == "p3-callout")),
        Some(2)
    );

    // Undo / Redo reorderPanel
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert_eq!(
        session
            .document()
            .layout_sheet(&SheetId::new("lay-eq"))
            .unwrap()
            .spec
            .items
            .iter()
            .position(|it| matches!(it, curvebook_doc::LayoutItem::TextCallout { id, .. } if id == "p3-callout")),
        Some(1)
    );
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(
        session
            .document()
            .layout_sheet(&SheetId::new("lay-eq"))
            .unwrap()
            .spec
            .items
            .iter()
            .position(|it| matches!(it, curvebook_doc::LayoutItem::TextCallout { id, .. } if id == "p3-callout")),
        Some(2)
    );
}
