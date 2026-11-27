use std::fs;

use curvebook_doc::{
    ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, ResultProvenance, ResultStatus, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_format::{ZipWriter, open_project_file, save_project_file};
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

fn build_promoted_archive_fixture_bytes() -> Result<Vec<u8>, String> {
    let mut zip = ZipWriter::new();
    let add_json = |zw: &mut ZipWriter, path: &str, value: Value| -> Result<(), String> {
        let bytes = serde_json::to_vec(&value).map_err(|e| format!("json {path}: {e}"))?;
        zw.add_file(path, bytes).map_err(|e| format!("zip {path}: {e}"))
    };
    add_json(
        &mut zip,
        "document.json",
        json!({
            "formatVersion": "1-6-0",
            "title": "CLI Promoted Archive",
            "sheets": { "data": ["d-groups", "d-single"], "analyses": ["an-ttest", "an-stale", "an-fallback"] },
            "sheetAttributesMap": {
                "d-groups": { "title": "Groups" },
                "d-single": { "title": "Single" },
                "an-ttest": { "title": "Agreeing T-Test" },
                "an-stale": { "title": "Stale T-Test" },
                "an-fallback": { "title": "Degenerate T-Test" }
            }
        }),
    )?;
    add_json(
        &mut zip,
        "data/sheets/d-groups/sheet.json",
        json!({"uid": "d-groups", "title": "Groups", "table": {"uid": "t-groups", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-a", "ds-b"]}}),
    )?;
    add_json(&mut zip, "data/sets/ds-a.json", json!({"title": "Control", "replicates": [{"valueType": "real"}]}))?;
    add_json(&mut zip, "data/sets/ds-b.json", json!({"title": "Treated", "replicates": [{"valueType": "real"}]}))?;
    zip.add_file("data/tables/t-groups/data.csv", b"10,20\n12,22\n14,24\n".to_vec()).map_err(|e| e.to_string())?;

    add_json(
        &mut zip,
        "data/sheets/d-single/sheet.json",
        json!({"uid": "d-single", "title": "Single", "table": {"uid": "t-single", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-only"]}}),
    )?;
    add_json(&mut zip, "data/sets/ds-only.json", json!({"title": "Only", "replicates": [{"valueType": "real"}]}))?;
    zip.add_file("data/tables/t-single/data.csv", b"1\n2\n3\n".to_vec()).map_err(|e| e.to_string())?;

    let ttest_params = json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}});
    add_json(
        &mut zip,
        "analyses/an-ttest/sheet.json",
        json!({"uid": "an-ttest", "title": "Agreeing T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC", "inputSheets": [{"uid": "d-groups"}]}),
    )?;
    add_json(&mut zip, "analyses/an-ttest/parameters.json", ttest_params.clone())?;
    add_json(
        &mut zip,
        "analyses/an-ttest/results.json",
        json!({"content": {"unpairedTTestModel": {"unpairedTTest": {"t": -6.123_724_356_957_945, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.003_602_232_609_1}}}}}),
    )?;

    add_json(
        &mut zip,
        "analyses/an-stale/sheet.json",
        json!({"uid": "an-stale", "title": "Stale T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC", "inputSheets": [{"uid": "d-groups"}]}),
    )?;
    add_json(&mut zip, "analyses/an-stale/parameters.json", ttest_params.clone())?;
    add_json(
        &mut zip,
        "analyses/an-stale/results.json",
        json!({"content": {"unpairedTTestModel": {"unpairedTTest": {"t": -6.123_724_356_957_945, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.156}}}}}),
    )?;

    add_json(
        &mut zip,
        "analyses/an-fallback/sheet.json",
        json!({
            "uid": "an-fallback",
            "title": "Degenerate T-Test",
            "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
            "inputSheets": [{"uid": "d-single"}],
            "preferredResultSheet": "rs-fb",
            "resultSheets": [{"uid": "rs-fb", "title": "Cached Fallback"}]
        }),
    )?;
    add_json(&mut zip, "analyses/an-fallback/parameters.json", ttest_params)?;
    add_json(&mut zip, "analyses/an-fallback/result_sheets/rs-fb.json", json!({"uid": "rs-fb", "name": "Cached Fallback", "dataSheet": "view-fb"}))?;
    add_json(
        &mut zip,
        "data/sheets/view-fb/sheet.json",
        json!({"uid": "view-fb", "flags": ["ANALYSIS_VIEW"], "table": {"uid": "t-view-fb", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-fb-rt", "dataSets": ["ds-fb-v"]}}),
    )?;
    add_json(&mut zip, "data/sets/ds-fb-rt.json", json!({"title": "Metric", "replicates": [{"valueType": "string"}]}))?;
    add_json(&mut zip, "data/sets/ds-fb-v.json", json!({"title": "Value", "replicates": [{"valueType": "string"}]}))?;
    zip.add_file("data/tables/t-view-fb/data.csv", b"P value,0.042\n".to_vec()).map_err(|e| e.to_string())?;

    zip.finish().map_err(|e| format!("zip finish: {e}"))
}

#[test]
fn cli_info_and_convert_surface_promoted_archive_disagreement_and_fallback_warnings() -> TestResult {
    let dir = TempDir::new("cli-promoted-archive")?;
    let archive_path = dir.child("promoted.prism");
    let out_json_cb = dir.child("promoted_json.curvebook");
    let out_human_cb = dir.child("promoted_human.curvebook");
    fs::write(&archive_path, build_promoted_archive_fixture_bytes()?).map_err(|e| format!("write archive: {e}"))?;

    let archive_str = archive_path.to_str().ok_or("non-utf8 archive path")?;
    let out_json_str = out_json_cb.to_str().ok_or("non-utf8 out json path")?;
    let out_human_str = out_human_cb.to_str().ok_or("non-utf8 out human path")?;

    // 1. `info --json` reports recomputed warnings in JSON and live vs Imported statuses.
    let info_val = CliHarness::run_ok_json(&["info", "--json", archive_str])?;
    let info_warnings = info_val["warnings"].as_array().ok_or("expected warnings array in info --json")?;
    assert!(
        info_warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("[RecomputedResultDisagreement]") && w.as_str().unwrap_or("").contains("Stale T-Test")),
        "missing disagreement warning in info --json: {info_warnings:?}"
    );
    assert!(
        info_warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("[AnalysisRecomputeFallback]") && w.as_str().unwrap_or("").contains("Degenerate T-Test")),
        "missing fallback warning in info --json: {info_warnings:?}"
    );
    assert!(
        !info_warnings.iter().any(|w| w.as_str().unwrap_or("").contains("analyses/an-ttest")),
        "unexpected warning for agreeing promoted analysis: {info_warnings:?}"
    );

    // 2. `convert --json` emits disagreement & fallback warnings with ZERO `pending_result_sheet`,
    //    and writes a `.curvebook` archive with live UpToDate promoted results and Imported fallback.
    let conv_val = CliHarness::run_ok_json(&["convert", "--json", archive_str, out_json_str])?;
    let conv_warnings = conv_val["warnings"].as_array().ok_or("expected warnings array in convert --json")?;
    assert!(
        conv_warnings.iter().any(|w| w.as_str().unwrap_or("").contains("[RecomputedResultDisagreement]")),
        "missing disagreement warning in convert --json: {conv_warnings:?}"
    );
    assert!(
        conv_warnings.iter().any(|w| w.as_str().unwrap_or("").contains("[AnalysisRecomputeFallback]")),
        "missing fallback warning in convert --json: {conv_warnings:?}"
    );
    assert!(
        !conv_warnings.iter().any(|w| w.as_str().unwrap_or("").contains("pending_result_sheet")),
        "promoted analyses must already be recomputed without pending_result_sheet: {conv_warnings:?}"
    );

    let converted_doc = open_project_file(&out_json_cb).map_err(|e| format!("open converted .curvebook: {e}"))?;
    for live_id in ["an-ttest", "an-stale"] {
        let rs = converted_doc.result_sheet(&SheetId::new(live_id)).ok_or_else(|| format!("missing {live_id}"))?;
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(rs.provenance, ResultProvenance::Computed);
    }
    let fb = converted_doc.result_sheet(&SheetId::new("an-fallback")).ok_or("missing an-fallback")?;
    assert_eq!(fb.status, ResultStatus::Imported);
    assert!(matches!(fb.provenance, ResultProvenance::Imported { .. }));
    assert_eq!(fb.tabs.len(), 1);

    // 3. Human `convert` prints both warning tags on stderr.
    let human_out = run_cli(&["convert", archive_str, out_human_str])?;
    if !human_out.status.success() {
        return Err(format!("human convert failed: {}", String::from_utf8_lossy(&human_out.stderr)));
    }
    let stderr = String::from_utf8_lossy(&human_out.stderr);
    assert!(stderr.contains("[RecomputedResultDisagreement]"), "missing disagreement warning on stderr: {stderr}");
    assert!(stderr.contains("[AnalysisRecomputeFallback]"), "missing fallback warning on stderr: {stderr}");
    Ok(())
}
