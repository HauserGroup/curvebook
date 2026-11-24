//! `R12.8`: keeps the claims on the public `docs/open-pzfx-files.html` page true.
//!
//! Each test runs the exact `curvebook-cli` command lines the page shows, on small hand-written
//! `.pzfx` / `.prism` files, and checks the behaviour the page describes: the first table is
//! exported by default, `--sheet` selects a table by title, excluded values keep a trailing `*`,
//! the whole file converts to a `.curvebook` project, and unrecognised `.prism` parts are skipped
//! with a warning. If one of these fails, update the page in the same change.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_format::ZipWriter;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Two tables: an XY table with two replicate Y subcolumns and one excluded value, and a column table.
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
    assert!(info.contains("0 analyses, 0 graphs, 0 layouts"), "page says graphs and analysis results are not imported: {info}");

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
