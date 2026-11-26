//! Integration tests verifying R1 budget caps and panic guards across doc, chart, pdf, and engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartError, ChartScene, compile_layout, multivariate_layout::MAX_LAYOUT_PRIMITIVES};
use curvebook_doc::{
    ColumnDesignHint, ColumnTable, DataSheet, DocError, EmbeddedTableStyle, GraphSheet, GroupedTable, LayoutItem, LayoutSheet, LayoutSpec,
    MAX_LAYOUT_ITEMS, MAX_PROJECT_CELLS, MAX_SHEET_CELLS, MultipleVariablesTable, NestedTable, ProjectDocument, RectPt, SheetId, SubcolumnMode,
    TableArchetype, XColumnMode, XyTable,
};
use curvebook_engine::{ErrorCode, Session, cmd, guard};
use curvebook_pdf::{MAX_PDF_PAGES, MAX_PDF_TOTAL_PRIMITIVES, PdfError, PdfExportOptions, render_pdf_pages};
use serde_json::json;

use super::common::{assert_err_cases, assert_err_code, ok_cmd, xy_sheet};

fn dense_graph_doc(id: &str, rows: usize, y_scale: f64) -> (ProjectDocument, DataSheet, GraphSheet) {
    let xs: Vec<f64> = (0..rows).map(|r| r as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| x * y_scale).collect();
    let ds = xy_sheet(&format!("d-{id}"), "Dense", "X", &xs, &[("Y", &ys)]);
    let mut g = GraphSheet::new_uninitialized_for_data(format!("g-{id}"), &ds).unwrap();
    g.initialized = true;
    let mut doc = ProjectDocument::new("Budget Test").unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    doc.add_graph_sheet(g.clone()).unwrap();
    (doc, ds, g)
}

#[test]
fn s07_05_doc_constructors_and_create_data_reject_oversized_cell_products_without_oom() {
    assert_eq!((MAX_SHEET_CELLS, MAX_PROJECT_CELLS), (30_000_000, 50_000_000));
    assert!(matches!(
        XyTable::new_with_x_mode(XColumnMode::default(), SubcolumnMode::RawReplicates(512), 1_024, 1_000_000),
        Err(DocError::TooManyCells { .. })
    ));
    assert!(matches!(
        XyTable::new_with_x_mode(XColumnMode::default(), SubcolumnMode::RawReplicates(8), 4, 1_000_000),
        Err(DocError::TooManyCells { .. })
    ));
    assert!(matches!(
        ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 100, 500_000),
        Err(DocError::TooManyCells { .. })
    ));
    assert!(matches!(GroupedTable::new(SubcolumnMode::RawReplicates(40), 100, 10_000), Err(DocError::TooManyCells { .. })));
    assert!(matches!(MultipleVariablesTable::new(100, 500_000), Err(DocError::TooManyCells { .. })));
    assert!(matches!(NestedTable::new(50, 100, 10_000), Err(DocError::TooManyCells { .. })));
    assert_eq!(MultipleVariablesTable::new(260, 100_000).unwrap().variables.len(), 260);

    let mut session = Session::new();
    assert_err_cases(
        &mut session,
        &[
            ("sheet.createData", json!({ "kind": "xy", "rows": 1_000_000, "cols": 1_024, "replicates": 512 }), ErrorCode::ResourceLimit),
            ("sheet.createData", json!({ "kind": "xy", "rows": 1_000_000, "cols": 4, "replicates": 8 }), ErrorCode::ResourceLimit),
        ],
    );
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "mv1", "kind": "multiple_variables", "cols": 200, "rows": 100_000 }));
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "mv2", "kind": "multiple_variables", "cols": 200, "rows": 100_000 }));
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "xy_base", "kind": "xy", "rows": 100_000, "cols": 1, "replicates": 2 }));
    assert_err_cases(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "mv3", "kind": "multiple_variables", "cols": 150, "rows": 100_000 }), ErrorCode::ResourceLimit),
            ("sheet.duplicate", json!({ "sheet_id": "mv1" }), ErrorCode::ResourceLimit),
            ("sheet.clonePipeline", json!({ "sheet_id": "xy_base", "override_replicates": 400 }), ErrorCode::ResourceLimit),
        ],
    );
}

#[test]
fn s07_11_pdf_export_caps_page_count_and_cumulative_primitives() {
    let (doc, _, _) = dense_graph_doc("1", 9_000, 0.5);
    let mut session = Session::from_document(doc);
    assert!(matches!(
        render_pdf_pages(&vec![ChartScene::new("s-empty", "Empty", 100.0, 100.0, None); MAX_PDF_PAGES + 1], &PdfExportOptions::default()),
        Err(PdfError::ResourceLimit { .. })
    ));

    let scene = session.render_sheet_scene(&SheetId::new("g-1")).unwrap();
    assert!(scene.primitives.len() >= 9_000 && 64 * scene.primitives.len() > MAX_PDF_TOTAL_PRIMITIVES);
    assert_err_cases(
        &mut session,
        &[
            ("export.pdf", json!({ "sheet_ids": vec!["g-1"; MAX_PDF_PAGES + 1] }), ErrorCode::ResourceLimit),
            ("export.pdf", json!({ "sheet_ids": vec!["g-1"; MAX_PDF_PAGES] }), ErrorCode::ResourceLimit),
        ],
    );
    assert_eq!(ok_cmd(&mut session, "export.pdf", json!({ "sheet_ids": ["g-1", "g-1"] })).payload["page_count"], json!(2));
}

#[test]
fn x1_01_layout_item_cap_and_primitive_budget_prevent_amplification() {
    let mut spec = LayoutSpec::default();
    let callout = |id: String, md: &str| LayoutItem::TextCallout {
        id,
        markdown: md.into(),
        bounds_pt: RectPt::new(10.0, 10.0, 40.0, 20.0),
        font_size_pt: 10.0,
    };
    for i in 0..MAX_LAYOUT_ITEMS {
        spec.add_item(callout(format!("t-{i}"), "ok")).unwrap();
    }
    let extra = callout("overflow".into(), "no");
    assert_eq!(spec.add_item(extra.clone()).unwrap_err().max, MAX_LAYOUT_ITEMS);
    spec.items.push(extra);
    assert!(matches!(LayoutSheet::new("l-bad", "Too Many", spec), Err(DocError::TooManyLayoutItems { .. })));

    let mut session = Session::new();
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay1", "grid_template": [8, 8] }));
    let over_items: Vec<_> = (0..=MAX_LAYOUT_ITEMS)
        .map(|i| json!({ "LiveGraph": { "id": format!("p-{i}"), "graph_sheet_id": "missing", "bounds_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 50.0 }, "scale_percent": 100.0, "badge_override": null } }))
        .collect();
    assert_err_code(&mut session, "layout.setSpec", json!({ "sheet_id": "lay1", "spec": { "items": over_items } }), ErrorCode::ResourceLimit);
    for i in 0..MAX_LAYOUT_ITEMS {
        ok_cmd(
            &mut session,
            "layout.addPanel",
            json!({ "sheet_id": "lay1", "kind": "text", "markdown": format!("P{i}"), "bounds_pt": [10.0, 10.0, 40.0, 20.0] }),
        );
    }
    assert_err_code(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay1", "kind": "text", "markdown": "overflow", "bounds_pt": [10.0, 10.0, 40.0, 20.0] }),
        ErrorCode::ResourceLimit,
    );

    let (mut doc, _, _) = dense_graph_doc("dense", 9_000, 1.0);
    let mut amp_spec = LayoutSpec::default();
    for i in 0..MAX_LAYOUT_ITEMS {
        amp_spec
            .add_item(LayoutItem::LiveGraph {
                id: format!("lg-{i}"),
                graph_sheet_id: SheetId::new("g-dense"),
                bounds_pt: RectPt::new(10.0, 10.0, 120.0, 90.0),
                scale_percent: 100.0,
                badge_override: None,
            })
            .unwrap();
    }
    let amp_layout = LayoutSheet::new("lay-amp", "Amplified", amp_spec).unwrap();
    doc.add_layout_sheet(amp_layout.clone()).unwrap();
    assert!(matches!(compile_layout(&amp_layout, &doc), Err(ChartError::ResourceLimit { .. })));
    assert_eq!(Session::from_document(doc).render_sheet_scene(&SheetId::new("lay-amp")).unwrap_err().code, ErrorCode::ResourceLimit);

    let mut tbl_spec = LayoutSpec::default();
    for i in 0..30 {
        tbl_spec
            .add_item(LayoutItem::LiveTable {
                id: format!("lt-{i}"),
                source_sheet_id: SheetId::new("d-tbl"),
                tab_id: None,
                bounds_pt: RectPt::new(10.0, 10.0, 300.0, 350.0),
                selected_rows: None,
                selected_cols: None,
                style: EmbeddedTableStyle::default(),
            })
            .unwrap();
    }
    let tbl_layout = LayoutSheet::new("lay-tbl", "Big Table", tbl_spec).unwrap();
    let mut tbl_doc = ProjectDocument::new("LiveTable Amp").unwrap();
    tbl_doc
        .add_data_sheet(
            DataSheet::new("d-tbl", "Wide Table", TableArchetype::MultipleVariables(MultipleVariablesTable::new(1_000, 20).unwrap())).unwrap(),
        )
        .unwrap();
    tbl_doc.add_layout_sheet(tbl_layout.clone()).unwrap();
    assert!(matches!(compile_layout(&tbl_layout, &tbl_doc), Err(ChartError::ResourceLimit { .. })));

    let (mut sm_doc, _, _) = dense_graph_doc("sm", 5, 2.0);
    let mut ok_spec = LayoutSpec::default();
    for i in 0..4 {
        ok_spec
            .add_item(LayoutItem::LiveGraph {
                id: format!("ok-{i}"),
                graph_sheet_id: SheetId::new("g-sm"),
                bounds_pt: RectPt::new(10.0 + (i as f64) * 100.0, 10.0, 90.0, 90.0),
                scale_percent: 100.0,
                badge_override: None,
            })
            .unwrap();
    }
    let ok_layout = LayoutSheet::new("lay-ok", "Small Layout", ok_spec).unwrap();
    sm_doc.add_layout_sheet(ok_layout.clone()).unwrap();
    assert!(compile_layout(&ok_layout, &sm_doc).unwrap().primitives.len() <= MAX_LAYOUT_PRIMITIVES);
}

#[test]
fn s12_04_panic_guard_converts_unwind_to_internal_panic() {
    let err = guard::run_guarded::<(), _>("render_sheet_scene.test", || panic!("simulated chart panic")).unwrap_err();
    assert!(err.code == ErrorCode::InternalPanic && err.message().contains("simulated chart panic"));
    assert_eq!(cmd::helpers::compile_sheet_scene(Session::new().document(), &SheetId::new("missing")).unwrap_err().code, ErrorCode::NotFound);
}
