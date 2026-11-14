//! `lumencraft-cli bench <file>` and synthetic `lumencraft-cli perf` benchmarks.

use std::path::Path;
use std::time::Instant;

use lumencraft_chart::{ChartScene, compile_graph, compile_layout};
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, GraphSheet, MultipleVariablesTable, NonlinearFitSpec, ProjectDocument, RegressionOverlaySpec,
    ResultSheet, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use lumencraft_engine::Session;
use lumencraft_format::{ArchiveKind, open_archive_bytes, save_project_bytes};
use lumencraft_pdf::{PdfExportOptions, render_pdf};
use lumencraft_render::{RenderTargetSpec, encode_png, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
use lumencraft_text::{measure_text_pt, parse_markup};
use serde_json::{Value, json};

use crate::cmd_convert::load_project_from_path;
use crate::error::CliError;

const PERF_MV_ROWS: usize = 100_000;
const PERF_MV_BUDGET_MS: f64 = 250.0;
const PERF_BATCH_CURVES: usize = 96;
const PERF_96_CURVE_BUDGET_MS: f64 = 50.0;
const PERF_RASTER_DPI: u16 = 300;
const MAX_RASTER_EDGE_PX: f64 = 16_384.0;

struct SingleFlagArgs {
    file: Option<String>,
    json: bool,
}

/// Execute `lumencraft-cli bench <file> [--json]`.
pub fn run_bench(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_bench_args(args, true, "usage: lumencraft-cli bench <file> [--json]")?;
    let Some(file) = parsed.file else {
        return Err(CliError::Usage(String::from("usage: lumencraft-cli bench <file> [--json]")));
    };
    let path = Path::new(&file);

    let t_parse = Instant::now();
    let (loaded, input_bytes) = load_project_from_path(path, false)?;
    let parse_ms = elapsed_ms(t_parse);

    let t_recalc = Instant::now();
    let mut session = Session::from_document(loaded.doc);
    session.recalculate_all()?;
    let recalc_ms = elapsed_ms(t_recalc);
    let doc = session.document();

    let t_compile = Instant::now();
    let scenes = compile_project_scenes(doc)?;
    let compile_ms = elapsed_ms(t_compile);

    let mut svg_ms = 0.0;
    let mut pdf_ms = 0.0;
    let mut png_ms = 0.0;
    let mut total_svg_bytes = 0_usize;
    let mut total_pdf_bytes = 0_usize;
    let mut total_png_bytes = 0_usize;

    for scene in &scenes {
        let t_svg = Instant::now();
        let svg = render_svg(scene, &SvgExportOptions::default())?;
        svg_ms += elapsed_ms(t_svg);
        total_svg_bytes += svg.len();

        let t_pdf = Instant::now();
        let pdf = render_pdf(scene, &PdfExportOptions::default())?;
        pdf_ms += elapsed_ms(t_pdf);
        total_pdf_bytes += pdf.len();

        let t_png = Instant::now();
        let png = rasterize_and_encode_png(scene, PERF_RASTER_DPI)?;
        png_ms += elapsed_ms(t_png);
        total_png_bytes += png.len();
    }

    let report = json!({
        "ok": true,
        "file": file,
        "format": loaded.format_tag,
        "input_bytes": input_bytes,
        "tables": doc.data_sheets.len(),
        "analyses": doc.result_sheets.len(),
        "graphs": doc.graph_sheets.len(),
        "layouts": doc.layout_sheets.len(),
        "scenes_compiled": scenes.len(),
        "timings_ms": {
            "parse": parse_ms,
            "recalc": recalc_ms,
            "compile": compile_ms,
            "svg": svg_ms,
            "pdf": pdf_ms,
            "png": png_ms,
            "total": parse_ms + recalc_ms + compile_ms + svg_ms + pdf_ms + png_ms,
        },
        "output_bytes": {
            "svg": total_svg_bytes,
            "pdf": total_pdf_bytes,
            "png": total_png_bytes,
        }
    });

    emit_report(&report, parsed.json, || {
        println!(
            "bench `{file}` ({}, {} bytes): parse={parse_ms:.2}ms recalc={recalc_ms:.2}ms compile={compile_ms:.2}ms ({} scene(s)) svg={svg_ms:.2}ms pdf={pdf_ms:.2}ms png={png_ms:.2}ms",
            loaded.format_tag,
            input_bytes,
            scenes.len()
        );
    })
}

/// Execute `lumencraft-cli perf [--json]` covering synthetic G12 performance checks.
pub fn run_perf(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_bench_args(args, false, "usage: lumencraft-cli perf [--json]")?;

    let mv_bench = bench_mv_100k_calculated_variable()?;
    let (batch_bench, batch_doc, batch_scene) = bench_96_curve_dose_response()?;
    let archive_bench = bench_lumencraft_archive_roundtrip(&batch_doc)?;
    let export_bench = bench_exports(&batch_scene)?;

    let (report, budget_result) = build_perf_report(mv_bench, batch_bench, archive_bench, export_bench);
    let mv_check = &report["checks"]["mv_100k_calculated_variable"];
    let curve_check = &report["checks"]["dose_response_96_curves"];
    let arch_check = &report["checks"]["archive_roundtrip"];
    let exp_check = &report["checks"]["exports"];

    emit_report(&report, parsed.json, || {
        println!(
            "G12 perf results:\n  (a) 100k MV CalcVar: {} ms (budget < {PERF_MV_BUDGET_MS:.0} ms, within_budget={})\n  (b) 96-curve 4PL fit + compile: fit={} ms (budget <= {PERF_96_CURVE_BUDGET_MS:.0} ms, within_budget={}), compile={} ms, total={} ms (total_within_budget={})\n  (c) .lumencraft save/reopen: save={} ms, open={} ms, total={} ms ({} bytes)\n  (d) exports ({} DPI): svg={} ms ({} B), pdf={} ms ({} B), png={} ms ({} B)",
            mv_check["eval_ms"],
            mv_check["within_budget"],
            curve_check["fit_ms"],
            curve_check["within_budget"],
            curve_check["compile_ms"],
            curve_check["total_ms"],
            curve_check["total_within_budget"],
            arch_check["save_ms"],
            arch_check["open_ms"],
            arch_check["total_ms"],
            arch_check["archive_bytes"],
            PERF_RASTER_DPI,
            exp_check["svg_ms"],
            exp_check["svg_bytes"],
            exp_check["pdf_ms"],
            exp_check["pdf_bytes"],
            exp_check["png_ms"],
            exp_check["png_bytes"],
        );
    })?;

    budget_result
}

fn build_perf_report(mv_bench: Value, batch_bench: Value, archive_bench: Value, export_bench: Value) -> (Value, Result<(), CliError>) {
    let mv_within = mv_bench["within_budget"].as_bool().unwrap_or(false);
    let curve_within = batch_bench["within_budget"].as_bool().unwrap_or(false);
    let all_within_budget = mv_within && curve_within;

    let report = json!({
        "ok": all_within_budget,
        "checks": {
            "mv_100k_calculated_variable": mv_bench,
            "dose_response_96_curves": batch_bench,
            "archive_roundtrip": archive_bench,
            "exports": export_bench,
        }
    });

    let status = if all_within_budget {
        Ok(())
    } else {
        Err(CliError::Runtime(format!(
            "G12 performance budget exceeded (mv_100k within_budget={mv_within}, dose_response_96_curves within_budget={curve_within})"
        )))
    };

    (report, status)
}

fn bench_mv_100k_calculated_variable() -> Result<Value, CliError> {
    let mut mv = MultipleVariablesTable::new(4, PERF_MV_ROWS)?;
    mv.variables[0].title = String::from("Dose_uM");
    mv.variables[1].title = String::from("Signal");
    mv.variables[2].title = String::from("Baseline");
    mv.variables[3].title = String::from("LogResponse");

    for r in 0..PERF_MV_ROWS {
        let rf = f64_from_usize(r + 1);
        mv.variables[0].cells[r] = Cell::number(0.001 * rf);
        mv.variables[1].cells[r] = Cell::number(10.0 + 90.0 * (f64_from_usize(r % 200) / 200.0));
        mv.variables[2].cells[r] = Cell::number(2.0 + 0.25 * f64_from_usize(r % 10));
    }
    mv.variables[3].set_calculated_formula("=log10(Signal - Baseline + 1.0) * Dose_uM");

    let ds = DataSheet::new(SheetId::new("mv_100k"), "MV 100k", TableArchetype::MultipleVariables(mv))?;
    let mut doc = ProjectDocument::new("Perf MV 100k")?;
    doc.add_data_sheet(ds)?;
    let mut session = Session::from_document(doc);

    let t0 = Instant::now();
    session.recalculate_all()?;
    let eval_ms = elapsed_ms(t0);

    let evaluated_ok = match &session.document().data_sheets[0].archetype {
        TableArchetype::MultipleVariables(table) => {
            table.variables[3].cells.first().and_then(|c| c.value.finite_f64()).is_some()
                && table.variables[3].cells.last().and_then(|c| c.value.finite_f64()).is_some()
        }
        _ => false,
    };
    if !evaluated_ok {
        return Err(CliError::Runtime(String::from("100k MV calculated variable produced non-finite output")));
    }

    Ok(json!({
        "rows": PERF_MV_ROWS,
        "eval_ms": eval_ms,
        "budget_ms": PERF_MV_BUDGET_MS,
        "within_budget": eval_ms < PERF_MV_BUDGET_MS,
    }))
}

fn bench_96_curve_dose_response() -> Result<(Value, ProjectDocument, ChartScene), CliError> {
    let log_concs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), PERF_BATCH_CURVES, log_concs.len())?;
    xy.x_title = String::from("Log[Agonist] (M)");

    for (r, &lx) in log_concs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(lx);
        for c in 0..PERF_BATCH_CURVES {
            let bottom = 5.0 + f64_from_usize(c % 4);
            let top = 95.0 + 2.0 * f64_from_usize(c % 5);
            let log_ec50 = -7.2 + 0.015 * f64_from_usize(c);
            let hill = 1.0 + 0.08 * f64_from_usize(c % 4);
            let mean_y = bottom + (top - bottom) / (1.0 + 10.0_f64.powf((log_ec50 - lx) * hill));
            let jitter = [-0.6, 0.0, 0.6];
            for (rep, &jit) in jitter.iter().enumerate() {
                xy.data_sets[c].cells[r][rep] = Cell::number(mean_y + jit);
            }
        }
    }

    let ds = DataSheet::new(SheetId::new("dr_96"), "96-Well Dose Response", TableArchetype::Xy(xy))?;
    let mut fit_spec = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit_spec.profile_likelihood_ci = false;

    let res_id = SheetId::new("fit_96");
    let analysis = AnalysisSpec::new(res_id.clone(), ds.id.clone(), AnalysisKind::NonlinearRegression(fit_spec))?;
    let res = ResultSheet::new(analysis)?;

    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph_96"), &ds)?;
    graph.initialized = true;
    graph.spec.y_axis.title = String::from("Normalized Response (%)");
    let overlay = RegressionOverlaySpec {
        result_sheet_id: res_id,
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    };
    for s in &mut graph.spec.series {
        s.regression_overlay = Some(overlay.clone());
    }

    let mut doc = ProjectDocument::new("Perf 96-Curve 4PL")?;
    doc.add_data_sheet(ds)?;
    doc.add_result_sheet(res)?;
    doc.add_graph_sheet(graph)?;

    let mut session = Session::from_document(doc);
    let t_fit = Instant::now();
    session.recalculate_all()?;
    let fit_ms = elapsed_ms(t_fit);

    let fitted_doc = session.document().clone();
    let fitted_res = fitted_doc.result_sheets.first();
    let fitted_curves = fitted_res.map_or(0, |r| r.fitted_curves.len());
    if fitted_curves != PERF_BATCH_CURVES {
        let status_desc = fitted_res
            .map_or_else(|| String::from("missing result sheet"), |r| format!("{:?} (tabs={}, scalars={})", r.status, r.tabs.len(), r.scalars.len()));
        return Err(CliError::Runtime(format!(
            "expected {PERF_BATCH_CURVES} fitted curves in perf batch, got {fitted_curves}; status={status_desc}"
        )));
    }

    let t_compile = Instant::now();
    let scene = compile_graph(&fitted_doc.graph_sheets[0], &fitted_doc)?;
    let compile_ms = elapsed_ms(t_compile);
    let total_ms = ((fit_ms + compile_ms) * 1_000.0).round() / 1_000.0;
    let fit_within_budget = fit_ms <= PERF_96_CURVE_BUDGET_MS;
    let total_within_budget = total_ms <= PERF_96_CURVE_BUDGET_MS;

    let summary = json!({
        "curves": fitted_curves,
        "fit_ms": fit_ms,
        "compile_ms": compile_ms,
        "total_ms": total_ms,
        "budget_ms": PERF_96_CURVE_BUDGET_MS,
        "within_budget": fit_within_budget,
        "total_within_budget": total_within_budget,
        "primitives": scene.primitives.len(),
    });
    Ok((summary, fitted_doc, scene))
}

fn bench_lumencraft_archive_roundtrip(doc: &ProjectDocument) -> Result<Value, CliError> {
    let t_save = Instant::now();
    let archive_bytes = save_project_bytes(doc)?;
    let save_ms = elapsed_ms(t_save);

    let t_open = Instant::now();
    let (kind, reopened) = open_archive_bytes(&archive_bytes)?;
    let open_ms = elapsed_ms(t_open);

    if kind != ArchiveKind::Project
        || reopened.data_sheets.len() != doc.data_sheets.len()
        || reopened.result_sheets.len() != doc.result_sheets.len()
        || reopened.graph_sheets.len() != doc.graph_sheets.len()
    {
        return Err(CliError::Runtime(String::from("archive round-trip structural mismatch")));
    }

    Ok(json!({
        "archive_bytes": archive_bytes.len(),
        "save_ms": save_ms,
        "open_ms": open_ms,
        "total_ms": save_ms + open_ms,
    }))
}

fn bench_exports(scene: &ChartScene) -> Result<Value, CliError> {
    let t_text = Instant::now();
    let rich = parse_markup("<b>Log[Agonist]</b> (EC<sub>50</sub>, HillSlope = 1.0)");
    let metrics = measure_text_pt(&rich, 11.0);
    let text_ms = elapsed_ms(t_text);
    if metrics.width_pt <= 0.0 {
        return Err(CliError::Runtime(String::from("text measurement returned zero width")));
    }

    let t_svg = Instant::now();
    let svg = render_svg(scene, &SvgExportOptions::default())?;
    let svg_ms = elapsed_ms(t_svg);

    let t_pdf = Instant::now();
    let pdf = render_pdf(scene, &PdfExportOptions::default())?;
    let pdf_ms = elapsed_ms(t_pdf);

    let t_png = Instant::now();
    let png = rasterize_and_encode_png(scene, PERF_RASTER_DPI)?;
    let png_ms = elapsed_ms(t_png);

    Ok(json!({
        "dpi": PERF_RASTER_DPI,
        "text_ms": text_ms,
        "svg_ms": svg_ms,
        "svg_bytes": svg.len(),
        "pdf_ms": pdf_ms,
        "pdf_bytes": pdf.len(),
        "png_ms": png_ms,
        "png_bytes": png.len(),
        "total_ms": svg_ms + pdf_ms + png_ms,
    }))
}

fn compile_project_scenes(doc: &ProjectDocument) -> Result<Vec<ChartScene>, CliError> {
    let mut scenes = Vec::new();
    let any_initialized = doc.graph_sheets.iter().any(|g| g.initialized);
    for g in &doc.graph_sheets {
        if !any_initialized || g.initialized {
            scenes.push(compile_graph(g, doc)?);
        }
    }
    for l in &doc.layout_sheets {
        scenes.push(compile_layout(l, doc)?);
    }
    Ok(scenes)
}

fn rasterize_and_encode_png(scene: &ChartScene, dpi: u16) -> Result<Vec<u8>, CliError> {
    let scale = f64::from(dpi) / 72.0;
    let width_px = clamped_px(scene.width_pt.max(1.0) * scale);
    let height_px = clamped_px(scene.height_pt.max(1.0) * scale);
    let img = render_scene(scene, RenderTargetSpec { width_px, height_px, dpi })?;
    Ok(encode_png(&img)?)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamped_px(raw: f64) -> u32 {
    raw.round().clamp(1.0, MAX_RASTER_EDGE_PX) as u32
}

#[allow(clippy::cast_precision_loss)]
fn f64_from_usize(v: usize) -> f64 {
    v as f64
}

fn elapsed_ms(start: Instant) -> f64 {
    (start.elapsed().as_secs_f64() * 1_000.0 * 1_000.0).round() / 1_000.0
}

fn emit_report(report: &Value, as_json: bool, print_human: impl FnOnce()) -> Result<(), CliError> {
    if as_json {
        let text = serde_json::to_string_pretty(report).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
        println!("{text}");
    } else {
        print_human();
    }
    Ok(())
}

fn parse_bench_args(args: &[String], require_file: bool, usage: &str) -> Result<SingleFlagArgs, CliError> {
    let mut file = None;
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(CliError::Usage(format!("unknown flag `{arg}`\n\n{usage}")));
        } else if require_file && file.is_none() {
            file = Some(arg.clone());
        } else {
            return Err(CliError::Usage(String::from(usage)));
        }
    }
    if require_file && file.is_none() {
        return Err(CliError::Usage(String::from(usage)));
    }
    Ok(SingleFlagArgs { file, json })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PERF_96_CURVE_BUDGET_MS, PERF_MV_BUDGET_MS, build_perf_report};
    use crate::error::CliError;

    #[test]
    fn perf_report_fails_when_either_budget_is_exceeded() {
        let mv_ok = json!({ "rows": 100_000, "eval_ms": 12.0, "budget_ms": PERF_MV_BUDGET_MS, "within_budget": true });
        let mv_slow = json!({ "rows": 100_000, "eval_ms": 300.0, "budget_ms": PERF_MV_BUDGET_MS, "within_budget": false });
        let curve_ok = json!({ "curves": 96, "fit_ms": 15.0, "budget_ms": PERF_96_CURVE_BUDGET_MS, "within_budget": true });
        let curve_slow = json!({ "curves": 96, "fit_ms": 55.0, "budget_ms": PERF_96_CURVE_BUDGET_MS, "within_budget": false });
        let dummy = json!({});

        let (ok_report, ok_res) = build_perf_report(mv_ok.clone(), curve_ok.clone(), dummy.clone(), dummy.clone());
        assert_eq!(ok_report["ok"], json!(true));
        assert!(ok_res.is_ok());

        let (slow_mv_report, slow_mv_res) = build_perf_report(mv_slow, curve_ok, dummy.clone(), dummy.clone());
        assert_eq!(slow_mv_report["ok"], json!(false));
        assert!(matches!(slow_mv_res, Err(CliError::Runtime(_))));

        let (slow_curve_report, slow_curve_res) = build_perf_report(mv_ok, curve_slow, dummy.clone(), dummy);
        assert_eq!(slow_curve_report["ok"], json!(false));
        assert!(matches!(slow_curve_res, Err(CliError::Runtime(_))));
    }
}
