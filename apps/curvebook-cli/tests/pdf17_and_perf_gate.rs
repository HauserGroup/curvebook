#![forbid(unsafe_code)]
//! Integration tests for CLI PDF 1.7 / 1.4 export, `--ttf-font` embedding,
//! `WorkdirGuard` font path containment, and G12 `< 50 ms` 96-curve / `< 250 ms` 100k MV perf gates.

#[path = "common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{CliHarness, TempDir};
use curvebook_text::{build_minimal_truetype_font, parse_truetype_font_info};
use serde_json::{Value, json};

type TestResult = Result<(), String>;

fn path_str(p: &Path) -> Result<&str, String> {
    CliHarness::path_str(p)
}

fn run_cli(args: &[&str]) -> Result<std::process::Output, String> {
    CliHarness::run(args)
}

fn run_cli_ok(args: &[&str]) -> Result<String, String> {
    CliHarness::run_ok(args, None)
}

fn create_sample_archive(work: &Path) -> Result<PathBuf, String> {
    let csv_path = work.join("dose.csv");
    fs::write(&csv_path, "LogConc,Response\n-9.0,10.2\n-8.0,14.5\n-7.0,34.0\n-6.0,78.0\n-5.0,96.5\n").map_err(|e| format!("write csv: {e}"))?;
    let archive_path = work.join("sample.curvebook");
    run_cli_ok(&["convert", path_str(&csv_path)?, path_str(&archive_path)?])?;
    Ok(archive_path)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn convert_pdf_version_defaults_to_1_7_and_supports_1_4() -> TestResult {
    let work = TempDir::new("cb_pdf_version")?;
    let archive_path = create_sample_archive(work.path())?;
    let archive_arg = path_str(&archive_path)?;

    let default_pdf = work.path().join("default.pdf");
    let default_out = run_cli_ok(&["convert", archive_arg, path_str(&default_pdf)?, "--json"])?;
    let default_json: Value = serde_json::from_str(&default_out).map_err(|e| format!("parse json: {e}"))?;
    assert_eq!(default_json["ok"], json!(true));
    assert_eq!(default_json["format"], json!("pdf"));
    assert_eq!(default_json["pdf_version"], json!("1.7"));
    let default_bytes = fs::read(&default_pdf).map_err(|e| format!("read default.pdf: {e}"))?;
    assert!(default_bytes.starts_with(b"%PDF-1.7\n"), "default PDF export must emit %PDF-1.7 header");
    assert!(default_bytes.ends_with(b"%%EOF\n"), "PDF must end with %%EOF");

    let pdf14_path = work.path().join("v14.pdf");
    let pdf14_out = run_cli_ok(&["convert", archive_arg, path_str(&pdf14_path)?, "--pdf-version", "1.4", "--json"])?;
    let pdf14_json: Value = serde_json::from_str(&pdf14_out).map_err(|e| format!("parse json: {e}"))?;
    assert_eq!(pdf14_json["pdf_version"], json!("1.4"));
    let pdf14_bytes = fs::read(&pdf14_path).map_err(|e| format!("read v14.pdf: {e}"))?;
    assert!(pdf14_bytes.starts_with(b"%PDF-1.4\n"), "--pdf-version 1.4 must emit %PDF-1.4 header");

    let pdf17_eq_path = work.path().join("v17_eq.pdf");
    run_cli_ok(&["convert", archive_arg, path_str(&pdf17_eq_path)?, "--pdf-version=1.7"])?;
    let pdf17_eq_bytes = fs::read(&pdf17_eq_path).map_err(|e| format!("read v17_eq.pdf: {e}"))?;
    assert!(pdf17_eq_bytes.starts_with(b"%PDF-1.7\n"), "--pdf-version=1.7 must emit %PDF-1.7 header");

    let bad_pdf_path = work.path().join("bad_ver.pdf");
    let bad_ver = run_cli(&["convert", archive_arg, path_str(&bad_pdf_path)?, "--pdf-version", "1.5"])?;
    assert_eq!(bad_ver.status.code(), Some(2), "unsupported --pdf-version must return exit code 2");
    assert!(!bad_pdf_path.exists(), "failed convert must not write output PDF");

    Ok(())
}

#[test]
fn convert_ttf_font_embeds_fontfile2_and_rejects_invalid_sfnt() -> TestResult {
    let work = TempDir::new("cb_ttf_embed")?;
    let archive_path = create_sample_archive(work.path())?;
    let archive_arg = path_str(&archive_path)?;

    let ttf_bytes = build_minimal_truetype_font();
    let info = parse_truetype_font_info(&ttf_bytes).map_err(|e| format!("parse minimal ttf: {e}"))?;
    let ttf_path = work.path().join("minimal.ttf");
    fs::write(&ttf_path, &ttf_bytes).map_err(|e| format!("write minimal.ttf: {e}"))?;

    let embedded_pdf = work.path().join("embedded.pdf");
    let out = run_cli_ok(&["convert", archive_arg, path_str(&embedded_pdf)?, "--ttf-font", path_str(&ttf_path)?, "--json"])?;
    let out_json: Value = serde_json::from_str(&out).map_err(|e| format!("parse json: {e}"))?;
    assert_eq!(out_json["ok"], json!(true));
    assert_eq!(out_json["ttf_font_embedded"], json!(true));

    let pdf_bytes = fs::read(&embedded_pdf).map_err(|e| format!("read embedded.pdf: {e}"))?;
    assert!(pdf_bytes.starts_with(b"%PDF-1.7\n"));
    assert!(contains_bytes(&pdf_bytes, b"/Subtype /TrueType"), "embedded PDF must declare /Subtype /TrueType");
    assert!(
        contains_bytes(&pdf_bytes, format!("/BaseFont /{}", info.postscript_name).as_bytes()),
        "embedded PDF must name custom SFNT PostScript font"
    );
    assert!(contains_bytes(&pdf_bytes, b"/FontDescriptor"), "embedded PDF must include /FontDescriptor");
    assert!(contains_bytes(&pdf_bytes, b"/FontFile2"), "embedded PDF must include /FontFile2");
    assert!(
        contains_bytes(&pdf_bytes, format!("/Length1 {}", ttf_bytes.len()).as_bytes()),
        "embedded /FontFile2 must record exact uncompressed SFNT /Length1"
    );

    // Invalid SFNT bytes, empty file, OTTO CFF SFNT, and missing path must all fail cleanly (exit 1).
    let bad_ttf = work.path().join("corrupt.ttf");
    fs::write(&bad_ttf, b"not-a-valid-sfnt").map_err(|e| format!("write corrupt.ttf: {e}"))?;
    let bad_out = work.path().join("should_not_exist_1.pdf");
    let bad_res = run_cli(&["convert", archive_arg, path_str(&bad_out)?, "--ttf-font", path_str(&bad_ttf)?])?;
    assert_eq!(bad_res.status.code(), Some(1), "corrupt SFNT must fail with exit code 1");
    assert!(!bad_out.exists());

    let empty_ttf = work.path().join("empty.ttf");
    fs::write(&empty_ttf, b"").map_err(|e| format!("write empty.ttf: {e}"))?;
    let empty_out = work.path().join("should_not_exist_2.pdf");
    let empty_res = run_cli(&["convert", archive_arg, path_str(&empty_out)?, "--ttf-font", path_str(&empty_ttf)?])?;
    assert_eq!(empty_res.status.code(), Some(1), "empty SFNT must fail with exit code 1");
    assert!(!empty_out.exists());

    let mut otto_bytes = ttf_bytes;
    otto_bytes[0..4].copy_from_slice(b"OTTO");
    let otto_ttf = work.path().join("cff_otto.ttf");
    fs::write(&otto_ttf, &otto_bytes).map_err(|e| format!("write cff_otto.ttf: {e}"))?;
    let otto_out = work.path().join("should_not_exist_3.pdf");
    let otto_res = run_cli(&["convert", archive_arg, path_str(&otto_out)?, "--ttf-font", path_str(&otto_ttf)?])?;
    assert_eq!(otto_res.status.code(), Some(1), "OTTO CFF SFNT must fail cleanly for --ttf-font");
    assert!(!otto_out.exists());

    let missing_out = work.path().join("should_not_exist_4.pdf");
    let missing_res = run_cli(&["convert", archive_arg, path_str(&missing_out)?, "--ttf-font", path_str(&work.path().join("missing.ttf"))?])?;
    assert_eq!(missing_res.status.code(), Some(1), "missing --ttf-font path must fail with exit code 1");
    assert!(!missing_out.exists());

    let empty_flag_res = run_cli(&["convert", archive_arg, path_str(&missing_out)?, "--ttf-font="])?;
    assert_eq!(empty_flag_res.status.code(), Some(2), "empty --ttf-font= flag must return usage exit code 2");

    Ok(())
}

#[test]
fn batch_workdir_guard_blocks_outside_ttf_and_supports_pdf_options() -> TestResult {
    let work = TempDir::new("cb_batch_ttf_work")?;
    let outside = TempDir::new("cb_batch_ttf_outside")?;
    let workdir_str = path_str(work.path())?;

    let ttf_bytes = build_minimal_truetype_font();
    let info = parse_truetype_font_info(&ttf_bytes).map_err(|e| format!("parse minimal ttf: {e}"))?;
    fs::create_dir_all(work.path().join("inputs")).map_err(|e| format!("create inputs: {e}"))?;
    fs::create_dir_all(work.path().join("fonts")).map_err(|e| format!("create fonts: {e}"))?;
    fs::write(work.path().join("inputs/dose.csv"), "LogConc,Response\n-9.0,10.0\n-7.0,50.0\n-5.0,92.0\n").map_err(|e| format!("write csv: {e}"))?;
    fs::write(work.path().join("fonts/inside.ttf"), &ttf_bytes).map_err(|e| format!("write inside.ttf: {e}"))?;
    fs::write(work.path().join("fonts/bad.ttf"), b"bad-font-bytes").map_err(|e| format!("write bad.ttf: {e}"))?;
    let outside_ttf = outside.path().join("outside.ttf");
    fs::write(&outside_ttf, &ttf_bytes).map_err(|e| format!("write outside.ttf: {e}"))?;

    // 1. WorkdirGuard must block `../outside.ttf` with exit code 2.
    let escape_manifest = work.path().join("escape_rel.json");
    let escape_doc = json!({
        "schema": "curvebook.batch.v1",
        "inputs": [{ "csv": "inputs/dose.csv", "kind": "xy" }],
        "exports": [{ "format": "pdf", "out": "out/escaped.pdf", "ttf_font": "../outside.ttf" }]
    });
    fs::write(&escape_manifest, serde_json::to_string(&escape_doc).map_err(|e| format!("json: {e}"))?).map_err(|e| format!("write: {e}"))?;
    let esc_res = run_cli(&["batch", path_str(&escape_manifest)?, "--workdir", workdir_str])?;
    assert_eq!(esc_res.status.code(), Some(2), "batch `../outside.ttf` must be blocked with exit code 2");
    assert!(!work.path().join("out/escaped.pdf").exists());

    // 2. WorkdirGuard must block absolute outside `ttf_font` paths too.
    let abs_manifest = work.path().join("escape_abs.json");
    let abs_doc = json!({
        "schema": "curvebook.batch.v1",
        "inputs": [{ "csv": "inputs/dose.csv", "kind": "xy" }],
        "exports": [{ "format": "pdf", "out": "out/escaped_abs.pdf", "ttf_font": outside_ttf }]
    });
    fs::write(&abs_manifest, serde_json::to_string(&abs_doc).map_err(|e| format!("json: {e}"))?).map_err(|e| format!("write: {e}"))?;
    let abs_res = run_cli(&["batch", path_str(&abs_manifest)?, "--workdir", workdir_str])?;
    assert_eq!(abs_res.status.code(), Some(2), "batch absolute outside ttf_font must be blocked with exit code 2");

    // 3. Valid in-workdir `pdf_version` + `ttf_font` export succeeds cleanly.
    let valid_manifest = work.path().join("valid.json");
    let valid_doc = json!({
        "schema": "curvebook.batch.v1",
        "inputs": [{ "sheet": "raw", "csv": "inputs/dose.csv", "kind": "xy" }],
        "commands": [
            { "cmd": "graph.create", "params": { "sheet_id": "g_1", "data_sheet_id": "raw", "title": "Batch PDF Curve" } },
            { "cmd": "graph.initialize", "params": { "sheet_id": "g_1", "kind": "xy" } }
        ],
        "exports": [
            { "sheet": "g_1", "format": "pdf", "out": "out/fig_v14_ttf.pdf", "pdf_version": "1.4", "ttf_font": "fonts/inside.ttf" },
            { "sheet": "g_1", "format": "pdf", "out": "out/fig_default.pdf" }
        ]
    });
    fs::write(&valid_manifest, serde_json::to_string(&valid_doc).map_err(|e| format!("json: {e}"))?).map_err(|e| format!("write: {e}"))?;
    let valid_out = run_cli_ok(&["batch", path_str(&valid_manifest)?, "--workdir", workdir_str, "--json"])?;
    let valid_json: Value = serde_json::from_str(&valid_out).map_err(|e| format!("parse batch json: {e}"))?;
    assert_eq!(valid_json["ok"], json!(true));
    assert_eq!(valid_json["exports_written"], json!(2));

    let v14_pdf = fs::read(work.path().join("out/fig_v14_ttf.pdf")).map_err(|e| format!("read fig_v14_ttf.pdf: {e}"))?;
    assert!(v14_pdf.starts_with(b"%PDF-1.4\n"));
    assert!(contains_bytes(&v14_pdf, b"/FontFile2"));
    assert!(contains_bytes(&v14_pdf, format!("/BaseFont /{}", info.postscript_name).as_bytes()));
    assert!(contains_bytes(&v14_pdf, format!("/Length1 {}", ttf_bytes.len()).as_bytes()));

    let def_pdf = fs::read(work.path().join("out/fig_default.pdf")).map_err(|e| format!("read fig_default.pdf: {e}"))?;
    assert!(def_pdf.starts_with(b"%PDF-1.7\n"));

    // 4. Corrupt in-workdir `ttf_font` fails cleanly with exit code 1.
    let bad_manifest = work.path().join("bad_ttf.json");
    let bad_doc = json!({
        "schema": "curvebook.batch.v1",
        "inputs": [{ "sheet": "raw", "csv": "inputs/dose.csv", "kind": "xy" }],
        "commands": [
            { "cmd": "graph.create", "params": { "sheet_id": "g_1", "data_sheet_id": "raw", "title": "Batch PDF Curve" } },
            { "cmd": "graph.initialize", "params": { "sheet_id": "g_1", "kind": "xy" } }
        ],
        "exports": [{ "sheet": "g_1", "format": "pdf", "out": "out/bad_ttf.pdf", "ttf_font": "fonts/bad.ttf" }]
    });
    fs::write(&bad_manifest, serde_json::to_string(&bad_doc).map_err(|e| format!("json: {e}"))?).map_err(|e| format!("write: {e}"))?;
    let bad_res = run_cli(&["batch", path_str(&bad_manifest)?, "--workdir", workdir_str])?;
    assert_eq!(bad_res.status.code(), Some(1), "corrupt in-workdir ttf_font must fail with exit code 1");
    assert!(!work.path().join("out/bad_ttf.pdf").exists());

    Ok(())
}

#[test]
fn perf_gate_passes_100k_mv_and_96_curve_budgets() -> TestResult {
    let out = run_cli_ok(&["perf", "--json"])?;
    let report: Value = serde_json::from_str(&out).map_err(|e| format!("parse perf json: {e}"))?;
    assert_eq!(report["ok"], json!(true));

    let mv = &report["checks"]["mv_100k_calculated_variable"];
    assert_eq!(mv["rows"], json!(100_000));
    assert_eq!(mv["budget_ms"], json!(250.0));
    assert_eq!(mv["within_budget"], json!(true));
    let eval_ms = mv["eval_ms"].as_f64().ok_or_else(|| String::from("missing eval_ms"))?;
    assert!(eval_ms < 250.0, "100k MV eval_ms {eval_ms} exceeded 250 ms budget");

    let curves = &report["checks"]["dose_response_96_curves"];
    assert_eq!(curves["curves"], json!(96));
    assert_eq!(curves["budget_ms"], json!(50.0));
    assert_eq!(curves["within_budget"], json!(true));
    let fit_ms = curves["fit_ms"].as_f64().ok_or_else(|| String::from("missing fit_ms"))?;
    let total_ms = curves["total_ms"].as_f64().ok_or_else(|| String::from("missing total_ms"))?;
    assert!(fit_ms <= 50.0, "96-curve 4PL fit_ms {fit_ms} exceeded 50 ms budget");
    assert!(total_ms.is_finite() && total_ms >= fit_ms, "total_ms {total_ms} must cover fit_ms {fit_ms}");

    Ok(())
}
