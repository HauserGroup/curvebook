use std::fs;

use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype};
use curvebook_format::{ZipWriter, save_project_file};
use serde_json::{Value, json};

use crate::common::{CliHarness, TempDir};

type TestResult = Result<(), String>;

fn run_cli(args: &[&str]) -> Result<std::process::Output, String> {
    CliHarness::run(args)
}

#[test]
fn cli_convert_and_info_surface_pzfx_and_prism_import_warnings() -> TestResult {
    let dir = TempDir::new("cli-warn")?;
    let pzfx_path = dir.path().join("legacy.pzfx");
    let archive_path = dir.path().join("legacy.prism");
    let out_curvebook = dir.path().join("converted.curvebook");

    let xml = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile>\n",
        "  <TableSequence>\n",
        "    <Table ID=\"t1\" TableType=\"XY\" YFormat=\"PlusMinus\">\n",
        "      <XColumn><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>\n",
        "      <YColumn><Subcolumn><d>10</d><d>20</d></Subcolumn><Subcolumn><d>0.5</d><d>1.0</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        "  </TableSequence>\n",
        "  <UserEquations><Equation Name=\"Custom\"/></UserEquations>\n",
        "</PrismFile>",
    );
    fs::write(&pzfx_path, xml).map_err(|e| format!("write pzfx: {e}"))?;

    let mut zw = ZipWriter::new();
    zw.add_file("manifest.json", br#"{"title":"Bundle","analyses":[{"id":"a1"}]}"#.to_vec()).map_err(|e| format!("zip manifest: {e}"))?;
    zw.add_file("sheets/s1.csv", b"X,Y\n1,2\n3,4\n".to_vec()).map_err(|e| format!("zip csv: {e}"))?;
    zw.add_file("db/cache.sqlite", b"SQLite format 3\0data".to_vec()).map_err(|e| format!("zip sqlite: {e}"))?;
    let zip_bytes = zw.finish().map_err(|e| format!("zip finish: {e}"))?;
    fs::write(&archive_path, zip_bytes).map_err(|e| format!("write archive: {e}"))?;

    // 1. Human `convert` surfaces formatted warnings on stderr
    let pzfx_str = pzfx_path.to_str().ok_or("non-utf8 pzfx path")?;
    let out_str = out_curvebook.to_str().ok_or("non-utf8 out path")?;
    let convert_human = run_cli(&["convert", pzfx_str, out_str])?;
    if !convert_human.status.success() {
        return Err(format!("human convert failed: {}", String::from_utf8_lossy(&convert_human.stderr)));
    }
    let stderr = String::from_utf8_lossy(&convert_human.stderr);
    assert!(stderr.contains("warning:"), "expected stderr warnings: {stderr}");
    assert!(stderr.contains("[CoercedValue]"), "expected CoercedValue tag in stderr: {stderr}");
    assert!(stderr.contains("[UnsupportedEquation]"), "expected UnsupportedEquation tag in stderr: {stderr}");

    // 2. JSON `convert` and `info` preserve warnings cleanly for `.prism`
    let archive_str = archive_path.to_str().ok_or("non-utf8 archive path")?;
    let info_json_out = run_cli(&["info", "--json", archive_str])?;
    if !info_json_out.status.success() {
        return Err(format!(
            "json info failed: stdout={} stderr={}",
            String::from_utf8_lossy(&info_json_out.stdout),
            String::from_utf8_lossy(&info_json_out.stderr)
        ));
    }
    let info_val: Value = serde_json::from_slice(&info_json_out.stdout).map_err(|e| format!("parse info json: {e}"))?;
    let warnings = info_val["warnings"].as_array().ok_or("expected warnings array in info --json")?;
    assert!(warnings.iter().any(|w| w.as_str().unwrap_or("").contains("[UnsupportedAnalysis]")), "warnings: {warnings:?}");
    assert!(warnings.iter().any(|w| w.as_str().unwrap_or("").contains("SQLite")), "warnings: {warnings:?}");

    let archive_out_cb = dir.path().join("converted_archive.curvebook");
    let archive_out_str = archive_out_cb.to_str().ok_or("non-utf8 out path")?;
    let conv_json_out = run_cli(&["convert", "--json", archive_str, archive_out_str])?;
    if !conv_json_out.status.success() {
        return Err(format!(
            "json convert failed: stdout={} stderr={}",
            String::from_utf8_lossy(&conv_json_out.stdout),
            String::from_utf8_lossy(&conv_json_out.stderr)
        ));
    }
    let conv_val: Value = serde_json::from_slice(&conv_json_out.stdout).map_err(|e| format!("parse convert json: {e}"))?;
    let conv_warnings = conv_val["warnings"].as_array().ok_or("expected warnings array in convert --json")?;
    assert!(conv_warnings.iter().any(|w| w.as_str().unwrap_or("").contains("[UnsupportedAnalysis]")), "warnings: {conv_warnings:?}");

    // 3. `info --json` reports Column subcolumn_mode (`MeanSdN`).
    let mut doc = ProjectDocument::new("Column Summary Study").map_err(|e| e.to_string())?;
    let mut col = ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 2).map_err(|e| e.to_string())?;
    col.subcolumn_mode = SubcolumnMode::MeanSdN;
    doc.add_data_sheet(DataSheet::new("col-1", "Summary Column", TableArchetype::Column(col)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let proj = dir.child("column_summary.curvebook");
    save_project_file(&proj, &doc).map_err(|e| e.to_string())?;
    let col_info = CliHarness::run_ok_json(&["info", &proj.display().to_string(), "--json"])?;
    assert_eq!(col_info["tables"][0]["subcolumn_mode"], json!("MeanSdN"));

    // 4. `run` rejects unknown command keys (`param` typo) with exit 2; `batch` propagates base `.pzfx` warnings to JSON & stderr.
    let bad_script = dir.child("bad_script.json");
    fs::write(&bad_script, serde_json::to_vec(&json!([{ "cmd": "sheet.createData", "param": { "kind": "Column" } }])).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    assert_eq!(run_cli(&["run", &bad_script.display().to_string(), "--json"])?.status.code(), Some(2));

    let warn_pzfx = dir.child("warn_base.pzfx");
    fs::write(
        &warn_pzfx,
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile><TableSequence><Table ID="Table0" TableType="OneWay"><Title>Data 1</Title><YColumn Width="81" Decimals="1" Subcolumns="1"><Title>Ctrl</Title><Subcolumn><d>1.0</d><d>2.0</d></Subcolumn></YColumn></Table></TableSequence><Graph ID="Graph0"><Title>Unsupported Legacy Graph</Title></Graph></PrismFile>"#,
    )
    .map_err(|e| e.to_string())?;
    let manifest = dir.child("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "schema": "curvebook.batch.v1",
            "project": "warn_base.pzfx",
            "exports": [{ "format": "curvebook", "out": "out.curvebook" }]
        }))
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let (batch_json, batch_stderr) = CliHarness::run_checked_json(&["batch", &manifest.display().to_string(), "--json"])?;
    assert!(!batch_json["warnings"].as_array().ok_or("missing batch warnings")?.is_empty());
    assert!(batch_stderr.contains("warning:"), "missing stderr warning: {batch_stderr}");
    Ok(())
}
