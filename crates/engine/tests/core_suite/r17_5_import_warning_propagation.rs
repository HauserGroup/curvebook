use std::fs;

use curvebook_engine::Session;
use curvebook_format::ZipWriter;
use serde_json::json;

use super::common::TempDir;

#[test]
fn engine_table_import_pzfx_and_prism_propagate_structured_warnings() -> Result<(), String> {
    let xml = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile>\n",
        "  <TableSequence>\n",
        "    <Table ID=\"t1\" TableType=\"XY\" YFormat=\"PlusMinus\">\n",
        "      <XColumn><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>\n",
        "      <YColumn><Subcolumn><d>10</d><d>20</d></Subcolumn><Subcolumn><d>1.2</d><d>2.4</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        "  </TableSequence>\n",
        "  <UserEquations><Equation Name=\"Cust\"/></UserEquations>\n",
        "</PrismFile>",
    );

    let mut pzfx_session = Session::new();
    let pzfx_out = pzfx_session.execute("table.importPzfx", json!({ "xml": xml })).map_err(|e| format!("table.importPzfx failed: {e}"))?;
    assert!(pzfx_out.warnings.iter().any(|w| w.contains("[CoercedValue]") && w.contains("t1")));
    assert!(pzfx_out.warnings.iter().any(|w| w.contains("[UnsupportedEquation]") && w.contains("UserEquations")));
    let payload_warnings = pzfx_out.payload["warnings"].as_array().ok_or("expected structured payload warnings")?;
    assert_eq!(payload_warnings.len(), pzfx_out.warnings.len());

    let mut zw = ZipWriter::new();
    zw.add_file("manifest.json", br#"{"title":"Legacy Bundle","analyses":[{"id":"a1"}]}"#.to_vec()).map_err(|e| format!("zip manifest: {e}"))?;
    zw.add_file("sheets/t1.csv", b"X,Y\n1,5\n2,8\n".to_vec()).map_err(|e| format!("zip csv: {e}"))?;
    zw.add_file("graphs/fig1.json", br#"{"title":"Main Plot","axes":[]}"#.to_vec()).map_err(|e| format!("zip graph: {e}"))?;
    let zip_bytes = zw.finish().map_err(|e| format!("zip finish: {e}"))?;
    let dir = TempDir::try_new("r17-5")?;
    let zip_path = dir.child("bundle.prism");
    fs::write(&zip_path, &zip_bytes).map_err(|e| format!("write temp archive: {e}"))?;

    let mut archive_session = Session::new();
    let archive_out =
        archive_session.execute("table.importPrism", json!({ "path": zip_path.to_str() })).map_err(|e| format!("table.importPrism failed: {e}"))?;
    assert!(archive_out.warnings.iter().any(|w| w.contains("[UnsupportedAnalysis]")));
    assert!(archive_out.warnings.iter().any(|w| w.contains("[UnsupportedGraphOrLayout]")));
    let archive_payload_warnings = archive_out.payload["warnings"].as_array().ok_or("expected structured payload warnings")?;
    assert_eq!(archive_payload_warnings.len(), archive_out.warnings.len());
    Ok(())
}

#[test]
fn engine_table_imports_refuse_pcff_and_external_indexed_archives_and_warn_on_no_n_summary() -> Result<(), String> {
    use curvebook_doc::ErrorCode;
    use curvebook_format::BINARY_PZF_UNSUPPORTED_MESSAGE;

    let dir = TempDir::try_new("r17-5-hardening")?;

    // 1. `table.importPzfx` refuses binary `PCFF` payloads with `ErrorCode::UnsupportedFormat`.
    let pcff_path = dir.child("legacy_binary.pzfx");
    fs::write(&pcff_path, b"PCFF\x00\x01\xFF\xFEbinary").map_err(|e| format!("write pcff fixture: {e}"))?;
    let mut s1 = Session::new();
    let pcff_err = s1.execute("table.importPzfx", json!({ "path": pcff_path.to_str() })).expect_err("binary PCFF .pzfx must be refused");
    assert_eq!(pcff_err.code, ErrorCode::UnsupportedFormat);
    assert!(pcff_err.message().contains(BINARY_PZF_UNSUPPORTED_MESSAGE), "unexpected error: {}", pcff_err.message());

    // 2. `table.importPzfx` maps 2-subcolumn `SD` natively to `MeanSd` (zero warnings) and surfaces
    //    no-N `CV` summary MissingMetadata warnings cleanly.
    let no_n_xml = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile><TableSequence>",
        "<Table ID=\"sd2\" TableType=\"Column\" YFormat=\"SD\">",
        "<YColumn Subcolumns=\"2\"><Title>Group</Title><Subcolumn><d>5.0</d></Subcolumn><Subcolumn><d>0.5</d></Subcolumn></YColumn>",
        "</Table>",
        "<Table ID=\"cv2\" TableType=\"Column\" YFormat=\"CV\">",
        "<YColumn Subcolumns=\"2\"><Title>Group</Title><Subcolumn><d>20.0</d></Subcolumn><Subcolumn><d>10.0</d></Subcolumn></YColumn>",
        "</Table></TableSequence></PrismFile>",
    );
    let mut s2 = Session::new();
    let no_n_out = s2.execute("table.importPzfx", json!({ "xml": no_n_xml })).map_err(|e| format!("no-N import failed: {e}"))?;
    assert!(!no_n_out.warnings.iter().any(|w| w.contains("sd2")), "expected native MeanSd without warnings: {:?}", no_n_out.warnings);
    assert!(no_n_out.warnings.iter().any(|w| w.contains("[MissingMetadata]") && w.contains("cv2")));

    // 3. `table.importPrism` imports valid external ID-indexed `.prism` archives without losing
    //    row 0 of headerless CSVs, and refuses malformed external `document.json` without falling back.
    let mut zw = ZipWriter::new();
    zw.add_file("document.json", br#"{"sheets":{"data":["s1"]},"sheetAttributesMap":{"s1":{"title":"Dose"}}}"#.to_vec())
        .map_err(|e| format!("zip doc: {e}"))?;
    zw.add_file(
        "data/sheets/s1/sheet.json",
        br#"{"title":"Dose","table":{"uid":"t1","format":"xy","dataFormat":"y_sd_n","xDataSet":"ds-x","xDataSetFormat":"y_single","dataSets":["ds-y1"]}}"#.to_vec(),
    )
    .map_err(|e| format!("zip sheet: {e}"))?;
    zw.add_file(
        "data/sets/ds-x.json",
        br#"{"@class":"DataSet","uid":"ds-x","title":"Conc","format":"y_single","replicates":[{"valueType":"real"}]}"#.to_vec(),
    )
    .map_err(|e| format!("zip ds-x: {e}"))?;
    zw.add_file(
        "data/sets/ds-y1.json",
        br#"{"@class":"DataSet","uid":"ds-y1","title":"Response","format":"y_sd_n","replicates":[{"valueType":"real"},{"valueType":"real"},{"valueType":"int"}]}"#.to_vec(),
    )
    .map_err(|e| format!("zip ds-y1: {e}"))?;
    zw.add_file("data/tables/t1/data.csv", b"1.0,10.0,1.0,3\n2.0,20.0,2.0,3\n".to_vec()).map_err(|e| format!("zip csv: {e}"))?;
    let ext_zip_bytes = zw.finish().map_err(|e| format!("zip finish: {e}"))?;
    let ext_path = dir.child("external_indexed.prism");
    fs::write(&ext_path, &ext_zip_bytes).map_err(|e| format!("write ext fixture: {e}"))?;

    let mut s3 = Session::new();
    let ext_out = s3.execute("table.importPrism", json!({ "path": ext_path.to_str() })).map_err(|e| format!("external .prism import failed: {e}"))?;
    assert_eq!(ext_out.payload["sheet_count"], json!(1));
    assert_eq!(s3.document().data_sheets.len(), 1);
    assert_eq!(s3.document().data_sheets[0].archetype.row_count(), 2, "headerless row 0 must not be dropped");

    let mut bad_zw = ZipWriter::new();
    bad_zw.add_file("document.json", b"{\"sheets\":{\"data\":[\"s1\"]".to_vec()).map_err(|e| format!("zip bad doc: {e}"))?;
    bad_zw.add_file("data/tables/t1/data.csv", b"1.0,10.0\n2.0,20.0\n".to_vec()).map_err(|e| format!("zip bad csv: {e}"))?;
    let bad_zip_bytes = bad_zw.finish().map_err(|e| format!("zip bad finish: {e}"))?;
    let bad_path = dir.child("external_malformed.prism");
    fs::write(&bad_path, &bad_zip_bytes).map_err(|e| format!("write bad fixture: {e}"))?;

    let mut s4 = Session::new();
    let bad_err =
        s4.execute("table.importPrism", json!({ "path": bad_path.to_str() })).expect_err("malformed external document.json must be refused");
    assert_eq!(bad_err.code, ErrorCode::CorruptData);
    Ok(())
}

#[test]
fn issue_81_imported_rebuilt_graphs_compile_render_and_edit_via_engine() -> Result<(), String> {
    use curvebook_chart::{ScenePrimitive, compile_graph};
    use curvebook_doc::{GraphKind, SheetId};
    use curvebook_format::{REBUILT_GRAPH_WARNING_LABEL, open_project_bytes, save_project_bytes};

    let mut zw = ZipWriter::new();
    let doc_json = json!({
        "formatVersion": "1-6-1",
        "title": "Engine Rebuilt Graphs",
        "sheets": {
            "data": ["s-xy", "s-col", "s-grp", "s-surv", "s-cont", "s-pow", "s-nest", "s-mv"],
            "graphs": ["g-xy", "g-col", "g-grp", "g-surv", "g-cont", "g-pow", "g-nest", "g-mv"]
        },
        "sheetAttributesMap": {
            "g-xy": { "title": "XY Dose Curve" },
            "g-col": { "title": "Column Summary" },
            "g-grp": { "title": "Grouped Means" },
            "g-surv": { "title": "Survival Curve" },
            "g-cont": { "title": "Contingency Bars" },
            "g-pow": { "title": "Composition Donut" },
            "g-nest": { "title": "Nested Groups" },
            "g-mv": { "title": "Multivariate Plot" }
        }
    });
    zw.add_file("document.json", serde_json::to_vec(&doc_json).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;

    let entries: Vec<(&str, Vec<u8>)> = vec![
        (
            "data/sheets/s-xy/sheet.json",
            br#"{"table":{"uid":"t-xy","format":"xy","dataFormat":"y_single","xDataSet":"dx-xy","dataSets":["dy-xy"]}}"#.to_vec(),
        ),
        ("data/sets/dx-xy.json", br#"{"title":"Dose","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/sets/dy-xy.json", br#"{"title":"Resp","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/tables/t-xy/data.csv", b"1,10\n2,20\n4,35\n".to_vec()),
        (
            "data/sheets/s-col/sheet.json",
            br#"{"table":{"uid":"t-col","format":"column","dataFormat":"y_single","dataSets":["dy-c1","dy-c2"]}}"#.to_vec(),
        ),
        ("data/sets/dy-c1.json", br#"{"title":"Ctrl","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/sets/dy-c2.json", br#"{"title":"Drug","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/tables/t-col/data.csv", b"4,8\n5,9\n6,10\n".to_vec()),
        (
            "data/sheets/s-grp/sheet.json",
            br#"{"table":{"uid":"t-grp","format":"grouped","dataFormat":"y_replicates","replicatesCount":2,"rowTitlesDataSet":"rt-g","dataSets":["dy-g1"]}}"#.to_vec(),
        ),
        ("data/sets/rt-g.json", br#"{"title":"Time","replicates":[{"valueType":"string"}]}"#.to_vec()),
        ("data/sets/dy-g1.json", br#"{"title":"Arm","replicates":[{"valueType":"real"},{"valueType":"real"}]}"#.to_vec()),
        ("data/tables/t-grp/data.csv", b"T1,5,6\nT2,11,13\n".to_vec()),
        (
            "data/sheets/s-surv/sheet.json",
            br#"{"table":{"uid":"t-surv","format":"survival","dataFormat":"y_single","xDataSet":"dx-s","dataSets":["dy-s1"]}}"#.to_vec(),
        ),
        ("data/sets/dx-s.json", br#"{"title":"Days","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/sets/dy-s1.json", br#"{"title":"Cohort","replicates":[{"valueType":"int"}]}"#.to_vec()),
        ("data/tables/t-surv/data.csv", b"5,1\n10,1\n15,0\n".to_vec()),
        (
            "data/sheets/s-cont/sheet.json",
            br#"{"table":{"uid":"t-cont","format":"contingency","dataFormat":"y_single","rowTitlesDataSet":"rt-k","dataSets":["dy-k1","dy-k2"]}}"#.to_vec(),
        ),
        ("data/sets/rt-k.json", br#"{"title":"Outcome","replicates":[{"valueType":"string"}]}"#.to_vec()),
        ("data/sets/dy-k1.json", br#"{"title":"Yes","replicates":[{"valueType":"int"}]}"#.to_vec()),
        ("data/sets/dy-k2.json", br#"{"title":"No","replicates":[{"valueType":"int"}]}"#.to_vec()),
        ("data/tables/t-cont/data.csv", b"A,12,4\nB,5,15\n".to_vec()),
        (
            "data/sheets/s-pow/sheet.json",
            br#"{"table":{"uid":"t-pow","format":"parts_of_whole","dataFormat":"y_single","rowTitlesDataSet":"rt-p","dataSets":["dy-p1"]}}"#.to_vec(),
        ),
        ("data/sets/rt-p.json", br#"{"title":"Part","replicates":[{"valueType":"string"}]}"#.to_vec()),
        ("data/sets/dy-p1.json", br#"{"title":"Share","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/tables/t-pow/data.csv", b"X,40\nY,60\n".to_vec()),
        (
            "data/sheets/s-nest/sheet.json",
            br#"{"table":{"uid":"t-nest","format":"nested","dataFormat":"y_replicates","replicatesCount":2,"dataSets":["dy-n1"]}}"#.to_vec(),
        ),
        ("data/sets/dy-n1.json", br#"{"title":"Nest","replicates":[{"valueType":"real"},{"valueType":"real"}]}"#.to_vec()),
        ("data/tables/t-nest/data.csv", b"2,3\n4,5\n".to_vec()),
        (
            "data/sheets/s-mv/sheet.json",
            br#"{"table":{"uid":"t-mv","format":"multivariable","dataFormat":"y_single","dataSets":["dy-mv1","dy-mv2"]}}"#.to_vec(),
        ),
        ("data/sets/dy-mv1.json", br#"{"title":"V1","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/sets/dy-mv2.json", br#"{"title":"V2","replicates":[{"valueType":"real"}]}"#.to_vec()),
        ("data/tables/t-mv/data.csv", b"1,2\n3,6\n5,9\n".to_vec()),
        ("graphs/g-xy/sheet.json", br#"{"uid":"g-xy","inputDataSets":["dy-xy"],"floatingNotes":["n-1"]}"#.to_vec()),
        ("graphs/g-xy/floating_notes/n-1.json", br#"{"uid":"n-1","position":"-1;-1","text":"Rebuilt annotation text"}"#.to_vec()),
        ("graphs/g-xy/data.bin", b"PCFFGRA4opaque".to_vec()),
        ("graphs/g-col/sheet.json", br#"{"uid":"g-col","inputDataSets":["dy-c1","dy-c2"]}"#.to_vec()),
        ("graphs/g-grp/sheet.json", br#"{"uid":"g-grp","inputDataSets":["dy-g1"]}"#.to_vec()),
        ("graphs/g-surv/sheet.json", br#"{"uid":"g-surv","inputDataSets":["dy-s1"]}"#.to_vec()),
        ("graphs/g-cont/sheet.json", br#"{"uid":"g-cont","inputDataSets":["dy-k1","dy-k2"]}"#.to_vec()),
        ("graphs/g-pow/sheet.json", br#"{"uid":"g-pow","inputDataSets":["dy-p1"]}"#.to_vec()),
        ("graphs/g-nest/sheet.json", br#"{"uid":"g-nest","inputDataSets":["dy-n1"]}"#.to_vec()),
        ("graphs/g-mv/sheet.json", br#"{"uid":"g-mv","inputDataSets":["dy-mv1","dy-mv2"]}"#.to_vec()),
    ];
    for (path, data) in entries {
        zw.add_file(path, data).map_err(|e| format!("{e}"))?;
    }
    let archive_bytes = zw.finish().map_err(|e| format!("{e}"))?;
    let dir = TempDir::try_new("issue-81-engine")?;
    let archive_path = dir.child("rebuilt_graphs.prism");
    fs::write(&archive_path, &archive_bytes).map_err(|e| format!("{e}"))?;

    let mut session = Session::new();
    let import_out = session.execute("table.importPrism", json!({ "path": archive_path.to_str() })).map_err(|e| format!("{e}"))?;
    assert!(
        import_out.warnings.iter().any(|w| w.contains("[UnsupportedGraphOrLayout]") && w.contains(REBUILT_GRAPH_WARNING_LABEL)),
        "expected rebuilt graph warning label in engine output: {:?}",
        import_out.warnings
    );

    session.recalculate_all().map_err(|e| format!("recalculate_all failed: {e}"))?;
    assert_eq!(session.document().graph_sheets.len(), 8);

    // Every rebuilt graph across all 8 archetypes must compile cleanly into a non-empty ChartScene.
    for gs in &session.document().graph_sheets {
        assert!(gs.initialized);
        let scene = compile_graph(gs, session.document()).map_err(|e| format!("compile_graph({}) failed: {e}", gs.id))?;
        assert!(!scene.primitives.is_empty(), "compiled scene for {} was empty", gs.id);
        if gs.id == SheetId::new("g-xy") {
            let has_note_text = scene.primitives.iter().any(|p| match p {
                ScenePrimitive::Text(t) => t.rich.plain_text().contains("Rebuilt annotation text"),
                _ => false,
            });
            assert!(has_note_text, "expected imported floatingNote text in compiled g-xy scene");
        }
    }

    // Rebuilt graphs are standard editable CurveBook sheets (e.g., graph.setAxes).
    session
        .execute(
            "graph.setAxes",
            json!({
                "sheet_id": "g-xy",
                "x_axis": { "title": "Edited Dose (nM)" },
                "y_axis": { "title": "Edited Response (%)" }
            }),
        )
        .map_err(|e| format!("graph.setAxes failed on rebuilt graph: {e}"))?;
    let g_xy = session.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
    assert_eq!(g_xy.spec.kind, GraphKind::Xy);
    assert_eq!(g_xy.spec.x_axis.title, "Edited Dose (nM)");
    assert_eq!(g_xy.spec.y_axis.title, "Edited Response (%)");

    let saved = save_project_bytes(session.document()).map_err(|e| format!("save failed: {e}"))?;
    let reopened = open_project_bytes(&saved).map_err(|e| format!("open failed: {e}"))?;
    assert_eq!(reopened.graph_sheets, session.document().graph_sheets);
    Ok(())
}
