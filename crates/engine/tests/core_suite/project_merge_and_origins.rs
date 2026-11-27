//! Integration tests for `project.merge`, `sheet.clearOrigin`, and `SheetOrigin` provenance
//! across `.curvebook`, `.curvebook.d/`, `.cbtemplate`, `.pzfx`, `.prism`, and `.xlsx` (§10.1, §10.3, §10.6).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::fs;

use curvebook_doc::{
    AnalysisSpec, CURRENT_SCHEMA_VERSION, Cell, ColumnDesignHint, ColumnTable, CropInsetsPt, DataSheet, EmbeddedTableStyle, ErrorCode, FloatingNote,
    GraphSheet, GraphSpec, ImageMimeKind, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, MAX_PROJECT_IMAGE_ASSETS, MAX_SHEETS_PER_PROJECT, NoteId,
    ProjectDocument, ProjectImageAsset, RectPt, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SheetOrigin, SubcolumnMode,
    TableArchetype, XyTable, content_addressed_asset_id,
};
use curvebook_engine::Session;
use curvebook_format::{
    ZipWriter, save_project_bytes, save_project_file, save_project_unpacked_dir, save_template_file, strip_project_data_for_template,
};
use curvebook_palette::Rgba8;
use serde_json::json;

use super::common::{TempDir, encode_base64};

const PNG_1X1_A: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

const PNG_1X1_B: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn make_imported_result(id: &str, source: &str, kind_label: &str, stale: bool) -> ResultSheet {
    let spec = AnalysisSpec::new_imported_snapshot(id, Some(SheetId::new(source)), kind_label).unwrap();
    let mut rs = ResultSheet::new_imported(spec, "external_archive", kind_label, "Imported snapshot").unwrap();
    if stale {
        rs.status = ResultStatus::ImportedStale;
    }
    rs.tabs.push(ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Value")],
        row_labels: vec![String::from("Mean")],
        rows: vec![vec![ResultCell::Number { value: 12.5, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    rs
}

fn build_rich_project(title: &str, first_y: f64, png_bytes: &[u8], asset_id: &str, include_imported_results: bool) -> ProjectDocument {
    let mut doc = ProjectDocument::new(title).unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
    xy.data_sets[0].cells = vec![vec![Cell::number(first_y)], vec![Cell::number(first_y * 2.0)]];
    let mut ds = DataSheet::new("d1", format!("{title} Table"), TableArchetype::Xy(xy)).unwrap();
    ds.info_sheet_id = Some(SheetId::new("i1"));
    ds.floating_notes.push(FloatingNote {
        id: NoteId::new("n1"),
        fill_color: Rgba8::rgba(255, 250, 200, 255),
        bounds_pt: RectPt::new(0.0, 0.0, 50.0, 30.0),
        minimized: false,
        body_markdown: String::from("Lot {{info:i1:Lot}}"),
    });
    doc.add_data_sheet(ds).unwrap();

    let mut info = InfoSheet::new_preseeded("i1", format!("{title} Info")).unwrap();
    info.linked_data_sheets.push(SheetId::new("d1"));
    info.notes_markdown = String::from("Ref {{info:i1:Lot}}");
    doc.add_info_sheet(info).unwrap();

    if include_imported_results {
        doc.add_result_sheet(make_imported_result("r1", "d1", "Imported Fit Fresh", false)).unwrap();
        doc.add_result_sheet(make_imported_result("r2", "d1", "Imported Fit Stale", true)).unwrap();
    }

    let asset = ProjectImageAsset { asset_id: asset_id.to_owned(), media_type: ImageMimeKind::Png, pixel_size: (1, 1), bytes: png_bytes.to_vec() };
    doc.upsert_image_asset(asset).unwrap();

    let mut gspec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
    gspec.series[0].legend_label = Some(String::from("Series {{info:i1:Lot}}"));
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g1"),
        title: format!("{title} Graph"),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    })
    .unwrap();

    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: SheetId::new("g1"),
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 90.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    lspec.items.push(LayoutItem::LiveTable {
        id: String::from("p2"),
        source_sheet_id: SheetId::new("d1"),
        tab_id: None,
        bounds_pt: RectPt::new(140.0, 10.0, 120.0, 90.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    });
    lspec.items.push(LayoutItem::EmbeddedImage {
        id: String::from("p3"),
        asset_id: asset_id.to_owned(),
        bounds_pt: RectPt::new(10.0, 110.0, 40.0, 40.0),
        rotation_deg: 0.0,
        lock_aspect_ratio: true,
        crop: CropInsetsPt::default(),
        border: None,
        badge_override: None,
    });
    doc.add_layout_sheet(LayoutSheet {
        id: SheetId::new("l1"),
        title: format!("{title} Layout"),
        spec: lspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    })
    .unwrap();

    doc
}

fn assert_merged_references(doc: &ProjectDocument) {
    assert_eq!(doc.data_sheets.len(), 2);
    assert_eq!(doc.info_sheets.len(), 2);
    assert_eq!(doc.result_sheets.len(), 2);
    assert_eq!(doc.graph_sheets.len(), 2);
    assert_eq!(doc.layout_sheets.len(), 2);
    assert_eq!(doc.image_assets.len(), 2);

    let merged_ds = &doc.data_sheets[1];
    let merged_info = &doc.info_sheets[1];
    let merged_graph = &doc.graph_sheets[1];
    let merged_layout = &doc.layout_sheets[1];
    assert_ne!(merged_ds.id.as_str(), "d1");
    assert_ne!(merged_info.id.as_str(), "i1");
    assert_ne!(merged_graph.id.as_str(), "g1");
    assert_ne!(merged_layout.id.as_str(), "l1");
    assert_eq!(merged_ds.title, "Source Table", "titles must never be prefixed or altered");
    assert_eq!(merged_ds.info_sheet_id.as_ref(), Some(&merged_info.id));
    assert_eq!(merged_info.linked_data_sheets, vec![merged_ds.id.clone()]);
    assert_eq!(merged_graph.spec.primary_source_sheet_id, merged_ds.id);
    assert!(merged_ds.floating_notes[0].body_markdown.contains(&format!("{{{{info:{}:Lot}}}}", merged_info.id)));
    assert!(merged_graph.spec.series[0].legend_label.as_deref().unwrap().contains(&format!("{{{{info:{}:Lot}}}}", merged_info.id)));
    match &merged_layout.spec.items[0] {
        LayoutItem::LiveGraph { graph_sheet_id, .. } => assert_eq!(graph_sheet_id, &merged_graph.id),
        other => panic!("expected LiveGraph, got {other:?}"),
    }
    match &merged_layout.spec.items[1] {
        LayoutItem::LiveTable { source_sheet_id, .. } => assert_eq!(source_sheet_id, &merged_ds.id),
        other => panic!("expected LiveTable, got {other:?}"),
    }
}

fn assert_merged_origins_and_results(doc: &ProjectDocument, expected_origin: &SheetOrigin) {
    let merged_ds = &doc.data_sheets[1];
    assert_eq!(doc.result_sheets[0].status, ResultStatus::Imported);
    assert_eq!(doc.result_sheets[1].status, ResultStatus::ImportedStale);
    assert_eq!(doc.result_sheets[0].analysis.source_sheet_id.as_ref(), Some(&merged_ds.id));
    assert_eq!(doc.result_sheets[1].analysis.source_sheet_id.as_ref(), Some(&merged_ds.id));

    assert_eq!(doc.data_sheets[0].origin, None);
    assert_eq!(doc.info_sheets[0].origin, None);
    assert_eq!(doc.graph_sheets[0].origin, None);
    assert_eq!(doc.layout_sheets[0].origin, None);
    assert_eq!(merged_ds.origin.as_ref(), Some(expected_origin));
    assert_eq!(doc.info_sheets[1].origin.as_ref(), Some(expected_origin));
    assert_eq!(doc.result_sheets[0].origin.as_ref(), Some(expected_origin));
    assert_eq!(doc.result_sheets[1].origin.as_ref(), Some(expected_origin));
    assert_eq!(doc.graph_sheets[1].origin.as_ref(), Some(expected_origin));
    assert_eq!(doc.layout_sheets[1].origin.as_ref(), Some(expected_origin));
    assert!(!expected_origin.source_label.contains('/') && !expected_origin.source_label.contains('\\'));
}

#[test]
fn project_merge_curvebook_colliding_ids_assets_imported_results_and_exact_undo() {
    let dir = TempDir::try_new("cb-proj-merge-rich").unwrap();
    let asset_a = content_addressed_asset_id(PNG_1X1_A);
    let asset_b = content_addressed_asset_id(PNG_1X1_B);

    let mut target_session = Session::from_document(build_rich_project("Target", 10.0, PNG_1X1_A, &asset_a, false));
    let target_file = dir.path().join("target_study.curvebook");
    target_session.set_file_path(Some(target_file.clone()), true);
    target_session.execute("project.createCheckpoint", json!({ "name": "Target CP", "timestamp_utc": "unix:1700000001" })).unwrap();
    target_session.execute("sheet.select", json!({ "sheet_id": "d1", "mode": "navigator" })).unwrap();
    let before_doc = target_session.document().clone();

    let mut source_session = Session::from_document(build_rich_project("Source", 50.0, PNG_1X1_B, &asset_b, true));
    source_session.execute("project.createCheckpoint", json!({ "name": "Source CP", "timestamp_utc": "unix:1700000002" })).unwrap();
    source_session.execute("sheet.select", json!({ "sheet_id": "g1", "mode": "navigator" })).unwrap();
    let source_path = dir.path().join("nested").join("source_study.curvebook");
    fs::create_dir_all(source_path.parent().unwrap()).unwrap();
    save_project_file(&source_path, source_session.document()).unwrap();

    let outcome = target_session.execute("project.merge", json!({ "path": source_path.to_string_lossy(), "merged_at": "unix:1700000123" })).unwrap();
    assert!(outcome.mutated);
    assert_eq!(outcome.payload["source_label"], json!("source_study.curvebook"));
    assert_eq!(outcome.payload["source_format"], json!("curvebook"));
    assert_eq!(outcome.payload["merged_at"], json!("unix:1700000123"));
    assert_eq!(outcome.payload["added_sheets"], json!(6));

    let doc = target_session.document();
    assert_eq!(doc.title, "Target");
    assert_eq!(target_session.file_path(), Some(target_file.as_path()));
    assert_eq!(doc.checkpoints.len(), 1);
    assert_eq!(doc.checkpoints[0].name, "Target CP");
    assert_eq!(doc.active_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));
    assert_eq!(doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

    let expected_origin = SheetOrigin::new("source_study.curvebook", "curvebook", "unix:1700000123").unwrap();
    assert_merged_references(doc);
    assert_merged_origins_and_results(doc, &expected_origin);

    assert!(target_session.undo().unwrap());
    assert_eq!(target_session.document(), &before_doc);
    assert_eq!(target_session.file_path(), Some(target_file.as_path()));

    assert!(target_session.redo().unwrap());
    assert_eq!(target_session.document().total_sheet_count(), before_doc.total_sheet_count() + 6);
}

#[test]
fn project_merge_unpacked_dir_template_inline_and_future_version_refusal() {
    let dir = TempDir::try_new("cb-proj-merge-variants").unwrap();
    let mut base = Session::from_document(ProjectDocument::new("Base").unwrap());
    base.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "Local", "kind": "column", "cols": 1, "rows": 2 })).unwrap();

    // 1. Unpacked .curvebook.d/ directory
    let mut unpacked_doc = ProjectDocument::new("Unpacked Source").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    col.data_sets[0].cells = vec![vec![Cell::number(4.0)], vec![Cell::number(5.0)]];
    unpacked_doc.add_data_sheet(DataSheet::new("d1", "Unpacked Table", TableArchetype::Column(col)).unwrap()).unwrap();
    let unpacked_dir = dir.path().join("assay_bundle.curvebook.d");
    save_project_unpacked_dir(&unpacked_doc, &unpacked_dir).unwrap();

    let unpacked_path_with_slash = format!("{}/", unpacked_dir.display());
    base.execute("project.merge", json!({ "path": unpacked_path_with_slash, "merged_at": "unix:1700000200" })).unwrap();
    let unpacked_merged = base.document().data_sheets.last().unwrap();
    assert_eq!(unpacked_merged.title, "Unpacked Table");
    assert_eq!(unpacked_merged.origin, Some(SheetOrigin::new("assay_bundle.curvebook.d", "curvebook.d", "unix:1700000200").unwrap()));

    // 2. .cbtemplate archive (strips raw data values and stamps source_format = "cbtemplate")
    let tpl_path = dir.path().join("assay_template.cbtemplate");
    save_template_file(&tpl_path, &unpacked_doc).unwrap();
    base.execute("project.merge", json!({ "path": tpl_path.to_string_lossy(), "merged_at": "unix:1700000300" })).unwrap();
    let tpl_merged = base.document().data_sheets.last().unwrap();
    assert_eq!(tpl_merged.origin, Some(SheetOrigin::new("assay_template.cbtemplate", "cbtemplate", "unix:1700000300").unwrap()));
    match &tpl_merged.archetype {
        TableArchetype::Column(c) => assert!(c.data_sets[0].cells.iter().all(|row| row[0].active_number().is_none())),
        other => panic!("expected Column, got {other:?}"),
    }

    // 3. Inline bytes_base64 uses clean fallback basename and rejects separator-bearing source_label
    let inline_b64 = encode_base64(&save_project_bytes(&unpacked_doc).unwrap());
    let bad_label_err =
        base.execute("project.merge", json!({ "bytes_base64": inline_b64, "source_label": "/secret/path/study.curvebook" })).unwrap_err();
    assert_eq!(bad_label_err.code, ErrorCode::BadInput);

    base.execute("project.merge", json!({ "bytes_base64": inline_b64, "merged_at": "unix:1700000400" })).unwrap();
    let inline_merged = base.document().data_sheets.last().unwrap();
    assert_eq!(inline_merged.origin, Some(SheetOrigin::new("inline.curvebook", "curvebook", "unix:1700000400").unwrap()));

    // 4. Future schema_version is refused with UnsupportedVersion without mutating doc
    let future_ver = u64::from(CURRENT_SCHEMA_VERSION) + 1;
    let mut zw = ZipWriter::new();
    let doc_json = format!(r#"{{"schema_version":{future_ver},"title":"Future"}}"#).into_bytes();
    let manifest_json =
        format!(r#"{{"format":"curvebook-archive","schema_version":{future_ver},"kind":"Project","title":"Future","document_crc32":0,"sheets":[]}}"#)
            .into_bytes();
    zw.add_file("manifest.json", manifest_json).unwrap();
    zw.add_file("document.json", doc_json).unwrap();
    let future_b64 = encode_base64(&zw.finish().unwrap());
    let before = base.document().clone();
    let future_err = base.execute("project.merge", json!({ "bytes_base64": future_b64 })).unwrap_err();
    assert_eq!(future_err.code, ErrorCode::UnsupportedVersion);
    assert_eq!(base.document(), &before);
}

#[test]
fn project_merge_enforces_sheet_cap_asset_cap_and_cycle_rejection() {
    // 1. Sheet cap
    let mut full_doc = ProjectDocument::new("At Cap").unwrap();
    for i in 0..MAX_SHEETS_PER_PROJECT {
        full_doc.add_info_sheet(InfoSheet::new_preseeded(format!("i-{i}"), format!("Info {i}")).unwrap()).unwrap();
    }
    let mut one_sheet_doc = ProjectDocument::new("One More").unwrap();
    one_sheet_doc.add_info_sheet(InfoSheet::new_preseeded("extra", "Extra").unwrap()).unwrap();
    let one_b64 = encode_base64(&save_project_bytes(&one_sheet_doc).unwrap());

    let mut cap_session = Session::from_document(full_doc);
    let cap_before = cap_session.document().clone();
    let cap_err = cap_session.execute("project.merge", json!({ "bytes_base64": one_b64 })).unwrap_err();
    assert_eq!(cap_err.code, ErrorCode::ResourceLimit);
    assert_eq!(cap_session.document(), &cap_before);

    // 2. Image asset cap
    let mut asset_full_doc = ProjectDocument::new("Asset Cap").unwrap();
    for i in 0..MAX_PROJECT_IMAGE_ASSETS {
        let id = format!("img-{i:016x}");
        asset_full_doc
            .upsert_image_asset(ProjectImageAsset { asset_id: id, media_type: ImageMimeKind::Png, pixel_size: (1, 1), bytes: PNG_1X1_A.to_vec() })
            .unwrap();
    }
    let mut incoming_asset_doc = ProjectDocument::new("Extra Asset").unwrap();
    incoming_asset_doc
        .upsert_image_asset(ProjectImageAsset {
            asset_id: content_addressed_asset_id(PNG_1X1_B),
            media_type: ImageMimeKind::Png,
            pixel_size: (1, 1),
            bytes: PNG_1X1_B.to_vec(),
        })
        .unwrap();
    let incoming_asset_b64 = encode_base64(&save_project_bytes(&incoming_asset_doc).unwrap());
    let mut asset_session = Session::from_document(asset_full_doc);
    let asset_before = asset_session.document().clone();
    let asset_err = asset_session.execute("project.merge", json!({ "bytes_base64": incoming_asset_b64 })).unwrap_err();
    assert_eq!(asset_err.code, ErrorCode::ResourceLimit);
    assert_eq!(asset_session.document(), &asset_before);

    // 3. Cycle rejection (cyclic result sheet references in archive)
    let mut cyclic_doc = ProjectDocument::new("Cyclic").unwrap();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    cyclic_doc.add_data_sheet(DataSheet::new("d1", "Data", TableArchetype::Column(col)).unwrap()).unwrap();
    let r_a = ResultSheet::new(AnalysisSpec::new("ra", "rb", curvebook_doc::AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
    let r_b = ResultSheet::new(AnalysisSpec::new("rb", "ra", curvebook_doc::AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
    cyclic_doc.add_result_sheet(r_a).unwrap();
    cyclic_doc.add_result_sheet(r_b).unwrap();
    let doc_bytes = serde_json::to_vec(&cyclic_doc).unwrap();
    let crc = {
        let mut c = 0xFFFF_FFFF_u32;
        for &b in &doc_bytes {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        !c
    };
    let manifest_bytes = serde_json::to_vec(&json!({
        "format": "curvebook-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": "Cyclic",
        "document_crc32": crc,
        "sheets": []
    }))
    .unwrap();
    let mut zw = ZipWriter::new();
    zw.add_file("manifest.json", manifest_bytes).unwrap();
    zw.add_file("document.json", doc_bytes).unwrap();
    let cyclic_b64 = encode_base64(&zw.finish().unwrap());

    let mut clean_session = Session::from_document(ProjectDocument::new("Clean").unwrap());
    let clean_before = clean_session.document().clone();
    let cyc_err = clean_session.execute("project.merge", json!({ "bytes_base64": cyclic_b64 })).unwrap_err();
    assert_eq!(cyc_err.code, ErrorCode::CycleDetected);
    assert_eq!(clean_session.document(), &clean_before);
}

fn assert_clear_origin_commands(session: &mut Session, alpha1_id: &SheetId, alpha2_id: &SheetId, beta_id: &SheetId, gamma_id: &SheetId) {
    let single = session.execute("sheet.clearOrigin", json!({ "sheet_id": alpha1_id.as_str() })).unwrap();
    assert!(single.mutated);
    assert_eq!(single.payload["cleared_count"], json!(1));
    assert!(session.document().data_sheet(alpha1_id).unwrap().origin.is_none());
    assert!(session.document().data_sheet(alpha2_id).unwrap().origin.is_some());

    let noop = session.execute("sheet.clearOrigin", json!({ "sheet_id": alpha1_id.as_str() })).unwrap();
    assert!(!noop.mutated);
    assert_eq!(noop.payload["cleared_count"], json!(0));

    assert!(session.undo().unwrap());
    assert!(session.document().data_sheet(alpha1_id).unwrap().origin.is_some());

    let all_src = session.execute("sheet.clearOrigin", json!({ "sheet_id": alpha1_id.as_str(), "all_from_same_source": true })).unwrap();
    assert!(all_src.mutated);
    assert_eq!(all_src.payload["cleared_count"], json!(2));
    assert!(session.document().data_sheet(alpha1_id).unwrap().origin.is_none());
    assert!(session.document().data_sheet(alpha2_id).unwrap().origin.is_none());
    assert!(session.document().data_sheet(beta_id).unwrap().origin.is_some());
    assert!(session.document().data_sheet(gamma_id).unwrap().origin.is_some());
    assert!(session.undo().unwrap());
    assert!(session.document().data_sheet(alpha1_id).unwrap().origin.is_some());
    assert!(session.document().data_sheet(alpha2_id).unwrap().origin.is_some());

    let by_label = session.execute("sheet.clearOrigin", json!({ "source_label": "bundle_beta.prism" })).unwrap();
    assert!(by_label.mutated);
    assert_eq!(by_label.payload["cleared_count"], json!(1));
    assert!(session.document().data_sheet(beta_id).unwrap().origin.is_none());

    assert_eq!(session.execute("sheet.clearOrigin", json!({})).unwrap_err().code, ErrorCode::BadInput);
    assert_eq!(
        session.execute("sheet.clearOrigin", json!({ "sheet_id": alpha1_id.as_str(), "source_label": "plate_alpha.pzfx" })).unwrap_err().code,
        ErrorCode::BadInput
    );
    assert_eq!(session.execute("sheet.clearOrigin", json!({ "sheet_id": "missing-sheet" })).unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn sheet_origin_set_on_all_merge_paths_absent_on_replace_cleared_by_command_and_template() {
    let dir = TempDir::try_new("cb-origin-lifecycle").unwrap();
    let sub = dir.path().join("private_user_dir");
    fs::create_dir_all(&sub).unwrap();

    let pzfx_path = sub.join("plate_alpha.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile><Table ID="T1" TableType="OneWay"><Title>Alpha</Title><YColumn><Title>A</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></YColumn></Table><Table ID="T2" TableType="OneWay"><Title>Alpha2</Title><YColumn><Title>B</Title><Subcolumn><d>3</d><d>4</d></Subcolumn></YColumn></Table></PrismFile>"#,
    )
    .unwrap();

    let prism_path = sub.join("bundle_beta.prism");
    let mut prism_zip = ZipWriter::new();
    prism_zip.add_file("data/beta.csv", b"G1,G2\n1,2\n3,4\n".to_vec()).unwrap();
    fs::write(&prism_path, prism_zip.finish().unwrap()).unwrap();

    let xlsx_path = sub.join("book_gamma.xlsx");
    let mut xlsx_zip = ZipWriter::new();
    xlsx_zip
        .add_file(
            "xl/workbook.xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Gamma" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_vec(),
        )
        .unwrap();
    xlsx_zip
        .add_file(
            "xl/_rels/workbook.xml.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_vec(),
        )
        .unwrap();
    xlsx_zip
        .add_file(
            "xl/worksheets/sheet1.xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>X</t></is></c><c r="B1" t="inlineStr"><is><t>Y</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>9</v></c></row></sheetData></worksheet>"#.to_vec(),
        )
        .unwrap();
    fs::write(&xlsx_path, xlsx_zip.finish().unwrap()).unwrap();

    let mut replace_sess = Session::new();
    replace_sess.execute("table.importPzfx", json!({ "path": pzfx_path.to_string_lossy() })).unwrap();
    assert!(replace_sess.document().data_sheets.iter().all(|s| s.origin.is_none()));
    replace_sess.execute("table.importPrism", json!({ "path": prism_path.to_string_lossy() })).unwrap();
    assert!(replace_sess.document().data_sheets.iter().all(|s| s.origin.is_none()));
    replace_sess.execute("table.importXlsx", json!({ "path": xlsx_path.to_string_lossy() })).unwrap();
    assert!(replace_sess.document().data_sheets.iter().all(|s| s.origin.is_none()));

    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "base", "title": "Base", "kind": "column", "cols": 1, "rows": 2 })).unwrap();
    session.execute("table.importPzfx", json!({ "path": pzfx_path.to_string_lossy(), "merge": true, "merged_at": "unix:100" })).unwrap();
    session.execute("table.importPrism", json!({ "path": prism_path.to_string_lossy(), "merge": true, "merged_at": "unix:200" })).unwrap();
    session.execute("table.importXlsx", json!({ "path": xlsx_path.to_string_lossy(), "merge": true, "merged_at": "unix:300" })).unwrap();

    assert_eq!(session.document().data_sheets[0].origin, None);
    let alpha1_id = session.document().data_sheets[1].id.clone();
    let alpha2_id = session.document().data_sheets[2].id.clone();
    let beta_id = session.document().data_sheets[3].id.clone();
    let gamma_id = session.document().data_sheets[4].id.clone();
    assert_eq!(session.document().data_sheet(&alpha1_id).unwrap().origin, Some(SheetOrigin::new("plate_alpha.pzfx", "pzfx", "unix:100").unwrap()));
    assert_eq!(session.document().data_sheet(&alpha2_id).unwrap().origin, Some(SheetOrigin::new("plate_alpha.pzfx", "pzfx", "unix:100").unwrap()));
    let archive_format_id = curvebook_format::registry::FormatRegistry::default_registry().importer_by_format("prism").unwrap().descriptor().id;
    assert!(!archive_format_id.starts_with('.'));
    assert_eq!(
        session.document().data_sheet(&beta_id).unwrap().origin,
        Some(SheetOrigin::new("bundle_beta.prism", archive_format_id, "unix:200").unwrap())
    );
    assert_eq!(session.document().data_sheet(&gamma_id).unwrap().origin, Some(SheetOrigin::new("book_gamma.xlsx", "xlsx", "unix:300").unwrap()));

    let stripped = strip_project_data_for_template(session.document()).unwrap();
    assert!(stripped.data_sheets.iter().all(|s| s.origin.is_none()));
    assert!(stripped.info_sheets.iter().all(|s| s.origin.is_none()));
    assert!(stripped.result_sheets.iter().all(|s| s.origin.is_none()));
    assert!(stripped.graph_sheets.iter().all(|s| s.origin.is_none()));
    assert!(stripped.layout_sheets.iter().all(|s| s.origin.is_none()));

    assert_clear_origin_commands(&mut session, &alpha1_id, &alpha2_id, &beta_id, &gamma_id);
}
