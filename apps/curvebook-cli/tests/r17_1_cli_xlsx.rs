#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{Cell, TableArchetype};
use curvebook_format::open_project_file;
use curvebook_format::zip::ZipWriter;
use serde_json::Value;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-cli-r17-1-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|err| format!("create temp dir `{}`: {err}", dir.display()))?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_curvebook-cli")
}

fn run_cli(args: &[&str]) -> Result<Output, String> {
    Command::new(cli_bin())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("spawn cli {args:?}: {err}"))
}

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

#[test]
fn cli_info_lists_xlsx_sheets_and_convert_selects_subset() -> Result<(), String> {
    let tmp = TempDir::new("xlsx")?;
    let xlsx_path = tmp.path().join("workbook.xlsx");
    let out_path = tmp.path().join("converted.curvebook");

    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="RunOne" sheetId="1" r:id="rId1"/>
    <sheet name="RunTwo" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Resp</t></is></c></row>
    <row r="2"><c r="A2"><v>0.5</v></c><c r="B2"><v>11.0</v></c></row>
  </sheetData>
</worksheet>"#;
    let sheet2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Ctrl</t></is></c><c r="B1" t="inlineStr"><is><t>Drug</t></is></c></row>
    <row r="2"><c r="A2"><v>2.5</v></c><c r="B2"><v>8.5</v></c></row>
  </sheetData>
</worksheet>"#;

    let bytes = build_xlsx_zip(&[
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/worksheets/sheet2.xml", sheet2),
    ]);
    fs::write(&xlsx_path, bytes).map_err(|e| format!("write xlsx: {e}"))?;

    let info_out = run_cli(&["info", xlsx_path.to_str().ok_or("bad path")?, "--list-sheets", "--json"])?;
    if !info_out.status.success() {
        return Err(format!(
            "cli info failed: stdout={} stderr={}",
            String::from_utf8_lossy(&info_out.stdout),
            String::from_utf8_lossy(&info_out.stderr)
        ));
    }
    let info_json: Value = serde_json::from_slice(&info_out.stdout).map_err(|e| format!("parse info json: {e}"))?;
    assert_eq!(info_json["sheet_count"], 2);
    assert_eq!(info_json["sheets"][0]["name"], "RunOne");
    assert_eq!(info_json["sheets"][1]["name"], "RunTwo");

    let conv_out =
        run_cli(&["convert", xlsx_path.to_str().ok_or("bad path")?, out_path.to_str().ok_or("bad out path")?, "--xlsx-sheets", "RunTwo", "--json"])?;
    if !conv_out.status.success() {
        return Err(format!(
            "cli convert failed: stdout={} stderr={}",
            String::from_utf8_lossy(&conv_out.stdout),
            String::from_utf8_lossy(&conv_out.stderr)
        ));
    }
    let doc = open_project_file(&out_path).map_err(|e| format!("load converted project: {e}"))?;
    assert_eq!(doc.data_sheets.len(), 1);
    assert_eq!(doc.data_sheets[0].title, "RunTwo");
    let TableArchetype::Xy(xy) = &doc.data_sheets[0].archetype else {
        return Err(format!("expected XY sheet, got {:?}", doc.data_sheets[0].archetype));
    };
    assert_eq!(xy.x_cells[0][0], Cell::number(2.5));
    assert_eq!(xy.data_sets[0].cells[0][0], Cell::number(8.5));
    Ok(())
}
