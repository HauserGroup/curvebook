//! Integration tests for Wave 6 scientific/graph/export QoL commands (`G11`, `QOL-6`, `QOL-8`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnnotationShapeKind, FigureCanvasPreset, GraphAnnotation, GraphicalDataObject, LayoutItem, LayoutSheet, LayoutSpec, PhysicalUnit, RectPt, SheetId,
};
use curvebook_engine::Session;
use curvebook_palette::{LineDashStyle, Rgba8};
use serde_json::json;

use super::common::{TempDir, assert_bad_input, col_sheet, decode_base64, err_cmd, gs, ls, ok_cmd, run_cmds, session_with, xy_4pl_sheet};

fn anova_col(id: &str) -> curvebook_doc::DataSheet {
    col_sheet(
        id,
        "Treatment Response",
        &[("Vehicle", &[10.2, 11.0, 9.8, 10.5, 10.1]), ("Low Dose", &[14.5, 15.2, 13.9, 14.8, 15.1]), ("High Dose", &[22.4, 23.1, 21.8, 22.9, 23.0])],
    )
}

fn seeded_sci_session() -> Session {
    let mut session = session_with("Wave 6 Sci QoL Project", vec![xy_4pl_sheet("xy-1"), anova_col("col-1")]);
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy", "title": "Dose Curve" })),
            ("graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column", "title": "Group Summary" })),
            ("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1", "preset": "double_column", "grid_template": [1, 2] })),
            ("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy", "row": 0, "col": 0 })),
            ("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-col", "row": 0, "col": 1 })),
        ],
    );
    session
}

#[test]
fn g11_rejects_oversized_or_invalid_raster_dimensions_and_zero_dpi() {
    let mut session = seeded_sci_session();
    let undo_before = session.undo_depth();

    for (cmd, params) in [
        ("export.png", json!({ "sheet_id": "g-xy", "width_px": u32::MAX, "height_px": 1 })),
        ("graph.exportPng", json!({ "sheet_id": "g-xy", "width_px": u32::MAX, "height_px": u32::MAX })),
        ("export.png", json!({ "sheet_id": "g-xy", "width_px": 16_384, "height_px": 16_384 })),
        ("export.tiff", json!({ "sheet_id": "g-xy", "width_px": 16_384, "height_px": 16_384 })),
        ("export.png", json!({ "sheet_id": "g-xy", "width_px": 8_193, "height_px": 8_192 })),
        ("export.png", json!({ "sheet_id": "g-xy", "width_px": 16_385, "height_px": 1 })),
        ("export.png", json!({ "sheet_id": "g-xy", "width_px": 20_000, "height_px": 100 })),
        ("export.png", json!({ "sheet_id": "g-xy", "width_px": 0, "height_px": 100 })),
        ("export.tiff", json!({ "sheet_id": "g-xy", "width_px": 100, "height_px": 0 })),
        ("export.png", json!({ "sheet_id": "g-xy", "dpi": 0 })),
        ("export.tiff", json!({ "sheet_id": "g-xy", "width_px": -5 })),
        ("export.png", json!({ "sheet_id": "g-xy", "dpi": "bad" })),
    ] {
        assert!(!err_cmd(&mut session, cmd, params).message().trim().is_empty());
    }

    let ok_png = ok_cmd(&mut session, "graph.exportPng", json!({ "sheet_id": "g-xy", "width_px": 120, "height_px": 90, "dpi": 150 }));
    assert_eq!((&ok_png.payload["width_px"], &ok_png.payload["height_px"], &ok_png.payload["dpi"]), (&json!(120), &json!(90), &json!(150)));
    assert_eq!(
        ok_cmd(&mut session, "export.tiff", json!({ "sheet_id": "g-xy", "width_px": 120, "height_px": 90, "dpi": 300.0 })).payload["dpi"],
        300
    );
    assert_eq!(session.undo_depth(), undo_before);
}

#[test]
fn g11_graph_and_layout_export_emf_emit_valid_header_and_are_read_only() {
    let dir = TempDir::new("g11-emf");
    let mut session = seeded_sci_session();
    let (undo_before, redo_before) = (session.undo_depth(), session.redo_depth());

    let g_in_mem = ok_cmd(&mut session, "graph.exportEmf", json!({ "sheet_id": "g-xy" }));
    assert_eq!(g_in_mem.payload["bytes_written"], 0);
    assert_eq!(g_in_mem.payload["header_magic"], "0x464D4520");
    assert!(g_in_mem.payload["byte_len"].as_u64().unwrap() > 100);

    for (cmd_id, sheet_id, fname) in [("graph.exportEmf", "g-xy", "graph_out.emf"), ("layout.exportEmf", "lay-1", "layout_out.emf")] {
        let path = dir.child(fname);
        let out = ok_cmd(&mut session, cmd_id, json!({ "sheet_id": sheet_id, "path": path.to_string_lossy() }));
        assert_eq!(out.payload["header_magic"], "0x464D4520");
        assert_eq!(out.payload["bytes_written"], out.payload["byte_len"]);
        let disk = std::fs::read(&path).unwrap();
        assert_eq!(decode_base64(out.payload["base64"].as_str().unwrap()), disk);
        assert!(
            disk.len() >= 88
                && u32::from_le_bytes(disk[0..4].try_into().unwrap()) == 1
                && u32::from_le_bytes(disk[40..44].try_into().unwrap()) == 0x464D_4520
        );
    }

    let _ = err_cmd(&mut session, "graph.exportEmf", json!({ "sheet_id": "lay-1" }));
    let _ = err_cmd(&mut session, "layout.exportEmf", json!({ "sheet_id": "g-xy" }));
    assert_eq!((session.undo_depth(), session.redo_depth()), (undo_before, redo_before));
}

#[test]
fn qol6_result_generate_report_renders_plain_markdown_and_latex_read_only() {
    let mut session = session_with("Methods Report Project", vec![xy_4pl_sheet("xy-1"), anova_col("col-1")]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova" })),
            ("fit.run", json!({ "sheet_id": "res-4pl", "source_sheet_id": "xy-1", "kind": "four_parameter_logistic" })),
        ],
    );
    let (undo_before, redo_before) = (session.undo_depth(), session.redo_depth());

    let default_rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-anova" }));
    assert_eq!((&default_rep.payload["sheet_id"], &default_rep.payload["format"]), (&json!("res-anova"), &json!("markdown")));
    let md = default_rep.payload["report"].as_str().unwrap();
    assert!(!md.trim().is_empty() && (md.contains('#') || md.contains("**")));

    let plain_rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-anova", "format": "plain" }));
    let plain = plain_rep.payload["report"].as_str().unwrap();
    assert_eq!(plain_rep.payload["format"], "plain");
    assert!(!plain.trim().is_empty() && !plain.contains("\\section"));

    let latex_rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-4pl", "format": "latex" }));
    let latex = latex_rep.payload["report"].as_str().unwrap();
    assert_eq!(latex_rep.payload["format"], "latex");
    assert!(!latex.trim().is_empty() && latex.contains('\\'));

    let _ = err_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "missing-res" }));
    let _ = err_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-anova", "format": "docx" }));
    assert_eq!((session.undo_depth(), session.redo_depth()), (undo_before, redo_before));
}

#[test]
fn qol8_graph_canvas_presets_keep_typography_vs_proportional_and_undo_redo() {
    let mut session = seeded_sci_session();
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova" })),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g-col", "spec": { "canvas_size_pt": { "width": 864.0, "height": 648.0 }, "x_axis": { "stroke_width_pt": 0.4 }, "legend": { "visible": true, "position_pt": [680.0, 80.0], "bordered": false } } }),
            ),
            (
                "graph.setSeries",
                json!({ "sheet_id": "g-xy", "series_index": 0, "marker": { "size_pt": 8.0, "stroke_width_pt": 0.4 }, "connecting_line": { "stroke_width_pt": 0.5 } }),
            ),
            ("graph.setSpec", json!({ "sheet_id": "g-xy", "spec": { "canvas_size_pt": { "width": 864.0, "height": 648.0 } } })),
            (
                "graph.addComparisonBrackets",
                json!({ "sheet_id": "g-col", "result_sheet_id": "res-anova", "significant_only": false, "font_size_pt": 10.0, "stroke_width_pt": 0.5 }),
            ),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g-col", "annotation": GraphAnnotation::Shape { kind: AnnotationShapeKind::Rectangle, bounds_pt: RectPt::new(100.0, 100.0, 200.0, 100.0), stroke_color: Rgba8::BLACK, stroke_width_pt: 0.4, dash: LineDashStyle::Solid, fill_color: None } }),
            ),
        ],
    );

    let single_pt = FigureCanvasPreset::SingleColumn88mm.dimensions_pt();
    let keep_out = ok_cmd(
        &mut session,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-col", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }),
    );
    assert!(
        (keep_out.payload["width_pt"].as_f64().unwrap() - single_pt.width).abs() < 1e-3
            && (keep_out.payload["height_pt"].as_f64().unwrap() - single_pt.height).abs() < 1e-3
    );
    let g = gs(&session, "g-col");
    assert_eq!(g.spec.comparisons[0].font_size_pt, 10.0);
    assert!(
        g.spec.x_axis.stroke_width_pt >= 0.25
            && g.spec.comparisons[0].stroke_width_pt >= 0.25
            && g.spec.legend.position_pt.unwrap().0 < single_pt.width
    );

    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!((gs(&session, "g-col").spec.canvas_size_pt.width - 864.0).abs() < 1e-6);
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert!((gs(&session, "g-col").spec.canvas_size_pt.width - single_pt.width).abs() < 1e-3);

    ok_cmd(&mut session, "edit.undo", json!({}));
    let prop_out = ok_cmd(
        &mut session,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-col", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }),
    );
    let issues = prop_out.payload["issues"].as_array().unwrap();
    assert!(issues.iter().any(|i| i["code"] == "min_font_size") && issues.iter().any(|i| i["code"] == "hairline_stroke"));
    assert!(gs(&session, "g-col").spec.comparisons[0].font_size_pt < 5.0 && gs(&session, "g-col").spec.x_axis.stroke_width_pt < 0.25);

    ok_cmd(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }));
    let GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } = &gs(&session, "g-xy").spec.series[0].gdo else { panic!("expected marker") };
    assert!((m.size_pt - 8.0).abs() < 1e-6);
    run_cmds(
        &mut session,
        &[
            ("edit.undo", json!({})),
            ("graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" })),
        ],
    );
    let GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } = &gs(&session, "g-xy").spec.series[0].gdo else { panic!("expected marker") };
    assert!(m.size_pt < 4.0);
}

#[test]
fn qol8_layout_canvas_presets_preflight_and_all_presets() {
    let mut session = seeded_sci_session();
    run_cmds(
        &mut session,
        &[
            (
                "graph.setSpec",
                json!({ "sheet_id": "g-col", "spec": { "canvas_size_pt": { "width": 864.0, "height": 648.0 }, "x_axis": { "stroke_width_pt": 0.4 } } }),
            ),
            ("graph.applyCanvasPreset", json!({ "sheet_id": "g-col", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" })),
        ],
    );

    let single_pt = FigureCanvasPreset::SingleColumn88mm.dimensions_pt();
    let mut layout_doc = session.document().clone();
    layout_doc
        .add_layout_sheet(
            LayoutSheet::new(
                SheetId::new("lay-big"),
                "Big Layout",
                LayoutSpec {
                    width: 864.0,
                    height: 648.0,
                    unit: PhysicalUnit::Points,
                    items: vec![
                        LayoutItem::LiveGraph {
                            id: String::from("panel-a"),
                            graph_sheet_id: SheetId::new("g-col"),
                            bounds_pt: RectPt::new(40.0, 40.0, 400.0, 300.0),
                            scale_percent: 100.0,
                            badge_override: None,
                        },
                        LayoutItem::TextCallout {
                            id: String::from("callout-a"),
                            markdown: String::from("Mean +/- SD"),
                            bounds_pt: RectPt::new(480.0, 40.0, 260.0, 80.0),
                            font_size_pt: 10.0,
                        },
                    ],
                    ..LayoutSpec::default()
                },
            )
            .unwrap(),
        )
        .unwrap();
    let mut s = Session::from_document(layout_doc);

    let lay_keep =
        ok_cmd(&mut s, "layout.applyCanvasPreset", json!({ "sheet_id": "lay-big", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }));
    assert!((lay_keep.payload["width_pt"].as_f64().unwrap() - single_pt.width).abs() < 1e-3);
    let LayoutItem::TextCallout { font_size_pt, bounds_pt, .. } = &ls(&s, "lay-big").spec.items[1] else { panic!("expected TextCallout") };
    assert!(*font_size_pt == 10.0 && bounds_pt.x < single_pt.width);

    ok_cmd(&mut s, "edit.undo", json!({}));
    assert!((ls(&s, "lay-big").spec.page_size_pt().width - 864.0).abs() < 1e-6);
    ok_cmd(&mut s, "edit.redo", json!({}));
    assert!((ls(&s, "lay-big").spec.page_size_pt().width - single_pt.width).abs() < 1e-3);
    ok_cmd(&mut s, "edit.undo", json!({}));

    let lay_prop = ok_cmd(
        &mut s,
        "layout.applyCanvasPreset",
        json!({ "sheet_id": "lay-big", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }),
    );
    assert!(lay_prop.payload["issues"].as_array().unwrap().iter().any(|i| i["code"] == "min_font_size"));

    let (undo_before, redo_before) = (s.undo_depth(), s.redo_depth());
    let pf_graph = ok_cmd(&mut s, "graph.preflightCheck", json!({ "sheet_id": "g-col" }));
    assert_eq!(pf_graph.payload["sheet_id"], "g-col");
    assert!(pf_graph.payload["issue_count"].as_u64().unwrap() >= 1 && pf_graph.payload["passed"] == false);
    let pf_layout = ok_cmd(&mut s, "graph.preflightCheck", json!({ "sheet_id": "lay-big" }));
    assert!(pf_layout.payload["sheet_id"] == "lay-big" && pf_layout.payload["passed"] == false);
    assert_eq!((s.undo_depth(), s.redo_depth()), (undo_before, redo_before));

    for preset_name in ["SingleColumn88mm", "ColumnAndHalf130mm", "DoubleColumn180mm", "FullPagePortrait", "Slide16x9", "PosterPanel"] {
        let expected_pt = FigureCanvasPreset::from_name(preset_name).unwrap().dimensions_pt();
        for (cmd_id, sheet_id) in [("graph.applyCanvasPreset", "g-xy"), ("layout.applyCanvasPreset", "lay-big")] {
            let out = ok_cmd(&mut s, cmd_id, json!({ "sheet_id": sheet_id, "preset": preset_name, "resize_mode": "KeepTypographyPt" }));
            assert!((out.payload["width_pt"].as_f64().unwrap() - expected_pt.width).abs() < 1e-3);
            assert!((out.payload["height_pt"].as_f64().unwrap() - expected_pt.height).abs() < 1e-3);
        }
    }

    // AUD-02-09: proportional scale clamps tiny TextCallout font_size_pt >= 0.5 and keeps layout editable.
    run_cmds(
        &mut s,
        &[
            ("layout.create", json!({ "sheet_id": "lay-clamp", "width": 2000.0, "height": 2000.0, "unit": "pt" })),
            (
                "layout.addPanel",
                json!({ "sheet_id": "lay-clamp", "kind": "text", "markdown": "Small note", "font_size_pt": 1.0, "bounds_pt": [10.0, 10.0, 100.0, 20.0] }),
            ),
            ("layout.applyCanvasPreset", json!({ "sheet_id": "lay-clamp", "preset": "SingleColumn88mm", "resize_mode": "proportional_scale" })),
        ],
    );
    let LayoutItem::TextCallout { font_size_pt, .. } = &ls(&s, "lay-clamp").spec.items[0] else { panic!("expected TextCallout") };
    assert!(*font_size_pt >= 0.5);
    ok_cmd(
        &mut s,
        "layout.addPanel",
        json!({ "sheet_id": "lay-clamp", "kind": "text", "markdown": "Follow-up", "bounds_pt": [5.0, 5.0, 40.0, 15.0] }),
    );

    // AUD-02-13: layout panel IDs stay unique after deletion (auto addPanel + edit.pasteResultTable) and explicit duplicates fail.
    run_cmds(
        &mut s,
        &[
            ("stats.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-1", "kind": "descriptive" })),
            ("layout.create", json!({ "sheet_id": "lay-id" })),
            ("layout.addPanel", json!({ "sheet_id": "lay-id", "kind": "text", "markdown": "One" })),
            ("layout.addPanel", json!({ "sheet_id": "lay-id", "kind": "text", "markdown": "Two" })),
        ],
    );
    let rem = ls(&s, "lay-id").spec.items[1].clone();
    run_cmds(
        &mut s,
        &[
            ("layout.setSpec", json!({ "sheet_id": "lay-id", "spec": { "items": [rem] } })),
            ("layout.addPanel", json!({ "sheet_id": "lay-id", "kind": "text", "markdown": "Three" })),
        ],
    );
    let items = &ls(&s, "lay-id").spec.items;
    assert!(items.len() == 2 && items[0].id() != items[1].id());
    assert_ne!(
        ok_cmd(&mut s, "edit.pasteResultTable", json!({ "source_sheet_id": "res-desc", "target_sheet_id": "lay-id" })).payload["panel_id"],
        "panel-2"
    );
    assert_bad_input(&mut s, "layout.addPanel", json!({ "sheet_id": "lay-id", "kind": "text", "markdown": "Dup", "panel_id": "panel-2" }));
}
