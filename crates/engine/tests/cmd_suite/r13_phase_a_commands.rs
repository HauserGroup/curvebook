#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ScenePrimitive, compile_graph, compile_layout};
use curvebook_doc::{
    CartesianOrientation, ErrorBarMetric, ErrorCode, GraphicalDataObject, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapRowLabelMode,
    LayoutItem, MAX_SHEETS_PER_PROJECT, ProjectDocument, ReplicateLineMode, SheetId, SurvivalDisplayMetric, TickDirection, TickLabelFormat,
};
use curvebook_engine::Session;
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_render::{RenderTargetSpec, encode_png, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use serde_json::json;

use super::common::{assert_err_code, col_sheet, gs, ls, ok_cmd, run_cmds, session_with};

fn seed_column_doc() -> ProjectDocument {
    session_with(
        "R13 Phase A",
        vec![col_sheet("d-col", "Groups", &[("A", &[10.0, 12.0, 11.0, 13.0]), ("B", &[24.0, 26.0, 25.0, 27.0]), ("C", &[40.0, 44.0, 42.0, 46.0])])],
    )
    .document()
    .clone()
}

fn fill_to_cap(doc: &mut ProjectDocument, leave_slots: usize) {
    let needed = MAX_SHEETS_PER_PROJECT - doc.total_sheet_count() - leave_slots;
    for i in 0..needed {
        doc.add_data_sheet(col_sheet(&format!("fill-{i}"), "Fill", &[("A", &[])])).unwrap();
    }
}

#[test]
fn graph_create_one_per_data_set_y_title_atomic_cap_and_undo() {
    let mut session = Session::from_document(seed_column_doc());

    let res = ok_cmd(
        &mut session,
        "graph.create",
        json!({ "data_sheet_id": "d-col", "one_per_data_set": true, "y_axis_title": "Normalized Signal", "kind": "column" }),
    );
    assert_eq!(res.payload["created_count"], 3);
    let ids = res.payload["sheet_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 3);
    for (idx, v) in ids.iter().enumerate() {
        let g = gs(&session, v.as_str().unwrap());
        assert_eq!(g.spec.series.len(), 1);
        assert_eq!(g.spec.series[0].data_set_index, idx);
        assert_eq!(g.spec.y_axis.title, "Normalized Signal");
    }

    assert_eq!(session.document().graph_sheets.len(), 3);
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert_eq!(session.document().graph_sheets.len(), 0);
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(session.document().graph_sheets.len(), 3);

    // Upfront MAX_SHEETS_PER_PROJECT check prevents partial insertion.
    let mut capped_doc = seed_column_doc();
    fill_to_cap(&mut capped_doc, 2);
    let mut capped_sess = Session::from_document(capped_doc);
    let before = capped_sess.document().total_sheet_count();
    let _ =
        assert_err_code(&mut capped_sess, "graph.create", json!({ "data_sheet_id": "d-col", "one_per_data_set": true }), ErrorCode::ResourceLimit);
    assert_eq!(capped_sess.document().total_sheet_count(), before);
}

#[test]
fn graph_apply_style_from_batch_skips_donor_atomic_and_single_undo() {
    let mut session = Session::from_document(seed_column_doc());
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-donor", "data_sheet_id": "d-col", "kind": "column" })),
            ("graph.create", json!({ "sheet_id": "g-t1", "data_sheet_id": "d-col", "kind": "column" })),
            ("graph.create", json!({ "sheet_id": "g-t2", "data_sheet_id": "d-col", "kind": "column" })),
            ("graph.setSpec", json!({ "sheet_id": "g-donor", "paper_fill": "#F2F4F8" })),
            ("graph.setAxes", json!({ "sheet_id": "g-donor", "axis": "y", "tick_direction": "inside", "show_tick_labels": false })),
        ],
    );

    let _ = assert_err_code(
        &mut session,
        "graph.applyStyleFrom",
        json!({ "donor_sheet_id": "g-donor", "target_sheet_ids": ["g-t1", "missing-target"] }),
        ErrorCode::NotFound,
    );
    assert!(gs(&session, "g-t1").spec.y_axis.show_tick_labels);

    let out = ok_cmd(&mut session, "graph.applyStyleFrom", json!({ "donor_sheet_id": "g-donor", "target_sheet_ids": ["g-donor", "g-t1", "g-t2"] }));
    assert_eq!(out.payload["skipped_sheet_ids"], json!(["g-donor"]));
    assert_eq!(out.payload["applied_sheet_ids"], json!(["g-t1", "g-t2"]));
    assert!(!gs(&session, "g-t1").spec.y_axis.show_tick_labels && !gs(&session, "g-t2").spec.y_axis.show_tick_labels);

    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(gs(&session, "g-t1").spec.y_axis.show_tick_labels && gs(&session, "g-t2").spec.y_axis.show_tick_labels);
}

#[test]
fn graph_set_spec_axes_and_series_cover_all_r13_fields() {
    let mut session = Session::from_document(seed_column_doc());
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d-xy", "title": "XY", "archetype": "xy", "cols": 2, "rows": 3, "replicates": 2 })),
            ("sheet.createData", json!({ "sheet_id": "d-surv", "title": "Surv", "archetype": "survival", "cols": 2, "rows": 3 })),
            ("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "d-xy", "kind": "xy" })),
            ("graph.create", json!({ "sheet_id": "g-surv", "data_sheet_id": "d-surv", "kind": "survival" })),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g-xy", "paper_fill": null, "orientation": "horizontal", "forest_show_pooled_diamond": false, "heatmap_labels": { "row_labels": "row_numbers", "column_labels": "column_letters", "column_angle": "vertical_90" } }),
            ),
            (
                "graph.setAxes",
                json!({ "sheet_id": "g-xy", "x_axis": { "tick_direction": "both", "show_tick_labels": false, "tick_format": "absolute_decimal" } }),
            ),
            (
                "graph.setSeries",
                json!({ "sheet_id": "g-xy", "all_series": true, "offset": { "x_data": 0.25, "x_pt": 6.0, "y_pt": -4.0 }, "bar_width_pt": 16.0, "show_raw_data": false, "replicate_lines": "individual_replicate_trajectories", "alpha": 0.5 }),
            ),
            (
                "graph.setSeries",
                json!({ "sheet_id": "g-surv", "all_series": true, "display_metric": "percent_cumulative_event", "staircase": false, "error_display": "sem" }),
            ),
        ],
    );

    let g_xy = gs(&session, "g-xy");
    assert_eq!(g_xy.spec.paper_fill, None);
    assert_eq!(g_xy.spec.orientation, CartesianOrientation::Horizontal);
    assert!(!g_xy.spec.forest_show_pooled_diamond);
    assert_eq!(g_xy.spec.heatmap_labels.row_labels, HeatmapRowLabelMode::RowNumbers);
    assert_eq!(g_xy.spec.heatmap_labels.column_labels, HeatmapColumnLabelMode::ColumnLetters);
    assert_eq!(g_xy.spec.heatmap_labels.column_angle, HeatmapColumnLabelAngle::Vertical90);
    assert_eq!(g_xy.spec.x_axis.tick_direction, TickDirection::Both);
    assert!(!g_xy.spec.x_axis.show_tick_labels);
    assert_eq!(g_xy.spec.x_axis.tick_format, TickLabelFormat::AbsoluteDecimal);
    for s in &g_xy.spec.series {
        assert!((s.offset.x_data - 0.25).abs() < 1e-9 && (s.offset.x_pt - 6.0).abs() < 1e-9 && (s.offset.y_pt + 4.0).abs() < 1e-9);
        assert_eq!(s.bar_width_override_pt, Some(16.0));
        assert!(!s.show_raw_data);
        if let GraphicalDataObject::ScatterAndLine { replicate_lines, line, .. } = s.gdo {
            assert_eq!(replicate_lines, ReplicateLineMode::IndividualReplicateTrajectories);
            assert_eq!(line.unwrap().color.a, 128);
        }
    }

    let GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, .. } = gs(&session, "g-surv").spec.series[0].gdo else {
        panic!("expected KaplanMeierStep GDO");
    };
    assert_eq!(display_metric, SurvivalDisplayMetric::PercentCumulativeEvent);
    assert!(!staircase);
    assert_eq!(error_display, Some(ErrorBarMetric::Sem));
}

#[test]
fn transform_run_create_graph_reactive_undo_and_atomic_cap() {
    let mut session = Session::from_document(seed_column_doc());
    let out = ok_cmd(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "res-norm", "source_sheet_id": "d-col", "kind": "normalize", "create_graph": true, "graph_sheet_id": "g-norm" }),
    );
    assert_eq!(out.payload["graph_sheet_id"], "g-norm");
    let g = gs(&session, "g-norm");
    assert_eq!(g.spec.primary_source_sheet_id.as_str(), "res-norm");
    assert!(compile_graph(g, session.document()).is_ok());

    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(
        session.document().result_sheet(&SheetId::new("res-norm")).is_none() && session.document().graph_sheet(&SheetId::new("g-norm")).is_none()
    );
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert!(session.document().graph_sheet(&SheetId::new("g-norm")).is_some());

    let mut capped_doc = seed_column_doc();
    fill_to_cap(&mut capped_doc, 1);
    let mut capped = Session::from_document(capped_doc);
    let before = capped.document().total_sheet_count();
    let _ = assert_err_code(
        &mut capped,
        "transform.run",
        json!({ "sheet_id": "res-over", "source_sheet_id": "d-col", "kind": "normalize", "create_graph": true }),
        ErrorCode::ResourceLimit,
    );
    assert_eq!(capped.document().total_sheet_count(), before);
}

#[test]
fn edit_paste_result_table_renders_across_png_svg_pdf_and_updates_on_upstream_edit() {
    let mut session = Session::from_document(seed_column_doc());
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "res-anova", "source_sheet_id": "d-col", "kind": "one_way_anova" })),
            ("graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "d-col", "kind": "column" })),
            ("layout.create", json!({ "sheet_id": "lay-tbl", "preset": "double_column" })),
            (
                "edit.pasteResultTable",
                json!({ "source_sheet_id": "res-anova", "target_sheet_id": "g-col", "bounds_pt": [60.0, 30.0, 260.0, 120.0], "transpose": true, "style": { "bold_headers": true, "outer_border_width_pt": 1.0, "border_color": "#202020" } }),
            ),
            (
                "edit.pasteResultTable",
                json!({ "source_sheet_id": "res-anova", "target_sheet_id": "lay-tbl", "bounds_pt": [20.0, 20.0, 320.0, 140.0], "panel_id": "p-anova-tbl" }),
            ),
        ],
    );

    let g_scene = compile_graph(gs(&session, "g-col"), session.document()).unwrap();
    let l_scene = compile_layout(ls(&session, "lay-tbl"), session.document()).unwrap();
    for scene in [&g_scene, &l_scene] {
        let png = encode_png(&render_scene(scene, RenderTargetSpec { width_px: 360, height_px: 220, dpi: 96 }).expect("rgba")).expect("png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let svg = render_svg(scene, &SvgExportOptions::default()).expect("svg");
        validate_svg_structure(&svg).expect("valid svg");
        assert!(svg.contains("<text"));
        assert!(render_pdf(scene, &PdfExportOptions::default()).expect("pdf").starts_with(b"%PDF-1.7"));
    }

    let svg_before = render_svg(&g_scene, &SvgExportOptions::default()).unwrap();
    ok_cmd(&mut session, "table.setCells", json!({ "sheet_id": "d-col", "edits": [{ "row": 0, "col": 2, "value": 999.0 }] }));
    let svg_after = render_svg(&compile_graph(gs(&session, "g-col"), session.document()).unwrap(), &SvgExportOptions::default()).unwrap();
    assert_ne!(svg_before, svg_after);

    run_cmds(&mut session, &[("edit.undo", json!({})), ("edit.undo", json!({}))]);
    assert!(ls(&session, "lay-tbl").spec.items.is_empty());
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(ls(&session, "lay-tbl").spec.items.len(), 1);
}

#[test]
fn layout_equalize_scaling_and_reorder_panel_with_undo_redo() {
    let mut session = Session::from_document(seed_column_doc());
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "d-col", "kind": "column" })),
            ("graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "d-col", "kind": "column" })),
            ("layout.create", json!({ "sheet_id": "lay-eq", "preset": "double_column" })),
            (
                "layout.addPanel",
                json!({ "sheet_id": "lay-eq", "panel_id": "p1", "kind": "graph", "graph_sheet_id": "g-1", "bounds_pt": [10.0, 10.0, 240.0, 160.0] }),
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "lay-eq", "panel_id": "p2", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": [260.0, 10.0, 140.0, 95.0] }),
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "kind": "text", "markdown": "FrontLabel", "bounds_pt": [30.0, 30.0, 90.0, 24.0] }),
            ),
        ],
    );

    for eq_params in [
        json!({ "sheet_id": "lay-eq", "mode": "match_smallest" }),
        json!({ "sheet_id": "lay-eq", "mode": "match_largest" }),
        json!({ "sheet_id": "lay-eq", "mode": "explicit_percent", "percent": 60.0 }),
    ] {
        let out = ok_cmd(&mut session, "layout.equalizeScaling", eq_params);
        let panels = out.payload["panels"].as_array().unwrap();
        assert!((panels[0]["rendered_plot_width_pt"].as_f64().unwrap() - panels[1]["rendered_plot_width_pt"].as_f64().unwrap()).abs() <= 0.25);
        let sub0 = compile_graph(gs(&session, "g-1"), session.document()).unwrap();
        let sub1 = compile_graph(gs(&session, "g-2"), session.document()).unwrap();
        let [LayoutItem::LiveGraph { bounds_pt: b0, scale_percent: sp0, .. }, LayoutItem::LiveGraph { bounds_pt: b1, scale_percent: sp1, .. }, _] =
            &ls(&session, "lay-eq").spec.items[..]
        else {
            panic!("expected 3 layout items");
        };
        let eff0 = (b0.width / sub0.width_pt).min(b0.height / sub0.height_pt) * (sp0 / 100.0);
        let eff1 = (b1.width / sub1.width_pt).min(b1.height / sub1.height_pt) * (sp1 / 100.0);
        assert!((sub0.plot_rect_pt.width * eff0 - sub1.plot_rect_pt.width * eff1).abs() <= 0.25);
    }

    let callout_pos =
        |s: &Session| ls(s, "lay-eq").spec.items.iter().position(|it| matches!(it, LayoutItem::TextCallout { id, .. } if id == "p3-callout"));

    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "action": "send_to_back" }));
    let idx_back = compile_layout(ls(&session, "lay-eq"), session.document())
        .unwrap()
        .primitives
        .iter()
        .position(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "FrontLabel"))
        .unwrap();
    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_index": 0, "action": "bring_to_front" }));
    let idx_front = compile_layout(ls(&session, "lay-eq"), session.document())
        .unwrap()
        .primitives
        .iter()
        .position(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "FrontLabel"))
        .unwrap();
    assert!(idx_front > idx_back);

    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "action": "send_backward" }));
    assert_eq!(callout_pos(&session), Some(1));
    ok_cmd(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-eq", "panel_id": "p3-callout", "action": "bring_forward" }));
    assert_eq!(callout_pos(&session), Some(2));
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert_eq!(callout_pos(&session), Some(1));
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(callout_pos(&session), Some(2));
}
