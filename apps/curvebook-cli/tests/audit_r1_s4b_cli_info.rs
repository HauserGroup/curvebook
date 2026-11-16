//! Integration tests for `curvebook-cli` Stage 4B fixes:
//! - `R1-S12-08`: `curvebook-cli info` sanitizes terminal escape / control characters in project and sheet metadata.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, InfoSheet, LayoutSheet, LayoutSpec, ProjectDocument, SheetId, TableArchetype,
};
use curvebook_format::save_project_file;
use serde_json::Value;

type TestResult = Result<(), String>;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-cli-s4b-{label}-{}-{seq}", std::process::id()));
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

#[test]
fn r1_s12_08_info_sanitizes_control_and_escape_bytes_in_human_output() -> TestResult {
    let dir = TempDir::new("info-esc")?;
    let proj_path = dir.path().join("escaped_titles.curvebook");

    let mut doc = ProjectDocument::new("proj\u{1b}[31m-red\u{7}\u{9b}2J\nspoof").map_err(|e| e.to_string())?;
    let ds = DataSheet::new(
        SheetId::new("data-1"),
        "ok\u{1b}[1A\u{1b}[2K\u{1b}]52;c;ZWNobyBwd25lZA==\u{7}",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?),
    )
    .map_err(|e| e.to_string())?;
    let info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Exp \u{7f} Notes\rTitle").map_err(|e| e.to_string())?;
    let graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &ds).map_err(|e| e.to_string())?;
    let mut layout = LayoutSheet::new(SheetId::new("layout-1"), "Fig 1 \u{1b}]0;owned\u{7}", LayoutSpec::default()).map_err(|e| e.to_string())?;
    layout.title = String::from("Fig αβγ \u{1b}[2K\u{9d}");

    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(graph).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(layout).map_err(|e| e.to_string())?;
    save_project_file(&proj_path, &doc).map_err(|e| e.to_string())?;

    let path_str = proj_path.to_str().ok_or("non-utf8 path")?;
    let out = run_cli(&["info", path_str])?;
    if !out.status.success() {
        return Err(format!("cli info failed: {}", String::from_utf8_lossy(&out.stderr)));
    }

    assert!(!out.stdout.contains(&0x1b), "raw ESC 0x1b leaked to stdout: {:?}", String::from_utf8_lossy(&out.stdout));
    assert!(!out.stdout.contains(&0x07), "raw BEL 0x07 leaked to stdout");
    assert!(!out.stdout.contains(&0x7f), "raw DEL 0x7f leaked to stdout");
    assert!(!out.stdout.contains(&0x0d), "raw CR 0x0d leaked to stdout");

    let text = String::from_utf8(out.stdout.clone()).map_err(|e| format!("non-utf8 stdout: {e}"))?;
    assert!(!text.chars().any(|c| c != '\n' && c.is_control()), "unexpected control character in human stdout: {text:?}");
    assert!(text.contains(r"\u{1b}[1A"), "expected escaped ESC sequence in output: {text}");
    assert!(text.contains("Fig αβγ"), "expected valid UTF-8 printable chars preserved: {text}");

    let json_out = run_cli(&["info", "--json", path_str])?;
    if !json_out.status.success() {
        return Err(format!("cli info --json failed: {}", String::from_utf8_lossy(&json_out.stderr)));
    }
    let parsed: Value = serde_json::from_slice(&json_out.stdout).map_err(|e| format!("invalid json output: {e}"))?;
    assert_eq!(parsed["tables"][0]["title"], "ok\u{1b}[1A\u{1b}[2K\u{1b}]52;c;ZWNobyBwd25lZA==\u{7}");

    Ok(())
}
