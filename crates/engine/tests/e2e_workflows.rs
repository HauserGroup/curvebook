//! End-to-end verification journeys through `curvebook_engine::Session` covering
//! Alpha workflows 1-5, Core reactive/statistical/graphical workflows, and Beta G7-G11 additions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, BreakOrnament, CellCoord, CellValue, DataSheet, ErrorPropagationMode,
    FactorialAnovaSpec, FitMethod, GlmFamily, GlmSpec, GraphicalDataObject, HookSource, MathTransformPreset, ModelComparisonSpec, MultiplicityMethod,
    MultiplicitySpec, NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, ProjectDocument, RepeatedMeasuresDesign, ResultCell,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, TableArchetype, TransformAnalysisSpec, VariableRole,
};
use curvebook_engine::{CommandOutcome, Session};
use curvebook_format::ZipWriter;
use curvebook_palette::Rgba8;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Shared test harness & clean assertion helpers
// ---------------------------------------------------------------------------

fn exec_ok(session: &mut Session, command_id: &str, params: Value) -> CommandOutcome {
    session.execute(command_id, params).unwrap_or_else(|err| panic!("command `{command_id}` failed: {err}"))
}

fn payload_str<'a>(outcome: &'a CommandOutcome, key: &str) -> &'a str {
    outcome
        .payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string `{key}` in `{}` payload: {}", outcome.command_id, outcome.payload))
}

fn decode_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = 0_u32;
    let mut bits = 0_u8;
    for byte in input.bytes().filter(|b| !b.is_ascii_whitespace()) {
        if byte == b'=' {
            break;
        }
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("unexpected base64 byte {other}"),
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    out
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(*chunk.first().unwrap_or(&0));
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let encode = |shift: usize| TABLE.get(((triple >> shift) & 0x3f) as usize).copied().unwrap_or(b'A') as char;
        out.push(encode(18));
        out.push(encode(12));
        out.push(if chunk.len() > 1 { encode(6) } else { '=' });
        out.push(if chunk.len() > 2 { encode(0) } else { '=' });
    }
    out
}

fn find_tab(sheet: &ResultSheet, kind: ResultTabKind) -> &ResultTab {
    sheet.tabs.iter().find(|t| t.kind == kind).unwrap_or_else(|| panic!("missing result tab `{kind:?}` in `{}`", sheet.id))
}

fn result_sheet<'a>(session: &'a Session, id: &str) -> &'a ResultSheet {
    session.document().result_sheet(&SheetId::new(id)).unwrap_or_else(|| panic!("missing result sheet `{id}`"))
}

fn scalar_val(sheet: &ResultSheet, key: &str, ds: Option<usize>) -> f64 {
    sheet.lookup_scalar(key, ds).unwrap_or_else(|| panic!("missing scalar `{key}` (ds={ds:?}) in `{}`", sheet.id))
}

fn hill_response(log_x: f64, bottom: f64, top: f64, log_ec50: f64, slope: f64) -> f64 {
    bottom + (top - bottom) / (1.0 + 10_f64.powf((log_ec50 - log_x) * slope))
}

fn set_cell(session: &mut Session, sheet_id: &str, target_x: bool, row: usize, col: usize, subcol: usize, value: f64) {
    let mut entry = json!({ "row": row, "col": col, "subcol": subcol, "value": value });
    if target_x && let Some(obj) = entry.as_object_mut() {
        obj.insert(String::from("target"), json!("x"));
    }
    exec_ok(session, "table.setCells", json!({ "sheet_id": sheet_id, "edits": [entry] }));
}

fn populate_xy_dose_response(session: &mut Session, sheet_id: &str, log_x: bool, include_unknowns: bool, top: f64) {
    let rows = if include_unknowns { 10 } else { 8 };
    exec_ok(
        session,
        "sheet.createData",
        json!({
            "sheet_id": sheet_id,
            "title": "Agonist Dose Response",
            "archetype": "xy",
            "rows": rows,
            "cols": 1,
            "replicates": 2,
            "x_title": if log_x { "Log10 [Agonist]" } else { "Agonist [M]" },
            "auto_info": true,
            "auto_graph": false
        }),
    );

    let log_doses = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut x_cells = Vec::with_capacity(log_doses.len());
    let mut y_cells = Vec::with_capacity(log_doses.len() * 2 + 4);
    for (row, &lx) in log_doses.iter().enumerate() {
        x_cells.push(json!({ "row": row, "value": if log_x { lx } else { 10_f64.powf(lx) } }));
        let ideal = hill_response(lx, 5.0, top, -6.0, 1.15);
        y_cells.push(json!({ "row": row, "col": 0, "subcol": 0, "value": ideal - 0.8 }));
        y_cells.push(json!({ "row": row, "col": 0, "subcol": 1, "value": ideal + 0.8 }));
    }

    if include_unknowns {
        // Blank X rows with observed Y trigger inverse standard-curve X interpolation.
        let unk1 = hill_response(-6.2, 5.0, top, -6.0, 1.15);
        let unk2 = hill_response(-5.4, 5.0, top, -6.0, 1.15);
        y_cells.push(json!({ "row": 8, "col": 0, "subcol": 0, "value": unk1 }));
        y_cells.push(json!({ "row": 9, "col": 0, "subcol": 0, "value": unk2 }));
    }

    exec_ok(session, "table.setXCells", json!({ "sheet_id": sheet_id, "edits": x_cells }));
    exec_ok(session, "table.setCells", json!({ "sheet_id": sheet_id, "edits": y_cells }));
}

fn export_svg_text(session: &mut Session, sheet_id: &str) -> String {
    let out = exec_ok(session, "export.svg", json!({ "sheet_id": sheet_id }));
    let svg = payload_str(&out, "svg").to_owned();
    assert!(svg.contains("<svg") && svg.contains("</svg>"), "invalid SVG output for `{sheet_id}`");
    svg
}

fn export_pdf_bytes(session: &mut Session, sheet_id: &str, color_space: &str, compress_streams: bool) -> Vec<u8> {
    let out = exec_ok(session, "export.pdf", json!({ "sheet_id": sheet_id, "color_space": color_space, "compress_streams": compress_streams }));
    let bytes = decode_base64(payload_str(&out, "base64"));
    assert!(bytes.starts_with(b"%PDF-"), "expected valid PDF header for `{sheet_id}`");
    bytes
}

fn export_png_bytes(session: &mut Session, sheet_id: &str, dpi: u64) -> (Vec<u8>, u64, u64) {
    let out = exec_ok(session, "export.png", json!({ "sheet_id": sheet_id, "dpi": dpi }));
    let bytes = decode_base64(payload_str(&out, "base64"));
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]), "invalid PNG magic bytes");
    let w = out.payload.get("width_px").and_then(Value::as_u64).unwrap();
    let h = out.payload.get("height_px").and_then(Value::as_u64).unwrap();
    assert_eq!(out.payload.get("dpi").and_then(Value::as_u64).unwrap(), dpi);
    (bytes, w, h)
}

fn make_nonlinear_spec(
    result_id: &str,
    source_id: &str,
    eq_id: &str,
    method: FitMethod,
    rules: Vec<ParameterRule>,
    comparison: Option<ModelComparisonSpec>,
    interpolate_unknowns: bool,
) -> AnalysisSpec {
    let nl = NonlinearFitSpec {
        equation_id: eq_id.to_owned(),
        method,
        parameter_rules: rules,
        comparison,
        confidence_bands: true,
        interpolate_unknowns,
        ..NonlinearFitSpec::new(eq_id)
    };
    AnalysisSpec::new(result_id, source_id, AnalysisKind::NonlinearRegression(nl)).unwrap()
}

fn xy_x_number(ds: &DataSheet, row: usize) -> Option<f64> {
    match &ds.archetype {
        TableArchetype::Xy(xy) => xy.x_cells.get(row).and_then(|subs| subs.first()).and_then(|c| c.active_number()),
        _ => None,
    }
}

fn assert_data_cell_number(doc: &ProjectDocument, sheet_id: &str, row: usize, col: usize, expected: f64, tol: f64) {
    let ds = doc.data_sheet(&SheetId::new(sheet_id)).unwrap_or_else(|| panic!("missing data sheet `{sheet_id}`"));
    let val = ds
        .cell(CellCoord::new(row, col, 0))
        .and_then(|c| c.active_number())
        .unwrap_or_else(|| panic!("expected active number at ({row},{col}) in `{sheet_id}`"));
    assert!((val - expected).abs() <= tol, "cell ({row},{col}) in `{sheet_id}` was {val}, expected {expected} +/- {tol}");
}

fn build_cleanroom_pzfx_xml() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile Version="2.0" Title="Cleanroom Assay Batch">
  <InfoSequence>
    <Info ID="info-study">
      <Title>Assay Metadata</Title>
      <Constant><Name>Protocol</Name><Value>ELISA-v3</Value></Constant>
      <Constant><Name>TopConstraint</Name><Value>100.0</Value></Constant>
      <Notes>Cleanroom generated XML project fixture.</Notes>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="tbl-col" TableType="OneWay" Replicates="1">
      <Title>Cohort Biomarker</Title>
      <YColumn Width="80" Decimals="2" Subcolumns="1">
        <Title>Vehicle</Title>
        <Subcolumn><d>12.0</d><d>13.4</d><d>11.8</d><d>12.9</d><d Excluded="1">98.5</d></Subcolumn>
      </YColumn>
      <YColumn Width="80" Decimals="2" Subcolumns="1">
        <Title>Agonist 10nM</Title>
        <Subcolumn><d>24.5</d><d>26.1</d><d>25.2</d><d>27.0</d></Subcolumn>
      </YColumn>
      <YColumn Width="80" Decimals="2" Subcolumns="1">
        <Title>Agonist 100nM</Title>
        <Subcolumn><d>41.2</d><d>43.0</d><d>39.8</d><d>42.4</d></Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#,
    )
}

fn build_cleanroom_prism_zip_bytes() -> Vec<u8> {
    let mut zip = ZipWriter::new();
    let manifest = json!({
        "title": "Cleanroom ZIP Study",
        "sheets": [{
            "id": "zip-xy",
            "title": "Imported Log-Dose Curve",
            "kind": "xy",
            "replicates": 1,
            "csv_path": "data/dose_curve.csv"
        }]
    });
    let info_meta = json!({
        "id": "zip-info",
        "title": "Study Constants",
        "notes": "Cleanroom ZIP study info",
        "constants": {
            "AssayCode": "ZIP-901",
            "IncubationMin": 45
        }
    });
    let csv = "LogConc,Response\n-9,5.2\n-8,9.8\n-7,24.1\n-6,52.0\n-5,84.3\n-4,97.4\n";
    zip.add_file("manifest.json", serde_json::to_vec(&manifest).unwrap()).unwrap();
    zip.add_file("info/study.json", serde_json::to_vec(&info_meta).unwrap()).unwrap();
    zip.add_file("data/dose_curve.csv", csv.as_bytes().to_vec()).unwrap();
    zip.finish().unwrap()
}

// ---------------------------------------------------------------------------
// 1. Alpha Workflows 1-5: Dose-response, transforms, interpolation, exports,
//    .curvebook round-trip + edit.undo/edit.redo + multi-hop analyzable DAG
// ---------------------------------------------------------------------------

fn assert_4pl_replicate_interpolation(session: &Session, fit_res: &ResultSheet) {
    let mean_tab = fit_res
        .tabs
        .iter()
        .find(|t| t.id == "nonlinear_interpolated_mean_x")
        .unwrap_or_else(|| panic!("missing nonlinear_interpolated_mean_x tab in `{}`", fit_res.id));
    assert_eq!(mean_tab.title, "Interpolated Mean X");
    assert_eq!(mean_tab.kind, ResultTabKind::InterpolatedUnknowns);
    assert_eq!(mean_tab.rows.len(), 2, "expected 2 mean-X interpolated rows in `{}`", fit_res.id);
    let rep_tab = fit_res
        .tabs
        .iter()
        .find(|t| t.id == "nonlinear_interpolated_replicates")
        .unwrap_or_else(|| panic!("missing nonlinear_interpolated_replicates tab in `{}`", fit_res.id));
    assert_eq!(rep_tab.title, "Interpolated Replicates");
    assert_eq!(rep_tab.kind, ResultTabKind::InterpolatedUnknowns);
    assert_eq!(rep_tab.rows.len(), 2, "expected 2 replicate interpolated rows in `{}`", fit_res.id);
    for (idx, expected_lx) in [-6.2_f64, -5.4_f64].iter().enumerate() {
        let Some(ResultCell::Number { value: mean_est, .. }) = mean_tab.rows.get(idx).and_then(|r| r.get(1)) else {
            panic!("expected numeric interpolated mean X in row {idx}");
        };
        assert!((mean_est - expected_lx).abs() < 0.05, "4PL mean interpolated X row {idx}: got {mean_est}, expected {expected_lx}");
        let Some(ResultCell::Number { value: rep_est, .. }) = rep_tab.rows.get(idx).and_then(|r| r.get(1)) else {
            panic!("expected numeric interpolated replicate X in row {idx}");
        };
        assert!((rep_est - expected_lx).abs() < 0.05, "4PL replicate interpolated X row {idx}: got {rep_est}, expected {expected_lx}");
    }

    // Verify 4PL inverse unknown-X interpolation directly from `curvebook_fit::fit_nonlinear`
    // alongside the `Session`-emitted result tabs and linear standard curve (`fit-lin-std`).
    let doc_ref = session.document();
    let dose_log_ds = doc_ref.data_sheet(&SheetId::new("dose-log")).unwrap();
    let ds_vec = curvebook_fit::extract_datasets_from_sheet(dose_log_ds, &[]).unwrap();
    let model_4pl = curvebook_fit::get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let hooks = curvebook_fit::HookContext {
        info_sheets: &doc_ref.info_sheets,
        result_sheets: &doc_ref.result_sheets,
        data_sheets: &doc_ref.data_sheets,
        primary_info_sheet_id: None,
        visited_sheets: std::collections::HashSet::new(),
    };
    let nl_res = curvebook_fit::fit_nonlinear(
        model_4pl.as_ref(),
        &ds_vec,
        &hooks,
        &curvebook_fit::SolverConfig { interpolate_unknowns: true, ..curvebook_fit::SolverConfig::default() },
    )
    .unwrap();
    assert_eq!(nl_res.interpolated_x.len(), 4, "expected 4 interpolated unknown 4PL entries (2 means + 2 replicates)");
    let mean_items: Vec<_> = nl_res.interpolated_x.iter().filter(|item| item.replicate_mean_count.is_some()).collect();
    let rep_items: Vec<_> = nl_res.interpolated_x.iter().filter(|item| item.replicate_subcolumn == Some(0)).collect();
    assert_eq!(mean_items.len(), 2, "expected 2 replicate-mean interpolated entries");
    assert_eq!(rep_items.len(), 2, "expected 2 single-replicate interpolated entries");
    for (idx, (&expected_lx, (mean_item, rep_item))) in [-6.2_f64, -5.4_f64].iter().zip(mean_items.iter().zip(rep_items.iter())).enumerate() {
        assert!(
            (mean_item.output_estimate - expected_lx).abs() < 0.05,
            "4PL interpolated mean X row {idx}: got {}, expected {expected_lx}",
            mean_item.output_estimate
        );
        assert!(
            (rep_item.output_estimate - expected_lx).abs() < 0.05,
            "4PL interpolated replicate X row {idx}: got {}, expected {expected_lx}",
            rep_item.output_estimate
        );
    }
}

#[test]
fn alpha_workflows_1_to_3_xy_transform_4pl_interpolation_exports_and_roundtrip() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Alpha Workflows 1-3" }));
    populate_xy_dose_response(&mut session, "dose-raw", false, false, 100.0);
    exec_ok(&mut session, "info.setConstant", json!({ "sheet_id": "dose-raw", "name": "Compound", "value": "LC-402" }));

    // Transform commands produce analyzable DataSheet snapshots inside ResultSheet::analyzable_data.
    exec_ok(
        &mut session,
        "transform.run",
        json!({
            "source_sheet_id": "dose-raw",
            "sheet_id": "log-x-res",
            "kind": AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: Some(MathTransformPreset::Log10),
                y_transform: None,
                error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            })
        }),
    );
    exec_ok(
        &mut session,
        "transform.run",
        json!({ "source_sheet_id": "dose-raw", "sheet_id": "norm-res", "kind": "normalize", "zero_anchor": "smallest", "hundred_anchor": "largest" }),
    );
    let log_data = result_sheet(&session, "log-x-res").analyzable_data.as_ref().unwrap();
    assert!((xy_x_number(log_data, 0).unwrap() - (-9.0)).abs() < 1e-9);
    assert!((log_data.cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap() - 4.23).abs() < 0.05);
    let norm_data = result_sheet(&session, "norm-res").analyzable_data.as_ref().unwrap();
    assert!(norm_data.cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap() < 5.0);

    // Chained multi-hop DAG via analyzable ResultSheets: dose-raw -> log10 -> normalize -> 4PL fit.
    let mut chained_doc = session.document().clone();
    let chain_norm = AnalysisSpec::new(
        "chain-norm",
        "log-x-res",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        }),
    )
    .unwrap();
    chained_doc.add_result_sheet(ResultSheet::new(chain_norm).unwrap()).unwrap();
    let chain_fit = make_nonlinear_spec("chain-fit", "chain-norm", "log_agonist_vs_response_4pl", FitMethod::LeastSquares, Vec::new(), None, false);
    chained_doc.add_result_sheet(ResultSheet::new(chain_fit).unwrap()).unwrap();
    let mut chained_session = Session::from_document(chained_doc);
    chained_session.recalculate_all().unwrap();
    let chain_ec50 = scalar_val(result_sheet(&chained_session, "chain-fit"), "LogEC50", Some(0));
    assert!((chain_ec50 - (-6.0)).abs() < 0.08, "chained LogEC50 was {chain_ec50}");

    // Direct 4PL + unknown X interpolation + Graph + CI ribbon + SVG/PDF/300 DPI PNG exports + save/open/undo/redo.
    populate_xy_dose_response(&mut session, "dose-log", true, true, 100.0);
    exec_ok(
        &mut session,
        "fit.run",
        json!({
            "source_sheet_id": "dose-log",
            "sheet_id": "fit-4pl",
            "kind": "four_parameter_logistic",
            "interpolate_unknowns": true,
            "confidence_bands": true
        }),
    );

    let fit_res = result_sheet(&session, "fit-4pl");
    assert_eq!(fit_res.status, ResultStatus::UpToDate);
    let log_ec50 = scalar_val(fit_res, "LogEC50", Some(0));
    assert!((log_ec50 - (-6.0)).abs() < 0.08, "expected LogEC50 near -6.0, got {log_ec50}");
    assert!(scalar_val(fit_res, "R2", None) > 0.99);
    assert_4pl_replicate_interpolation(&session, fit_res);

    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "std-curve", "title": "Linear Standard Curve", "archetype": "xy", "rows": 5, "cols": 1, "replicates": 1 }),
    );
    exec_ok(
        &mut session,
        "table.setXCells",
        json!({ "sheet_id": "std-curve", "edits": [{ "row": 0, "value": 1.0 }, { "row": 1, "value": 2.0 }, { "row": 2, "value": 4.0 }] }),
    );
    exec_ok(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "std-curve",
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 3.0 },
                { "row": 1, "col": 0, "subcol": 0, "value": 5.0 },
                { "row": 2, "col": 0, "subcol": 0, "value": 9.0 },
                { "row": 3, "col": 0, "subcol": 0, "value": 7.0 },
                { "row": 4, "col": 0, "subcol": 0, "value": 11.0 }
            ]
        }),
    );
    exec_ok(
        &mut session,
        "fit.run",
        json!({ "source_sheet_id": "std-curve", "sheet_id": "fit-lin-std", "kind": "linear_regression", "interpolate_unknowns": true }),
    );
    let interp_tab = find_tab(result_sheet(&session, "fit-lin-std"), ResultTabKind::InterpolatedUnknowns);
    assert_eq!(interp_tab.rows.len(), 2, "expected 2 interpolated unknown rows");
    for (idx, expected_x) in [3.0_f64, 5.0_f64].iter().enumerate() {
        let ResultCell::Number { value, .. } = interp_tab.rows[idx][1] else {
            panic!("expected numeric interpolated X in row {idx}");
        };
        assert!((value - expected_x).abs() < 1e-6, "interpolated X row {idx}: got {value}, expected {expected_x}");
    }

    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-dose", "data_sheet_id": "dose-log", "kind": "xy", "title": "4PL Standard Curve" }));
    exec_ok(
        &mut session,
        "graph.setAxes",
        json!({ "sheet_id": "g-dose", "x_axis": { "title": "Log10 [Agonist]" }, "y_axis": { "title": "Response (%)" } }),
    );
    exec_ok(
        &mut session,
        "graph.setRegressionOverlay",
        json!({ "sheet_id": "g-dose", "result_sheet_id": "fit-4pl", "show_confidence_band": true, "shaded_ribbon": true }),
    );

    let pre_save_svg = export_svg_text(&mut session, "g-dose");
    let _pdf = export_pdf_bytes(&mut session, "g-dose", "srgb", true);
    let (_png, png_w, png_h) = export_png_bytes(&mut session, "g-dose", 300);
    assert!(png_w > 500 && png_h > 300);

    let save_out = exec_ok(&mut session, "project.save", json!({ "return_bytes": true }));
    let archive_b64 = payload_str(&save_out, "bytes_base64").to_owned();

    // Mutate Y cell on source table -> verify downstream fit shifts -> exercise edit.undo / edit.redo.
    set_cell(&mut session, "dose-log", false, 3, 0, 0, 88.0);
    let log_ec50_mutated = scalar_val(result_sheet(&session, "fit-4pl"), "LogEC50", Some(0));
    assert!((log_ec50_mutated - log_ec50).abs() > 0.02);

    let undo_out = exec_ok(&mut session, "edit.undo", json!({}));
    assert_eq!(undo_out.payload["applied"], json!(true));
    assert!((scalar_val(result_sheet(&session, "fit-4pl"), "LogEC50", Some(0)) - log_ec50).abs() < 1e-12);
    assert_eq!(export_svg_text(&mut session, "g-dose"), pre_save_svg);

    let redo_out = exec_ok(&mut session, "edit.redo", json!({}));
    assert_eq!(redo_out.payload["applied"], json!(true));
    assert!((scalar_val(result_sheet(&session, "fit-4pl"), "LogEC50", Some(0)) - log_ec50_mutated).abs() < 1e-12);

    exec_ok(&mut session, "edit.undo", json!({}));
    assert!((scalar_val(result_sheet(&session, "fit-4pl"), "LogEC50", Some(0)) - log_ec50).abs() < 1e-12);

    // Reopen saved .curvebook bytes in a fresh Session -> recalculate -> identical ResultSheet & SVG.
    let mut reopened = Session::new();
    exec_ok(&mut reopened, "project.open", json!({ "bytes_base64": archive_b64 }));
    exec_ok(&mut reopened, "project.recalculate", json!({}));
    assert!((scalar_val(result_sheet(&reopened, "fit-4pl"), "LogEC50", Some(0)) - log_ec50).abs() < 1e-12);
    assert_eq!(export_svg_text(&mut reopened, "g-dose"), pre_save_svg);
}

#[test]
fn alpha_workflows_4_and_5_column_grouped_descriptive_t_anova_tukey_and_exports() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Alpha Workflows 4-5" }));

    // Workflow 4: Column table -> descriptive stats + unpaired t + one-way ANOVA with Tukey.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "col-3g", "title": "Biomarker Cohorts", "archetype": "column", "rows": 5, "cols": 3, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "col-3g", "titles": ["Vehicle", "LowDose", "HighDose"] }));
    exec_ok(
        &mut session,
        "table.paste",
        json!({ "sheet_id": "col-3g", "text": "10.2\t18.1\t32.4\n11.4\t19.0\t34.1\n9.8\t17.4\t30.9\n10.6\t18.8\t33.5\n11.0\t18.5\t31.8" }),
    );

    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "desc-col", "kind": "descriptive" }));
    let desc_tab = find_tab(result_sheet(&session, "desc-col"), ResultTabKind::DescriptiveSummary);
    assert!(!desc_tab.rows.is_empty());

    exec_ok(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "col-3g", "sheet_id": "anova-1w", "kind": "one_way_anova", "multiplicity": "tukey" }),
    );
    let anova_res = result_sheet(&session, "anova-1w");
    assert!(scalar_val(anova_res, "F", None) > 50.0);
    assert_eq!(anova_res.pairwise_comparisons.len(), 3);
    assert!(anova_res.pairwise_comparisons.iter().all(|c| c.significant));

    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-3g", "kind": "column", "title": "Cohorts" }));
    let _col_svg = export_svg_text(&mut session, "g-col");
    let _col_pdf = export_pdf_bytes(&mut session, "g-col", "srgb", true);

    // 2-group unpaired t-test & paired t-test.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "col-unpaired", "title": "Two Group Screen", "archetype": "column", "rows": 5, "cols": 2, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "col-unpaired", "text": "4.1\t9.2\n4.5\t9.8\n3.9\t8.9\n4.3\t9.5\n4.2\t9.4" }));
    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "col-unpaired", "sheet_id": "t-unpaired", "kind": "unpaired_t" }));
    assert!(scalar_val(result_sheet(&session, "t-unpaired"), "PValue", None) < 0.001);

    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "col-paired",
            "title": "Before After Assay",
            "archetype": "column",
            "design_hint": "paired",
            "rows": 5,
            "cols": 2,
            "auto_graph": false
        }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "col-paired", "text": "20.0\t27.5\n22.1\t29.4\n18.9\t26.0\n21.4\t28.9\n19.6\t27.1" }));
    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "col-paired", "sheet_id": "t-paired", "kind": "paired_t" }));
    assert!(scalar_val(result_sheet(&session, "t-paired"), "PValue", None) < 0.001);

    // Workflow 5: Grouped Two-Way ANOVA + Tukey + Grouped graph export.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "grp-2w",
            "title": "Treatment x Genotype",
            "archetype": "grouped",
            "rows": 2,
            "cols": 2,
            "replicates": 3,
            "row_factor_title": "Treatment",
            "column_factor_title": "Genotype",
            "auto_graph": false
        }),
    );
    exec_ok(&mut session, "table.setRowTitles", json!({ "sheet_id": "grp-2w", "titles": ["Control", "Drug"] }));
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "grp-2w", "titles": ["WT", "KO"] }));
    let grp_cells = [
        (0, 0, 0, 10.0),
        (0, 0, 1, 10.5),
        (0, 0, 2, 9.8),
        (0, 1, 0, 11.2),
        (0, 1, 1, 10.8),
        (0, 1, 2, 11.4),
        (1, 0, 0, 24.0),
        (1, 0, 1, 25.1),
        (1, 0, 2, 24.4),
        (1, 1, 0, 14.5),
        (1, 1, 1, 15.0),
        (1, 1, 2, 14.2),
    ];
    let cell_json: Vec<Value> = grp_cells.iter().map(|&(r, c, s, v)| json!({ "row": r, "col": c, "subcol": s, "value": v })).collect();
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": "grp-2w", "edits": cell_json }));

    exec_ok(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "grp-2w", "sheet_id": "anova-2w", "kind": "two_way_anova", "multiplicity": "tukey" }),
    );
    let tw_res = result_sheet(&session, "anova-2w");
    assert_eq!(tw_res.status, ResultStatus::UpToDate);
    assert!(!find_tab(tw_res, ResultTabKind::AnovaTable).rows.is_empty());

    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-grp", "data_sheet_id": "grp-2w", "kind": "grouped", "title": "2-Way Grouped" }));
    let _grp_svg = export_svg_text(&mut session, "g-grp");
}

// ---------------------------------------------------------------------------
// 2. Core Workflow Additions: pasteLink, reactive transpose, 4-stage CSV paste
// ---------------------------------------------------------------------------

#[test]
fn core_workflow_live_paste_link_reactive_transpose_and_4stage_csv_paste() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Core Link and 4-Stage CSV" }));

    // 4-stage CSV paste: European semicolon+comma decimals, row/col slice, decimate_every_k, transpose, asterisk exclusion.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "csv-target", "title": "4-Stage CSV", "archetype": "column", "rows": 4, "cols": 4, "auto_graph": false }),
    );
    let raw_eu_csv = "\
10,5;20,5;30,5;99,9\n\
1,0;2,0;3,0;88,8\n\
12,5*;22,5;32,5;77,7\n\
2,0;4,0;6,0;66,6\n\
14,0;24,0;34,0;55,5\n";
    exec_ok(
        &mut session,
        "table.paste",
        json!({
            "sheet_id": "csv-target",
            "text": raw_eu_csv,
            "delimiter": ";",
            "decimal_separator": ",",
            "transpose": true,
            "filter": {
                "row_start": 0,
                "row_end": 4,
                "col_start": 0,
                "col_end": 2,
                "decimate_every_k": 2,
                "parse_asterisk_exclusions": true
            }
        }),
    );
    // Kept rows {0, 2, 4} and cols {0, 1, 2}, then transposed => row 0..3 x col 0..3.
    let csv_ds = session.document().data_sheet(&SheetId::new("csv-target")).unwrap();
    assert_eq!(csv_ds.cell(CellCoord::new(0, 0, 0)).unwrap().active_number(), Some(10.5));
    let excluded_cell = csv_ds.cell(CellCoord::new(0, 1, 0)).unwrap();
    assert!(excluded_cell.excluded, "asterisk value `12,5*` must be marked excluded after transpose");
    assert_eq!(excluded_cell.value, CellValue::Number(12.5));
    assert_eq!(csv_ds.cell(CellCoord::new(2, 2, 0)).unwrap().active_number(), Some(34.0));

    // Live cross-table pasteLink + reactive transpose + baseline remove transform + downstream analysis reactivity.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "src-plate", "title": "Plate Raw", "archetype": "column", "rows": 3, "cols": 3, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "src-plate", "titles": ["Blank", "DoseA", "DoseB"] }));
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "src-plate", "text": "2.0\t12.0\t22.0\n2.0\t14.0\t24.0\n2.0\t16.0\t26.0" }));

    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "dst-linked",
            "title": "Linked Transposed",
            "archetype": "column",
            "rows": 3,
            "cols": 3,
            "auto_info": true,
            "auto_graph": false
        }),
    );
    let link_out = exec_ok(
        &mut session,
        "table.pasteLink",
        json!({
            "source_sheet_id": "src-plate",
            "target_sheet_id": "dst-linked",
            "transpose": true,
            "propagate_title_header": true
        }),
    );
    assert_eq!(link_out.payload["linked_regions"], json!(1));
    // Source row 0 [2, 12, 22] transposed => col 0 of dst-linked is [2, 12, 22].
    assert_data_cell_number(session.document(), "dst-linked", 1, 0, 12.0, 1e-9);
    assert_data_cell_number(session.document(), "dst-linked", 2, 0, 22.0, 1e-9);

    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "dst-linked", "sheet_id": "dst-desc", "kind": "descriptive" }));
    let mean_before = scalar_val(result_sheet(&session, "dst-desc"), "Mean", Some(0));
    assert!((mean_before - 12.0).abs() < 1e-9);

    // Mutate upstream DoseB in row 0 from 22.0 -> 52.0 (+30) => linked transposed (row 2, col 0) updates 22 -> 52, mean updates 12 -> 22.
    set_cell(&mut session, "src-plate", false, 0, 2, 0, 52.0);
    assert_data_cell_number(session.document(), "dst-linked", 2, 0, 52.0, 1e-9);
    let mean_after = scalar_val(result_sheet(&session, "dst-desc"), "Mean", Some(0));
    assert!((mean_after - 22.0).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// 3. Core Workflow Additions: Kruskal-Wallis + Dunn & RM / REML Mixed ANOVA
// ---------------------------------------------------------------------------

#[test]
fn core_workflow_kruskal_wallis_dunn_and_rm_reml_mixed_anova() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Nonparametric and REML Mixed ANOVA" }));

    // Kruskal-Wallis + Dunn post-hoc on skewed non-normal Column data.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "kw-data", "title": "Skewed Counts", "archetype": "column", "rows": 5, "cols": 3, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "kw-data", "titles": ["Ctrl", "Mid", "High"] }));
    exec_ok(
        &mut session,
        "table.paste",
        json!({ "sheet_id": "kw-data", "text": "1.0\t12.0\t80.0\n2.0\t15.0\t95.0\n1.5\t14.0\t110.0\n3.0\t18.0\t150.0\n2.5\t16.0\t220.0" }),
    );
    exec_ok(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "kw-data", "sheet_id": "kw-res", "kind": "kruskal_wallis", "multiplicity": "dunn" }),
    );
    let kw_res = result_sheet(&session, "kw-res");
    assert_eq!(kw_res.status, ResultStatus::UpToDate);
    assert!(scalar_val(kw_res, "Kruskal-Wallis H", None) > 10.0);
    assert!(scalar_val(kw_res, "P value", None) < 0.01);
    assert_eq!(kw_res.pairwise_comparisons.len(), 3);
    assert!(kw_res.pairwise_comparisons.iter().all(|c| c.raw_p_or_q.is_finite()));

    // Balanced RM Two-Way ANOVA, followed by missing-replicate REML Mixed-Effects fallback.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "rm-data",
            "title": "Longitudinal Cohort",
            "archetype": "grouped",
            "rows": 2,
            "cols": 3,
            "replicates": 4,
            "row_factor_title": "Arm",
            "column_factor_title": "Timepoint",
            "auto_graph": false
        }),
    );
    let rm_vals = [
        (0, 0, [10.0, 11.0, 9.5, 10.5]),
        (0, 1, [11.0, 12.0, 10.8, 11.4]),
        (0, 2, [11.5, 12.2, 11.1, 11.9]),
        (1, 0, [10.2, 10.9, 9.8, 10.4]),
        (1, 1, [18.5, 19.8, 17.6, 18.9]),
        (1, 2, [26.0, 28.1, 25.3, 27.2]),
    ];
    let mut rm_cells = Vec::new();
    for &(r, c, vals) in &rm_vals {
        for (s, &v) in vals.iter().enumerate() {
            rm_cells.push(json!({ "row": r, "col": c, "subcol": s, "value": v }));
        }
    }
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": "rm-data", "edits": rm_cells }));

    let rm_spec = AnalysisSpec::new(
        "rm-balanced",
        "rm-data",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
            fit_reml_on_missing: true,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::HolmSidak, ..MultiplicitySpec::default() },
            ..FactorialAnovaSpec::default()
        }),
    )
    .unwrap();
    exec_ok(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "rm-data", "sheet_id": "rm-balanced", "spec": serde_json::to_value(&rm_spec).unwrap() }),
    );
    let balanced_res = result_sheet(&session, "rm-balanced");
    assert!(!find_tab(balanced_res, ResultTabKind::AnovaTable).rows.is_empty());

    // Introduce missing replicates -> verify REML mixed-effects fallback runs cleanly on incomplete RM data.
    exec_ok(
        &mut session,
        "table.clearCells",
        json!({ "sheet_id": "rm-data", "cells": [{ "row": 1, "col": 2, "subcol": 1 }, { "row": 0, "col": 1, "subcol": 3 }] }),
    );
    let reml_res = result_sheet(&session, "rm-balanced");
    assert_eq!(reml_res.status, ResultStatus::UpToDate);
    let reml_tab = find_tab(reml_res, ResultTabKind::AnovaTable);
    assert!(!reml_tab.rows.is_empty(), "REML fallback must populate mixed ANOVA table when replicates are missing");

    // Unstructured residual covariance via shorthand `stats.run` + `methods_report` check.
    exec_ok(
        &mut session,
        "stats.run",
        json!({
            "source_sheet_id": "rm-data",
            "sheet_id": "rm-unstructured",
            "kind": "two_way_anova",
            "repeated_measures": "column",
            "residual_covariance": "unstructured"
        }),
    );
    let un_res = result_sheet(&session, "rm-unstructured");
    assert_eq!(un_res.status, ResultStatus::UpToDate);
    let cov_tabs: Vec<&ResultTab> = un_res.tabs.iter().filter(|t| t.kind == ResultTabKind::CovarianceMatrix).collect();
    assert_eq!(cov_tabs.len(), 2, "expected covariance and correlation tabs, got {}", cov_tabs.len());
    assert!(un_res.scalars.iter().any(|s| s.key == "REML mean diagonal variance" && s.value.is_finite()));
    let methods = curvebook_engine::methods_report::generate_methods_report(
        session.document(),
        &SheetId::new("rm-unstructured"),
        curvebook_engine::methods_report::ReportFormat::PlainText,
    )
    .unwrap();
    assert!(methods.contains("unstructured residual covariance"), "methods report missing unstructured wording:\n{methods}");
}

// ---------------------------------------------------------------------------
// 4. Core Workflow Additions: ROUT + Shared Global 4PL + AICc / Extra-SS F + CI ribbons
// ---------------------------------------------------------------------------

#[test]
fn core_workflow_rout_shared_global_4pl_aicc_extra_ss_f_and_ci_ribbons() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "ROUT Global 4PL Comparison" }));
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "xy-2curves",
            "title": "Two Agonists Shared Asymptotes",
            "archetype": "xy",
            "rows": 9,
            "cols": 2,
            "replicates": 2,
            "auto_graph": false
        }),
    );
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "xy-2curves", "titles": ["Agonist A", "Agonist B"] }));

    let log_x = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.0, -4.0];
    let mut x_cells = Vec::new();
    let mut y_cells = Vec::new();
    for (r, &lx) in log_x.iter().enumerate() {
        x_cells.push(json!({ "row": r, "value": lx }));
        let ya = hill_response(lx, 10.0, 95.0, -7.0, 1.5);
        let yb = hill_response(lx, 10.0, 95.0, -5.8, 1.5);
        y_cells.push(json!({ "row": r, "col": 0, "subcol": 0, "value": ya - 0.6 }));
        // Single blatant technical outlier on the upper plateau replicate 2.
        let ya_rep2 = if r == 7 { 195.0 } else { ya + 0.6 };
        y_cells.push(json!({ "row": r, "col": 0, "subcol": 1, "value": ya_rep2 }));
        y_cells.push(json!({ "row": r, "col": 1, "subcol": 0, "value": yb - 0.6 }));
        y_cells.push(json!({ "row": r, "col": 1, "subcol": 1, "value": yb + 0.6 }));
    }
    exec_ok(&mut session, "table.setXCells", json!({ "sheet_id": "xy-2curves", "edits": x_cells }));
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": "xy-2curves", "edits": y_cells }));

    // Dedicated ROUT outlier detection on column table:
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "rout-col", "title": "Outlier Screen", "archetype": "column", "rows": 8, "cols": 1, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "rout-col", "text": "10.1\n10.4\n9.9\n10.2\n10.0\n280.0\n10.3\n9.8" }));
    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "rout-col", "sheet_id": "rout-res", "kind": "outliers", "q_percent": 1.0 }));
    assert!(scalar_val(result_sheet(&session, "rout-res"), "OutlierCount", Some(0)) >= 1.0);

    let shared_rules = vec![
        ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::SharedGlobal,
            lower_bound: Some(-20.0),
            upper_bound: Some(40.0),
            initial_override: Some(10.0),
        },
        ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::SharedGlobal,
            lower_bound: Some(50.0),
            upper_bound: Some(150.0),
            initial_override: Some(95.0),
        },
    ];

    let spec_f = make_nonlinear_spec(
        "fit-global-f",
        "xy-2curves",
        "log_agonist_vs_response_4pl",
        FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        shared_rules.clone(),
        Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") }),
        false,
    );
    exec_ok(
        &mut session,
        "fit.run",
        json!({ "source_sheet_id": "xy-2curves", "sheet_id": "fit-global-f", "spec": serde_json::to_value(&spec_f).unwrap() }),
    );

    let spec_aicc = make_nonlinear_spec(
        "fit-global-aicc",
        "xy-2curves",
        "log_agonist_vs_response_4pl",
        FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        shared_rules,
        Some(ModelComparisonSpec::Aicc { alternative_equation_id: String::from("log_agonist_vs_response_3pl") }),
        false,
    );
    exec_ok(
        &mut session,
        "fit.run",
        json!({ "source_sheet_id": "xy-2curves", "sheet_id": "fit-global-aicc", "spec": serde_json::to_value(&spec_aicc).unwrap() }),
    );

    let res_f = result_sheet(&session, "fit-global-f");
    let res_aicc = result_sheet(&session, "fit-global-aicc");
    assert_eq!(res_f.status, ResultStatus::UpToDate);
    let ec50_a = scalar_val(res_f, "LogEC50", Some(0));
    let ec50_b = scalar_val(res_f, "LogEC50", Some(1));
    assert!((ec50_a - (-7.0)).abs() < 0.15, "ROUT must protect LogEC50[0] near -7.0, got {ec50_a}");
    assert!((ec50_b - (-5.8)).abs() < 0.15, "ROUT LogEC50[1] near -5.8, got {ec50_b}");
    assert!(!find_tab(res_f, ResultTabKind::ModelComparison).rows.is_empty());
    assert!(!find_tab(res_aicc, ResultTabKind::ModelComparison).rows.is_empty());

    // Graph with CI ribbons overlaid for both curves.
    exec_ok(
        &mut session,
        "graph.create",
        json!({ "sheet_id": "g-global", "data_sheet_id": "xy-2curves", "kind": "xy", "title": "Global 4PL + CI Ribbon" }),
    );
    exec_ok(
        &mut session,
        "graph.setRegressionOverlay",
        json!({
            "sheet_id": "g-global",
            "result_sheet_id": "fit-global-f",
            "all_series": true,
            "show_confidence_band": true,
            "shaded_ribbon": true
        }),
    );
    let svg = export_svg_text(&mut session, "g-global");
    assert!(svg.contains("<path") || svg.contains("<polygon"));
}

// ---------------------------------------------------------------------------
// 5. Core Workflow Additions: Broken axes, Violin, BeforeAfter, Stacked Brackets
// ---------------------------------------------------------------------------

#[test]
fn core_workflow_broken_axis_violin_before_after_and_stacked_brackets() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Broken Axes Violin Brackets" }));

    // Part A: Segmented broken Y axis (2 discontinuous segments with double-slash ornament).
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "xy-gap", "title": "Bimodal Range", "archetype": "xy", "rows": 6, "cols": 1, "replicates": 1, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "xy-gap", "text": "1\t5\n2\t9\n3\t14\n4\t220\n5\t360\n6\t480" }));
    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-broken", "data_sheet_id": "xy-gap", "kind": "xy", "title": "Segmented Y Axis" }));
    exec_ok(
        &mut session,
        "graph.setAxes",
        json!({
            "sheet_id": "g-broken",
            "y_axis": {
                "title": "Signal",
                "gap_width_pt": 7.0,
                "break_ornament": "double_slash",
                "segments": [
                    { "scale": "linear", "min": 0.0, "max": 20.0, "length_share": 0.4 },
                    { "scale": "linear", "min": 180.0, "max": 520.0, "length_share": 0.6 }
                ]
            }
        }),
    );
    let broken_spec = &session.document().graph_sheet(&SheetId::new("g-broken")).unwrap().spec;
    assert_eq!(broken_spec.y_axis.segments.len(), 2);
    assert_eq!(broken_spec.y_axis.break_ornament, BreakOrnament::DoubleSlash);
    let _broken_svg = export_svg_text(&mut session, "g-broken");

    // Verify 3-segment axis update via AxisSegment struct also compiles/renders cleanly.
    let three_seg = AxisSpec {
        segments: vec![
            AxisSegment { scale: AxisScale::Linear, min: Some(0.0), max: Some(20.0), length_share: 0.3, ..AxisSegment::default() },
            AxisSegment { scale: AxisScale::Linear, min: Some(150.0), max: Some(300.0), length_share: 0.35, ..AxisSegment::default() },
            AxisSegment { scale: AxisScale::Linear, min: Some(380.0), max: Some(500.0), length_share: 0.35, ..AxisSegment::default() },
        ],
        break_ornament: BreakOrnament::Zigzag,
        ..AxisSpec::linear("Signal")
    };
    exec_ok(&mut session, "graph.setAxes", json!({ "sheet_id": "g-broken", "y_axis": serde_json::to_value(&three_seg).unwrap() }));
    assert_eq!(session.document().graph_sheet(&SheetId::new("g-broken")).unwrap().spec.y_axis.segments.len(), 3);
    let _three_seg_svg = export_svg_text(&mut session, "g-broken");

    // Part B: Violin plot + auto-stacked pairwise comparison brackets hooked to ResultSheet.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "col-violin", "title": "Violin Cohorts", "archetype": "column", "rows": 8, "cols": 3, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "col-violin", "titles": ["Ctrl", "Tx1", "Tx2"] }));
    exec_ok(
        &mut session,
        "table.paste",
        json!({
            "sheet_id": "col-violin",
            "text": "10\t22\t38\n11\t24\t40\n9\t20\t36\n12\t25\t42\n10.5\t23\t39\n11.2\t21\t41\n9.6\t24\t37\n10.8\t26\t43"
        }),
    );
    exec_ok(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "col-violin", "sheet_id": "violin-anova", "kind": "one_way_anova", "multiplicity": "tukey" }),
    );
    exec_ok(
        &mut session,
        "graph.create",
        json!({ "sheet_id": "g-violin", "data_sheet_id": "col-violin", "kind": "column", "title": "Violin + Brackets" }),
    );
    let violin_gdo = GraphicalDataObject::Violin {
        fill_color: Rgba8::rgba(59, 130, 246, 175),
        stroke_color: Rgba8::rgba(30, 64, 175, 255),
        clip_to_observed_range: true,
        show_median_and_quartiles: true,
        half_violin_with_points: true,
    };
    let gdo_val = serde_json::to_value(&violin_gdo).unwrap();
    for i in 0..3 {
        exec_ok(&mut session, "graph.setSeries", json!({ "sheet_id": "g-violin", "series_index": i, "gdo": gdo_val }));
    }
    let bracket_out = exec_ok(
        &mut session,
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-violin",
            "result_sheet_id": "violin-anova",
            "style": "square",
            "label_mode": "numeric",
            "bracket_step_pt": 13.0
        }),
    );
    assert_eq!(bracket_out.payload["total_brackets"], json!(3));
    let violin_svg = export_svg_text(&mut session, "g-violin");
    assert!(violin_svg.contains('<'));

    // Part C: Paired BeforeAfterLines graph + paired comparison bracket hooked to ResultSheet.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "col-ba",
            "title": "Paired Response",
            "archetype": "column",
            "design_hint": "paired",
            "rows": 5,
            "cols": 2,
            "auto_graph": true,
            "graph_sheet_id": "g-ba"
        }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "col-ba", "text": "14\t22\n15\t24\n13\t21\n16\t25\n14.5\t23" }));
    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "col-ba", "sheet_id": "ba-t", "kind": "paired_t" }));
    let ba_series = &session.document().graph_sheet(&SheetId::new("g-ba")).unwrap().spec.series[0];
    assert!(matches!(ba_series.gdo, GraphicalDataObject::BeforeAfterLines { .. }));
    exec_ok(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-ba", "result_sheet_id": "ba-t", "style": "curly", "label_mode": "stars" }),
    );
    let _ba_svg = export_svg_text(&mut session, "g-ba");
}

// ---------------------------------------------------------------------------
// 6. Beta G9: .cbtemplate save + instantiate + InfoSheet constant hooks
// ---------------------------------------------------------------------------

#[test]
fn beta_g9_cbtemplate_save_instantiate_and_info_constant_hooks() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Assay Template Master" }));
    populate_xy_dose_response(&mut session, "tpl-xy", true, false, 100.0);

    let info_id = session.document().data_sheet(&SheetId::new("tpl-xy")).unwrap().info_sheet_id.as_ref().unwrap().as_str().to_owned();
    exec_ok(&mut session, "info.setConstant", json!({ "sheet_id": &info_id, "name": "TopConstraint", "value": 100.0 }));
    exec_ok(&mut session, "info.setConstant", json!({ "sheet_id": &info_id, "name": "AssayBatch", "value": "BATCH-A1" }));

    let hooked_rule = ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new(&info_id)),
            constant_name: String::from("TopConstraint"),
        }),
        lower_bound: None,
        upper_bound: None,
        initial_override: None,
    };
    let fit_spec = make_nonlinear_spec("tpl-fit", "tpl-xy", "log_agonist_vs_response_4pl", FitMethod::LeastSquares, vec![hooked_rule], None, false);
    exec_ok(&mut session, "fit.run", json!({ "source_sheet_id": "tpl-xy", "sheet_id": "tpl-fit", "spec": serde_json::to_value(&fit_spec).unwrap() }));

    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "tpl-graph", "data_sheet_id": "tpl-xy", "kind": "xy", "title": "Template Graph" }));
    exec_ok(
        &mut session,
        "graph.setAxes",
        json!({
            "sheet_id": "tpl-graph",
            "x_axis": { "title": "Log10 Dose — {{info:AssayBatch}}" },
            "y_axis": { "title": "Response (Cap={{info:TopConstraint}})" }
        }),
    );
    exec_ok(&mut session, "graph.setRegressionOverlay", json!({ "sheet_id": "tpl-graph", "result_sheet_id": "tpl-fit" }));

    // Save stripped .cbtemplate archive.
    let tpl_out = exec_ok(&mut session, "project.saveTemplate", json!({ "return_bytes": true }));
    let tpl_b64 = payload_str(&tpl_out, "bytes_base64").to_owned();

    // Instantiate template into a fresh Session: cells are cleared while constants, fit spec & title hooks remain intact.
    let mut run2 = Session::new();
    exec_ok(&mut run2, "project.instantiateTemplate", json!({ "bytes_base64": tpl_b64, "title": "Assay Run B2" }));
    assert!(run2.document().data_sheet(&SheetId::new("tpl-xy")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().is_empty());
    assert_eq!(result_sheet(&run2, "tpl-fit").status, ResultStatus::Dirty);

    // Populate new assay run with asymptote 140.0 -> verify InfoSheet constant hook drives BOTH fit Top & graph title!
    let mut x_cells = Vec::new();
    let mut y_cells = Vec::new();
    for (r, &lx) in [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0].iter().enumerate() {
        x_cells.push(json!({ "row": r, "value": lx }));
        let y = hill_response(lx, 8.0, 140.0, -6.1, 1.1);
        y_cells.push(json!({ "row": r, "col": 0, "subcol": 0, "value": y }));
        y_cells.push(json!({ "row": r, "col": 0, "subcol": 1, "value": y + 0.5 }));
    }
    exec_ok(&mut run2, "table.setXCells", json!({ "sheet_id": "tpl-xy", "edits": x_cells }));
    exec_ok(&mut run2, "table.setCells", json!({ "sheet_id": "tpl-xy", "edits": y_cells }));

    let svg_before = export_svg_text(&mut run2, "tpl-graph");
    assert!(svg_before.contains("BATCH-A1") && svg_before.contains("Cap=100"));
    assert!((scalar_val(result_sheet(&run2, "tpl-fit"), "Top", None) - 100.0).abs() < 1e-6);

    // Mutate InfoSheet constants -> both constrained 4PL Top parameter AND compiled graph title update automatically!
    exec_ok(&mut run2, "info.setConstant", json!({ "sheet_id": &info_id, "name": "TopConstraint", "value": 140.0 }));
    exec_ok(&mut run2, "info.setConstant", json!({ "sheet_id": &info_id, "name": "AssayBatch", "value": "BATCH-B2" }));

    assert!((scalar_val(result_sheet(&run2, "tpl-fit"), "Top", None) - 140.0).abs() < 1e-6);
    let svg_after = export_svg_text(&mut run2, "tpl-graph");
    assert!(svg_after.contains("BATCH-B2") && svg_after.contains("Cap=140"));
}

// ---------------------------------------------------------------------------
// 7. Beta G9: sheet.freeze / sheet.unfreeze & sheet.clonePipeline
// ---------------------------------------------------------------------------

#[test]
fn beta_g9_freeze_unfreeze_and_clone_pipeline_for_new_assay_run() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Freeze and ClonePipeline" }));

    // Part A: freeze blocks upstream pasteLink propagation until unfreeze.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "up", "title": "Upstream", "archetype": "column", "rows": 4, "cols": 1, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "up", "text": "10\n20\n30\n40" }));

    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "dn", "title": "Downstream", "archetype": "column", "rows": 4, "cols": 1, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.pasteLink", json!({ "source_sheet_id": "up", "target_sheet_id": "dn" }));
    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "dn", "sheet_id": "dn-desc", "kind": "descriptive" }));
    assert!((scalar_val(result_sheet(&session, "dn-desc"), "Mean", Some(0)) - 25.0).abs() < 1e-9);

    exec_ok(&mut session, "sheet.freeze", json!({ "sheet_id": "dn" }));
    assert!(session.document().data_sheet(&SheetId::new("dn")).unwrap().frozen);
    set_cell(&mut session, "up", false, 0, 0, 0, 410.0);
    // Still frozen -> linked cell & downstream ResultSheet remain at pre-freeze values.
    assert_data_cell_number(session.document(), "dn", 0, 0, 10.0, 1e-9);
    assert!((scalar_val(result_sheet(&session, "dn-desc"), "Mean", Some(0)) - 25.0).abs() < 1e-9);

    // Unfreezing catches up both linked data & downstream ResultSheet automatically!
    exec_ok(&mut session, "sheet.unfreeze", json!({ "sheet_id": "dn" }));
    assert!(!session.document().data_sheet(&SheetId::new("dn")).unwrap().frozen);
    assert_data_cell_number(session.document(), "dn", 0, 0, 410.0, 1e-9);
    assert!((scalar_val(result_sheet(&session, "dn-desc"), "Mean", Some(0)) - 125.0).abs() < 1e-9);

    // Part B: clonePipeline duplicates Data + Info + Analyses + Graphs + Layouts with cleared Y cells.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "pipe-xy", "title": "Run 1", "archetype": "xy", "rows": 8, "cols": 1, "replicates": 1, "auto_info": true, "auto_graph": false }),
    );
    let mut x_cells = Vec::new();
    let mut y_cells = Vec::new();
    for (r, &lx) in [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0].iter().enumerate() {
        x_cells.push(json!({ "row": r, "value": lx }));
        y_cells.push(json!({ "row": r, "col": 0, "subcol": 0, "value": hill_response(lx, 5.0, 100.0, -6.2, 1.0) }));
    }
    exec_ok(&mut session, "table.setXCells", json!({ "sheet_id": "pipe-xy", "edits": x_cells }));
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": "pipe-xy", "edits": y_cells }));

    exec_ok(&mut session, "fit.run", json!({ "source_sheet_id": "pipe-xy", "sheet_id": "pipe-fit", "kind": "four_parameter_logistic" }));
    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "pipe-graph", "data_sheet_id": "pipe-xy", "kind": "xy", "title": "Pipeline Graph" }));
    exec_ok(&mut session, "graph.setRegressionOverlay", json!({ "sheet_id": "pipe-graph", "result_sheet_id": "pipe-fit" }));
    exec_ok(&mut session, "layout.create", json!({ "sheet_id": "pipe-layout", "title": "Pipeline Figure" }));
    exec_ok(&mut session, "layout.addPanel", json!({ "sheet_id": "pipe-layout", "graph_sheet_id": "pipe-graph" }));

    let orig_ec50 = scalar_val(result_sheet(&session, "pipe-fit"), "LogEC50", Some(0));
    let clone_out = exec_ok(
        &mut session,
        "sheet.clonePipeline",
        json!({ "sheet_id": "pipe-xy", "new_sheet_id": "run2-xy", "title": "Run 2", "keep_x_values": true, "clear_y_values": true }),
    );
    let cloned_data_id = payload_str(&clone_out, "sheet_id").to_owned();
    let cloned_res_id = clone_out.payload["result_sheet_ids"][0].as_str().unwrap().to_owned();
    let cloned_graph_id = clone_out.payload["graph_sheet_ids"][0].as_str().unwrap().to_owned();
    let cloned_layout_id = clone_out.payload["layout_sheet_ids"][0].as_str().unwrap().to_owned();

    // X kept, Y cleared, cloned result marked Dirty, cloned graph & layout remapped cleanly.
    let cloned_ds = session.document().data_sheet(&SheetId::new(&cloned_data_id)).unwrap();
    assert_eq!(xy_x_number(cloned_ds, 0), Some(-9.0));
    assert!(cloned_ds.cell(CellCoord::new(0, 0, 0)).unwrap().is_empty());
    assert_eq!(result_sheet(&session, &cloned_res_id).status, ResultStatus::Dirty);
    assert_eq!(session.document().graph_sheet(&SheetId::new(&cloned_graph_id)).unwrap().spec.primary_source_sheet_id.as_str(), cloned_data_id);
    assert!(!session.document().layout_sheet(&SheetId::new(&cloned_layout_id)).unwrap().spec.items.is_empty());

    // Enter right-shifted potency Run 2 Y values -> recalculate -> cloned Result updates while Run 1 remains untouched.
    let run2_y: Vec<Value> = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0]
        .iter()
        .enumerate()
        .map(|(r, &lx)| json!({ "row": r, "col": 0, "subcol": 0, "value": hill_response(lx, 5.0, 100.0, -5.1, 1.0) }))
        .collect();
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": &cloned_data_id, "edits": run2_y }));
    exec_ok(&mut session, "project.recalculate", json!({}));

    let run2_ec50 = scalar_val(result_sheet(&session, &cloned_res_id), "LogEC50", Some(0));
    assert!((run2_ec50 - (-5.1)).abs() < 0.12);
    assert!((scalar_val(result_sheet(&session, "pipe-fit"), "LogEC50", Some(0)) - orig_ec50).abs() < 1e-12);
}

// ---------------------------------------------------------------------------
// 8. Beta G7-G8: MultipleVariables calculated vars -> Cox PH / Logistic / 5ch BubbleScatter
// ---------------------------------------------------------------------------

#[test]
fn beta_g7_g8_multivariate_calculated_vars_cox_logistic_and_bubble_scatter() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Multivariate Cohort Study" }));

    exec_ok(
        &mut session,
        "sheet.createData",
        json!({
            "sheet_id": "mv-cohort",
            "title": "Patient Biomarkers",
            "archetype": "multiple_variables",
            "rows": 16,
            "cols": 6,
            "variables": [
                { "title": "Time", "role": "continuous" },
                { "title": "Event", "role": "categorical" },
                { "title": "ConcRaw", "role": "continuous" },
                { "title": "Age", "role": "continuous" },
                { "title": "Biomarker", "role": "continuous" },
                { "title": "PatientId", "role": "label" }
            ],
            "auto_graph": false
        }),
    );

    // 16 synthetic cohort rows with strong relationship between ConcRaw/Age and survival Event & Time.
    let rows_data: [(f64, i32, f64, f64, f64, &str); 16] = [
        (24.0, 0, 1.0, 42.0, 1.2, "P01"),
        (22.0, 0, 2.0, 45.0, 1.5, "P02"),
        (20.0, 0, 3.0, 47.0, 1.8, "P03"),
        (19.0, 0, 5.0, 49.0, 2.1, "P04"),
        (18.0, 0, 4.0, 51.0, 2.0, "P05"),
        (16.0, 1, 8.0, 52.0, 2.9, "P06"),
        (15.0, 0, 10.0, 53.0, 3.2, "P07"),
        (14.0, 1, 15.0, 55.0, 3.8, "P08"),
        (12.0, 1, 20.0, 58.0, 4.2, "P09"),
        (11.0, 1, 25.0, 60.0, 4.5, "P10"),
        (10.0, 1, 40.0, 62.0, 5.1, "P11"),
        (8.0, 1, 60.0, 64.0, 5.8, "P12"),
        (7.0, 1, 80.0, 66.0, 6.2, "P13"),
        (6.0, 1, 100.0, 68.0, 6.9, "P14"),
        (5.0, 1, 150.0, 71.0, 7.4, "P15"),
        (4.0, 1, 200.0, 74.0, 8.0, "P16"),
    ];

    let mut cells = Vec::with_capacity(rows_data.len() * 6);
    for (r, &(t, ev, conc, age, bio, pid)) in rows_data.iter().enumerate() {
        cells.push(json!({ "row": r, "col": 0, "value": t }));
        cells.push(json!({ "row": r, "col": 1, "value": ev }));
        cells.push(json!({ "row": r, "col": 2, "value": conc }));
        cells.push(json!({ "row": r, "col": 3, "value": age }));
        cells.push(json!({ "row": r, "col": 4, "value": bio }));
        cells.push(json!({ "row": r, "col": 5, "value": pid }));
    }
    exec_ok(&mut session, "table.setCells", json!({ "sheet_id": "mv-cohort", "edits": cells }));

    // Add dependent calculated variables via topological DAG.
    exec_ok(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-cohort", "title": "LogConc", "formula": "log10(ConcRaw)" }));
    exec_ok(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-cohort", "title": "CenteredAge", "formula": "(Age - 50) / 10" }));
    assert_data_cell_number(session.document(), "mv-cohort", 13, 6, 2.0, 1e-9);
    assert_data_cell_number(session.document(), "mv-cohort", 13, 7, 1.8, 1e-9);

    exec_ok(&mut session, "table.setVariableRole", json!({ "sheet_id": "mv-cohort", "col": "Event", "role": "continuous" }));
    let mv_ds = session.document().data_sheet(&SheetId::new("mv-cohort")).unwrap();
    if let TableArchetype::MultipleVariables(mv) = &mv_ds.archetype {
        assert_eq!(mv.variables[1].role, VariableRole::Continuous);
    } else {
        panic!("expected MultipleVariables table");
    }

    // Cox Proportional Hazards (cols 6 = LogConc, 7 = CenteredAge).
    exec_ok(
        &mut session,
        "stats.run",
        json!({
            "source_sheet_id": "mv-cohort",
            "sheet_id": "cox-res",
            "kind": "cox_ph",
            "time_column": 0,
            "status_column": 1,
            "predictor_columns": [6, 7]
        }),
    );
    let cox_res = result_sheet(&session, "cox-res");
    assert_eq!(cox_res.status, ResultStatus::UpToDate);
    assert!(!find_tab(cox_res, ResultTabKind::ParametersAndConfidenceIntervals).rows.is_empty());
    assert!(cox_res.checklist.iter().any(|c| c.assumption.contains("Proportional") || c.assumption.contains("Cox")));

    // Logistic GLM (response = col 1 Event, predictors = [6 LogConc, 7 CenteredAge]).
    let glm_spec = AnalysisSpec::new(
        "logistic-res",
        "mv-cohort",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 1,
            predictor_columns: vec![6, 7],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )
    .unwrap();
    exec_ok(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "mv-cohort", "sheet_id": "logistic-res", "spec": serde_json::to_value(&glm_spec).unwrap() }),
    );
    let log_res = result_sheet(&session, "logistic-res");
    assert_eq!(log_res.status, ResultStatus::UpToDate);
    assert!(!find_tab(log_res, ResultTabKind::ParametersAndConfidenceIntervals).rows.is_empty());

    // 5-channel BubbleScatter: X=LogConc(6), Y=Biomarker(4), Color=Age(3), Size=ConcRaw(2), Label=PatientId(5).
    exec_ok(
        &mut session,
        "graph.create",
        json!({ "sheet_id": "g-bubble", "data_sheet_id": "mv-cohort", "kind": "multiple_variables", "title": "5-Channel Bubble Scatter" }),
    );
    exec_ok(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g-bubble",
            "multivariate": {
                "x_column": 6,
                "y_column": 4,
                "color": { "column": 3, "categorical": false },
                "size": { "column": 2, "min_radius_pt": 3.0, "max_radius_pt": 13.0 },
                "label_column": 5,
                "show_convex_hulls": false
            }
        }),
    );
    let bubble_svg = export_svg_text(&mut session, "g-bubble");
    assert!(bubble_svg.contains("P01") && bubble_svg.contains("P16"), "BubbleScatter SVG must render label channel");
}

// ---------------------------------------------------------------------------
// 9. Beta G11: Multi-panel LayoutSheet + align_plot_origins + badges + CMYK/600DPI/LZW
// ---------------------------------------------------------------------------

#[test]
fn beta_g11_multipanel_layout_aligned_origins_badges_and_publication_exports() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Publication Layout Suite" }));

    // Panel A source: XY curve.
    populate_xy_dose_response(&mut session, "lay-xy", true, false, 100.0);
    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-a", "data_sheet_id": "lay-xy", "kind": "xy", "title": "Potency" }));

    // Panel B source: Column cohorts.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "lay-col", "title": "Groups", "archetype": "column", "rows": 4, "cols": 2, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "lay-col", "text": "10\t25\n12\t28\n11\t26\n9\t24" }));
    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-b", "data_sheet_id": "lay-col", "kind": "column", "title": "Cohort Mean" }));

    // Panel C source: Paired responses.
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "lay-ba", "title": "Matched", "archetype": "column", "design_hint": "paired", "rows": 4, "cols": 2, "auto_graph": false }),
    );
    exec_ok(&mut session, "table.paste", json!({ "sheet_id": "lay-ba", "text": "5\t11\n6\t13\n7\t14\n5.5\t12" }));
    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-c", "data_sheet_id": "lay-ba", "kind": "column", "title": "Paired Shift" }));

    // Assemble 2x2 LayoutSheet with A, B, C badges & align_plot_origins enabled.
    exec_ok(
        &mut session,
        "layout.create",
        json!({
            "sheet_id": "fig-1",
            "title": "Figure 1",
            "width": 180.0,
            "height": 140.0,
            "unit": "mm",
            "grid_template": [2, 2],
            "align_plot_origins": true,
            "panel_badges": "uppercase"
        }),
    );
    exec_ok(&mut session, "layout.addPanel", json!({ "sheet_id": "fig-1", "graph_sheet_id": "g-a", "row": 0, "col": 0 }));
    exec_ok(&mut session, "layout.addPanel", json!({ "sheet_id": "fig-1", "graph_sheet_id": "g-b", "row": 0, "col": 1 }));
    exec_ok(&mut session, "layout.addPanel", json!({ "sheet_id": "fig-1", "graph_sheet_id": "g-c", "row": 1, "col": 0 }));
    let align_out = exec_ok(&mut session, "layout.alignOrigins", json!({ "sheet_id": "fig-1", "align": true, "normalize_bounds": true }));
    assert_eq!(align_out.payload["aligned_panels"], json!(3));

    // Export 1: Multi-panel SVG with A, B, C panel badges.
    let layout_svg = export_svg_text(&mut session, "fig-1");
    assert!(layout_svg.contains(">A<") && layout_svg.contains(">B<") && layout_svg.contains(">C<"), "Layout SVG missing A/B/C badges");

    // Export 2: CMYK PDF (uncompressed check confirms CMYK `k`/`K` paint operators).
    let cmyk_pdf = export_pdf_bytes(&mut session, "fig-1", "cmyk", false);
    let pdf_text = String::from_utf8_lossy(&cmyk_pdf);
    assert!(pdf_text.contains(" k\n") || pdf_text.contains(" K\n"), "CMYK PDF must emit CMYK color operators");

    // Export 3: 600 DPI PNG (~180mm / 25.4 * 600 >= 4200 px wide).
    let (_png600, w_px, h_px) = export_png_bytes(&mut session, "fig-1", 600);
    assert!(w_px >= 4000 && h_px >= 3000, "600 DPI 180x140mm PNG got {w_px}x{h_px}");

    // Export 4: LZW compressed TIFF.
    let tiff_out = exec_ok(&mut session, "export.tiff", json!({ "sheet_id": "fig-1", "dpi": 300.0, "compression": "lzw" }));
    assert_eq!(tiff_out.payload["compression"], json!("lzw"));
    let tiff_bytes = decode_base64(payload_str(&tiff_out, "base64"));
    let valid_tiff_magic = tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*");
    assert!(valid_tiff_magic && tiff_bytes.len() > 200);
}

// ---------------------------------------------------------------------------
// 10. Beta G10: .pzfx & .prism clean-room XML/ZIP import -> re-analysis -> SVG/PDF
// ---------------------------------------------------------------------------

#[test]
fn beta_g10_pzfx_and_prism_cleanroom_imports_reanalysis_and_exports() {
    // Sub-journey A: Clean-room .pzfx XML -> reactive stats re-analysis -> graph -> SVG & PDF.
    let mut pzfx_session = Session::new();
    let pzfx_xml = build_cleanroom_pzfx_xml();
    let pzfx_out = exec_ok(&mut pzfx_session, "table.importPzfx", json!({ "xml": pzfx_xml }));
    assert!(pzfx_out.payload["sheet_count"].as_u64().unwrap() >= 2);

    let pzfx_data_id = pzfx_session.document().data_sheets[0].id.as_str().to_owned();
    let pzfx_info_id = pzfx_session.document().info_sheets[0].id.as_str().to_owned();
    assert_eq!(
        pzfx_session.document().info_sheet(&SheetId::new(&pzfx_info_id)).unwrap().get_constant("Protocol").unwrap().value.display_text(),
        "ELISA-v3"
    );
    // Excluded observation in Vehicle row 4 must stay excluded after XML import.
    assert!(pzfx_session.document().data_sheet(&SheetId::new(&pzfx_data_id)).unwrap().cell(CellCoord::new(4, 0, 0)).unwrap().excluded);

    exec_ok(
        &mut pzfx_session,
        "stats.run",
        json!({ "source_sheet_id": &pzfx_data_id, "sheet_id": "pzfx-anova", "kind": "one_way_anova", "multiplicity": "tukey" }),
    );
    let f_before = scalar_val(result_sheet(&pzfx_session, "pzfx-anova"), "F", None);
    assert!(f_before > 100.0);

    // Edit imported cell -> ANOVA updates reactively.
    set_cell(&mut pzfx_session, &pzfx_data_id, false, 0, 2, 0, 65.0);
    let f_after = scalar_val(result_sheet(&pzfx_session, "pzfx-anova"), "F", None);
    assert!((f_after - f_before).abs() > 1.0);

    exec_ok(
        &mut pzfx_session,
        "graph.create",
        json!({ "sheet_id": "g-pzfx", "data_sheet_id": &pzfx_data_id, "kind": "column", "title": "Imported XML Cohort" }),
    );
    let _pzfx_svg = export_svg_text(&mut pzfx_session, "g-pzfx");
    let _pzfx_pdf = export_pdf_bytes(&mut pzfx_session, "g-pzfx", "srgb", true);

    // Sub-journey B: Clean-room .prism ZIP container -> reactive 4PL re-analysis -> graph -> SVG & PDF.
    let mut zip_session = Session::new();
    let zip_b64 = encode_base64(&build_cleanroom_prism_zip_bytes());
    let zip_out = exec_ok(&mut zip_session, "table.importPrism", json!({ "bytes_base64": zip_b64 }));
    assert!(zip_out.payload["sheet_count"].as_u64().unwrap() >= 2);

    let zip_data_id = zip_session.document().data_sheets[0].id.as_str().to_owned();
    let zip_info = &zip_session.document().info_sheets[0];
    assert_eq!(zip_info.get_constant("AssayCode").unwrap().value.display_text(), "ZIP-901");

    exec_ok(&mut zip_session, "fit.run", json!({ "source_sheet_id": &zip_data_id, "sheet_id": "zip-fit", "kind": "four_parameter_logistic" }));
    let ec50_before = scalar_val(result_sheet(&zip_session, "zip-fit"), "LogEC50", Some(0));
    assert!((ec50_before - (-6.05)).abs() < 0.3);

    set_cell(&mut zip_session, &zip_data_id, false, 3, 0, 0, 12.0);
    let ec50_after = scalar_val(result_sheet(&zip_session, "zip-fit"), "LogEC50", Some(0));
    assert!((ec50_after - ec50_before).abs() > 0.05);

    exec_ok(
        &mut zip_session,
        "graph.create",
        json!({ "sheet_id": "g-zip", "data_sheet_id": &zip_data_id, "kind": "xy", "title": "Imported Archive Fit" }),
    );
    exec_ok(&mut zip_session, "graph.setRegressionOverlay", json!({ "sheet_id": "g-zip", "result_sheet_id": "zip-fit" }));
    let _zip_svg = export_svg_text(&mut zip_session, "g-zip");
    let _zip_pdf = export_pdf_bytes(&mut zip_session, "g-zip", "srgb", true);
}
