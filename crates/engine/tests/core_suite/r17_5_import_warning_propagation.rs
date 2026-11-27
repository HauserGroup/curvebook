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
