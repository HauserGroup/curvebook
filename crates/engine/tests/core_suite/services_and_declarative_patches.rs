//! Regression tests for shared `curvebook_engine::services` (artifact export, project/sheet
//! inspection, atomic batch execution with eviction-safe rollback) and declarative serde enum
//! aliases + strict `#[serde(deny_unknown_fields)]` patch structs in `cmd::{analysis,graph}`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::AtomicBool;

use curvebook_doc::{
    CldOrdering, ClusterDistance, ClusterLinkage, CoxTieMethod, ErrorBarMetric, ErrorCode, ErrorPropagationMode, FramePreset, GlmFamily,
    MathTransformPreset, ResultStatus, SheetId, TickDirection, WeightingScheme,
};
use curvebook_engine::services::{
    EmfSheetKind, ExportArtifactFormat, ExportArtifactRequest, ExportTarget, RasterSizingPolicy, export_document_artifact, inspect_project_summary,
    inspect_sheet_detail,
};
use curvebook_engine::{BatchCommandStep, Caller, MAX_UNDO_ENTRIES, Session};
use curvebook_format::CsvOptions;
use curvebook_pdf::PdfExportOptions;
use curvebook_render::MAX_RASTER_DIM_PX;
use curvebook_svg::SvgExportOptions;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::common::{assert_bad_input, ok_cmd, run_cmds};

fn build_demo_session() -> Session {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "xy_1", "title": "Dose", "kind": "xy", "rows": 5, "cols": 1, "replicates": 2, "auto_info": true, "auto_graph": true }),
            ),
            ("table.setXCells", json!({ "sheet_id": "xy_1", "values": [1.0, 2.0, 3.0, 4.0, 5.0] })),
            (
                "table.setCells",
                json!({ "sheet_id": "xy_1", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 2.1 }, { "row": 0, "col": 0, "subcol": 1, "value": 2.3 }, { "row": 1, "col": 0, "subcol": 0, "value": 4.0 }, { "row": 1, "col": 0, "subcol": 1, "value": 4.2 }, { "row": 2, "col": 0, "subcol": 0, "value": 6.1 }, { "row": 2, "col": 0, "subcol": 1, "value": 5.9 }, { "row": 3, "col": 0, "subcol": 0, "value": 8.2 }, { "row": 3, "col": 0, "subcol": 1, "value": 7.9 }, { "row": 4, "col": 0, "subcol": 0, "value": 10.0 }, { "row": 4, "col": 0, "subcol": 1, "value": 10.2 }] }),
            ),
        ],
    );
    session
}

#[test]
fn atomic_batch_rollback_survives_undo_stack_eviction() {
    let mut session = build_demo_session();
    let (baseline_doc, baseline_undo, baseline_redo, baseline_dirty) =
        (session.document().clone(), session.undo_depth(), session.redo_depth(), session.is_dirty());

    let total_mutations = MAX_UNDO_ENTRIES + 12;
    let mut steps: Vec<BatchCommandStep> = (0..total_mutations)
        .map(|i| {
            BatchCommandStep::new(
                "table.setCells",
                json!({ "sheet_id": "xy_1", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 100.0 + i as f64 }] }),
            )
        })
        .collect();
    steps.push(BatchCommandStep::new("table.setCells", json!({ "sheet_id": "missing_sheet", "edits": [{ "row": 0, "col": 0, "value": 1.0 }] })));

    let err = session.execute_batch_atomic(&steps, Caller::Ui, &AtomicBool::new(false)).expect_err("failing batch must roll back");
    assert_eq!(
        (err.failed_index, err.failed_command_id.as_str(), err.error.code, err.completed_before_failure.len()),
        (total_mutations, "table.setCells", ErrorCode::NotFound, total_mutations)
    );
    assert!(err.persisted_filesystem_steps.is_empty());
    assert_eq!(
        (session.document(), session.undo_depth(), session.redo_depth(), session.is_dirty()),
        (&baseline_doc, baseline_undo, baseline_redo, baseline_dirty)
    );
}

#[test]
fn atomic_batch_commit_collapses_undo_readonly_stays_clean_and_cancel_rolls_back() {
    let mut session = Session::from_document(build_demo_session().document().clone());
    let cancel = AtomicBool::new(false);
    let undo_before = session.undo_depth();

    let ro_steps = vec![BatchCommandStep::new("project.inspect", json!({})), BatchCommandStep::new("project.inspect", json!({ "sheet_id": "xy_1" }))];
    assert_eq!(session.execute_batch_atomic(&ro_steps, Caller::Ui, &cancel).unwrap().len(), 2);
    assert!(!session.is_dirty() && session.undo_depth() == undo_before);

    let before_batch_doc = session.document().clone();
    let mut_steps = vec![
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "xy_1", "title": "Renamed 1" })),
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "xy_1", "title": "Renamed 2" })),
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "xy_1", "title": "Renamed 3" })),
    ];
    assert_eq!(session.execute_batch_atomic(&mut_steps, Caller::Ui, &cancel).unwrap().len(), 3);
    assert!(session.is_dirty() && session.undo_depth() == undo_before + 1);
    assert!(session.undo().unwrap() && session.document() == &before_batch_doc);

    let cancel_err = session.execute_batch_atomic(&mut_steps, Caller::Ui, &AtomicBool::new(true)).expect_err("cancelled batch must fail");
    assert_eq!((cancel_err.failed_index, cancel_err.error.code), (0, ErrorCode::Cancelled));
    assert_eq!(session.document(), &before_batch_doc);
}

#[test]
fn export_service_guards_failed_results_caps_rasters_and_exports_all_graphs_pdf() {
    let mut session = build_demo_session();
    run_cmds(
        &mut session,
        &[
            ("analysis.run", json!({ "source_sheet_id": "xy_1", "sheet_id": "res_1", "kind": "linear_regression" })),
            ("graph.create", json!({ "sheet_id": "g_extra", "data_sheet_id": "xy_1", "kind": "xy", "title": "Second Graph" })),
            ("layout.create", json!({ "sheet_id": "lay_1", "title": "Figure 1" })),
        ],
    );
    let graph_id = session.document().graph_sheets[0].id.clone();
    let res_id = SheetId::try_new("res_1").unwrap();

    let pdf_all = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::AutoOrExplicit(None),
            format: ExportArtifactFormat::Pdf { options: PdfExportOptions::default(), all_graphs: true, sheet_ids: None },
        },
        None,
        "test.pdf.all",
    )
    .unwrap();
    assert_eq!((pdf_all.target_id.as_str(), pdf_all.page_count), ("all", 3));
    assert!(pdf_all.bytes.starts_with(b"%PDF-"));

    let svg = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(graph_id.clone()),
            format: ExportArtifactFormat::Svg { options: SvgExportOptions::default() },
        },
        None,
        "test.svg",
    )
    .unwrap();
    assert!(svg.into_utf8_string().unwrap().contains("<svg"));

    let emf = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(graph_id.clone()),
            format: ExportArtifactFormat::Emf { require_kind: Some(EmfSheetKind::Graph) },
        },
        None,
        "test.emf",
    )
    .unwrap();
    assert!(emf.bytes.len() > 80);

    let strict_err = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(graph_id.clone()),
            format: ExportArtifactFormat::Png {
                width_px: None,
                height_px: None,
                dpi: Some(3000),
                sizing: RasterSizingPolicy::StrictLimits,
                transparent_bg: false,
            },
        },
        None,
        "test.png.strict",
    )
    .unwrap_err();
    assert_eq!(strict_err.code, ErrorCode::ResourceLimit);

    let capped = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(graph_id),
            format: ExportArtifactFormat::Png {
                width_px: None,
                height_px: None,
                dpi: Some(3000),
                sizing: RasterSizingPolicy::CapToLimits { default_dpi: 150 },
                transparent_bg: false,
            },
        },
        None,
        "test.png.capped",
    )
    .unwrap();
    let meta = capped.raster.unwrap();
    assert!(capped.bytes.starts_with(b"\x89PNG\r\n\x1a\n") && meta.width_px <= MAX_RASTER_DIM_PX && meta.height_px <= MAX_RASTER_DIM_PX);

    let tidy_err = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(res_id.clone()),
            format: ExportArtifactFormat::Csv { options: CsvOptions::default(), tab_id: Some(String::from("summary")), tab_index: None, tidy: true },
        },
        None,
        "test.csv.tidy_conflict",
    )
    .unwrap_err();
    assert_eq!(tidy_err.code, ErrorCode::BadInput);

    let project_json = export_document_artifact(
        session.document(),
        &ExportArtifactRequest { target: ExportTarget::AutoOrExplicit(None), format: ExportArtifactFormat::Json { pretty: false } },
        None,
        "test.json.project",
    )
    .unwrap();
    assert_eq!(project_json.target_id, "project");
    assert!(serde_json::from_str::<Value>(&project_json.into_utf8_string().unwrap()).unwrap().get("data_sheets").is_some());

    let mut failed_doc = session.document().clone();
    failed_doc.result_sheet_mut(&res_id).unwrap().status =
        ResultStatus::Failed { code: ErrorCode::InternalError, message: String::from("synthetic convergence failure") };
    let failed_csv = export_document_artifact(
        &failed_doc,
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(res_id),
            format: ExportArtifactFormat::Csv { options: CsvOptions::default(), tab_id: None, tab_index: None, tidy: false },
        },
        None,
        "test.csv.failed",
    )
    .unwrap_err();
    assert_eq!(failed_csv.code, ErrorCode::BadInput);
}

#[test]
fn inspect_services_match_project_inspect_and_sheet_detail() {
    let mut session = build_demo_session();
    let cmd_out = ok_cmd(&mut session, "project.inspect", json!({ "sheet_id": "xy_1" }));
    let focus_id = SheetId::try_new("xy_1").unwrap();
    let summary = inspect_project_summary(session.document(), session.is_dirty(), session.file_path(), Some(&focus_id)).unwrap();
    assert_eq!(summary.to_project_inspect_json(), cmd_out.payload);

    let detail = inspect_sheet_detail(session.document(), &focus_id, true).unwrap();
    assert_eq!((&detail["section"], &detail["id"]), (&json!("Data"), &json!("xy_1")));
    assert!(detail["csv_preview"].as_str().is_some_and(|s| s.contains("2.1")));
}

fn check_alias<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(val: T, canonical: &str, alias: &str) {
    assert_eq!(serde_json::to_value(&val).unwrap(), json!(canonical));
    assert_eq!(serde_json::from_value::<T>(json!(alias)).unwrap(), val);
}

#[test]
fn doc_enum_aliases_preserve_canonical_serialization_and_graph_patches_reject_unknown_fields() {
    check_alias(FramePreset::OpenL, "OpenL", "open_l");
    check_alias(TickDirection::Inside, "Inside", "inside");
    check_alias(ErrorBarMetric::Sem, "Sem", "sem");
    check_alias(CldOrdering::AscendingMean, "AscendingMean", "ascending");
    check_alias(GlmFamily::Logistic, "Logistic", "logistic");
    check_alias(ClusterDistance::Euclidean, "Euclidean", "l2");
    check_alias(ClusterLinkage::Upgma, "Upgma", "average");
    check_alias(CoxTieMethod::Efron, "Efron", "efron");
    check_alias(WeightingScheme::RelativeSquare, "RelativeSquare", "1/y2");
    check_alias(ErrorPropagationMode::ReplicateEndpoints, "ReplicateEndpoints", "asymmetric");
    check_alias(MathTransformPreset::Log10, "Log10", "log10");

    let mut session = build_demo_session();
    let graph_id = session.document().graph_sheets[0].id.as_str().to_owned();
    ok_cmd(&mut session, "graph.setAxes", json!({ "sheet_id": &graph_id, "x_axis": { "scale": "log10", "title": "Log Dose" } }));
    let _ = assert_bad_input(&mut session, "graph.setAxes", json!({ "sheet_id": &graph_id, "x_axis": { "nonexistent_axis_key": true } }));
    let _ = assert_bad_input(
        &mut session,
        "graph.setSeries",
        json!({ "sheet_id": &graph_id, "series": [{ "index": 0, "marker": { "shape": "diamond", "bogus_field": 123 } }] }),
    );
}
