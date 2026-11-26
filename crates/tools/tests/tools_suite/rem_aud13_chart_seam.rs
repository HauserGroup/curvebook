//! Regression tests for `AUD-13-07` in `curvebook-tools`: `resolve_category_slots` resolving
//! `ResultSheet::analyzable_data` (including multi-hop transform chains) and rejecting failed
//! result sheets cleanly.

use curvebook_chart::{PointPt, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, ErrorPropagationMode, GraphKind, GraphSheet, GraphSpec,
    GroupedArrangement, GroupedTable, ProjectDocument, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec,
};
use curvebook_tools::canvas::{hit_test_category_slot, resolve_category_slots};

#[test]
fn aud_13_07_resolve_category_slots_resolves_analyzable_data_for_column_and_grouped() {
    // Raw 1-column source sheet vs transformed 3-column analyzable_data result sheet.
    let mut raw_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).expect("raw col");
    raw_col.data_sets[0].title = String::from("RawOnly");
    for r in 0..3 {
        raw_col.data_sets[0].cells[r][0] = Cell::number(1.0 + r as f64);
    }
    let raw_ds = DataSheet::new(SheetId::new("raw-col"), "Raw", TableArchetype::Column(raw_col)).expect("raw ds");

    let mut tx_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 3).expect("tx col");
    for (c, title) in ["Ctrl", "Low", "High"].into_iter().enumerate() {
        tx_col.data_sets[c].title = String::from(title);
        for r in 0..3 {
            tx_col.data_sets[c].cells[r][0] = Cell::number(10.0 * (c + 1) as f64 + r as f64);
        }
    }
    let tx_ds = DataSheet::new(SheetId::new("tx-col-data"), "Transformed", TableArchetype::Column(tx_col)).expect("tx ds");

    let tx_spec = AnalysisSpec::new(
        SheetId::new("res-tx-col"),
        raw_ds.id.clone(),
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: None,
            error_propagation: ErrorPropagationMode::default(),
        }),
    )
    .expect("tx spec");
    let mut tx_res = ResultSheet::new(tx_spec).expect("tx res");
    tx_res.status = ResultStatus::UpToDate;
    tx_res.analyzable_data = Some(tx_ds.clone());

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(raw_ds).expect("add raw");
    doc.add_result_sheet(tx_res).expect("add tx res");

    // Create Column graph targeting the transformed ResultSheet.
    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("g-tx-col"), &tx_ds).expect("graph");
    graph.spec.kind = GraphKind::Column;
    graph.spec.primary_source_sheet_id = SheetId::new("res-tx-col");
    for s in &mut graph.spec.series {
        s.source_sheet_id = SheetId::new("res-tx-col");
    }

    let scene = compile_graph(&graph, &doc).expect("compile");
    let slots = resolve_category_slots(&scene, &graph, &doc);
    assert_eq!(slots.len(), 3, "expected 3 category slots from analyzable_data, got {slots:?}");
    let hit = hit_test_category_slot(&slots, &scene, PointPt::new(slots[1].center_pt, scene.plot_rect_pt.y + 12.0));
    assert_eq!(hit, Some(1), "expected hit on second transformed group slot");

    // Failed ResultSheet must yield zero slots (never leak stale/raw upstream data).
    doc.result_sheets[0].status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("failed") };
    let failed_slots = resolve_category_slots(&scene, &graph, &doc);
    assert!(failed_slots.is_empty(), "failed result sheet must yield no slots, got {failed_slots:?}");

    // Grouped table inside analyzable_data also resolves cleanly.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 2).expect("grp");
    grp.row_titles = vec![String::from("Week 1"), String::from("Week 2")];
    grp.data_sets[0].title = String::from("Vehicle");
    grp.data_sets[1].title = String::from("Drug");
    for r in 0..2 {
        grp.data_sets[0].cells[r][0] = Cell::number(5.0 + r as f64);
        grp.data_sets[1].cells[r][0] = Cell::number(12.0 + r as f64);
    }
    let grp_ds = DataSheet::new(SheetId::new("tx-grp-data"), "Transformed Grouped", TableArchetype::Grouped(grp)).expect("grp ds");
    let grp_spec = AnalysisSpec::new(
        SheetId::new("res-tx-grp"),
        SheetId::new("raw-col"),
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: None,
            error_propagation: ErrorPropagationMode::default(),
        }),
    )
    .expect("grp spec");
    let mut grp_res = ResultSheet::new(grp_spec).expect("grp res");
    grp_res.status = ResultStatus::UpToDate;
    grp_res.analyzable_data = Some(grp_ds.clone());
    doc.add_result_sheet(grp_res).expect("add grp res");

    let mut grp_graph = GraphSheet::new_uninitialized_for_data(SheetId::new("g-tx-grp"), &grp_ds).expect("grp graph");
    grp_graph.spec = GraphSpec {
        kind: GraphKind::Grouped,
        grouped_arrangement: GroupedArrangement::Interleaved,
        primary_source_sheet_id: SheetId::new("res-tx-grp"),
        ..grp_graph.spec
    };
    for s in &mut grp_graph.spec.series {
        s.source_sheet_id = SheetId::new("res-tx-grp");
    }
    let grp_scene = compile_graph(&grp_graph, &doc).expect("compile grp");
    let grp_slots = resolve_category_slots(&grp_scene, &grp_graph, &doc);
    assert_eq!(grp_slots.len(), 4, "expected 4 interleaved (2x2) grouped category slots from analyzable_data, got {grp_slots:?}");
}
