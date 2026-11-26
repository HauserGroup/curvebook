use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_format::ZipWriter;
use serde_json::Value;

type TestResult = Result<(), String>;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-cli-r17-5-{label}-{}-{seq}", std::process::id()));
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

fn run_cli(args: &[&str]) -> Result<Output, String> {
    Command::new(env!("CARGO_BIN_EXE_curvebook-cli"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("spawn cli {args:?}: {err}"))
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
    Ok(())
}
