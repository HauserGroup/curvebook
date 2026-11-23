//! Integration tests for Wave 6 scientific/graph/export QoL commands (`G11`, `QOL-6`, `QOL-8`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, FigureCanvasPreset, GraphAnnotation, GraphicalDataObject, LayoutItem, LayoutSheet, LayoutSpec,
    PhysicalUnit, ProjectDocument, RectPt, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::{CommandOutcome, Session};
use curvebook_palette::Rgba8;
use serde_json::{Value, json};

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn temp_test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join("curvebook-sci-qol-tests").join(name)
}

fn decode_base64(input: &str) -> Vec<u8> {
    fn val(b: u8) -> u8 {
        match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.as_chunks::<4>().0 {
        let [c0, c1, c2, c3] = *chunk;
        let n = (u32::from(val(c0)) << 18) | (u32::from(val(c1)) << 12) | (u32::from(val(c2)) << 6) | u32::from(val(c3));
        out.push(((n >> 16) & 0xFF) as u8);
        if c2 != b'=' {
            out.push(((n >> 8) & 0xFF) as u8);
        }
        if c3 != b'=' {
            out.push((n & 0xFF) as u8);
        }
    }
    out
}

fn sample_4pl_xy_sheet(id: &str) -> DataSheet {
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut table = XyTable::new(SubcolumnMode::RawReplicates(2), 1, xs.len()).unwrap();
    table.x_title = String::from("Log10 Concentration (M)");
    if let Some(ds0) = table.data_sets.first_mut() {
        ds0.title = String::from("Compound A");
        for ((&x, x_row), row) in xs.iter().zip(table.x_cells.iter_mut()).zip(ds0.cells.iter_mut()) {
            if let Some(x_cell) = x_row.first_mut() {
                *x_cell = Cell::number(x);
            }
            let y_true = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.1));
            if let [c0, c1] = row.as_mut_slice() {
                *c0 = Cell::number(y_true - 0.8);
                *c1 = Cell::number(y_true + 0.8);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Dose Response", TableArchetype::Xy(table)).unwrap()
}

fn sample_anova_column_sheet(id: &str) -> DataSheet {
    let groups =
        [("Vehicle", [10.2_f64, 11.0, 9.8, 10.5, 10.1]), ("Low Dose", [14.5, 15.2, 13.9, 14.8, 15.1]), ("High Dose", [22.4, 23.1, 21.8, 22.9, 23.0])];
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, groups.len(), 5).unwrap();
    for (ds, (name, vals)) in table.data_sets.iter_mut().zip(groups.iter()) {
        ds.title = (*name).to_owned();
        for (row, &v) in ds.cells.iter_mut().zip(vals.iter()) {
            if let Some(cell) = row.first_mut() {
                *cell = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Treatment Response", TableArchetype::Column(table)).unwrap()
}

fn seeded_sci_session() -> Session {
    let mut doc = ProjectDocument::new("Wave 6 Sci QoL Project").unwrap();
    doc.add_data_sheet(sample_4pl_xy_sheet("xy-1")).unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("col-1")).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy", "title": "Dose Curve" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column", "title": "Group Summary" }));
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1", "preset": "double_column", "grid_template": [1, 2] }));
    ok_cmd(&mut session, "layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy", "row": 0, "col": 0 }));
    ok_cmd(&mut session, "layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-col", "row": 0, "col": 1 }));
    session
}

#[test]
fn g11_rejects_oversized_or_invalid_raster_dimensions_and_zero_dpi() {
    let mut session = seeded_sci_session();
    let undo_before = session.undo_depth();

    let invalid_cases = [
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
    ];

    for (cmd, params) in invalid_cases {
        let err = session.execute(cmd, params.clone()).expect_err(&format!("expected error for `{cmd}` with {params}"));
        assert!(!err.message().trim().is_empty());
    }

    let ok_png = ok_cmd(&mut session, "graph.exportPng", json!({ "sheet_id": "g-xy", "width_px": 120, "height_px": 90, "dpi": 150 }));
    assert_eq!(ok_png.payload["width_px"], json!(120));
    assert_eq!(ok_png.payload["height_px"], json!(90));
    assert_eq!(ok_png.payload["dpi"], json!(150));

    let ok_tiff = ok_cmd(&mut session, "export.tiff", json!({ "sheet_id": "g-xy", "width_px": 120, "height_px": 90, "dpi": 300.0 }));
    assert_eq!(ok_tiff.payload["dpi"], json!(300));
    assert_eq!(session.undo_depth(), undo_before);
}

#[test]
fn g11_graph_and_layout_export_emf_emit_valid_header_and_are_read_only() {
    let mut session = seeded_sci_session();
    let undo_before = session.undo_depth();
    let redo_before = session.redo_depth();

    let graph_emf_path = temp_test_path("graph_out.emf");
    let layout_emf_path = temp_test_path("layout_out.emf");
    let _ = std::fs::remove_file(&graph_emf_path);
    let _ = std::fs::remove_file(&layout_emf_path);

    let g_in_mem = ok_cmd(&mut session, "graph.exportEmf", json!({ "sheet_id": "g-xy" }));
    assert_eq!(g_in_mem.payload["bytes_written"], json!(0));
    assert_eq!(g_in_mem.payload["header_magic"], json!("0x464D4520"));
    assert!(g_in_mem.payload["byte_len"].as_u64().unwrap() > 100);

    let g_file = ok_cmd(&mut session, "graph.exportEmf", json!({ "sheet_id": "g-xy", "path": graph_emf_path.to_string_lossy() }));
    assert_eq!(g_file.payload["header_magic"], json!("0x464D4520"));
    assert_eq!(g_file.payload["bytes_written"], g_file.payload["byte_len"]);
    let g_b64_bytes = decode_base64(g_file.payload["base64"].as_str().unwrap());
    let g_disk_bytes = std::fs::read(&graph_emf_path).unwrap();
    assert_eq!(g_b64_bytes, g_disk_bytes);
    assert!(g_disk_bytes.len() >= 88);
    assert_eq!(u32::from_le_bytes(g_disk_bytes[0..4].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(g_disk_bytes[40..44].try_into().unwrap()), 0x464D_4520);

    let l_file = ok_cmd(&mut session, "layout.exportEmf", json!({ "sheet_id": "lay-1", "path": layout_emf_path.to_string_lossy() }));
    assert_eq!(l_file.payload["header_magic"], json!("0x464D4520"));
    assert_eq!(l_file.payload["bytes_written"], l_file.payload["byte_len"]);
    let l_disk_bytes = std::fs::read(&layout_emf_path).unwrap();
    assert_eq!(u32::from_le_bytes(l_disk_bytes[0..4].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(l_disk_bytes[40..44].try_into().unwrap()), 0x464D_4520);

    // Wrong sheet types fail cleanly.
    assert!(session.execute("graph.exportEmf", json!({ "sheet_id": "lay-1" })).is_err());
    assert!(session.execute("layout.exportEmf", json!({ "sheet_id": "g-xy" })).is_err());

    // Exports must not touch the undo/redo stacks.
    assert_eq!(session.undo_depth(), undo_before);
    assert_eq!(session.redo_depth(), redo_before);

    let _ = std::fs::remove_file(&graph_emf_path);
    let _ = std::fs::remove_file(&layout_emf_path);
}

#[test]
fn qol6_result_generate_report_renders_plain_markdown_and_latex_read_only() {
    let mut doc = ProjectDocument::new("Methods Report Project").unwrap();
    doc.add_data_sheet(sample_4pl_xy_sheet("xy-1")).unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("col-1")).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-anova", "source_sheet": "col-1", "kind": "one_way_anova" }));
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "res-4pl", "source_sheet": "xy-1", "kind": "four_parameter_logistic" }));

    let undo_before = session.undo_depth();
    let redo_before = session.redo_depth();

    let default_rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-anova" }));
    assert_eq!(default_rep.payload["sheet_id"], json!("res-anova"));
    assert_eq!(default_rep.payload["format"], json!("markdown"));
    let md_report = default_rep.payload["report"].as_str().unwrap();
    assert!(!md_report.trim().is_empty());
    assert!(md_report.contains('#') || md_report.contains("**"));

    let plain_rep = ok_cmd(&mut session, "result.generateReport", json!({ "sheet_id": "res-anova", "format": "plain" }));
    assert_eq!(plain_rep.payload["format"], json!("plain"));
    let plain_report = plain_rep.payload["report"].as_str().unwrap();
    assert!(!plain_report.trim().is_empty());
    assert!(!plain_report.contains("\\section"));

    let latex_rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-4pl", "format": "latex" }));
    assert_eq!(latex_rep.payload["format"], json!("latex"));
    let latex_report = latex_rep.payload["report"].as_str().unwrap();
    assert!(!latex_report.trim().is_empty());
    assert!(latex_report.contains('\\'));

    assert!(session.execute("result.generateReport", json!({ "result_sheet_id": "missing-res" })).is_err());
    assert!(session.execute("result.generateReport", json!({ "result_sheet_id": "res-anova", "format": "docx" })).is_err());

    assert_eq!(session.undo_depth(), undo_before);
    assert_eq!(session.redo_depth(), redo_before);
}

#[test]
fn qol8_graph_canvas_presets_keep_typography_vs_proportional_and_undo_redo() {
    let mut session = seeded_sci_session();

    // Seed a large graph (PosterPanel = 864x648 pt) with sub-1pt strokes, bracket typography, and an inside annotation.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-anova", "source_sheet": "col-1", "kind": "one_way_anova" }));
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g-col",
            "spec": {
                "canvas_size_pt": { "width": 864.0, "height": 648.0 },
                "x_axis": { "stroke_width_pt": 0.4 },
                "legend": { "visible": true, "position_pt": [680.0, 80.0], "bordered": false }
            }
        }),
    );
    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "series_index": 0,
            "marker": { "size_pt": 8.0, "stroke_width_pt": 0.4 },
            "line": { "stroke_width_pt": 0.5 }
        }),
    );
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g-xy",
            "spec": {
                "canvas_size_pt": { "width": 864.0, "height": 648.0 }
            }
        }),
    );
    ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-col",
            "result_sheet_id": "res-anova",
            "significant_only": false,
            "font_size_pt": 10.0,
            "stroke_width_pt": 0.5
        }),
    );
    ok_cmd(
        &mut session,
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-col",
            "annotation": GraphAnnotation::Shape {
                kind: curvebook_doc::AnnotationShapeKind::Rectangle,
                bounds_pt: RectPt::new(100.0, 100.0, 200.0, 100.0),
                stroke_color: Rgba8::BLACK,
                stroke_width_pt: 0.4,
                dash: curvebook_palette::LineDashStyle::Solid,
                fill_color: None
            }
        }),
    );

    // 1) Graph KeepTypographyPt: preserves font & marker sizes, clamps strokes >= 0.25pt, scales geometry.
    let single_pt = FigureCanvasPreset::SingleColumn88mm.dimensions_pt();
    let keep_out = ok_cmd(
        &mut session,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-col", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }),
    );
    assert!((keep_out.payload["width_pt"].as_f64().unwrap() - single_pt.width).abs() < 1e-3);
    assert!((keep_out.payload["height_pt"].as_f64().unwrap() - single_pt.height).abs() < 1e-3);
    {
        let g = session.document().graph_sheet(&SheetId::new("g-col")).unwrap();
        assert_eq!(g.spec.comparisons[0].font_size_pt, 10.0);
        assert!(g.spec.x_axis.stroke_width_pt >= 0.25);
        assert!(g.spec.comparisons[0].stroke_width_pt >= 0.25);
        let legend_pos = g.spec.legend.position_pt.unwrap();
        assert!(legend_pos.0 < single_pt.width);
    }

    // Undo / Redo round-trip restores 864x648 and reapplies SingleColumn88mm cleanly.
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!((session.document().graph_sheet(&SheetId::new("g-col")).unwrap().spec.canvas_size_pt.width - 864.0).abs() < 1e-6);
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert!((session.document().graph_sheet(&SheetId::new("g-col")).unwrap().spec.canvas_size_pt.width - single_pt.width).abs() < 1e-3);

    // 2) Undo back to PosterPanel, then apply ProportionalScale: scales strokes below 0.25pt & bracket fonts below 5pt -> triggers preflight issues.
    ok_cmd(&mut session, "edit.undo", json!({}));
    let prop_out = ok_cmd(
        &mut session,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-col", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }),
    );
    let prop_issues = prop_out.payload["issues"].as_array().unwrap();
    assert!(prop_issues.iter().any(|i| i["code"] == "min_font_size"), "expected min_font_size issue: {prop_issues:?}");
    assert!(prop_issues.iter().any(|i| i["code"] == "hairline_stroke"), "expected hairline_stroke issue: {prop_issues:?}");
    {
        let g = session.document().graph_sheet(&SheetId::new("g-col")).unwrap();
        assert!(g.spec.comparisons[0].font_size_pt < 5.0);
        assert!(g.spec.x_axis.stroke_width_pt < 0.25);
    }

    // Also test KeepTypographyPt keeps marker size intact while ProportionalScale scales marker size.
    ok_cmd(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }));
    if let GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } =
        &session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.series[0].gdo
    {
        assert!((m.size_pt - 8.0).abs() < 1e-6);
    } else {
        panic!("expected ScatterAndLine marker on g-xy");
    }
    ok_cmd(&mut session, "edit.undo", json!({}));
    ok_cmd(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }));
    if let GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } =
        &session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.series[0].gdo
    {
        assert!(m.size_pt < 4.0);
    } else {
        panic!("expected ScatterAndLine marker on g-xy");
    }
}

#[test]
fn qol8_layout_canvas_presets_preflight_and_all_presets() {
    let mut session = seeded_sci_session();
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g-col",
            "spec": { "canvas_size_pt": { "width": 864.0, "height": 648.0 }, "x_axis": { "stroke_width_pt": 0.4 } }
        }),
    );
    ok_cmd(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-col", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }));

    let single_pt = FigureCanvasPreset::SingleColumn88mm.dimensions_pt();
    let mut layout_doc = session.document().clone();
    let big_layout = LayoutSpec {
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
    };
    layout_doc.add_layout_sheet(LayoutSheet::new(SheetId::new("lay-big"), "Big Layout", big_layout).unwrap()).unwrap();
    let mut layout_session = Session::from_document(layout_doc);

    let lay_keep = ok_cmd(
        &mut layout_session,
        "layout.applyCanvasPreset",
        json!({ "sheet_id": "lay-big", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }),
    );
    assert!((lay_keep.payload["width_pt"].as_f64().unwrap() - single_pt.width).abs() < 1e-3);
    if let LayoutItem::TextCallout { font_size_pt, bounds_pt, .. } =
        &layout_session.document().layout_sheet(&SheetId::new("lay-big")).unwrap().spec.items[1]
    {
        assert_eq!(*font_size_pt, 10.0);
        assert!(bounds_pt.x < single_pt.width);
    } else {
        panic!("expected TextCallout item");
    }

    ok_cmd(&mut layout_session, "edit.undo", json!({}));
    assert!((layout_session.document().layout_sheet(&SheetId::new("lay-big")).unwrap().spec.page_size_pt().width - 864.0).abs() < 1e-6);
    ok_cmd(&mut layout_session, "edit.redo", json!({}));
    assert!((layout_session.document().layout_sheet(&SheetId::new("lay-big")).unwrap().spec.page_size_pt().width - single_pt.width).abs() < 1e-3);
    ok_cmd(&mut layout_session, "edit.undo", json!({}));

    let lay_prop = ok_cmd(
        &mut layout_session,
        "layout.applyCanvasPreset",
        json!({ "sheet_id": "lay-big", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }),
    );
    let lay_issues = lay_prop.payload["issues"].as_array().unwrap();
    assert!(lay_issues.iter().any(|i| i["code"] == "min_font_size"), "expected layout min_font_size issue: {lay_issues:?}");

    let undo_before = layout_session.undo_depth();
    let redo_before = layout_session.redo_depth();
    let pf_graph = ok_cmd(&mut layout_session, "graph.preflightCheck", json!({ "sheet_id": "g-col" }));
    assert_eq!(pf_graph.payload["sheet_id"], json!("g-col"));
    assert!(pf_graph.payload["issue_count"].as_u64().unwrap() >= 1);
    assert_eq!(pf_graph.payload["passed"], json!(false));

    let pf_layout = ok_cmd(&mut layout_session, "graph.preflightCheck", json!({ "sheet_id": "lay-big" }));
    assert_eq!(pf_layout.payload["sheet_id"], json!("lay-big"));
    assert!(!pf_layout.payload["passed"].as_bool().unwrap());
    assert_eq!(layout_session.undo_depth(), undo_before);
    assert_eq!(layout_session.redo_depth(), redo_before);

    // Exercise every standardized FigureCanvasPreset on both graph and layout.
    let all_presets = ["SingleColumn88mm", "ColumnAndHalf130mm", "DoubleColumn180mm", "FullPagePortrait", "Slide16x9", "PosterPanel"];
    for preset_name in all_presets {
        let expected_pt = FigureCanvasPreset::from_name(preset_name).unwrap().dimensions_pt();
        let g_out = ok_cmd(
            &mut layout_session,
            "graph.applyCanvasPreset",
            json!({ "sheet_id": "g-xy", "preset": preset_name, "resize_mode": "KeepTypographyPt" }),
        );
        assert!((g_out.payload["width_pt"].as_f64().unwrap() - expected_pt.width).abs() < 1e-3);
        assert!((g_out.payload["height_pt"].as_f64().unwrap() - expected_pt.height).abs() < 1e-3);

        let l_out = ok_cmd(
            &mut layout_session,
            "layout.applyCanvasPreset",
            json!({ "sheet_id": "lay-big", "preset": preset_name, "resize_mode": "KeepTypographyPt" }),
        );
        assert!((l_out.payload["width_pt"].as_f64().unwrap() - expected_pt.width).abs() < 1e-3);
        assert!((l_out.payload["height_pt"].as_f64().unwrap() - expected_pt.height).abs() < 1e-3);
    }
}
