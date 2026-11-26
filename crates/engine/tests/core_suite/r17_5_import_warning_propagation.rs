use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_engine::Session;
use curvebook_format::ZipWriter;
use serde_json::json;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(filename: &str, bytes: &[u8]) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb_eng_r17_5_{}_{seq}_{filename}", std::process::id()));
        fs::write(&path, bytes).map_err(|e| format!("write temp archive: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

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
    let zip_file = TempFile::new("bundle.prism", &zip_bytes)?;

    let mut archive_session = Session::new();
    let archive_out = archive_session
        .execute("table.importPrism", json!({ "path": zip_file.path.to_str() }))
        .map_err(|e| format!("table.importPrism failed: {e}"))?;
    assert!(archive_out.warnings.iter().any(|w| w.contains("[UnsupportedAnalysis]")));
    assert!(archive_out.warnings.iter().any(|w| w.contains("[UnsupportedGraphOrLayout]")));
    let archive_payload_warnings = archive_out.payload["warnings"].as_array().ok_or("expected structured payload warnings")?;
    assert_eq!(archive_payload_warnings.len(), archive_out.warnings.len());
    Ok(())
}
