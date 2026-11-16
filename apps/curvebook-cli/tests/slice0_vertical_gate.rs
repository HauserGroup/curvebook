//! Roadmap Slice 0 Vertical Gate integration suite driving the compiled `curvebook-cli` binary.
#![allow(clippy::cast_precision_loss)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_testkit::metrics::log_relative_error;
use curvebook_testkit::nist_strd::{default_nist_strd_dir, load_corpus_nonlinear};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

type GateResult = Result<(), String>;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), seq));
        fs::create_dir_all(&dir).map_err(|e| format!("create temp dir `{}`: {e}", dir.display()))?;
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

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| format!("non-UTF-8 path: {}", p.display()))
}

fn run_cli(args: &[&str], stdin_bytes: Option<&[u8]>) -> Result<Output, String> {
    let mut cmd = Command::new(cli_bin());
    cmd.args(args);
    if stdin_bytes.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn cli: {e}"))?;
    if let Some(bytes) = stdin_bytes {
        let mut stdin = child.stdin.take().ok_or_else(|| String::from("failed to take child stdin"))?;
        stdin.write_all(bytes).map_err(|e| format!("write child stdin: {e}"))?;
    }
    child.wait_with_output().map_err(|e| format!("wait child: {e}"))
}

fn run_cli_ok(args: &[&str], stdin_bytes: Option<&[u8]>) -> Result<String, String> {
    let out = run_cli(args, stdin_bytes)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.code() != Some(0) {
        return Err(format!("CLI failed for {args:?} (code={:?})\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status.code()));
    }
    Ok(stdout)
}

fn write_jsonl_lines(path: &Path, lines: &[Value]) -> GateResult {
    let mut text = String::new();
    for line in lines {
        let line_json = serde_json::to_string(line).map_err(|e| format!("serialize JSONL line: {e}"))?;
        text.push_str(&line_json);
        text.push('\n');
    }
    fs::write(path, text).map_err(|e| format!("write `{}`: {e}", path.display()))
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read `{}`: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("parse JSON `{}`: {e}", path.display()))
}

fn eval_4pl(x: f64, bottom: f64, top: f64, log_ec50: f64, hill: f64) -> f64 {
    let p = 10.0_f64.powf((log_ec50 - x) * hill);
    bottom + (top - bottom) / (1.0 + p)
}

fn grad_4pl(x: f64, bottom: f64, top: f64, log_ec50: f64, hill: f64) -> [f64; 4] {
    let p = 10.0_f64.powf((log_ec50 - x) * hill);
    let sig = 1.0 / (1.0 + p);
    let common = (top - bottom) * (-(sig * sig)) * p * std::f64::consts::LN_10;
    [1.0 - sig, sig, common * hill, common * (log_ec50 - x)]
}

fn invert_4x4(mut a: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut inv = [[0.0_f64; 4]; 4];
    for (i, row) in inv.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for col in 0..4 {
        let mut pivot_row = col;
        for r in (col + 1)..4 {
            if a[r][col].abs() > a[pivot_row][col].abs() {
                pivot_row = r;
            }
        }
        a.swap(col, pivot_row);
        inv.swap(col, pivot_row);
        let div = a[col][col];
        for j in 0..4 {
            a[col][j] /= div;
            inv[col][j] /= div;
        }
        for r in 0..4 {
            if r != col {
                let factor = a[r][col];
                for j in 0..4 {
                    a[r][j] -= factor * a[col][j];
                    inv[r][j] -= factor * inv[col][j];
                }
            }
        }
    }
    inv
}

fn extract_result_scalar(doc: &Value, key: &str) -> Result<f64, String> {
    doc["scalars"]
        .as_array()
        .ok_or_else(|| String::from("result missing scalars array"))?
        .iter()
        .find(|entry| entry["key"].as_str() == Some(key))
        .and_then(|entry| entry["value"].as_f64())
        .ok_or_else(|| format!("missing scalar `{key}` in result JSON"))
}

fn extract_tab<'a>(doc: &'a Value, id: &str) -> Result<&'a Value, String> {
    doc["tabs"]
        .as_array()
        .ok_or_else(|| String::from("result missing tabs array"))?
        .iter()
        .find(|tab| tab["id"].as_str() == Some(id))
        .ok_or_else(|| format!("missing tab `{id}` in result JSON"))
}

fn extract_cell_number(cell: &Value) -> Result<f64, String> {
    cell["Number"]["value"].as_f64().or_else(|| cell["value"].as_f64()).ok_or_else(|| format!("expected numeric ResultCell, got {cell}"))
}

fn extract_cell_ci(cell: &Value) -> Result<(f64, f64), String> {
    let ci = cell.get("ConfidenceInterval").ok_or_else(|| format!("expected ConfidenceInterval cell, got {cell}"))?;
    let lo = ci["lower"]["Finite"].as_f64().ok_or_else(|| format!("expected Finite lower CI in {cell}"))?;
    let hi = ci["upper"]["Finite"].as_f64().ok_or_else(|| format!("expected Finite upper CI in {cell}"))?;
    Ok((lo, hi))
}

fn tab_row_index(tab: &Value, label: &str) -> Result<usize, String> {
    tab["row_labels"]
        .as_array()
        .ok_or_else(|| format!("tab {} missing row_labels", tab["id"]))?
        .iter()
        .position(|v| v.as_str() == Some(label))
        .ok_or_else(|| format!("missing row label `{label}` in tab {}", tab["id"]))
}

fn check_approx(actual: f64, expected: f64, tol: f64, label: &str) -> GateResult {
    let diff = (actual - expected).abs();
    if diff <= tol { Ok(()) } else { Err(format!("{label}: actual={actual:.12e}, expected={expected:.12e}, diff={diff:.3e} > tol={tol:.3e}")) }
}

fn check_lre(actual: f64, certified: f64, min_lre: f64, label: &str) -> GateResult {
    let lre = log_relative_error(actual, certified);
    if lre >= min_lre {
        Ok(())
    } else {
        Err(format!("{label}: LRE={lre:.2} < required {min_lre:.2} (actual={actual:.12e}, certified={certified:.12e})"))
    }
}

fn check_json_floats_bit_identical(a: &Value, b: &Value, path: &str) -> GateResult {
    match (a, b) {
        (Value::Null, Value::Null) => Ok(()),
        (Value::Bool(x), Value::Bool(y)) if x == y => Ok(()),
        (Value::Bool(x), Value::Bool(y)) => Err(format!("bool mismatch at {path}: {x} vs {y}")),
        (Value::String(x), Value::String(y)) if x == y => Ok(()),
        (Value::String(x), Value::String(y)) => Err(format!("string mismatch at {path}: {x} vs {y}")),
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                if xf.to_bits() == yf.to_bits() { Ok(()) } else { Err(format!("float bit mismatch at {path}: {xf} vs {yf}")) }
            } else if x == y {
                Ok(())
            } else {
                Err(format!("number mismatch at {path}: {x} vs {y}"))
            }
        }
        (Value::Array(xs), Value::Array(ys)) => {
            if xs.len() != ys.len() {
                return Err(format!("array len mismatch at {path}: {} vs {}", xs.len(), ys.len()));
            }
            for (idx, (xv, yv)) in xs.iter().zip(ys.iter()).enumerate() {
                check_json_floats_bit_identical(xv, yv, &format!("{path}[{idx}]"))?;
            }
            Ok(())
        }
        (Value::Object(xm), Value::Object(ym)) => {
            if xm.len() != ym.len() {
                return Err(format!("object key count mismatch at {path}"));
            }
            for (k, xv) in xm {
                let yv = ym.get(k).ok_or_else(|| format!("missing key `{k}` at {path}"))?;
                check_json_floats_bit_identical(xv, yv, &format!("{path}.{k}"))?;
            }
            Ok(())
        }
        _ => Err(format!("JSON type mismatch at {path}: {a} vs {b}")),
    }
}

#[test]
fn slice0_end_to_end_dose_response_vertical_gate() -> GateResult {
    let work = TempDir::new("cb_slice0_gate")?;
    let workdir_str = path_str(work.path())?;

    // 1. Exact analytical 4PL dose-response ground truth: 8 log-spaced concentrations x 3 symmetric replicates.
    let bottom_true = 12.0_f64;
    let top_true = 98.0_f64;
    let log_ec50_true = -6.5_f64;
    let hill_true = 1.25_f64;
    let ec50_true = 10.0_f64.powf(log_ec50_true);
    let xs = [-9.5_f64, -8.5, -7.5, -6.8, -6.2, -5.5, -4.5, -3.5];
    let offsets = [-1.5_f64, 0.0, 1.5];

    let mut cell_entries = Vec::with_capacity(xs.len() * offsets.len());
    let mut y_vals = Vec::with_capacity(xs.len() * offsets.len());
    for (row, &x) in xs.iter().enumerate() {
        let mu = eval_4pl(x, bottom_true, top_true, log_ec50_true, hill_true);
        for (subcol, &delta) in offsets.iter().enumerate() {
            let y = mu + delta;
            y_vals.push(y);
            cell_entries.push(json!({ "row": row, "col": 0, "subcol": subcol, "value": y }));
        }
    }

    // Independent analytical quantities: J^T J, asymptotic SEs, R^2, AICc.
    let mut jtj = [[0.0_f64; 4]; 4];
    for &x in &xs {
        let g = grad_4pl(x, bottom_true, top_true, log_ec50_true, hill_true);
        for i in 0..4 {
            for j in 0..4 {
                jtj[i][j] += 3.0 * g[i] * g[j];
            }
        }
    }
    let inv_jtj = invert_4x4(jtj);
    let n_obs = y_vals.len() as f64;
    let df_true = n_obs - 4.0;
    let rss_true = 8.0 * (offsets[0] * offsets[0] + offsets[2] * offsets[2]);
    let s2_true = rss_true / df_true;
    let se_true =
        [(s2_true * inv_jtj[0][0]).sqrt(), (s2_true * inv_jtj[1][1]).sqrt(), (s2_true * inv_jtj[2][2]).sqrt(), (s2_true * inv_jtj[3][3]).sqrt()];
    let y_mean = y_vals.iter().sum::<f64>() / n_obs;
    let sst_true: f64 = y_vals.iter().map(|y| (y - y_mean).powi(2)).sum();
    let r2_true = 1.0 - (rss_true / sst_true);
    let k_aicc = 5.0_f64;
    let aicc_true = n_obs * (rss_true / n_obs).ln() + 2.0 * k_aicc + (2.0 * k_aicc * (k_aicc + 1.0)) / (n_obs - k_aicc - 1.0);
    let t_crit_20 = 2.085_963_447_265_837_f64;

    // 2-4. Drive CLI run with JSONL (`cmd`, `command`, and `id`+`method` forms all covered!).
    let script1_path = work.path().join("step1_build_and_export.jsonl");
    write_jsonl_lines(
        &script1_path,
        &[
            json!({
                "cmd": "sheet.createData",
                "params": {
                    "sheet_id": "dr_xy",
                    "title": "Agonist 4PL Dose Response",
                    "kind": "xy",
                    "rows": 8,
                    "cols": 1,
                    "replicates": 3,
                    "subcolumn_mode": "replicate",
                    "x_title": "Log[Agonist] (M)",
                    "auto_graph": true,
                    "graph_sheet_id": "dr_graph"
                }
            }),
            json!({ "command": "table.setXCells", "params": { "sheet_id": "dr_xy", "values": xs } }),
            json!({ "id": "set-y", "method": "table.setCells", "params": { "sheet_id": "dr_xy", "cells": cell_entries } }),
            json!({
                "cmd": "fit.run",
                "params": {
                    "source_sheet": "dr_xy",
                    "sheet_id": "dr_fit",
                    "equation_id": "log_agonist_vs_response_4pl",
                    "weighting": "none",
                    "confidence_percent": 95.0
                }
            }),
            json!({ "cmd": "graph.initialize", "params": { "sheet_id": "dr_graph", "kind": "xy", "title": "Agonist 4PL Curve" } }),
            json!({
                "cmd": "graph.setAxes",
                "params": {
                    "sheet_id": "dr_graph",
                    "x_axis": { "title": "Log10[Agonist] (M)" },
                    "y_axis": { "title": "Biological Response (%)" }
                }
            }),
            json!({
                "cmd": "graph.setSeries",
                "params": {
                    "sheet_id": "dr_graph",
                    "series_index": 0,
                    "color": "#1B9E77",
                    "error_bars": { "metric": "sem", "direction": "both" }
                }
            }),
            json!({
                "cmd": "graph.setRegressionOverlay",
                "params": {
                    "sheet_id": "dr_graph",
                    "result_sheet_id": "dr_fit",
                    "series_index": 0,
                    "sample_segments": 256,
                    "show_confidence_band": false
                }
            }),
            json!({ "cmd": "analysis.exportJson", "params": { "sheet_id": "dr_fit", "path": "before_fit.json", "pretty": true } }),
            json!({ "cmd": "export.svg", "params": { "sheet_id": "dr_graph", "path": "before.svg" } }),
            json!({ "cmd": "export.pdf", "params": { "sheet_id": "dr_graph", "path": "before.pdf" } }),
            json!({ "cmd": "project.save", "params": { "path": "dose_response.curvebook" } }),
        ],
    )?;

    let run1_stdout = run_cli_ok(&["run", path_str(&script1_path)?, "--workdir", workdir_str, "--json"], None)?;
    let run1_json: Value = serde_json::from_str(&run1_stdout).map_err(|e| format!("parse step1 JSON: {e}"))?;
    assert_eq!(run1_json["ok"], json!(true));
    assert_eq!(run1_json["commands_executed"], json!(12));

    let before_fit = read_json_file(&work.path().join("before_fit.json"))?;
    let est_bottom = extract_result_scalar(&before_fit, "Bottom")?;
    let est_top = extract_result_scalar(&before_fit, "Top")?;
    let est_log_ec50 = extract_result_scalar(&before_fit, "LogEC50")?;
    let est_hill = extract_result_scalar(&before_fit, "HillSlope")?;
    let est_ec50 = extract_result_scalar(&before_fit, "EC50")?;
    let est_r2 = extract_result_scalar(&before_fit, "R2")?;
    let est_rss = extract_result_scalar(&before_fit, "RSS")?;

    check_approx(est_bottom, bottom_true, 1e-8, "Bottom")?;
    check_approx(est_top, top_true, 1e-8, "Top")?;
    check_approx(est_log_ec50, log_ec50_true, 1e-8, "LogEC50")?;
    check_approx(est_hill, hill_true, 1e-8, "HillSlope")?;
    check_approx(est_ec50, ec50_true, 1e-15, "EC50")?;
    check_approx(est_r2, r2_true, 1e-9, "R^2")?;
    check_approx(est_rss, rss_true, 1e-8, "RSS")?;

    let curve_pts = before_fit["fitted_curves"][0]["points"].as_array().ok_or_else(|| String::from("fitted_curves[0].points missing"))?;
    assert!(curve_pts.len() >= 100, "smooth fitted curve should have dense samples (got {})", curve_pts.len());

    let param_tab = extract_tab(&before_fit, "nonlinear_parameters")?;
    let expected_names = ["Bottom", "Top", "LogEC50", "HillSlope"];
    let expected_vals = [bottom_true, top_true, log_ec50_true, hill_true];
    for (idx, (&name, &truth)) in expected_names.iter().zip(expected_vals.iter()).enumerate() {
        let r = tab_row_index(param_tab, name)?;
        let row = &param_tab["rows"][r];
        let tab_est = extract_cell_number(&row[0])?;
        let tab_se = extract_cell_number(&row[1])?;
        let (ci_lo, ci_hi) = extract_cell_ci(&row[2])?;
        check_approx(tab_est, truth, 1e-8, &format!("tab estimate {name}"))?;
        check_approx(tab_se, se_true[idx], 1e-6, &format!("asymptotic SE {name}"))?;
        assert!(ci_lo < tab_est && tab_est < ci_hi, "{name} CI ({ci_lo}, {ci_hi}) must bracket {tab_est}");
        let asym_half = t_crit_20 * se_true[idx];
        check_approx(tab_est - ci_lo, asym_half, 0.25 * asym_half, &format!("{name} 95% CI lower half"))?;
        check_approx(ci_hi - tab_est, asym_half, 0.25 * asym_half, &format!("{name} 95% CI upper half"))?;
    }

    let gof_tab = extract_tab(&before_fit, "nonlinear_gof")?;
    let gof_r2 = extract_cell_number(&gof_tab["rows"][tab_row_index(gof_tab, "R squared")?][0])?;
    let gof_aicc = extract_cell_number(&gof_tab["rows"][tab_row_index(gof_tab, "AICc")?][0])?;
    let gof_df = extract_cell_number(&gof_tab["rows"][tab_row_index(gof_tab, "Degrees of Freedom")?][0])?;
    check_approx(gof_df, df_true, 1e-12, "GOF DF")?;
    check_approx(gof_r2, r2_true, 1e-9, "GOF R squared")?;
    check_approx(gof_aicc, aicc_true, 1e-8, "GOF AICc")?;

    // Check SVG and PDF outputs.
    let before_svg = fs::read(work.path().join("before.svg")).map_err(|e| format!("read before.svg: {e}"))?;
    let before_svg_str = std::str::from_utf8(&before_svg).map_err(|e| format!("SVG UTF-8: {e}"))?;
    assert!(before_svg_str.contains("<svg"), "valid SVG root required");
    assert!(before_svg_str.contains("Log10[Agonist] (M)"), "X axis title present in SVG");
    assert!(before_svg_str.contains("Biological Response (%)"), "Y axis title present in SVG");
    assert!(before_svg_str.to_ascii_lowercase().contains("#1b9e77"), "custom series color #1B9E77 present in SVG");
    assert!(before_svg_str.contains("<path"), "fitted curve & geometric primitives must appear in SVG");

    let before_pdf = fs::read(work.path().join("before.pdf")).map_err(|e| format!("read before.pdf: {e}"))?;
    assert!(before_pdf.starts_with(b"%PDF-"), "PDF magic header required");
    assert!(before_pdf.len() > 1000, "non-trivial PDF payload required");

    // 5-6. Reopen .curvebook in FRESH CLI processes (`info`, `convert`, and `run`) and check exact identity.
    let archive_path = work.path().join("dose_response.curvebook");
    let archive_arg = path_str(&archive_path)?;
    let info_stdout = run_cli_ok(&["info", archive_arg, "--json"], None)?;
    let info_json: Value = serde_json::from_str(&info_stdout).map_err(|e| format!("parse info JSON: {e}"))?;
    assert_eq!(info_json["counts"]["tables"], json!(1));
    assert_eq!(info_json["counts"]["analyses"], json!(1));
    assert_eq!(info_json["counts"]["graphs"], json!(1));

    let after_convert_svg_path = work.path().join("after_convert.svg");
    let after_convert_fit_path = work.path().join("after_convert_fit.json");
    run_cli_ok(&["convert", archive_arg, path_str(&after_convert_svg_path)?, "--sheet", "dr_graph"], None)?;
    run_cli_ok(&["convert", archive_arg, path_str(&after_convert_fit_path)?, "--sheet", "dr_fit"], None)?;

    let script2_path = work.path().join("step2_reopen_inspect.jsonl");
    write_jsonl_lines(
        &script2_path,
        &[
            json!({ "cmd": "project.open", "params": { "path": "dose_response.curvebook" } }),
            json!({ "cmd": "analysis.exportJson", "params": { "sheet_id": "dr_fit", "path": "after_run_fit.json", "pretty": true } }),
            json!({ "cmd": "export.svg", "params": { "sheet_id": "dr_graph", "path": "after_run.svg" } }),
        ],
    )?;
    run_cli_ok(&["run", path_str(&script2_path)?, "--workdir", workdir_str], None)?;

    let after_convert_fit = read_json_file(&after_convert_fit_path)?;
    let after_run_fit = read_json_file(&work.path().join("after_run_fit.json"))?;
    check_json_floats_bit_identical(&before_fit, &after_convert_fit, "before_vs_after_convert_fit")?;
    check_json_floats_bit_identical(&before_fit, &after_run_fit, "before_vs_after_run_fit")?;

    let after_convert_svg = fs::read(&after_convert_svg_path).map_err(|e| format!("read after_convert.svg: {e}"))?;
    let after_run_svg = fs::read(work.path().join("after_run.svg")).map_err(|e| format!("read after_run.svg: {e}"))?;
    assert_eq!(before_svg, after_convert_svg, "convert SVG must be byte-for-byte identical after save/reopen");
    assert_eq!(before_svg, after_run_svg, "run SVG must be byte-for-byte identical after save/reopen");

    // 7. Live reactivity: edit one replicate Y value -> EC50 & SVG curve update while custom styling stays intact!
    let script3_path = work.path().join("step3_reactive_edit.jsonl");
    write_jsonl_lines(
        &script3_path,
        &[
            json!({ "cmd": "project.open", "params": { "path": "dose_response.curvebook" } }),
            json!({
                "cmd": "table.setCells",
                "params": { "sheet_id": "dr_xy", "cells": [{ "row": 2, "col": 0, "subcol": 1, "value": 92.0 }] }
            }),
            json!({ "cmd": "analysis.exportJson", "params": { "sheet_id": "dr_fit", "path": "reactive_fit.json", "pretty": true } }),
            json!({ "cmd": "export.svg", "params": { "sheet_id": "dr_graph", "path": "reactive.svg" } }),
        ],
    )?;
    run_cli_ok(&["run", path_str(&script3_path)?, "--workdir", workdir_str], None)?;

    let reactive_fit = read_json_file(&work.path().join("reactive_fit.json"))?;
    let new_ec50 = extract_result_scalar(&reactive_fit, "EC50")?;
    assert!((new_ec50 - est_ec50).abs() > 1e-8, "EC50 must update reactively ({new_ec50} vs {est_ec50})");

    let reactive_svg = fs::read(work.path().join("reactive.svg")).map_err(|e| format!("read reactive.svg: {e}"))?;
    assert_ne!(before_svg, reactive_svg, "SVG curve must update after reactive cell edit");
    let reactive_svg_str = std::str::from_utf8(&reactive_svg).map_err(|e| format!("reactive SVG UTF-8: {e}"))?;
    assert!(reactive_svg_str.contains("Log10[Agonist] (M)"), "X axis title must survive live reactivity");
    assert!(reactive_svg_str.contains("Biological Response (%)"), "Y axis title must survive live reactivity");
    assert!(reactive_svg_str.to_ascii_lowercase().contains("#1b9e77"), "custom color #1B9E77 must survive live reactivity");

    Ok(())
}

#[test]
fn nist_strd_eckerle4_and_rat43_through_cli() -> GateResult {
    let dir = default_nist_strd_dir();
    if !curvebook_testkit::nist_strd::corpus_available(&dir) {
        return Ok(());
    }

    let eckerle = load_corpus_nonlinear(&dir, "Eckerle4").map_err(|e| format!("load Eckerle4: {e}"))?;
    let work = TempDir::new("cb_slice0_nist")?;
    let workdir_str = path_str(work.path())?;

    let x_vals: Vec<f64> = eckerle.predictors.iter().map(|row| row[0]).collect();
    let y_cells: Vec<Value> = eckerle.y.iter().enumerate().map(|(row, &y)| json!({ "row": row, "col": 0, "subcol": 0, "value": y })).collect();

    let script_path = work.path().join("eckerle4.jsonl");
    write_jsonl_lines(
        &script_path,
        &[
            json!({
                "cmd": "sheet.createData",
                "params": { "sheet_id": "eck_xy", "title": "NIST Eckerle4", "kind": "xy", "rows": eckerle.y.len(), "cols": 1 }
            }),
            json!({ "cmd": "table.setXCells", "params": { "sheet_id": "eck_xy", "values": x_vals } }),
            json!({ "cmd": "table.setCells", "params": { "sheet_id": "eck_xy", "cells": y_cells } }),
            json!({
                "cmd": "fit.run",
                "params": {
                    "source_sheet": "eck_xy",
                    "sheet_id": "eck_fit",
                    "equation_id": "nist_eckerle4",
                    "weighting": "none",
                    "confidence_percent": 95.0
                }
            }),
            json!({ "cmd": "analysis.exportJson", "params": { "sheet_id": "eck_fit", "path": "eckerle4_fit.json", "pretty": true } }),
        ],
    )?;
    run_cli_ok(&["run", path_str(&script_path)?, "--workdir", workdir_str], None)?;

    let fit_json = read_json_file(&work.path().join("eckerle4_fit.json"))?;
    let param_tab = extract_tab(&fit_json, "nonlinear_parameters")?;
    for (idx, (&cert_b, &cert_se)) in eckerle.certified_betas.iter().zip(eckerle.certified_se_betas.iter()).enumerate() {
        let name = format!("b{}", idx + 1);
        let r = tab_row_index(param_tab, &name)?;
        let est = extract_cell_number(&param_tab["rows"][r][0])?;
        let se = extract_cell_number(&param_tab["rows"][r][1])?;
        check_lre(est, cert_b, 6.5, &format!("CLI Eckerle4 {name}"))?;
        check_lre(se, cert_se, 5.0, &format!("CLI Eckerle4 SE({name})"))?;
    }
    let rss = extract_result_scalar(&fit_json, "RSS")?;
    check_lre(rss, eckerle.certified_rss, 7.0, "CLI Eckerle4 RSS")?;

    let rat43 = load_corpus_nonlinear(&dir, "Rat43").map_err(|e| format!("load Rat43: {e}"))?;
    let rat_x: Vec<f64> = rat43.predictors.iter().map(|row| row[0]).collect();
    let rat_cells: Vec<Value> = rat43.y.iter().enumerate().map(|(row, &y)| json!({ "row": row, "col": 0, "subcol": 0, "value": y })).collect();
    let rat_rules: Vec<Value> =
        rat43.start_2.iter().enumerate().map(|(i, &v)| json!({ "name": format!("b{}", i + 1), "scope": "local", "initial_override": v })).collect();

    let rat_script = work.path().join("rat43.jsonl");
    write_jsonl_lines(
        &rat_script,
        &[
            json!({
                "cmd": "sheet.createData",
                "params": { "sheet_id": "rat_xy", "title": "NIST Rat43", "kind": "xy", "rows": rat43.y.len(), "cols": 1 }
            }),
            json!({ "cmd": "table.setXCells", "params": { "sheet_id": "rat_xy", "values": rat_x } }),
            json!({ "cmd": "table.setCells", "params": { "sheet_id": "rat_xy", "cells": rat_cells } }),
            json!({
                "cmd": "fit.run",
                "params": {
                    "source_sheet": "rat_xy",
                    "sheet_id": "rat_fit",
                    "equation_id": "nist_rat43",
                    "weighting": "none",
                    "parameter_rules": rat_rules
                }
            }),
            json!({ "cmd": "analysis.exportJson", "params": { "sheet_id": "rat_fit", "path": "rat43_fit.json", "pretty": true } }),
        ],
    )?;
    run_cli_ok(&["run", path_str(&rat_script)?, "--workdir", workdir_str], None)?;

    let rat_json = read_json_file(&work.path().join("rat43_fit.json"))?;
    let rat_tab = extract_tab(&rat_json, "nonlinear_parameters")?;
    for (idx, (&cert_b, &cert_se)) in rat43.certified_betas.iter().zip(rat43.certified_se_betas.iter()).enumerate() {
        let name = format!("b{}", idx + 1);
        let r = tab_row_index(rat_tab, &name)?;
        let est = extract_cell_number(&rat_tab["rows"][r][0])?;
        let se = extract_cell_number(&rat_tab["rows"][r][1])?;
        check_lre(est, cert_b, 6.5, &format!("CLI Rat43 {name}"))?;
        check_lre(se, cert_se, 5.0, &format!("CLI Rat43 SE({name})"))?;
    }
    let rat_rss = extract_result_scalar(&rat_json, "RSS")?;
    check_lre(rat_rss, rat43.certified_rss, 7.0, "CLI Rat43 RSS")?;

    Ok(())
}

#[test]
fn cli_batch_template_convert_perf_bench_mcp_and_exit_codes() -> GateResult {
    let work = TempDir::new("cb_slice0_batch")?;
    let outside = TempDir::new("cb_slice0_outside")?;
    let workdir_str = path_str(work.path())?;

    // 1. Save a reusable `.cbtemplate` via `run` and inspect it via `info`.
    let tpl_script = work.path().join("make_template.jsonl");
    write_jsonl_lines(
        &tpl_script,
        &[
            json!({ "cmd": "sheet.createData", "params": { "sheet_id": "raw", "title": "Plate Data", "kind": "xy", "rows": 4, "cols": 1, "auto_info": true, "info_sheet_id": "info_1" } }),
            json!({ "cmd": "graph.create", "params": { "sheet_id": "g_1", "source_sheet_id": "raw", "title": "Batch Dose Curve" } }),
            json!({ "cmd": "graph.initialize", "params": { "sheet_id": "g_1", "kind": "xy" } }),
            json!({ "cmd": "project.saveTemplate", "params": { "path": "tpl/plate.cbtemplate" } }),
        ],
    )?;
    run_cli_ok(&["run", path_str(&tpl_script)?, "--workdir", workdir_str], None)?;
    let tpl_info_out = run_cli_ok(&["info", path_str(&work.path().join("tpl/plate.cbtemplate"))?, "--json"], None)?;
    let tpl_info: Value = serde_json::from_str(&tpl_info_out).map_err(|e| format!("parse tpl info JSON: {e}"))?;
    assert_eq!(tpl_info["format"], json!("cbtemplate"));

    // 2. Write CSV & execute `curvebook.batch.v1` manifest against that `.cbtemplate`.
    fs::create_dir_all(work.path().join("inputs")).map_err(|e| format!("create inputs: {e}"))?;
    let csv_path = work.path().join("inputs/dose.csv");
    fs::write(&csv_path, "LogConc,Response\n-9.0,10.2\n-8.0,12.5\n-7.0,32.0\n-6.5,55.0\n-6.0,78.0\n-5.0,96.5\n-4.0,99.1\n")
        .map_err(|e| format!("write csv: {e}"))?;

    let manifest_path = work.path().join("batch.json");
    let manifest = json!({
        "schema": "curvebook.batch.v1",
        "template": "tpl/plate.cbtemplate",
        "inputs": [
            { "sheet": "raw", "csv": "inputs/dose.csv", "kind": "xy", "subcolumn_mode": "single" }
        ],
        "commands": [
            { "cmd": "info.setConstant", "params": { "name": "Lot", "value": "A17" } },
            { "cmd": "fit.run", "params": { "source_sheet": "raw", "sheet_id": "fit_1", "equation_id": "log_agonist_vs_response_4pl" } },
            { "cmd": "graph.setRegressionOverlay", "params": { "sheet_id": "g_1", "result_sheet_id": "fit_1", "series_index": 0 } }
        ],
        "exports": [
            { "out": "out/project.curvebook" },
            { "sheet": "g_1", "format": "svg", "out": "out/curve.svg" },
            { "sheet": "g_1", "format": "pdf", "out": "out/curve.pdf" },
            { "sheet": "g_1", "format": "png", "out": "out/curve.png", "dpi": 150 },
            { "sheet": "g_1", "format": "tiff", "out": "out/curve.tiff", "dpi": 150 },
            { "sheet": "fit_1", "format": "json", "out": "out/fit.json" },
            { "sheet": "raw", "format": "tsv", "out": "out/raw.tsv" }
        ]
    });
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| format!("serialize manifest: {e}"))?;
    fs::write(&manifest_path, manifest_text).map_err(|e| format!("write manifest: {e}"))?;

    let batch_out = run_cli_ok(&["batch", path_str(&manifest_path)?, "--workdir", workdir_str, "--json"], None)?;
    let batch_json: Value = serde_json::from_str(&batch_out).map_err(|e| format!("parse batch JSON: {e}"))?;
    assert_eq!(batch_json["ok"], json!(true));
    assert_eq!(batch_json["exports_written"], json!(7));
    assert!(fs::read(work.path().join("out/curve.pdf")).map_err(|e| format!("read pdf: {e}"))?.starts_with(b"%PDF-"));
    assert!(fs::read(work.path().join("out/curve.png")).map_err(|e| format!("read png: {e}"))?.starts_with(b"\x89PNG"));

    // 3. Test `bench` & `convert` & `mcp` stdio handshake.
    let bench_out = run_cli_ok(&["bench", path_str(&work.path().join("out/project.curvebook"))?, "--json"], None)?;
    let bench_json: Value = serde_json::from_str(&bench_out).map_err(|e| format!("parse bench JSON: {e}"))?;
    assert_eq!(bench_json["ok"], json!(true));
    assert!(bench_json["scenes_compiled"].as_u64().unwrap_or(0) >= 1);

    let csv_to_curvebook = work.path().join("out/from_csv.curvebook");
    run_cli_ok(&["convert", path_str(&csv_path)?, path_str(&csv_to_curvebook)?], None)?;
    assert!(csv_to_curvebook.exists());

    let mcp_req = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
    let mcp_out = run_cli_ok(&["mcp"], Some(mcp_req))?;
    assert!(mcp_out.contains("\"id\":1") && mcp_out.contains("\"id\":2"), "MCP stdio reply missing expected IDs: {mcp_out}");

    // 4. Test `--workdir` containment (parent `..`, absolute escape, symlink escape -> exit code 2).
    let escape_target = outside.path().join("pwned.curvebook");
    let escape_rel = work.path().join("escape_rel.jsonl");
    write_jsonl_lines(&escape_rel, &[json!({ "cmd": "project.save", "params": { "path": "../escaped.curvebook" } })])?;
    let out_rel = run_cli(&["run", path_str(&escape_rel)?, "--workdir", workdir_str], None)?;
    assert_eq!(out_rel.status.code(), Some(2), "parent traversal `..` must return exit code 2");

    let escape_abs = work.path().join("escape_abs.jsonl");
    write_jsonl_lines(&escape_abs, &[json!({ "cmd": "project.save", "params": { "path": escape_target } })])?;
    let out_abs = run_cli(&["run", path_str(&escape_abs)?, "--workdir", workdir_str], None)?;
    assert_eq!(out_abs.status.code(), Some(2), "absolute escape outside workdir must return exit code 2");
    assert!(!escape_target.exists(), "must never create files outside --workdir");

    #[cfg(unix)]
    {
        let link_path = work.path().join("outside_link");
        std::os::unix::fs::symlink(outside.path(), &link_path).map_err(|e| format!("create test symlink: {e}"))?;
        let escape_symlink = work.path().join("escape_sym.jsonl");
        write_jsonl_lines(&escape_symlink, &[json!({ "cmd": "project.save", "params": { "path": "outside_link/sym.curvebook" } })])?;
        let out_sym = run_cli(&["run", path_str(&escape_symlink)?, "--workdir", workdir_str], None)?;
        assert_eq!(out_sym.status.code(), Some(2), "symlink escape outside workdir must return exit code 2");
        assert!(!outside.path().join("sym.curvebook").exists());
    }

    let escape_manifest = work.path().join("escape_manifest.json");
    let bad_manifest = json!({
        "schema": "curvebook.batch.v1",
        "exports": [{ "out": "../../escape.svg", "sheet": "g_1" }]
    });
    fs::write(&escape_manifest, serde_json::to_string(&bad_manifest).map_err(|e| format!("json: {e}"))?).map_err(|e| format!("write: {e}"))?;
    let out_batch_esc = run_cli(&["batch", path_str(&escape_manifest)?, "--workdir", workdir_str], None)?;
    assert_eq!(out_batch_esc.status.code(), Some(2), "batch export escape must return exit code 2");

    // 5. Invalid schema -> exit code 2, missing file -> exit code 1, `perf --json` -> exit code 0.
    let bad_schema = work.path().join("bad_schema.json");
    fs::write(&bad_schema, r#"{"schema":"wrong.v0"}"#).map_err(|e| format!("write bad schema: {e}"))?;
    let out_schema = run_cli(&["batch", path_str(&bad_schema)?], None)?;
    assert_eq!(out_schema.status.code(), Some(2), "invalid manifest schema must return exit code 2");

    let out_missing = run_cli(&["info", path_str(&work.path().join("missing.curvebook"))?], None)?;
    assert_eq!(out_missing.status.code(), Some(1), "missing file runtime failure must return exit code 1");

    let perf_out = run_cli_ok(&["perf", "--json"], None)?;
    let perf_json: Value = serde_json::from_str(&perf_out).map_err(|e| format!("parse perf JSON: {e}"))?;
    assert_eq!(perf_json["ok"], json!(true));
    assert_eq!(perf_json["checks"]["mv_100k_calculated_variable"]["within_budget"], json!(true));
    assert_eq!(perf_json["checks"]["dose_response_96_curves"]["curves"], json!(96));

    Ok(())
}
