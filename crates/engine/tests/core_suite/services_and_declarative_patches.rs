//! Regression tests for shared `curvebook_engine::services` (artifact export, project/sheet
//! inspection, atomic batch execution with eviction-safe rollback) and declarative serde enum
//! aliases + strict `#[serde(deny_unknown_fields)]` patch structs in `cmd::{analysis,graph}`.

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
use serde_json::{Value, json};

fn build_demo_session() -> Result<Session, Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute(
        "sheet.createData",
        json!({
            "sheet_id": "xy_1",
            "title": "Dose",
            "kind": "xy",
            "rows": 5,
            "cols": 1,
            "replicates": 2,
            "auto_info": true,
            "auto_graph": true
        }),
    )?;
    session.execute(
        "table.setXCells",
        json!({
            "sheet_id": "xy_1",
            "edits": [
                { "row": 0, "value": 1.0 },
                { "row": 1, "value": 2.0 },
                { "row": 2, "value": 3.0 },
                { "row": 3, "value": 4.0 },
                { "row": 4, "value": 5.0 }
            ]
        }),
    )?;
    session.execute(
        "table.setCells",
        json!({
            "sheet_id": "xy_1",
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 2.1 },
                { "row": 0, "col": 0, "subcol": 1, "value": 2.3 },
                { "row": 1, "col": 0, "subcol": 0, "value": 4.0 },
                { "row": 1, "col": 0, "subcol": 1, "value": 4.2 },
                { "row": 2, "col": 0, "subcol": 0, "value": 6.1 },
                { "row": 2, "col": 0, "subcol": 1, "value": 5.9 },
                { "row": 3, "col": 0, "subcol": 0, "value": 8.2 },
                { "row": 3, "col": 0, "subcol": 1, "value": 7.9 },
                { "row": 4, "col": 0, "subcol": 0, "value": 10.0 },
                { "row": 4, "col": 0, "subcol": 1, "value": 10.2 }
            ]
        }),
    )?;
    Ok(session)
}

#[test]
fn atomic_batch_rollback_survives_undo_stack_eviction() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = build_demo_session()?;
    let baseline_doc = session.document().clone();
    let baseline_undo = session.undo_depth();
    let baseline_redo = session.redo_depth();
    let baseline_dirty = session.is_dirty();

    let total_mutations = MAX_UNDO_ENTRIES + 12;
    let mut steps = Vec::with_capacity(total_mutations + 1);
    for i in 0..total_mutations {
        steps.push(BatchCommandStep::new(
            "table.setCells",
            json!({
                "sheet_id": "xy_1",
                "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 100.0 + i as f64 }]
            }),
        ));
    }
    // Final failing step forces rollback after MAX_UNDO_ENTRIES undo entries were evicted.
    steps.push(BatchCommandStep::new("table.setCells", json!({ "sheet_id": "missing_sheet", "edits": [{ "row": 0, "col": 0, "value": 1.0 }] })));

    let cancel = AtomicBool::new(false);
    let err = match session.execute_batch_atomic(&steps, Caller::Ui, &cancel) {
        Ok(_) => return Err("expected failing batch to roll back".into()),
        Err(failure) => failure,
    };

    assert_eq!(err.failed_index, total_mutations);
    assert_eq!(err.failed_command_id, "table.setCells");
    assert_eq!(err.error.code, ErrorCode::NotFound);
    assert_eq!(err.completed_before_failure.len(), total_mutations);
    assert!(err.persisted_filesystem_steps.is_empty());

    assert_eq!(session.document(), &baseline_doc);
    assert_eq!(session.undo_depth(), baseline_undo);
    assert_eq!(session.redo_depth(), baseline_redo);
    assert_eq!(session.is_dirty(), baseline_dirty);
    Ok(())
}

#[test]
fn atomic_batch_commit_collapses_undo_readonly_stays_clean_and_cancel_rolls_back() -> Result<(), Box<dyn std::error::Error>> {
    let demo = build_demo_session()?;
    let mut session = Session::from_document(demo.document().clone());
    let cancel = AtomicBool::new(false);

    // Pure read-only batch preserves clean state and undo depth.
    let undo_before = session.undo_depth();
    let ro_steps = vec![BatchCommandStep::new("project.inspect", json!({})), BatchCommandStep::new("project.inspect", json!({ "sheet_id": "xy_1" }))];
    let ro_outcomes = session.execute_batch_atomic(&ro_steps, Caller::Ui, &cancel).map_err(|f| format!("read-only batch failed: {}", f.error))?;
    assert_eq!(ro_outcomes.len(), 2);
    assert!(!session.is_dirty());
    assert_eq!(session.undo_depth(), undo_before);

    // Multi-step mutating batch collapses into a single undo entry.
    let before_batch_doc = session.document().clone();
    let mut_steps = vec![
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "xy_1", "title": "Renamed 1" })),
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "xy_1", "title": "Renamed 2" })),
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "xy_1", "title": "Renamed 3" })),
    ];
    let mut_outcomes = session.execute_batch_atomic(&mut_steps, Caller::Ui, &cancel).map_err(|f| format!("mutating batch failed: {}", f.error))?;
    assert_eq!(mut_outcomes.len(), 3);
    assert!(session.is_dirty());
    assert_eq!(session.undo_depth(), undo_before + 1);
    assert!(session.undo()?);
    assert_eq!(session.document(), &before_batch_doc);

    // Pre-cancelled batch rolls back cleanly with ErrorCode::Cancelled.
    let cancelled = AtomicBool::new(true);
    let cancel_err = match session.execute_batch_atomic(&mut_steps, Caller::Ui, &cancelled) {
        Ok(_) => return Err("cancelled batch should fail".into()),
        Err(failure) => failure,
    };
    assert_eq!(cancel_err.failed_index, 0);
    assert_eq!(cancel_err.error.code, ErrorCode::Cancelled);
    assert_eq!(session.document(), &before_batch_doc);
    Ok(())
}

#[test]
fn export_service_guards_failed_results_caps_rasters_and_exports_all_graphs_pdf() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = build_demo_session()?;
    session.execute("analysis.run", json!({ "source_sheet_id": "xy_1", "sheet_id": "res_1", "kind": "linear_regression" }))?;
    session.execute("graph.create", json!({ "sheet_id": "g_extra", "data_sheet_id": "xy_1", "kind": "xy", "title": "Second Graph" }))?;
    session.execute("layout.create", json!({ "sheet_id": "lay_1", "title": "Figure 1" }))?;

    let graph_id = session.document().graph_sheets[0].id.clone();
    let res_id = SheetId::try_new("res_1")?;

    // All-graphs multi-page PDF exports every graph and layout in document order.
    let pdf_all = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::AutoOrExplicit(None),
            format: ExportArtifactFormat::Pdf { options: PdfExportOptions::default(), all_graphs: true, sheet_ids: None },
        },
        None,
        "test.pdf.all",
    )?;
    assert_eq!(pdf_all.target_id, "all");
    assert_eq!(pdf_all.page_count, 3);
    assert!(pdf_all.bytes.starts_with(b"%PDF-"));

    // SVG & EMF export valid vector payloads.
    let svg = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(graph_id.clone()),
            format: ExportArtifactFormat::Svg { options: SvgExportOptions::default() },
        },
        None,
        "test.svg",
    )?;
    assert!(svg.into_utf8_string()?.contains("<svg"));

    let emf = export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(graph_id.clone()),
            format: ExportArtifactFormat::Emf { require_kind: Some(EmfSheetKind::Graph) },
        },
        None,
        "test.emf",
    )?;
    assert!(emf.bytes.len() > 80);

    // Raster sizing: StrictLimits rejects oversized DPI while CapToLimits caps proportionally.
    let strict_err = match export_document_artifact(
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
    ) {
        Ok(_) => return Err("oversized strict raster should fail".into()),
        Err(err) => err,
    };
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
    )?;
    assert!(capped.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    let raster_meta = capped.raster.ok_or("missing raster metadata")?;
    assert!(raster_meta.width_px <= MAX_RASTER_DIM_PX);
    assert!(raster_meta.height_px <= MAX_RASTER_DIM_PX);

    // CSV tidy + tab_id conflict rejected cleanly.
    let tidy_conflict = match export_document_artifact(
        session.document(),
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(res_id.clone()),
            format: ExportArtifactFormat::Csv { options: CsvOptions::default(), tab_id: Some(String::from("summary")), tab_index: None, tidy: true },
        },
        None,
        "test.csv.tidy_conflict",
    ) {
        Ok(_) => return Err("tidy + tab_id should fail".into()),
        Err(err) => err,
    };
    assert_eq!(tidy_conflict.code, ErrorCode::BadInput);

    // JSON export supports both single-sheet and whole-project targets.
    let project_json = export_document_artifact(
        session.document(),
        &ExportArtifactRequest { target: ExportTarget::AutoOrExplicit(None), format: ExportArtifactFormat::Json { pretty: false } },
        None,
        "test.json.project",
    )?;
    assert_eq!(project_json.target_id, "project");
    let parsed_proj: Value = serde_json::from_str(&project_json.into_utf8_string()?)?;
    assert!(parsed_proj.get("data_sheets").is_some());

    // Failed result sheet CSV export is rejected uniformly by the guard.
    let mut failed_doc = session.document().clone();
    if let Some(rs) = failed_doc.result_sheet_mut(&res_id) {
        rs.status = ResultStatus::Failed { code: ErrorCode::InternalError, message: String::from("synthetic convergence failure") };
    }
    let failed_csv = match export_document_artifact(
        &failed_doc,
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(res_id),
            format: ExportArtifactFormat::Csv { options: CsvOptions::default(), tab_id: None, tab_index: None, tidy: false },
        },
        None,
        "test.csv.failed",
    ) {
        Ok(_) => return Err("failed result CSV export should be rejected".into()),
        Err(err) => err,
    };
    assert_eq!(failed_csv.code, ErrorCode::BadInput);
    Ok(())
}

#[test]
fn inspect_services_match_project_inspect_and_sheet_detail() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = build_demo_session()?;
    let cmd_out = session.execute("project.inspect", json!({ "sheet_id": "xy_1" }))?;
    let focus_id = SheetId::try_new("xy_1")?;
    let summary = inspect_project_summary(session.document(), session.is_dirty(), session.file_path(), Some(&focus_id))?;
    assert_eq!(summary.to_project_inspect_json(), cmd_out.payload);

    let detail = inspect_sheet_detail(session.document(), &focus_id, true)?;
    assert_eq!(detail.get("section").and_then(Value::as_str), Some("Data"));
    assert_eq!(detail.get("id").and_then(Value::as_str), Some("xy_1"));
    assert!(detail.get("csv_preview").and_then(Value::as_str).is_some_and(|s| s.contains("2.1")));
    Ok(())
}

#[test]
fn doc_enum_aliases_preserve_canonical_serialization_and_graph_patches_reject_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
    // Canonical PascalCase serialization stays untouched while snake/alias spellings deserialize.
    assert_eq!(serde_json::to_value(FramePreset::OpenL)?, json!("OpenL"));
    assert_eq!(serde_json::from_value::<FramePreset>(json!("open_l"))?, FramePreset::OpenL);
    assert_eq!(serde_json::to_value(TickDirection::Inside)?, json!("Inside"));
    assert_eq!(serde_json::from_value::<TickDirection>(json!("inside"))?, TickDirection::Inside);
    assert_eq!(serde_json::to_value(ErrorBarMetric::Sem)?, json!("Sem"));
    assert_eq!(serde_json::from_value::<ErrorBarMetric>(json!("sem"))?, ErrorBarMetric::Sem);
    assert_eq!(serde_json::to_value(CldOrdering::AscendingMean)?, json!("AscendingMean"));
    assert_eq!(serde_json::from_value::<CldOrdering>(json!("ascending"))?, CldOrdering::AscendingMean);
    assert_eq!(serde_json::to_value(GlmFamily::Logistic)?, json!("Logistic"));
    assert_eq!(serde_json::from_value::<GlmFamily>(json!("logistic"))?, GlmFamily::Logistic);
    assert_eq!(serde_json::to_value(ClusterDistance::Euclidean)?, json!("Euclidean"));
    assert_eq!(serde_json::from_value::<ClusterDistance>(json!("l2"))?, ClusterDistance::Euclidean);
    assert_eq!(serde_json::to_value(ClusterLinkage::Upgma)?, json!("Upgma"));
    assert_eq!(serde_json::from_value::<ClusterLinkage>(json!("average"))?, ClusterLinkage::Upgma);
    assert_eq!(serde_json::to_value(CoxTieMethod::Efron)?, json!("Efron"));
    assert_eq!(serde_json::from_value::<CoxTieMethod>(json!("efron"))?, CoxTieMethod::Efron);
    assert_eq!(serde_json::to_value(WeightingScheme::RelativeSquare)?, json!("RelativeSquare"));
    assert_eq!(serde_json::from_value::<WeightingScheme>(json!("1/y2"))?, WeightingScheme::RelativeSquare);
    assert_eq!(serde_json::to_value(ErrorPropagationMode::ReplicateEndpoints)?, json!("ReplicateEndpoints"));
    assert_eq!(serde_json::from_value::<ErrorPropagationMode>(json!("asymmetric"))?, ErrorPropagationMode::ReplicateEndpoints);
    assert_eq!(serde_json::to_value(MathTransformPreset::Log10)?, json!("Log10"));
    assert_eq!(serde_json::from_value::<MathTransformPreset>(json!("log10"))?, MathTransformPreset::Log10);

    // Declarative graph patch structs accept aliases and reject unknown keys with BadInput.
    let mut session = build_demo_session()?;
    let graph_id = session.document().graph_sheets[0].id.as_str().to_owned();

    session.execute(
        "graph.setAxes",
        json!({
            "sheet_id": graph_id,
            "x_axis": { "scale": "log10", "title": "Log Dose" }
        }),
    )?;

    let bad_axis = match session.execute(
        "graph.setAxes",
        json!({
            "sheet_id": graph_id,
            "x_axis": { "nonexistent_axis_key": true }
        }),
    ) {
        Ok(_) => return Err("unknown axis key should fail".into()),
        Err(err) => err,
    };
    assert_eq!(bad_axis.code, ErrorCode::BadInput);

    let bad_series = match session.execute(
        "graph.setSeries",
        json!({
            "sheet_id": graph_id,
            "series": [{ "index": 0, "marker": { "shape": "diamond", "bogus_field": 123 } }]
        }),
    ) {
        Ok(_) => return Err("unknown marker key should fail".into()),
        Err(err) => err,
    };
    assert_eq!(bad_series.code, ErrorCode::BadInput);
    Ok(())
}
