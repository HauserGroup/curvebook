#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression tests for `R1-S12-01` (+ `R1-S07-02`) and `R1-X1-04`:
//! - Stored `ExternalDelimitedFile.path` containment across `batch --workdir`, `run --workdir`, and
//!   direct `<command-id> --workdir`.
//! - End-to-end `curvebook-cli mcp --workdir <dir>` path confinement over framed stdio JSON-RPC.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    CellCoord, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId,
};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use serde_json::{Value, json};

static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirPair {
    root: PathBuf,
    workdir: PathBuf,
    outside: PathBuf,
}

impl TempDirPair {
    fn new(label: &str) -> Self {
        let id = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-cli-policy-{label}-{}-{id}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::create_dir_all(&outside).expect("create outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempDirPair {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn save_project_with_external_link(archive_path: &Path, link_target: &str) {
    let mut session = Session::from_document(ProjectDocument::new("Stored Link").expect("doc"));
    session.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "Data 1", "kind": "Column", "rows": 2, "cols": 1 })).expect("sheet");
    session.execute("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 12.0 }] })).expect("seed");
    let mut doc = session.document().clone();
    doc.data_sheet_mut(&SheetId::new("d1")).expect("d1").linked_regions.push(LinkedDataRegion {
        id: String::from("link-ext"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: link_target.to_owned(), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("external link"),
    });
    save_project_file(archive_path, &doc).expect("save project archive");
}

fn write_framed_rpc(writer: &mut impl Write, payload: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let body = serde_json::to_string(payload)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()?;
    Ok(())
}

fn read_framed_rpc(reader: &mut BufReader<impl Read>) -> Result<Value, Box<dyn std::error::Error>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Err("unexpected EOF while reading MCP headers".into());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            content_length = Some(value.trim().parse::<usize>()?);
        }
    }
    let len = content_length.ok_or("missing Content-Length header")?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

struct McpChild {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpChild {
    fn spawn(args: &[&str]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_curvebook-cli"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().ok_or("missing child stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("missing child stdout")?);
        Ok(Self { child, stdin, stdout })
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
        write_framed_rpc(&mut self.stdin, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        read_framed_rpc(&mut self.stdout)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), Box<dyn std::error::Error>> {
        write_framed_rpc(&mut self.stdin, &json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn call_tool(&mut self, id: u64, name: &str, arguments: Value) -> Result<Value, Box<dyn std::error::Error>> {
        self.request(id, "tools/call", json!({ "name": name, "arguments": arguments }))
    }

    fn finish(self) -> Result<(), Box<dyn std::error::Error>> {
        drop(self.stdin);
        let out = self.child.wait_with_output()?;
        assert!(out.status.success(), "curvebook-cli mcp failed ({:?}): {}", out.status, String::from_utf8_lossy(&out.stderr));
        Ok(())
    }
}

#[test]
fn r1_s12_01_batch_run_and_direct_workdir_reject_stored_external_link_escapes() {
    let dirs = TempDirPair::new("r1-s12-01");
    let bin = env!("CARGO_BIN_EXE_curvebook-cli");

    let secret_csv = dirs.outside.join("secret.csv");
    fs::write(&secret_csv, "999.0,secret_token_r1_s12\n").expect("write outside secret");

    let inside_csv = dirs.workdir.join("inside_data.csv");
    fs::write(&inside_csv, "77.5\n").expect("write inside csv");

    let linked_abs = dirs.workdir.join("linked_abs.curvebook");
    save_project_with_external_link(&linked_abs, &secret_csv.display().to_string());

    let linked_rel = dirs.workdir.join("linked_rel.curvebook");
    save_project_with_external_link(&linked_rel, "../outside/secret.csv");

    let mut attack_archives = vec![("abs", &linked_abs), ("rel", &linked_rel)];

    #[cfg(unix)]
    let linked_sym = {
        use std::os::unix::fs::symlink;
        symlink(&secret_csv, dirs.workdir.join("sym_secret.csv")).expect("symlink outside");
        let p = dirs.workdir.join("linked_sym.curvebook");
        save_project_with_external_link(&p, "sym_secret.csv");
        p
    };
    #[cfg(unix)]
    attack_archives.push(("sym", &linked_sym));

    let wd_str = dirs.workdir.display().to_string();

    // 1. `batch --workdir` must fail with exit code 2 and never export secret contents.
    for (label, archive) in &attack_archives {
        let out_csv = dirs.workdir.join(format!("out_{label}.csv"));
        let manifest_path = dirs.workdir.join(format!("manifest_{label}.json"));
        let archive_name = archive.file_name().expect("filename").to_string_lossy().into_owned();
        let manifest = json!({
            "schema": "curvebook.batch.v1",
            "project": archive_name,
            "commands": [{ "cmd": "project.recalculate", "params": { "include_external_links": true } }],
            "exports": [{ "format": "csv", "sheet": "d1", "out": format!("out_{label}.csv") }]
        });
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).expect("json")).expect("write manifest");

        let out =
            Command::new(bin).args(["batch", &manifest_path.display().to_string(), "--workdir", &wd_str, "--json"]).output().expect("run batch");
        assert!(!out.status.success(), "batch ({label}) should refuse escaping external link");
        assert_eq!(out.status.code(), Some(2), "expected exit code 2 for batch ({label})");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("path_escape"), "expected path_escape error for batch ({label}): {stdout}");
        assert!(!stdout.contains("secret_token_r1_s12"), "secret leaked into stdout for batch ({label})");
        assert!(!out_csv.exists(), "batch export should not be written when recalculate fails ({label})");
    }

    // 2. `run --workdir` opening + recalculating stored external link escape must also fail with exit code 2.
    let run_script = dirs.workdir.join("script.jsonl");
    let lines = format!(
        "{}\n{}\n",
        json!({ "cmd": "project.open", "params": { "path": "linked_abs.curvebook" } }),
        json!({ "cmd": "project.recalculate", "params": { "include_external_links": true } })
    );
    fs::write(&run_script, lines).expect("write run script");
    let run_out = Command::new(bin).args(["run", &run_script.display().to_string(), "--workdir", &wd_str, "--json"]).output().expect("run script");
    assert_eq!(run_out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&run_out.stdout).contains("path_escape"));

    // 3. Direct command `project.recalculate --in ... --workdir` must fail with exit code 2.
    let direct_out = Command::new(bin)
        .args(["project.recalculate", "--in", "linked_rel.curvebook", "--workdir", &wd_str, "--json", "--include_external_links", "true"])
        .output()
        .expect("direct project.recalculate");
    assert_eq!(direct_out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&direct_out.stdout).contains("path_escape"));

    // 4. Stored external link strictly inside `--workdir` succeeds cleanly and exports refreshed data.
    let linked_ok = dirs.workdir.join("linked_ok.curvebook");
    save_project_with_external_link(&linked_ok, "inside_data.csv");
    let manifest_ok = json!({
        "schema": "curvebook.batch.v1",
        "project": "linked_ok.curvebook",
        "commands": [{ "cmd": "project.recalculate", "params": { "include_external_links": true } }],
        "exports": [{ "format": "csv", "sheet": "d1", "out": "out_ok.csv" }]
    });
    let manifest_ok_path = dirs.workdir.join("manifest_ok.json");
    fs::write(&manifest_ok_path, serde_json::to_vec_pretty(&manifest_ok).expect("json")).expect("write ok manifest");
    let ok_out = Command::new(bin)
        .args(["batch", &manifest_ok_path.display().to_string(), "--workdir", &wd_str, "--json"])
        .output()
        .expect("run inside batch");
    assert!(ok_out.status.success(), "inside batch failed: {}", String::from_utf8_lossy(&ok_out.stderr));
    let exported_csv = fs::read_to_string(dirs.workdir.join("out_ok.csv")).expect("read ok export");
    assert!(exported_csv.contains("77.5"), "expected refreshed inside link value in CSV: {exported_csv}");
}

#[test]
fn r1_x1_04_mcp_workdir_enforces_containment_across_all_surfaces() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook-cli");
    let dirs = TempDirPair::new("r1-x1-04");

    let outside_csv = dirs.outside.join("secret.csv");
    fs::write(&outside_csv, "X,Y\n1,999\n")?;
    let inside_csv = dirs.workdir.join("inside.csv");
    fs::write(&inside_csv, "X,Y\n1,25\n2,50\n")?;

    let linked_escape = dirs.workdir.join("linked_escape.curvebook");
    save_project_with_external_link(&linked_escape, &outside_csv.display().to_string());

    // 1. CLI `--help` documents `--workdir <dir>` while preserving `--gui-only` substring.
    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout)?;
    assert!(help_text.contains("mcp [--control-port <port>] [--gui-only] [--workdir <dir>]"), "unexpected --help:\n{help_text}");

    // 2. Non-existent or non-directory `--workdir` exits cleanly with policy/usage exit code 2.
    let missing_wd = dirs.root.join("no-such-dir").display().to_string();
    let bad_missing = Command::new(bin).args(["mcp", "--workdir", &missing_wd]).output()?;
    assert_eq!(bad_missing.status.code(), Some(2));
    let bad_file = Command::new(bin).args(["mcp", "--workdir", &inside_csv.display().to_string()]).output()?;
    assert_eq!(bad_file.status.code(), Some(2));

    // 3. End-to-end stdio JSON-RPC session with `--workdir`.
    let wd_str = dirs.workdir.display().to_string();
    let mut mcp = McpChild::spawn(&["mcp", "--workdir", &wd_str])?;
    let init = mcp.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "x1-04-test", "version": "0.1.0" }
        }),
    )?;
    assert!(init.get("error").is_none());
    mcp.notify("notifications/initialized", json!({}))?;

    // Open & save outside workdir rejected with `path_escape`.
    let open_rel = mcp.call_tool(2, "open_file", json!({ "path": "../outside/secret.csv" }))?;
    assert_eq!(open_rel["result"]["isError"], json!(true));
    assert_eq!(open_rel["result"]["structuredContent"]["code"], json!("path_escape"));

    let open_abs = mcp.call_tool(3, "open_file", json!({ "path": outside_csv.display().to_string() }))?;
    assert_eq!(open_abs["result"]["isError"], json!(true));
    assert_eq!(open_abs["result"]["structuredContent"]["code"], json!("path_escape"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&outside_csv, dirs.workdir.join("sym_outside.csv"))?;
        let open_sym = mcp.call_tool(4, "open_file", json!({ "path": "sym_outside.csv" }))?;
        assert_eq!(open_sym["result"]["isError"], json!(true));
        assert_eq!(open_sym["result"]["structuredContent"]["code"], json!("path_escape"));
    }

    // Stored external link in an inside archive still blocked during `project.recalculate`.
    let open_linked = mcp.call_tool(5, "open_file", json!({ "path": "linked_escape.curvebook" }))?;
    assert_eq!(open_linked["result"]["isError"], json!(false));
    let recalc_err = mcp.call_tool(6, "command_run", json!({ "command_id": "project.recalculate", "params": { "include_external_links": true } }))?;
    assert_eq!(recalc_err["result"]["isError"], json!(true));
    assert_eq!(recalc_err["result"]["structuredContent"]["code"], json!("path_escape"));

    // Command write escape & export escape blocked.
    let cmd_save_esc =
        mcp.call_tool(7, "command_run", json!({ "command_id": "project.save", "params": { "path": "../outside/escape.curvebook" } }))?;
    assert_eq!(cmd_save_esc["result"]["isError"], json!(true));
    assert_eq!(cmd_save_esc["result"]["structuredContent"]["code"], json!("path_escape"));
    assert!(!dirs.outside.join("escape.curvebook").exists());

    // Valid inside operations succeed.
    let open_ok = mcp.call_tool(8, "open_file", json!({ "path": "inside.csv" }))?;
    assert_eq!(open_ok["result"]["isError"], json!(false));
    let export_ok = mcp.call_tool(9, "export", json!({ "format": "csv", "output_path": "nested/out.csv" }))?;
    assert_eq!(export_ok["result"]["isError"], json!(false));
    let save_ok = mcp.call_tool(10, "save_file", json!({ "path": "nested/saved.curvebook" }))?;
    assert_eq!(save_ok["result"]["isError"], json!(false));
    assert!(dirs.workdir.join("nested/out.csv").exists());
    assert!(dirs.workdir.join("nested/saved.curvebook").exists());

    mcp.finish()?;
    Ok(())
}
