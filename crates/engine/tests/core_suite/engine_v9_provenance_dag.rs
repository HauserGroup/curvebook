//! Engine DAG staleness, methods report, selection warning propagation, and consolidated
//! merge renumbering tests for schema v9 (§3.2, §3.4, §10.2).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CURRENT_SCHEMA_VERSION, Cell, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, ErrorCode, FloatingNote,
    GraphSheet, GraphSpec, HookSource, InfoSheet, NormalizeAnchor, NoteId, ProjectDocument, RectPt, ResultCell, ResultScalarEntry, ResultSheet,
    ResultStatus, ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::services::{load_project_from_bytes, load_project_from_path};
use curvebook_format::{save_project_bytes, save_project_unpacked_dir};
use curvebook_palette::Rgba8;
use serde_json::json;

fn ieee_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn make_imported_result(id: &str, source: Option<&str>, kind_label: &str) -> ResultSheet {
    let spec = AnalysisSpec::new_imported_snapshot(id, source.map(SheetId::new), kind_label).unwrap();
    let mut rs = ResultSheet::new_imported(spec, "external_archive", kind_label, "Not natively recomputable").unwrap();
    rs.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Estimate")],
        row_labels: vec![String::from("Slope")],
        rows: vec![vec![ResultCell::Number { value: 2.5, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    rs.scalars.push(ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 2.5, units: None });
    rs
}

#[test]
fn v9_dag_preserves_clean_imported_flips_stale_on_edit_or_delete_and_undo_restores() {
    let mut doc = ProjectDocument::new("Imported DAG Lifecycle").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    col.data_sets[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)], vec![Cell::number(3.0)]];
    doc.add_data_sheet(DataSheet::new("data-1", "Cohort", TableArchetype::Column(col)).unwrap()).unwrap();
    doc.add_result_sheet(make_imported_result("imp-linked", Some("data-1"), "Legacy Regression")).unwrap();
    doc.add_result_sheet(make_imported_result("imp-unlinked", None, "Orphan Summary")).unwrap();

    let mut session = Session::from_document(doc);

    // 1. Clean recalculate_all (dirty_seeds: None) MUST preserve untouched Imported sheets.
    session.recalculate_all().unwrap();
    assert_eq!(session.document().result_sheet(&SheetId::new("imp-linked")).unwrap().status, ResultStatus::Imported);
    assert_eq!(session.document().result_sheet(&SheetId::new("imp-unlinked")).unwrap().status, ResultStatus::Imported);

    // 2. Editing linked upstream source flips linked Imported -> ImportedStale preserving tabs,
    //    while leaving the unlinked imported sheet Imported.
    session.execute("table.setCell", json!({ "sheet_id": "data-1", "row": 0, "col": 0, "subcol": 0, "value": 9.5 })).unwrap();
    let linked_after_edit = session.document().result_sheet(&SheetId::new("imp-linked")).unwrap();
    assert_eq!(linked_after_edit.status, ResultStatus::ImportedStale);
    assert_eq!(linked_after_edit.tabs.len(), 1, "stale imported snapshot must preserve its tabs");
    assert_eq!(session.document().result_sheet(&SheetId::new("imp-unlinked")).unwrap().status, ResultStatus::Imported);

    // 3. Undo restores ResultStatus::Imported cleanly.
    assert!(session.undo().unwrap());
    assert_eq!(session.document().result_sheet(&SheetId::new("imp-linked")).unwrap().status, ResultStatus::Imported);

    // 4. `sheet.delete` without cascade refuses deleting a referenced source; and when a linked source
    //    is absent from the document, DAG recalculation transitions Imported -> ImportedStale preserving
    //    tabs (never Failed), while an unlinked imported snapshot remains Imported.
    let del_err = session.execute("sheet.delete", json!({ "sheet_id": "data-1", "cascade": false })).unwrap_err();
    assert_eq!(del_err.code, ErrorCode::Conflict);

    let mut missing_src_doc = session.document().clone();
    missing_src_doc.data_sheets.retain(|d| d.id.as_str() != "data-1");
    missing_src_doc.clear_dangling_selection_ids();
    let mut missing_src_session = Session::from_document(missing_src_doc);
    missing_src_session.recalculate_all().unwrap();
    let linked_missing = missing_src_session.document().result_sheet(&SheetId::new("imp-linked")).unwrap();
    assert_eq!(linked_missing.status, ResultStatus::ImportedStale);
    assert_eq!(linked_missing.tabs.len(), 1, "missing source must not wipe imported tabs into Failed");
    assert_eq!(missing_src_session.document().result_sheet(&SheetId::new("imp-unlinked")).unwrap().status, ResultStatus::Imported);
}

#[test]
fn v9_dag_blocks_downstream_consumption_and_commands_refuse_imported_mutation() {
    let mut doc = ProjectDocument::new("Imported Isolation").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    col.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(20.0)], vec![Cell::number(30.0)]];
    doc.add_data_sheet(DataSheet::new("data-1", "Group A", TableArchetype::Column(col)).unwrap()).unwrap();
    doc.add_result_sheet(make_imported_result("imp-1", Some("data-1"), "Imported Stats")).unwrap();

    // A computed analysis pointing directly at an imported result must fail cleanly in DAG (never treat as analyzable).
    let downstream_desc =
        ResultSheet::new(AnalysisSpec::new("desc-of-imp", "imp-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap();
    doc.add_result_sheet(downstream_desc).unwrap();

    // A computed transform hooking a scalar from an imported result must fail cleanly (hooks return None for imported).
    let norm_kind = AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::FixedValue(0.0),
        hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("imp-1"),
            parameter_key: String::from("Slope"),
            data_set_index: Some(0),
        }),
        output_percent: true,
        per_data_set: true,
    });
    let hooked_norm = ResultSheet::new(AnalysisSpec::new("norm-hooked", "data-1", norm_kind).unwrap()).unwrap();
    doc.add_result_sheet(hooked_norm).unwrap();

    let mut session = Session::from_document(doc);
    session.recalculate_all().unwrap();
    assert!(matches!(session.document().result_sheet(&SheetId::new("desc-of-imp")).unwrap().status, ResultStatus::Failed { .. }));
    assert!(matches!(session.document().result_sheet(&SheetId::new("norm-hooked")).unwrap().status, ResultStatus::Failed { .. }));

    // analysis.run refuses ImportedSnapshot.
    let run_err = session.execute("analysis.run", json!({ "source_sheet_id": "data-1", "kind": "ImportedSnapshot" })).unwrap_err();
    assert_eq!(run_err.code, ErrorCode::BadInput);

    // analysis.updateParams refuses mutating an imported result.
    let upd_err = session.execute("analysis.updateParams", json!({ "sheet_id": "imp-1", "title": "Edited" })).unwrap_err();
    assert_eq!(upd_err.code, ErrorCode::Conflict);

    // analysis.retargetData refuses retargeting an imported result.
    let retarget_err = session.execute("analysis.retargetData", json!({ "sheet_id": "imp-1", "source_sheet_id": "data-1" })).unwrap_err();
    assert_eq!(retarget_err.code, ErrorCode::Conflict);
}

#[test]
fn v9_methods_report_emits_imported_results_section_and_staleness_notice() {
    let mut doc = ProjectDocument::new("Methods Provenance").unwrap();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    doc.add_data_sheet(DataSheet::new("data-1", "Cohort", TableArchetype::Column(col)).unwrap()).unwrap();

    let fresh = make_imported_result("imp-fresh", Some("data-1"), "Two-way ANOVA");
    let mut stale = make_imported_result("imp-stale", Some("data-1"), "Nonlinear 4PL");
    stale.status = ResultStatus::ImportedStale;
    doc.add_result_sheet(fresh).unwrap();
    doc.add_result_sheet(stale).unwrap();

    let fresh_report = generate_methods_report(&doc, &SheetId::new("imp-fresh"), ReportFormat::Markdown).unwrap();
    assert!(fresh_report.contains("Imported results (not computed by CurveBook)"), "missing imported section:\n{fresh_report}");
    assert!(fresh_report.contains("Two-way ANOVA") && fresh_report.contains("external_archive"), "missing fresh entry:\n{fresh_report}");

    let stale_report = generate_methods_report(&doc, &SheetId::new("imp-stale"), ReportFormat::Markdown).unwrap();
    assert!(stale_report.contains("Imported results (not computed by CurveBook)"), "missing imported section:\n{stale_report}");
    assert!(stale_report.contains("Nonlinear 4PL") && stale_report.to_ascii_lowercase().contains("stale"), "missing stale marker:\n{stale_report}");
}

#[test]
fn v9_services_formats_propagates_dangling_selection_warnings_across_archive_unpacked_and_json() {
    let mut doc = ProjectDocument::new("Dangling Load Warnings").unwrap();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    doc.add_data_sheet(DataSheet::new("d1", "Table", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut raw_val = serde_json::to_value(&doc).unwrap();
    raw_val["active_sheet_id"] = json!("missing-active");
    raw_val["family_anchor_sheet_id"] = json!("missing-anchor");
    let raw_doc_bytes = serde_json::to_vec(&raw_val).unwrap();

    // 1. JSON load via load_project_from_bytes.
    let loaded_json = load_project_from_bytes(std::path::Path::new("study.json"), &raw_doc_bytes, false).unwrap();
    assert_eq!(loaded_json.doc.active_sheet_id, None);
    assert_eq!(loaded_json.doc.family_anchor_sheet_id, None);
    assert_eq!(loaded_json.warnings.len(), 2);

    // 2. Archive ZIP load via load_project_from_bytes.
    let manifest = json!({
        "format": "curvebook-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": doc.title,
        "document_crc32": ieee_crc32(&raw_doc_bytes),
        "sheets": []
    });
    let mut zw = curvebook_format::ZipWriter::new();
    zw.add_file("document.json", raw_doc_bytes.clone()).unwrap();
    zw.add_file("manifest.json", serde_json::to_vec(&manifest).unwrap()).unwrap();
    let zip_bytes = zw.finish().unwrap();

    let loaded_zip = load_project_from_bytes(std::path::Path::new("study.curvebook"), &zip_bytes, false).unwrap();
    assert_eq!(loaded_zip.doc.active_sheet_id, None);
    assert_eq!(loaded_zip.doc.family_anchor_sheet_id, None);
    assert_eq!(loaded_zip.warnings.len(), 2);

    // 3. Unpacked directory load via load_project_from_path.
    let unique = format!("cb-v9-dangling-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().join("target").join("test-tmp-v9").join(unique);
    let unpacked_dir = root.join("study.curvebook.d");
    save_project_unpacked_dir(&doc, &unpacked_dir).unwrap();
    std::fs::write(unpacked_dir.join("document.json"), &raw_doc_bytes).unwrap();
    std::fs::write(unpacked_dir.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let (loaded_dir, _) = load_project_from_path(&unpacked_dir, false).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(loaded_dir.doc.active_sheet_id, None);
    assert_eq!(loaded_dir.doc.family_anchor_sheet_id, None);
    assert_eq!(loaded_dir.warnings.len(), 2);
}

#[test]
fn v9_merge_imported_document_remaps_floating_notes_info_notes_and_legend_hooks() {
    // Build a target project with colliding IDs `d1`, `i1`, `g1`, and merge a second archive
    // containing identical IDs plus floating notes, info notes_markdown, and graph legend_label hooks.
    let make_doc = |title: &str| {
        let mut doc = ProjectDocument::new(title).unwrap();
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
        xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
        xy.data_sets[0].cells = vec![vec![Cell::number(3.0)], vec![Cell::number(6.0)]];
        let mut ds = DataSheet::new("d1", "Dose", TableArchetype::Xy(xy)).unwrap();
        ds.info_sheet_id = Some(SheetId::new("i1"));
        ds.floating_notes.push(FloatingNote {
            id: NoteId::new("n1"),
            fill_color: Rgba8::rgba(255, 250, 200, 255),
            bounds_pt: RectPt::new(0.0, 0.0, 60.0, 40.0),
            minimized: false,
            body_markdown: String::from("Constant {{info:i1:Lot}}"),
        });
        doc.add_data_sheet(ds).unwrap();

        let mut info = InfoSheet::new_preseeded("i1", "Meta").unwrap();
        info.linked_data_sheets.push(SheetId::new("d1"));
        info.notes_markdown = String::from("See {{info:i1:Lot}}");
        doc.add_info_sheet(info).unwrap();

        let mut gspec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
        gspec.series[0].legend_label = Some(String::from("Lot {{info:i1:Lot}}"));
        doc.add_graph_sheet(GraphSheet {
            id: SheetId::new("g1"),
            title: String::from("Plot"),
            initialized: true,
            spec: gspec,
            floating_notes: Vec::new(),
            color_tag: None,
            origin: None,
        })
        .unwrap();
        doc
    };

    let base = make_doc("Base");
    let incoming = make_doc("Incoming");
    let incoming_zip = save_project_bytes(&incoming).unwrap();
    let mut zw = curvebook_format::ZipWriter::new();
    let incoming_archive = curvebook_format::ZipArchive::from_bytes(&incoming_zip).unwrap();
    zw.add_file("document.json", incoming_archive.get("document.json").unwrap().to_vec()).unwrap();
    let container_bytes = zw.finish().unwrap();
    let b64 = {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in container_bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = chunk.get(1).copied().map(u32::from).unwrap_or(0);
            let b2 = chunk.get(2).copied().map(u32::from).unwrap_or(0);
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 { ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
            out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
        }
        out
    };

    let mut session = Session::from_document(base);
    session.execute("table.importPrism", json!({ "bytes_base64": b64, "merge": true })).unwrap();
    assert_eq!(session.document().data_sheets.len(), 2);
    assert_eq!(session.document().info_sheets.len(), 2);
    assert_eq!(session.document().graph_sheets.len(), 2);

    let merged_info = &session.document().info_sheets[1];
    assert_ne!(merged_info.id.as_str(), "i1");
    let expected_hook = format!("{{{{info:{}:Lot}}}}", merged_info.id);
    assert!(merged_info.notes_markdown.contains(&expected_hook), "notes_markdown not remapped: {}", merged_info.notes_markdown);
    assert!(
        session.document().data_sheets[1].floating_notes[0].body_markdown.contains(&expected_hook),
        "floating note not remapped: {}",
        session.document().data_sheets[1].floating_notes[0].body_markdown
    );
    assert_eq!(session.document().graph_sheets[1].spec.series[0].legend_label.as_deref(), Some(format!("Lot {expected_hook}").as_str()));
}

#[test]
fn v9_dag_clean_recalculate_all_preserves_imported_linked_to_up_to_date_computed_result() {
    use curvebook_doc::{ErrorPropagationMode, MathTransformPreset};

    let mut doc = ProjectDocument::new("Imported Downstream Of Computed").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
    xy.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(20.0)]];
    doc.add_data_sheet(DataSheet::new("xy-1", "Raw", TableArchetype::Xy(xy)).unwrap()).unwrap();

    // Computed transform result producing analyzable_data.
    let math_kind = AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform: None,
        y_transform: Some(MathTransformPreset::LinearScale { slope: 2.0, intercept: 0.0 }),
        error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
        pharmacology_preset: None,
        interchange_xy: false,
        zero_x_substitute: None,
        asymmetric_ci_policy: Default::default(),
    });
    let tx_res = ResultSheet::new(AnalysisSpec::new("tx-1", "xy-1", math_kind).unwrap()).unwrap();
    doc.add_result_sheet(tx_res).unwrap();

    let mut session = Session::from_document(doc);
    session.recalculate_all().unwrap();
    assert_eq!(session.document().result_sheet(&SheetId::new("tx-1")).unwrap().status, ResultStatus::UpToDate);

    // Attach an untouched Imported snapshot linked to the already-UpToDate computed transform result.
    let mut doc_with_imp = session.document().clone();
    doc_with_imp.add_result_sheet(make_imported_result("imp-of-tx", Some("tx-1"), "Downstream Snapshot")).unwrap();
    let mut session = Session::from_document(doc_with_imp);

    // Clean recalculate_all() with zero changes MUST preserve Imported status!
    session.recalculate_all().unwrap();
    assert_eq!(
        session.document().result_sheet(&SheetId::new("imp-of-tx")).unwrap().status,
        ResultStatus::Imported,
        "clean recalculate_all must not flip an Imported result linked to an unchanged UpToDate computed result"
    );

    // Editing the upstream raw DataSheet mutates `tx-1` and MUST flip `imp-of-tx` to ImportedStale!
    session.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "subcol": 0, "value": 50.0 })).unwrap();
    assert_eq!(session.document().result_sheet(&SheetId::new("imp-of-tx")).unwrap().status, ResultStatus::ImportedStale);
}

#[test]
fn v9_xy_mean_sd_to_column_conversion_keeps_downstream_graph_column_indices_aligned() {
    let mut doc = ProjectDocument::new("XY MeanSd Convert").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::MeanSd, 2, 2).unwrap();
    xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
    xy.data_sets[0].title = String::from("Ctrl");
    xy.data_sets[1].title = String::from("Drug");
    xy.data_sets[0].cells = vec![vec![Cell::number(10.0), Cell::number(1.0)], vec![Cell::number(12.0), Cell::number(1.2)]];
    xy.data_sets[1].cells = vec![vec![Cell::number(20.0), Cell::number(2.0)], vec![Cell::number(22.0), Cell::number(2.2)]];
    doc.add_data_sheet(DataSheet::new("xy-1", "Dose", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let gspec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
    assert_eq!(gspec.series.len(), 2);
    assert_eq!((gspec.series[0].data_set_index, gspec.series[1].data_set_index), (0, 1));
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-1"),
        title: String::from("Plot"),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    })
    .unwrap();

    let mut session = Session::from_document(doc);
    session.execute("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "column", "allow_lossy": true })).unwrap();

    let converted = session.document().data_sheet(&SheetId::new("xy-1")).unwrap();
    let TableArchetype::Column(col) = &converted.archetype else {
        panic!("expected Column table after conversion");
    };
    assert_eq!(col.subcolumn_mode, SubcolumnMode::MeanSd);
    assert_eq!(col.data_sets.len(), 2, "Single X must be promoted to row titles, not injected as col 0");

    let graph = session.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    assert_eq!(graph.spec.series.len(), 2);
    assert_eq!(
        (graph.spec.series[0].data_set_index, graph.spec.series[1].data_set_index),
        (0, 1),
        "downstream graph series indices must not shift off-by-one"
    );
}
