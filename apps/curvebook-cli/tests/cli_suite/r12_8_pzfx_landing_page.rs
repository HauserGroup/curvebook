//! `R12.8`: keeps the claims on the public `docs/open-pzfx-files.html` page (and companion format
//! docs) true.
//!
//! Each test runs the exact `curvebook-cli` command lines the page shows, on small hand-written
//! `.pzfx` / `.prism` files, and checks the behaviour the page describes: the first table is
//! exported by default, `--sheet` selects a table by title, excluded values keep a trailing `*`,
//! the whole file converts to a `.curvebook` project, legacy binary `.pzf` / `PCFF` files are
//! refused with a clear message, external ID-indexed `.prism` archives import headerless data
//! tables plus read-only preferred analysis snapshots, and unrecognised `.prism` parts are skipped
//! with a warning. If one of these fails, update the page in the same change.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_format::ZipWriter;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Two tables (XY with replicate Y subcolumns and an excluded value, plus a column table) and a graph node.
const SAMPLE_PZFX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="dose" TableType="XY" XFormat="numbers" YFormat="replicates" Replicates="2">
      <Title>Dose response</Title>
      <XColumn><Title>log dose</Title><Subcolumn><d>-9</d><d>-8</d><d>-7</d><d>-6</d></Subcolumn></XColumn>
      <YColumn><Title>Response</Title><Subcolumn><d>2.1</d><d>10.5</d><d>48.0</d><d>91.2</d></Subcolumn><Subcolumn><d>3.0</d><d Excluded="1">55.0</d><d>51.5</d><d>89.9</d></Subcolumn></YColumn>
    </Table>
    <Table ID="groups" TableType="OneWay">
      <Title>Groups</Title>
      <YColumn><Title>Control</Title><Subcolumn><d>1.2</d><d>1.4</d><d>1.1</d></Subcolumn></YColumn>
      <YColumn><Title>Treated</Title><Subcolumn><d>2.3</d><d>2.6</d><d>2.0</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
  <Graph ID="graph1"><Title>Dose curve</Title></Graph>
</PrismFile>
"#;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb_r12_8_{label}_{}_{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_cli(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_curvebook-cli")).current_dir(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "curvebook-cli {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn csv_rows(path: &Path) -> Vec<String> {
    fs::read_to_string(path).unwrap().lines().map(str::to_owned).collect()
}

#[test]
fn r12_8_pzfx_info_lists_tables_and_convert_exports_the_first_table() {
    let tmp = TempDir::new("info");
    fs::write(tmp.0.join("experiment.pzfx"), SAMPLE_PZFX).unwrap();

    let info = String::from_utf8(run_cli(&tmp.0, &["info", "experiment.pzfx"]).stdout).unwrap();
    assert!(info.contains("2 tables"), "{info}");
    assert!(info.contains("`Dose response`") && info.contains("`Groups`"), "{info}");
    assert!(info.contains("0 analyses, 0 graphs, 0 layouts"), "for .pzfx, graphs and analysis results are not imported: {info}");

    run_cli(&tmp.0, &["convert", "experiment.pzfx", "experiment.csv"]);
    assert_eq!(
        csv_rows(&tmp.0.join("experiment.csv")),
        ["log dose,Response:Y1,Response:Y2", "-9,2.1,3", "-8,10.5,55*", "-7,48,51.5", "-6,91.2,89.9"],
        "first table, replicate subcolumns side by side, excluded value marked with `*`"
    );
}

#[test]
fn r12_8_pzfx_sheet_flag_selects_a_table_by_title_and_project_conversion_works() {
    let tmp = TempDir::new("sheet");
    fs::write(tmp.0.join("experiment.pzfx"), SAMPLE_PZFX).unwrap();

    run_cli(&tmp.0, &["convert", "experiment.pzfx", "groups.csv", "--sheet", "Groups"]);
    assert_eq!(csv_rows(&tmp.0.join("groups.csv")), ["Control,Treated", "1.2,2.3", "1.4,2.6", "1.1,2"]);

    run_cli(&tmp.0, &["convert", "experiment.pzfx", "experiment.curvebook"]);
    let reopened = String::from_utf8(run_cli(&tmp.0, &["info", "experiment.curvebook"]).stdout).unwrap();
    assert!(reopened.contains("2 tables"), "{reopened}");

    run_cli(&tmp.0, &["convert", "experiment.curvebook", "export.pzfx"]);
    let exported = String::from_utf8(run_cli(&tmp.0, &["info", "export.pzfx"]).stdout).unwrap();
    assert!(exported.contains("2 tables") && exported.contains("`Dose response`") && exported.contains("`Groups`"), "{exported}");
}

#[test]
fn r12_8_prism_csv_tables_import_and_unknown_parts_are_skipped_with_a_warning() {
    let tmp = TempDir::new("prism");
    let mut zip = ZipWriter::new();
    zip.add_file("data/groups.csv", b"Control,Treated\n1.2,2.3\n1.4,2.6\n1.1,2.0\n".to_vec()).unwrap();
    zip.add_file("data/blob.bin", b"opaque".to_vec()).unwrap();
    fs::write(tmp.0.join("experiment.prism"), zip.finish().unwrap()).unwrap();

    let info = String::from_utf8(run_cli(&tmp.0, &["info", "experiment.prism"]).stdout).unwrap();
    assert!(info.contains("1 tables"), "{info}");
    assert!(info.contains("Warnings (1)") && info.contains("data/blob.bin"), "{info}");

    run_cli(&tmp.0, &["convert", "experiment.prism", "experiment.csv"]);
    assert_eq!(csv_rows(&tmp.0.join("experiment.csv")), ["Control,Treated", "1.2,2.3", "1.4,2.6", "1.1,2"]);
}

#[test]
fn r12_8_legacy_binary_pzf_and_pcff_pzfx_are_refused_with_clear_error() {
    let tmp = TempDir::new("legacy_pzf");
    let pcff_bytes = b"PCFF\x00\x01\x02\x03legacy-binary-payload";
    fs::write(tmp.0.join("legacy.pzfx"), pcff_bytes).unwrap();
    fs::write(tmp.0.join("legacy.pzf"), pcff_bytes).unwrap();

    for name in ["legacy.pzfx", "legacy.pzf"] {
        let out = Command::new(env!("CARGO_BIN_EXE_curvebook-cli")).current_dir(&tmp.0).args(["info", name]).output().unwrap();
        assert!(!out.status.success(), "expected failure for legacy binary file `{name}`");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("older binary") && stderr.contains(".pzf") && stderr.contains(".pzfx") && stderr.contains(".prism"),
            "unexpected error for `{name}`: {stderr}"
        );
    }
}

#[test]
fn r12_8_id_indexed_archive_imports_data_readonly_snapshot_and_warns_on_opaque_graph() {
    let tmp = TempDir::new("id_indexed");
    let mut zip = ZipWriter::new();
    zip.add_file(
        "document.json",
        br#"{"$id":"doc-1","@class":"Document","formatVersion":"1-6-0","title":"Study","sheets":{"data":["d-src"],"info":[],"analyses":["an-1"],"graphs":["g-1"],"layouts":[]},"sheetAttributesMap":{"d-src":{"title":"Groups"},"an-1":{"title":"Unpaired t test"},"g-1":{"title":"Group Plot"}}}"#
            .to_vec(),
    )
    .unwrap();
    zip.add_file(
        "data/sheets/d-src/sheet.json",
        br#"{"uid":"d-src","table":{"uid":"t-src","format":"column","dataFormat":"y_single","dataSets":["ds-a","ds-b"]}}"#.to_vec(),
    )
    .unwrap();
    zip.add_file("data/sets/ds-a.json", br#"{"title":"Control","replicates":[{"valueType":"real"}]}"#.to_vec()).unwrap();
    zip.add_file("data/sets/ds-b.json", br#"{"title":"Treated","replicates":[{"valueType":"real"}]}"#.to_vec()).unwrap();
    zip.add_file("data/tables/t-src/data.csv", b"1.2,2.3\n1.4,2.6\n1.1,2.0\n".to_vec()).unwrap();
    zip.add_file(
        "analyses/an-1/sheet.json",
        br#"{"uid":"an-1","title":"Unpaired t test","analysisClass":"T_TESTS_AND_NONPARAMETRIC","preferredResultSheet":"rs-1","inputSheets":[{"uid":"d-src","title":"Groups"}],"resultSheets":[{"uid":"rs-1","title":"Tabular results"}]}"#
            .to_vec(),
    )
    .unwrap();
    zip.add_file("analyses/an-1/result_sheets/rs-1.json", br#"{"uid":"rs-1","name":"Tabular results","dataSheet":"v-1"}"#.to_vec()).unwrap();
    zip.add_file(
        "data/sheets/v-1/sheet.json",
        br#"{"uid":"v-1","flags":["ANALYSIS_VIEW"],"table":{"uid":"t-v1","format":"view","dataFormat":"text","rowTitlesDataSet":"ds-k","dataSets":["ds-v"]}}"#
            .to_vec(),
    )
    .unwrap();
    zip.add_file("data/sets/ds-k.json", br#"{"title":"Metric","replicates":[{"valueType":"string"}]}"#.to_vec()).unwrap();
    zip.add_file("data/sets/ds-v.json", br#"{"title":"Value","replicates":[{"valueType":"string"}]}"#.to_vec()).unwrap();
    zip.add_file("data/tables/t-v1/data.csv", b"P value,0.0021\n".to_vec()).unwrap();
    zip.add_file("graphs/g-1/sheet.json", br#"{"uid":"g-1","title":"Group Plot"}"#.to_vec()).unwrap();
    zip.add_file("graphs/g-1/data.bin", b"PCFFGRA4opaque".to_vec()).unwrap();

    fs::write(tmp.0.join("study.prism"), zip.finish().unwrap()).unwrap();

    let info = String::from_utf8(run_cli(&tmp.0, &["info", "study.prism"]).stdout).unwrap();
    assert!(info.contains("1 tables"), "{info}");
    assert!(info.contains("1 analyses"), "preferred analysis snapshot should import as a read-only result: {info}");
    assert!(info.contains("Warnings ("), "opaque binary graph and unverified analysis should emit warnings: {info}");

    run_cli(&tmp.0, &["convert", "study.prism", "groups.csv"]);
    assert_eq!(csv_rows(&tmp.0.join("groups.csv")), ["Control,Treated", "1.2,2.3", "1.4,2.6", "1.1,2"], "headerless archive CSV must preserve row 0");

    run_cli(&tmp.0, &["convert", "study.prism", "study.curvebook"]);
    let reopened = String::from_utf8(run_cli(&tmp.0, &["info", "study.curvebook"]).stdout).unwrap();
    assert!(reopened.contains("1 tables") && reopened.contains("1 analyses"), "{reopened}");
}

#[test]
fn r12_8_public_format_docs_keep_honest_export_and_legacy_pzf_copy() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let docs = [("docs/open-pzfx-files.html", true), ("docs/faq.html", true), ("docs/file-formats.md", false), ("docs/reference/files.md", false)];
    for (rel, is_html) in docs {
        let content = fs::read_to_string(repo_root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"));
        let export_copy = if is_html {
            "<code>.pzfx</code> export saves your data tables and info sheets only. Analyses, graphs and layouts stay in your <code>.curvebook</code> project."
        } else {
            "`.pzfx` export saves your data tables and info sheets only. Analyses, graphs and layouts stay in your `.curvebook` project."
        };
        let untested_copy = if is_html {
            "Written to match the <code>.pzfx</code> structure of real files; not tested in other applications"
        } else {
            "Written to match the `.pzfx` structure of real files; not tested in other applications"
        };
        assert!(content.contains(export_copy), "{rel} missing §4.2 honest data-only export copy");
        assert!(content.contains(untested_copy), "{rel} missing §6.3 honest corpus-conformance copy");
        assert!(content.contains("PCFF"), "{rel} must document binary PCFF / .pzf refusal");
        assert!(content.contains("Merge Project…"), "{rel} must document File › Merge Project…");
    }
    let file_formats = fs::read_to_string(repo_root.join("docs/file-formats.md")).unwrap();
    assert!(file_formats.contains("CURRENT_SCHEMA_VERSION = 9"), "docs/file-formats.md must document schema v9");
    let ref_files = fs::read_to_string(repo_root.join("docs/reference/files.md")).unwrap();
    assert!(ref_files.contains("| 9 |"), "docs/reference/files.md must document schema v9");
    let landing = fs::read_to_string(repo_root.join("docs/open-pzfx-files.html")).unwrap();
    assert!(
        landing.contains("https://www.nature.com/articles/sdata201618")
            && landing.contains("PCFFGRA4")
            && landing.contains("<code>.dt</code>")
            && landing.contains("pzfx-format-specification.html")
            && landing.contains("File › Export › .pzfx file…")
            && landing.contains("curvebook-cli convert experiment.curvebook export.pzfx"),
        "docs/open-pzfx-files.html must include FAIR citation, empirical .prism format callout, spec page link, and .pzfx export paths"
    );
    let spec = fs::read_to_string(repo_root.join("docs/pzfx-format-specification.html")).unwrap();
    assert!(
        spec.contains("The closest thing we have to an actual specification")
            && spec.contains("proprietary blob hidden by the vendor")
            && spec.contains("130ab2f63ec643bee541b74c960e10db")
            && spec.contains("PCFFGRA4"),
        "docs/pzfx-format-specification.html must document empirical .prism/.pzfx structure and proprietary blobs"
    );
}
