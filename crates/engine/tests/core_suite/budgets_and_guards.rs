//! Integration tests verifying R1 budget caps and panic guards across doc, chart, pdf, and engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::AtomicBool;

use curvebook_chart::{ChartError, ChartScene, compile_layout, multivariate_layout::MAX_LAYOUT_PRIMITIVES};
use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, DocError, GraphSheet, GroupedTable, LayoutItem, LayoutSheet, LayoutSpec, MAX_LAYOUT_ITEMS,
    MAX_PROJECT_CELLS, MAX_SHEET_CELLS, MultipleVariablesTable, NestedTable, ProjectDocument, RectPt, SheetId, SubcolumnMode, TableArchetype,
    XColumnMode, XyTable,
};
use curvebook_engine::{Caller, ErrorCode, Session, cmd, guard};
use curvebook_pdf::{MAX_PDF_PAGES, MAX_PDF_TOTAL_PRIMITIVES, PdfError, PdfExportOptions, render_pdf_pages};
use serde_json::json;

fn exec(
    session: &mut Session,
    command_id: &str,
    params: serde_json::Value,
) -> Result<curvebook_engine::CommandOutcome, curvebook_engine::EngineError> {
    let cancel = AtomicBool::new(false);
    session.execute_with_caller(command_id, params, Caller::Cli, &cancel)
}

#[test]
fn s07_05_doc_constructors_and_create_data_reject_oversized_cell_products_without_oom() {
    assert_eq!(MAX_SHEET_CELLS, 30_000_000);
    assert_eq!(MAX_PROJECT_CELLS, 50_000_000);

    // Worst-case S07.md payload (~17 TB if unchecked) and 33M-cell payload fail before allocating.
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

    // Existing G12 26M-cell test remains within MAX_SHEET_CELLS.
    let ok_26m = MultipleVariablesTable::new(260, 100_000).expect("26M cell table must remain permitted");
    assert_eq!(ok_26m.variables.len(), 260);
    drop(ok_26m);

    // Engine `sheet.createData` rejects extreme products immediately with ResourceLimit.
    let mut session = Session::new();
    let extreme = exec(&mut session, "sheet.createData", json!({ "kind": "xy", "rows": 1_000_000, "cols": 1_024, "replicates": 512 }))
        .expect_err("17TB request must be rejected");
    assert_eq!(extreme.code, ErrorCode::ResourceLimit);

    let big_xy = exec(&mut session, "sheet.createData", json!({ "kind": "xy", "rows": 1_000_000, "cols": 4, "replicates": 8 }))
        .expect_err("33M cells must exceed MAX_SHEET_CELLS");
    assert_eq!(big_xy.code, ErrorCode::ResourceLimit);

    // Project cumulative budget MAX_PROJECT_CELLS prevents multiplicative OOM via createData / duplicate / clonePipeline.
    exec(&mut session, "sheet.createData", json!({ "sheet_id": "mv1", "kind": "multiple_variables", "cols": 200, "rows": 100_000 }))
        .expect("20M table 1");
    exec(&mut session, "sheet.createData", json!({ "sheet_id": "mv2", "kind": "multiple_variables", "cols": 200, "rows": 100_000 }))
        .expect("20M table 2 (total 40M <= 50M)");

    let over_proj = exec(&mut session, "sheet.createData", json!({ "sheet_id": "mv3", "kind": "multiple_variables", "cols": 150, "rows": 100_000 }))
        .expect_err("55M cumulative project cells must fail");
    assert_eq!(over_proj.code, ErrorCode::ResourceLimit);

    let dup_over = exec(&mut session, "sheet.duplicate", json!({ "sheet_id": "mv1" })).expect_err("duplicating 20M over 40M must hit 50M cap");
    assert_eq!(dup_over.code, ErrorCode::ResourceLimit);

    exec(&mut session, "sheet.createData", json!({ "sheet_id": "xy_base", "kind": "xy", "rows": 100_000, "cols": 1, "replicates": 2 }))
        .expect("small xy_base (300k)");
    let clone_over = exec(&mut session, "sheet.clonePipeline", json!({ "sheet_id": "xy_base", "override_replicates": 400 }))
        .expect_err("override_replicates expansion past MAX_SHEET_CELLS/MAX_PROJECT_CELLS must fail");
    assert_eq!(clone_over.code, ErrorCode::ResourceLimit);
}

#[test]
fn s07_11_pdf_export_caps_page_count_and_cumulative_primitives() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 9_000).expect("9k xy");
    for r in 0..9_000 {
        xy.x_cells[r][0] = Cell::number(r as f64);
        xy.data_sets[0].cells[r][0] = Cell::number((r as f64) * 0.5);
    }
    let ds = DataSheet::new("d1", "Dense", TableArchetype::Xy(xy)).expect("ds");
    let mut graph = GraphSheet::new_uninitialized_for_data("g1", &ds).expect("graph");
    graph.initialized = true;
    let mut doc = ProjectDocument::new("PDF Cap Test").expect("project");
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_graph_sheet(graph).expect("add graph");
    let mut session = Session::from_document(doc);

    // 1. Direct PDF writer page-count limit (> MAX_PDF_PAGES).
    let empty_pages = vec![ChartScene::new("s-empty", "Empty", 100.0, 100.0, None); MAX_PDF_PAGES + 1];
    assert!(matches!(render_pdf_pages(&empty_pages, &PdfExportOptions::default()), Err(PdfError::ResourceLimit { .. })));

    // 2. Engine export.pdf > MAX_PDF_PAGES rejected before compilation/rendering.
    let too_many_ids: Vec<&str> = vec!["g1"; MAX_PDF_PAGES + 1];
    let err_pages = exec(&mut session, "export.pdf", json!({ "sheet_ids": too_many_ids })).expect_err("65 pages rejected");
    assert_eq!(err_pages.code, ErrorCode::ResourceLimit);

    // 3. Engine export.pdf with 64 pages * >9000 primitives exceeds MAX_PDF_TOTAL_PRIMITIVES (500k).
    let scene = session.render_sheet_scene(&SheetId::new("g1")).expect("compile dense graph");
    assert!(scene.primitives.len() >= 9_000, "expected >= 9000 primitives, got {}", scene.primitives.len());
    assert!(64 * scene.primitives.len() > MAX_PDF_TOTAL_PRIMITIVES);
    let max_page_ids: Vec<&str> = vec!["g1"; MAX_PDF_PAGES];
    let err_prims = exec(&mut session, "export.pdf", json!({ "sheet_ids": max_page_ids })).expect_err("64 dense pages hit primitive cap");
    assert_eq!(err_prims.code, ErrorCode::ResourceLimit);

    // 4. Valid multi-page export succeeds cleanly.
    let ok = exec(&mut session, "export.pdf", json!({ "sheet_ids": ["g1", "g1"] })).expect("2 pages ok");
    assert_eq!(ok.payload["page_count"], json!(2));
}

#[test]
fn x1_01_layout_item_cap_and_primitive_budget_prevent_amplification() {
    // 1. Doc LayoutSpec item limit (MAX_LAYOUT_ITEMS = 64).
    let mut spec = LayoutSpec::default();
    for i in 0..MAX_LAYOUT_ITEMS {
        spec.add_item(LayoutItem::TextCallout {
            id: format!("t-{i}"),
            markdown: String::from("ok"),
            bounds_pt: RectPt::new(10.0, 10.0, 40.0, 20.0),
            font_size_pt: 10.0,
        })
        .expect("up to MAX_LAYOUT_ITEMS ok");
    }
    let extra_item = LayoutItem::TextCallout {
        id: String::from("overflow"),
        markdown: String::from("no"),
        bounds_pt: RectPt::new(10.0, 10.0, 40.0, 20.0),
        font_size_pt: 10.0,
    };
    let over = spec.add_item(extra_item.clone()).expect_err("65th layout item must fail");
    assert_eq!(over.max, MAX_LAYOUT_ITEMS);
    spec.items.push(extra_item);
    assert!(matches!(LayoutSheet::new("l-bad", "Too Many", spec), Err(DocError::TooManyLayoutItems { .. })));

    // 2. Engine layout.setSpec & layout.addPanel enforce MAX_LAYOUT_ITEMS cleanly.
    let mut session = Session::new();
    exec(&mut session, "layout.create", json!({ "sheet_id": "lay1", "grid_template": [8, 8] })).expect("create layout");
    let over_items: Vec<serde_json::Value> = (0..=MAX_LAYOUT_ITEMS)
        .map(|i| {
            json!({
                "LiveGraph": {
                    "id": format!("p-{i}"),
                    "graph_sheet_id": "missing",
                    "bounds_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 50.0 },
                    "scale_percent": 100.0,
                    "badge_override": null
                }
            })
        })
        .collect();
    let set_over = exec(&mut session, "layout.setSpec", json!({ "sheet_id": "lay1", "spec": { "items": over_items } }))
        .expect_err("setSpec > 64 items must fail");
    assert_eq!(set_over.code, ErrorCode::ResourceLimit);

    for i in 0..MAX_LAYOUT_ITEMS {
        exec(
            &mut session,
            "layout.addPanel",
            json!({ "sheet_id": "lay1", "kind": "text", "markdown": format!("P{i}"), "bounds_pt": [10.0, 10.0, 40.0, 20.0] }),
        )
        .expect("add panel within cap");
    }
    let add_65th = exec(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay1", "kind": "text", "markdown": "overflow", "bounds_pt": [10.0, 10.0, 40.0, 20.0] }),
    )
    .expect_err("65th panel rejected");
    assert_eq!(add_65th.code, ErrorCode::ResourceLimit);

    // 3. Chart compile_layout enforces MAX_LAYOUT_PRIMITIVES across duplicated LiveGraph panels.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 9_000).expect("9k xy");
    for r in 0..9_000 {
        xy.x_cells[r][0] = Cell::number(r as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(r as f64);
    }
    let ds = DataSheet::new("d-dense", "Dense", TableArchetype::Xy(xy)).expect("ds");
    let mut graph = GraphSheet::new_uninitialized_for_data("g-dense", &ds).expect("graph");
    graph.initialized = true;
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
            .expect("panel ok");
    }
    let amp_layout = LayoutSheet::new("lay-amp", "Amplified", amp_spec).expect("layout ok");
    let mut doc = ProjectDocument::new("Amp Test").expect("project");
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_graph_sheet(graph).expect("add graph");
    doc.add_layout_sheet(amp_layout.clone()).expect("add layout");

    let chart_err = compile_layout(&amp_layout, &doc).expect_err("64x 9k-point panel layout must hit MAX_LAYOUT_PRIMITIVES");
    assert!(matches!(chart_err, ChartError::ResourceLimit { .. }), "got {chart_err:?}");

    let amp_session = Session::from_document(doc);
    let eng_err = amp_session.render_sheet_scene(&SheetId::new("lay-amp")).expect_err("engine surfaces ResourceLimit");
    assert_eq!(eng_err.code, ErrorCode::ResourceLimit);

    // 4. LiveTable primitive budget rejects amplified multi-panel grids cleanly before rendering primitives.
    let wide_mv = MultipleVariablesTable::new(1_000, 20).expect("20k cells mv");
    let ds_tbl = DataSheet::new("d-tbl", "Wide Table", TableArchetype::MultipleVariables(wide_mv)).expect("ds");
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
                style: curvebook_doc::EmbeddedTableStyle::default(),
            })
            .expect("live table item");
    }
    let tbl_layout = LayoutSheet::new("lay-tbl", "Big Table", tbl_spec).expect("layout");
    let mut tbl_doc = ProjectDocument::new("LiveTable Amp").expect("project");
    tbl_doc.add_data_sheet(ds_tbl).expect("add ds");
    tbl_doc.add_layout_sheet(tbl_layout.clone()).expect("add layout");
    assert!(matches!(compile_layout(&tbl_layout, &tbl_doc), Err(ChartError::ResourceLimit { .. })));

    // 5. Small multi-panel layout compiles cleanly within MAX_LAYOUT_PRIMITIVES (dedup sanity).
    let mut xy_small = XyTable::new(SubcolumnMode::Single, 1, 5).expect("small xy");
    for r in 0..5 {
        xy_small.x_cells[r][0] = Cell::number(r as f64);
        xy_small.data_sets[0].cells[r][0] = Cell::number((r * 2) as f64);
    }
    let ds_small = DataSheet::new("d-sm", "Small", TableArchetype::Xy(xy_small)).expect("ds");
    let mut g_small = GraphSheet::new_uninitialized_for_data("g-sm", &ds_small).expect("graph");
    g_small.initialized = true;
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
            .expect("ok panel");
    }
    let ok_layout = LayoutSheet::new("lay-ok", "Small Layout", ok_spec).expect("layout");
    let mut small_doc = ProjectDocument::new("Small").expect("project");
    small_doc.add_data_sheet(ds_small).expect("ds");
    small_doc.add_graph_sheet(g_small).expect("graph");
    small_doc.add_layout_sheet(ok_layout.clone()).expect("layout");
    let compiled = compile_layout(&ok_layout, &small_doc).expect("small layout must compile cleanly");
    assert!(compiled.primitives.len() <= MAX_LAYOUT_PRIMITIVES);
}

#[test]
fn s12_04_panic_guard_converts_unwind_to_internal_panic() {
    let err = guard::run_guarded::<(), _>("render_sheet_scene.test", || panic!("simulated chart panic")).expect_err("panic must be caught");
    assert_eq!(err.code, ErrorCode::InternalPanic);
    assert!(err.message().contains("simulated chart panic"));

    // Sanity check cmd::helpers::compile_sheet_scene MissingSheet returns NotFound without panicking.
    let session = Session::new();
    let missing = cmd::helpers::compile_sheet_scene(session.document(), &SheetId::new("missing")).expect_err("not found");
    assert_eq!(missing.code, ErrorCode::NotFound);
}
