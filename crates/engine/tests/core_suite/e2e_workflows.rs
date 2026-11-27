//! End-to-end verification journeys through `curvebook_engine::Session` covering
//! Alpha workflows 1-5, Core reactive/statistical/graphical workflows, and Beta G7-G11 additions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, BreakOrnament, CellCoord, CellValue, ErrorPropagationMode, FactorialAnovaSpec,
    FitMethod, GlmFamily, GlmSpec, GraphicalDataObject, HookSource, MathTransformPreset, ModelComparisonSpec, MultiplicityMethod, MultiplicitySpec,
    NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, RepeatedMeasuresDesign, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SheetId, TableArchetype, TransformAnalysisSpec, VariableRole,
};
use curvebook_engine::{CommandOutcome, Session};
use curvebook_format::ZipWriter;
use curvebook_palette::Rgba8;
use serde_json::{Value, json};

use super::common::{cell_num, cell_val, decode_base64, ds, encode_base64, gs, info, ls, ok_cmd as exec_ok, rs, run_cmds, x_num};

fn payload_str<'a>(outcome: &'a CommandOutcome, key: &str) -> &'a str {
    outcome.payload.get(key).and_then(Value::as_str).unwrap_or_else(|| panic!("missing `{key}` in `{}`: {}", outcome.command_id, outcome.payload))
}

fn find_tab(sheet: &ResultSheet, kind: ResultTabKind) -> &ResultTab {
    sheet.tabs.iter().find(|t| t.kind == kind).unwrap_or_else(|| panic!("missing `{kind:?}` in `{}`", sheet.id))
}

fn scalar_val(sheet: &ResultSheet, key: &str, ds_idx: Option<usize>) -> f64 {
    sheet.lookup_scalar(key, ds_idx).unwrap_or_else(|| panic!("missing scalar `{key}` (ds={ds_idx:?}) in `{}`", sheet.id))
}

fn hill_response(log_x: f64, bottom: f64, top: f64, log_ec50: f64, slope: f64) -> f64 {
    bottom + (top - bottom) / (1.0 + 10_f64.powf((log_ec50 - log_x) * slope))
}

fn set_cell(session: &mut Session, sheet_id: &str, row: usize, col: usize, subcol: usize, value: f64) {
    exec_ok(session, "table.setCells", json!({ "sheet_id": sheet_id, "edits": [{ "row": row, "col": col, "subcol": subcol, "value": value }] }));
}

fn populate_xy_dose_response(session: &mut Session, sheet_id: &str, log_x: bool, include_unknowns: bool, top: f64) {
    let rows = if include_unknowns { 10 } else { 8 };
    exec_ok(
        session,
        "sheet.createData",
        json!({ "sheet_id": sheet_id, "title": "Agonist Dose Response", "archetype": "xy", "rows": rows, "cols": 1, "replicates": 2, "x_title": if log_x { "Log10 [Agonist]" } else { "Agonist [M]" }, "auto_info": true, "auto_graph": false }),
    );
    let log_doses = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let x_cells: Vec<Value> =
        log_doses.iter().enumerate().map(|(r, &lx)| json!({ "row": r, "value": if log_x { lx } else { 10_f64.powf(lx) } })).collect();
    let mut y_cells: Vec<Value> = log_doses
        .iter()
        .enumerate()
        .flat_map(|(r, &lx)| {
            let ideal = hill_response(lx, 5.0, top, -6.0, 1.15);
            [json!({ "row": r, "col": 0, "subcol": 0, "value": ideal - 0.8 }), json!({ "row": r, "col": 0, "subcol": 1, "value": ideal + 0.8 })]
        })
        .collect();
    if include_unknowns {
        y_cells.push(json!({ "row": 8, "col": 0, "subcol": 0, "value": hill_response(-6.2, 5.0, top, -6.0, 1.15) }));
        y_cells.push(json!({ "row": 9, "col": 0, "subcol": 0, "value": hill_response(-5.4, 5.0, top, -6.0, 1.15) }));
    }
    run_cmds(
        session,
        &[
            ("table.setXCells", json!({ "sheet_id": sheet_id, "edits": x_cells })),
            ("table.setCells", json!({ "sheet_id": sheet_id, "edits": y_cells })),
        ],
    );
}

fn export_svg_text(session: &mut Session, sheet_id: &str) -> String {
    let out = exec_ok(session, "export.svg", json!({ "sheet_id": sheet_id }));
    let svg = payload_str(&out, "svg").to_owned();
    assert!(svg.contains("<svg") && svg.contains("</svg>"), "invalid SVG for `{sheet_id}`");
    svg
}

fn export_pdf_bytes(session: &mut Session, sheet_id: &str, color_space: &str, compress_streams: bool) -> Vec<u8> {
    let out = exec_ok(session, "export.pdf", json!({ "sheet_id": sheet_id, "color_space": color_space, "compress_streams": compress_streams }));
    let bytes = decode_base64(payload_str(&out, "base64"));
    assert!(bytes.starts_with(b"%PDF-"), "expected PDF header for `{sheet_id}`");
    bytes
}

fn export_png_bytes(session: &mut Session, sheet_id: &str, dpi: u64) -> (Vec<u8>, u64, u64) {
    let out = exec_ok(session, "export.png", json!({ "sheet_id": sheet_id, "dpi": dpi }));
    let bytes = decode_base64(payload_str(&out, "base64"));
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]));
    assert_eq!(out.payload["dpi"].as_u64(), Some(dpi));
    (bytes, out.payload["width_px"].as_u64().unwrap(), out.payload["height_px"].as_u64().unwrap())
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
    AnalysisSpec::new(
        result_id,
        source_id,
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            equation_id: eq_id.to_owned(),
            method,
            parameter_rules: rules,
            comparison,
            confidence_bands: true,
            interpolate_unknowns,
            ..NonlinearFitSpec::new(eq_id)
        }),
    )
    .unwrap()
}

fn assert_data_cell_number(session: &Session, sheet_id: &str, row: usize, col: usize, expected: f64, tol: f64) {
    let val = ds(session, sheet_id).cell(CellCoord::new(row, col, 0)).and_then(|c| c.active_number()).unwrap();
    assert!((val - expected).abs() <= tol, "({row},{col}) in `{sheet_id}` = {val}, expected {expected} +/- {tol}");
}

fn build_cleanroom_pzfx_xml() -> &'static str {
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
      <YColumn Width="80" Decimals="2" Subcolumns="1"><Title>Vehicle</Title><Subcolumn><d>12.0</d><d>13.4</d><d>11.8</d><d>12.9</d><d Excluded="1">98.5</d></Subcolumn></YColumn>
      <YColumn Width="80" Decimals="2" Subcolumns="1"><Title>Agonist 10nM</Title><Subcolumn><d>24.5</d><d>26.1</d><d>25.2</d><d>27.0</d></Subcolumn></YColumn>
      <YColumn Width="80" Decimals="2" Subcolumns="1"><Title>Agonist 100nM</Title><Subcolumn><d>41.2</d><d>43.0</d><d>39.8</d><d>42.4</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#
}

fn build_cleanroom_prism_zip_bytes() -> Vec<u8> {
    let mut zip = ZipWriter::new();
    let manifest = json!({ "title": "Cleanroom ZIP Study", "sheets": [{ "id": "zip-xy", "title": "Imported Log-Dose Curve", "kind": "xy", "replicates": 1, "csv_path": "data/dose_curve.csv" }] });
    let info_meta = json!({ "id": "zip-info", "title": "Study Constants", "notes": "Cleanroom ZIP study info", "constants": { "AssayCode": "ZIP-901", "IncubationMin": 45 } });
    zip.add_file("manifest.json", serde_json::to_vec(&manifest).unwrap()).unwrap();
    zip.add_file("info/study.json", serde_json::to_vec(&info_meta).unwrap()).unwrap();
    zip.add_file("data/dose_curve.csv", b"LogConc,Response\n-9,5.2\n-8,9.8\n-7,24.1\n-6,52.0\n-5,84.3\n-4,97.4\n".to_vec()).unwrap();
    zip.finish().unwrap()
}

fn assert_4pl_replicate_interpolation(session: &Session, fit_res: &ResultSheet) {
    let mean_tab = fit_res.tabs.iter().find(|t| t.id == "nonlinear_interpolated_mean_x").unwrap();
    let rep_tab = fit_res.tabs.iter().find(|t| t.id == "nonlinear_interpolated_replicates").unwrap();
    assert_eq!((mean_tab.title.as_str(), mean_tab.kind, mean_tab.rows.len()), ("Interpolated Mean X", ResultTabKind::InterpolatedUnknowns, 2));
    assert_eq!((rep_tab.title.as_str(), rep_tab.kind, rep_tab.rows.len()), ("Interpolated Replicates", ResultTabKind::InterpolatedUnknowns, 2));
    for (idx, expected_lx) in [-6.2_f64, -5.4_f64].iter().enumerate() {
        for tab in [mean_tab, rep_tab] {
            let Some(ResultCell::Number { value, .. }) = tab.rows.get(idx).and_then(|r| r.get(1)) else { panic!("expected numeric X") };
            assert!((value - expected_lx).abs() < 0.05);
        }
    }

    let doc_ref = session.document();
    let ds_vec = curvebook_fit::extract_datasets_from_sheet(ds(session, "dose-log"), &[]).unwrap();
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
        &curvebook_fit::SolverConfig { interpolate_unknowns: true, ..Default::default() },
    )
    .unwrap();
    assert_eq!(nl_res.interpolated_x.len(), 4);
    let mean_items: Vec<_> = nl_res.interpolated_x.iter().filter(|i| i.replicate_mean_count.is_some()).collect();
    let rep_items: Vec<_> = nl_res.interpolated_x.iter().filter(|i| i.replicate_subcolumn == Some(0)).collect();
    assert_eq!((mean_items.len(), rep_items.len()), (2, 2));
    for (&expected_lx, (m, r)) in [-6.2_f64, -5.4_f64].iter().zip(mean_items.iter().zip(rep_items.iter())) {
        assert!((m.output_estimate - expected_lx).abs() < 0.05 && (r.output_estimate - expected_lx).abs() < 0.05);
    }
}

#[test]
fn alpha_workflows_1_to_3_xy_transform_4pl_interpolation_exports_and_roundtrip() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Alpha Workflows 1-3" }));
    populate_xy_dose_response(&mut session, "dose-raw", false, false, 100.0);
    run_cmds(
        &mut session,
        &[
            ("info.setConstant", json!({ "sheet_id": "dose-raw", "name": "Compound", "value": "LC-402" })),
            (
                "transform.run",
                json!({ "source_sheet_id": "dose-raw", "sheet_id": "log-x-res", "kind": AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), y_transform: None, error_propagation: ErrorPropagationMode::ReplicateEndpoints, pharmacology_preset: None, interchange_xy: false, zero_x_substitute: None, asymmetric_ci_policy: Default::default() }) }),
            ),
            (
                "transform.run",
                json!({ "source_sheet_id": "dose-raw", "sheet_id": "norm-res", "kind": "normalize", "zero_anchor": "smallest", "hundred_anchor": "largest" }),
            ),
        ],
    );
    let log_data = rs(&session, "log-x-res").analyzable_data.as_ref().unwrap();
    let TableArchetype::Xy(log_xy) = &log_data.archetype else { panic!("expected Xy") };
    assert!((log_xy.x_cells[0][0].active_number().unwrap() + 9.0).abs() < 1e-9);
    assert!((log_data.cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap() - 4.23).abs() < 0.05);
    assert!(rs(&session, "norm-res").analyzable_data.as_ref().unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap() < 5.0);

    // Chained multi-hop DAG via analyzable ResultSheets: dose-raw -> log10 -> normalize -> 4PL fit.
    let mut chained_doc = session.document().clone();
    chained_doc
        .add_result_sheet(
            ResultSheet::new(
                AnalysisSpec::new(
                    "chain-norm",
                    "log-x-res",
                    AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                        zero_anchor: NormalizeAnchor::ColumnMin,
                        hundred_anchor: NormalizeAnchor::ColumnMax,
                        output_percent: true,
                        per_data_set: true,
                    }),
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    chained_doc
        .add_result_sheet(
            ResultSheet::new(make_nonlinear_spec(
                "chain-fit",
                "chain-norm",
                "log_agonist_vs_response_4pl",
                FitMethod::LeastSquares,
                Vec::new(),
                None,
                false,
            ))
            .unwrap(),
        )
        .unwrap();
    let mut chained_session = Session::from_document(chained_doc);
    chained_session.recalculate_all().unwrap();
    assert!((scalar_val(rs(&chained_session, "chain-fit"), "LogEC50", Some(0)) + 6.0).abs() < 0.08);

    // Direct 4PL + unknown X interpolation + linear standard curve + Graph + exports + save/open/undo/redo.
    populate_xy_dose_response(&mut session, "dose-log", true, true, 100.0);
    exec_ok(
        &mut session,
        "fit.run",
        json!({ "source_sheet_id": "dose-log", "sheet_id": "fit-4pl", "kind": "four_parameter_logistic", "interpolate_unknowns": true, "confidence_bands": true }),
    );
    let fit_res = rs(&session, "fit-4pl");
    assert_eq!(fit_res.status, ResultStatus::UpToDate);
    let log_ec50 = scalar_val(fit_res, "LogEC50", Some(0));
    assert!((log_ec50 + 6.0).abs() < 0.08 && scalar_val(fit_res, "R2", None) > 0.99);
    assert_4pl_replicate_interpolation(&session, fit_res);

    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "std-curve", "title": "Linear Standard Curve", "archetype": "xy", "rows": 5, "cols": 1, "replicates": 1 }),
            ),
            (
                "table.setXCells",
                json!({ "sheet_id": "std-curve", "edits": [{ "row": 0, "value": 1.0 }, { "row": 1, "value": 2.0 }, { "row": 2, "value": 4.0 }] }),
            ),
            (
                "table.setCells",
                json!({ "sheet_id": "std-curve", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 3.0 }, { "row": 1, "col": 0, "subcol": 0, "value": 5.0 }, { "row": 2, "col": 0, "subcol": 0, "value": 9.0 }, { "row": 3, "col": 0, "subcol": 0, "value": 7.0 }, { "row": 4, "col": 0, "subcol": 0, "value": 11.0 }] }),
            ),
            (
                "fit.run",
                json!({ "source_sheet_id": "std-curve", "sheet_id": "fit-lin-std", "kind": "linear_regression", "interpolate_unknowns": true }),
            ),
        ],
    );
    let interp_tab = find_tab(rs(&session, "fit-lin-std"), ResultTabKind::InterpolatedUnknowns);
    assert_eq!(interp_tab.rows.len(), 2);
    for (idx, expected_x) in [3.0_f64, 5.0_f64].iter().enumerate() {
        let ResultCell::Number { value, .. } = interp_tab.rows[idx][1] else { panic!("expected numeric X") };
        assert!((value - expected_x).abs() < 1e-6);
    }

    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-dose", "data_sheet_id": "dose-log", "kind": "xy", "title": "4PL Standard Curve" })),
            ("graph.setAxes", json!({ "sheet_id": "g-dose", "x_axis": { "title": "Log10 [Agonist]" }, "y_axis": { "title": "Response (%)" } })),
            (
                "graph.setRegressionOverlay",
                json!({ "sheet_id": "g-dose", "result_sheet_id": "fit-4pl", "show_confidence_band": true, "shaded_ribbon": true }),
            ),
        ],
    );

    let pre_save_svg = export_svg_text(&mut session, "g-dose");
    let _pdf = export_pdf_bytes(&mut session, "g-dose", "srgb", true);
    let (_png, png_w, png_h) = export_png_bytes(&mut session, "g-dose", 300);
    assert!(png_w > 500 && png_h > 300);
    let archive_b64 = payload_str(&exec_ok(&mut session, "project.save", json!({ "return_bytes": true })), "bytes_base64").to_owned();

    set_cell(&mut session, "dose-log", 3, 0, 0, 88.0);
    let log_ec50_mut = scalar_val(rs(&session, "fit-4pl"), "LogEC50", Some(0));
    assert!((log_ec50_mut - log_ec50).abs() > 0.02);

    assert_eq!(exec_ok(&mut session, "edit.undo", json!({})).payload["applied"], true);
    assert!((scalar_val(rs(&session, "fit-4pl"), "LogEC50", Some(0)) - log_ec50).abs() < 1e-12);
    assert_eq!(export_svg_text(&mut session, "g-dose"), pre_save_svg);

    assert_eq!(exec_ok(&mut session, "edit.redo", json!({})).payload["applied"], true);
    assert!((scalar_val(rs(&session, "fit-4pl"), "LogEC50", Some(0)) - log_ec50_mut).abs() < 1e-12);
    exec_ok(&mut session, "edit.undo", json!({}));
    assert!((scalar_val(rs(&session, "fit-4pl"), "LogEC50", Some(0)) - log_ec50).abs() < 1e-12);

    let mut reopened = Session::new();
    run_cmds(&mut reopened, &[("project.open", json!({ "bytes_base64": archive_b64 })), ("project.recalculate", json!({}))]);
    assert!((scalar_val(rs(&reopened, "fit-4pl"), "LogEC50", Some(0)) - log_ec50).abs() < 1e-12);
    assert_eq!(export_svg_text(&mut reopened, "g-dose"), pre_save_svg);
}

#[test]
fn alpha_workflows_4_and_5_column_grouped_descriptive_t_anova_tukey_and_exports() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Alpha Workflows 4-5" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "col-3g", "title": "Biomarker Cohorts", "archetype": "column", "rows": 5, "cols": 3, "auto_graph": false }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "col-3g", "titles": ["Vehicle", "LowDose", "HighDose"] })),
            (
                "table.paste",
                json!({ "sheet_id": "col-3g", "text": "10.2\t18.1\t32.4\n11.4\t19.0\t34.1\n9.8\t17.4\t30.9\n10.6\t18.8\t33.5\n11.0\t18.5\t31.8" }),
            ),
            ("stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "desc-col", "kind": "descriptive" })),
            ("stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "anova-1w", "kind": "one_way_anova", "multiplicity": "tukey" })),
            ("graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-3g", "kind": "column", "title": "Cohorts" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "col-unpaired", "title": "Two Group Screen", "archetype": "column", "rows": 5, "cols": 2, "auto_graph": false }),
            ),
            ("table.paste", json!({ "sheet_id": "col-unpaired", "text": "4.1\t9.2\n4.5\t9.8\n3.9\t8.9\n4.3\t9.5\n4.2\t9.4" })),
            ("stats.run", json!({ "source_sheet_id": "col-unpaired", "sheet_id": "t-unpaired", "kind": "unpaired_t" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "col-paired", "title": "Before After Assay", "archetype": "column", "design_hint": "paired", "rows": 5, "cols": 2, "auto_graph": false }),
            ),
            ("table.paste", json!({ "sheet_id": "col-paired", "text": "20.0\t27.5\n22.1\t29.4\n18.9\t26.0\n21.4\t28.9\n19.6\t27.1" })),
            ("stats.run", json!({ "source_sheet_id": "col-paired", "sheet_id": "t-paired", "kind": "paired_t" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "grp-2w", "title": "Treatment x Genotype", "archetype": "grouped", "rows": 2, "cols": 2, "replicates": 3, "row_factor_title": "Treatment", "column_factor_title": "Genotype", "auto_graph": false }),
            ),
            ("table.setRowTitles", json!({ "sheet_id": "grp-2w", "titles": ["Control", "Drug"] })),
            ("table.setGroupTitles", json!({ "sheet_id": "grp-2w", "titles": ["WT", "KO"] })),
        ],
    );
    let anova_res = rs(&session, "anova-1w");
    assert!(!find_tab(rs(&session, "desc-col"), ResultTabKind::DescriptiveSummary).rows.is_empty());
    assert!(
        scalar_val(anova_res, "F", None) > 50.0
            && anova_res.pairwise_comparisons.len() == 3
            && anova_res.pairwise_comparisons.iter().all(|c| c.significant)
    );
    assert!(scalar_val(rs(&session, "t-unpaired"), "PValue", None) < 0.001 && scalar_val(rs(&session, "t-paired"), "PValue", None) < 0.001);
    let _ = (export_svg_text(&mut session, "g-col"), export_pdf_bytes(&mut session, "g-col", "srgb", true));

    let grp_edits: Vec<Value> = [(0, 0, [10.0, 10.5, 9.8]), (0, 1, [11.2, 10.8, 11.4]), (1, 0, [24.0, 25.1, 24.4]), (1, 1, [14.5, 15.0, 14.2])]
        .iter()
        .flat_map(|&(r, c, vs)| vs.into_iter().enumerate().map(move |(s, v)| json!({ "row": r, "col": c, "subcol": s, "value": v })))
        .collect();
    run_cmds(
        &mut session,
        &[
            ("table.setCells", json!({ "sheet_id": "grp-2w", "edits": grp_edits })),
            ("stats.run", json!({ "source_sheet_id": "grp-2w", "sheet_id": "anova-2w", "kind": "two_way_anova", "multiplicity": "tukey" })),
            ("graph.create", json!({ "sheet_id": "g-grp", "data_sheet_id": "grp-2w", "kind": "grouped", "title": "2-Way Grouped" })),
        ],
    );
    assert_eq!(rs(&session, "anova-2w").status, ResultStatus::UpToDate);
    assert!(!find_tab(rs(&session, "anova-2w"), ResultTabKind::AnovaTable).rows.is_empty());
    let _ = export_svg_text(&mut session, "g-grp");
}

#[test]
fn core_workflow_live_paste_link_reactive_transpose_and_4stage_csv_paste() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Core Link and 4-Stage CSV" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "csv-target", "title": "4-Stage CSV", "archetype": "column", "rows": 4, "cols": 4, "auto_graph": false }),
            ),
            (
                "table.paste",
                json!({
                    "sheet_id": "csv-target",
                    "text": "10,5;20,5;30,5;99,9\n1,0;2,0;3,0;88,8\n12,5*;22,5;32,5;77,7\n2,0;4,0;6,0;66,6\n14,0;24,0;34,0;55,5\n",
                    "delimiter": ";",
                    "decimal_separator": ",",
                    "transpose": true,
                    "filter": { "row_start": 0, "row_end": 4, "col_start": 0, "col_end": 2, "decimate_every_k": 2, "parse_asterisk_exclusions": true }
                }),
            ),
            (
                "sheet.createData",
                json!({ "sheet_id": "src-plate", "title": "Plate Raw", "archetype": "column", "rows": 3, "cols": 3, "auto_graph": false }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "src-plate", "titles": ["Blank", "DoseA", "DoseB"] })),
            ("table.paste", json!({ "sheet_id": "src-plate", "text": "2.0\t12.0\t22.0\n2.0\t14.0\t24.0\n2.0\t16.0\t26.0" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "dst-linked", "title": "Linked Transposed", "archetype": "column", "rows": 3, "cols": 3, "auto_info": true, "auto_graph": false }),
            ),
        ],
    );
    assert_eq!(cell_num(&session, "csv-target", 0, 0, 0), Some(10.5));
    let excl = cell_val(&session, "csv-target", 0, 1, 0).unwrap();
    assert!(excl.excluded && excl.value == CellValue::Number(12.5));
    assert_eq!(cell_num(&session, "csv-target", 2, 2, 0), Some(34.0));

    let link_out = exec_ok(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src-plate", "target_sheet_id": "dst-linked", "transpose": true, "propagate_title_header": true }),
    );
    assert_eq!(link_out.payload["linked_regions"], 1);
    assert_data_cell_number(&session, "dst-linked", 1, 0, 12.0, 1e-9);
    assert_data_cell_number(&session, "dst-linked", 2, 0, 22.0, 1e-9);

    exec_ok(&mut session, "stats.run", json!({ "source_sheet_id": "dst-linked", "sheet_id": "dst-desc", "kind": "descriptive" }));
    assert!((scalar_val(rs(&session, "dst-desc"), "Mean", Some(0)) - 12.0).abs() < 1e-9);

    set_cell(&mut session, "src-plate", 0, 2, 0, 52.0);
    assert_data_cell_number(&session, "dst-linked", 2, 0, 52.0, 1e-9);
    assert!((scalar_val(rs(&session, "dst-desc"), "Mean", Some(0)) - 22.0).abs() < 1e-9);
}

#[test]
fn core_workflow_kruskal_wallis_dunn_and_rm_reml_mixed_anova() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Nonparametric and REML Mixed ANOVA" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "kw-data", "title": "Skewed Counts", "archetype": "column", "rows": 5, "cols": 3, "auto_graph": false }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "kw-data", "titles": ["Ctrl", "Mid", "High"] })),
            (
                "table.paste",
                json!({ "sheet_id": "kw-data", "text": "1.0\t12.0\t80.0\n2.0\t15.0\t95.0\n1.5\t14.0\t110.0\n3.0\t18.0\t150.0\n2.5\t16.0\t220.0" }),
            ),
            ("stats.run", json!({ "source_sheet_id": "kw-data", "sheet_id": "kw-res", "kind": "kruskal_wallis", "multiplicity": "dunn" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "rm-data", "title": "Longitudinal Cohort", "archetype": "grouped", "rows": 2, "cols": 3, "replicates": 4, "row_factor_title": "Arm", "column_factor_title": "Timepoint", "auto_graph": false }),
            ),
        ],
    );
    let kw_res = rs(&session, "kw-res");
    assert_eq!(kw_res.status, ResultStatus::UpToDate);
    assert!(scalar_val(kw_res, "Kruskal-Wallis H", None) > 10.0 && scalar_val(kw_res, "P value", None) < 0.01);
    assert!(kw_res.pairwise_comparisons.len() == 3 && kw_res.pairwise_comparisons.iter().all(|c| c.raw_p_or_q.is_finite()));

    let rm_edits: Vec<Value> = [
        (0, 0, [10.0, 11.0, 9.5, 10.5]),
        (0, 1, [11.0, 12.0, 10.8, 11.4]),
        (0, 2, [11.5, 12.2, 11.1, 11.9]),
        (1, 0, [10.2, 10.9, 9.8, 10.4]),
        (1, 1, [18.5, 19.8, 17.6, 18.9]),
        (1, 2, [26.0, 28.1, 25.3, 27.2]),
    ]
    .iter()
    .flat_map(|&(r, c, vs)| vs.into_iter().enumerate().map(move |(s, v)| json!({ "row": r, "col": c, "subcol": s, "value": v })))
    .collect();
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
    run_cmds(
        &mut session,
        &[
            ("table.setCells", json!({ "sheet_id": "rm-data", "edits": rm_edits })),
            ("stats.run", json!({ "source_sheet_id": "rm-data", "sheet_id": "rm-balanced", "spec": serde_json::to_value(&rm_spec).unwrap() })),
        ],
    );
    assert!(!find_tab(rs(&session, "rm-balanced"), ResultTabKind::AnovaTable).rows.is_empty());

    run_cmds(
        &mut session,
        &[
            (
                "table.clearCells",
                json!({ "sheet_id": "rm-data", "cells": [{ "row": 1, "col": 2, "subcol": 1 }, { "row": 0, "col": 1, "subcol": 3 }] }),
            ),
            (
                "stats.run",
                json!({ "source_sheet_id": "rm-data", "sheet_id": "rm-unstructured", "kind": "two_way_anova", "repeated_measures": "column", "residual_covariance": "unstructured" }),
            ),
        ],
    );
    assert_eq!(rs(&session, "rm-balanced").status, ResultStatus::UpToDate);
    assert!(!find_tab(rs(&session, "rm-balanced"), ResultTabKind::AnovaTable).rows.is_empty());

    let un_res = rs(&session, "rm-unstructured");
    assert_eq!(un_res.status, ResultStatus::UpToDate);
    assert_eq!(un_res.tabs.iter().filter(|t| t.kind == ResultTabKind::CovarianceMatrix).count(), 2);
    assert!(un_res.scalars.iter().any(|s| s.key == "REML mean diagonal variance" && s.value.is_finite()));
    let methods = curvebook_engine::methods_report::generate_methods_report(
        session.document(),
        &SheetId::new("rm-unstructured"),
        curvebook_engine::methods_report::ReportFormat::PlainText,
    )
    .unwrap();
    assert!(methods.contains("unstructured residual covariance"));
}

#[test]
fn core_workflow_rout_shared_global_4pl_aicc_extra_ss_f_and_ci_ribbons() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "ROUT Global 4PL Comparison" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "xy-2curves", "title": "Two Agonists Shared Asymptotes", "archetype": "xy", "rows": 9, "cols": 2, "replicates": 2, "auto_graph": false }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "xy-2curves", "titles": ["Agonist A", "Agonist B"] })),
        ],
    );
    let log_x = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.0, -4.0];
    let x_cells: Vec<Value> = log_x.iter().enumerate().map(|(r, &lx)| json!({ "row": r, "value": lx })).collect();
    let y_cells: Vec<Value> = log_x
        .iter()
        .enumerate()
        .flat_map(|(r, &lx)| {
            let (ya, yb) = (hill_response(lx, 10.0, 95.0, -7.0, 1.5), hill_response(lx, 10.0, 95.0, -5.8, 1.5));
            [
                json!({ "row": r, "col": 0, "subcol": 0, "value": ya - 0.6 }),
                json!({ "row": r, "col": 0, "subcol": 1, "value": if r == 7 { 195.0 } else { ya + 0.6 } }),
                json!({ "row": r, "col": 1, "subcol": 0, "value": yb - 0.6 }),
                json!({ "row": r, "col": 1, "subcol": 1, "value": yb + 0.6 }),
            ]
        })
        .collect();
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
    let spec_aicc = make_nonlinear_spec(
        "fit-global-aicc",
        "xy-2curves",
        "log_agonist_vs_response_4pl",
        FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        shared_rules,
        Some(ModelComparisonSpec::Aicc { alternative_equation_id: String::from("log_agonist_vs_response_3pl") }),
        false,
    );

    run_cmds(
        &mut session,
        &[
            ("table.setXCells", json!({ "sheet_id": "xy-2curves", "edits": x_cells })),
            ("table.setCells", json!({ "sheet_id": "xy-2curves", "edits": y_cells })),
            (
                "sheet.createData",
                json!({ "sheet_id": "rout-col", "title": "Outlier Screen", "archetype": "column", "rows": 8, "cols": 1, "auto_graph": false }),
            ),
            ("table.paste", json!({ "sheet_id": "rout-col", "text": "10.1\n10.4\n9.9\n10.2\n10.0\n280.0\n10.3\n9.8" })),
            ("stats.run", json!({ "source_sheet_id": "rout-col", "sheet_id": "rout-res", "kind": "outliers", "q_percent": 1.0 })),
            ("fit.run", json!({ "source_sheet_id": "xy-2curves", "sheet_id": "fit-global-f", "spec": serde_json::to_value(&spec_f).unwrap() })),
            ("fit.run", json!({ "source_sheet_id": "xy-2curves", "sheet_id": "fit-global-aicc", "spec": serde_json::to_value(&spec_aicc).unwrap() })),
            ("graph.create", json!({ "sheet_id": "g-global", "data_sheet_id": "xy-2curves", "kind": "xy", "title": "Global 4PL + CI Ribbon" })),
            (
                "graph.setRegressionOverlay",
                json!({ "sheet_id": "g-global", "result_sheet_id": "fit-global-f", "all_series": true, "show_confidence_band": true, "shaded_ribbon": true }),
            ),
        ],
    );
    assert!(scalar_val(rs(&session, "rout-res"), "OutlierCount", Some(0)) >= 1.0);
    let res_f = rs(&session, "fit-global-f");
    assert_eq!(res_f.status, ResultStatus::UpToDate);
    assert!((scalar_val(res_f, "LogEC50", Some(0)) + 7.0).abs() < 0.15 && (scalar_val(res_f, "LogEC50", Some(1)) + 5.8).abs() < 0.15);
    assert!(
        !find_tab(res_f, ResultTabKind::ModelComparison).rows.is_empty()
            && !find_tab(rs(&session, "fit-global-aicc"), ResultTabKind::ModelComparison).rows.is_empty()
    );
    let svg = export_svg_text(&mut session, "g-global");
    assert!(svg.contains("<path") || svg.contains("<polygon"));
}

#[test]
fn core_workflow_broken_axis_violin_before_after_and_stacked_brackets() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Broken Axes Violin Brackets" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "xy-gap", "title": "Bimodal Range", "archetype": "xy", "rows": 6, "cols": 1, "replicates": 1, "auto_graph": false }),
            ),
            ("table.paste", json!({ "sheet_id": "xy-gap", "text": "1\t5\n2\t9\n3\t14\n4\t220\n5\t360\n6\t480" })),
            ("graph.create", json!({ "sheet_id": "g-broken", "data_sheet_id": "xy-gap", "kind": "xy", "title": "Segmented Y Axis" })),
            (
                "graph.setAxes",
                json!({ "sheet_id": "g-broken", "y_axis": { "title": "Signal", "gap_width_pt": 7.0, "break_ornament": "double_slash", "segments": [{ "scale": "linear", "min": 0.0, "max": 20.0, "length_share": 0.4 }, { "scale": "linear", "min": 180.0, "max": 520.0, "length_share": 0.6 }] } }),
            ),
        ],
    );
    assert_eq!(
        (gs(&session, "g-broken").spec.y_axis.segments.len(), gs(&session, "g-broken").spec.y_axis.break_ornament),
        (2, BreakOrnament::DoubleSlash)
    );
    let _ = export_svg_text(&mut session, "g-broken");

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
    assert_eq!(gs(&session, "g-broken").spec.y_axis.segments.len(), 3);
    let _ = export_svg_text(&mut session, "g-broken");

    // Violin + stacked brackets + paired BeforeAfterLines.
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "col-violin", "title": "Violin Cohorts", "archetype": "column", "rows": 8, "cols": 3, "auto_graph": false }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "col-violin", "titles": ["Ctrl", "Tx1", "Tx2"] })),
            (
                "table.paste",
                json!({ "sheet_id": "col-violin", "text": "10\t22\t38\n11\t24\t40\n9\t20\t36\n12\t25\t42\n10.5\t23\t39\n11.2\t21\t41\n9.6\t24\t37\n10.8\t26\t43" }),
            ),
            ("stats.run", json!({ "source_sheet_id": "col-violin", "sheet_id": "violin-anova", "kind": "one_way_anova", "multiplicity": "tukey" })),
            ("graph.create", json!({ "sheet_id": "g-violin", "data_sheet_id": "col-violin", "kind": "column", "title": "Violin + Brackets" })),
        ],
    );
    let gdo_val = serde_json::to_value(GraphicalDataObject::Violin {
        fill_color: Rgba8::rgba(59, 130, 246, 175),
        stroke_color: Rgba8::rgba(30, 64, 175, 255),
        clip_to_observed_range: true,
        show_median_and_quartiles: true,
        half_violin_with_points: true,
    })
    .unwrap();
    for i in 0..3 {
        exec_ok(&mut session, "graph.setSeries", json!({ "sheet_id": "g-violin", "series_index": i, "gdo": gdo_val }));
    }
    let bracket_out = exec_ok(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-violin", "result_sheet_id": "violin-anova", "style": "square", "label_mode": "numeric", "bracket_step_pt": 13.0 }),
    );
    assert_eq!(bracket_out.payload["total_brackets"], 3);
    assert!(export_svg_text(&mut session, "g-violin").contains('<'));

    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "col-ba", "title": "Paired Response", "archetype": "column", "design_hint": "paired", "rows": 5, "cols": 2, "auto_graph": true, "graph_sheet_id": "g-ba" }),
            ),
            ("table.paste", json!({ "sheet_id": "col-ba", "text": "14\t22\n15\t24\n13\t21\n16\t25\n14.5\t23" })),
            ("stats.run", json!({ "source_sheet_id": "col-ba", "sheet_id": "ba-t", "kind": "paired_t" })),
            ("graph.addComparisonBrackets", json!({ "sheet_id": "g-ba", "result_sheet_id": "ba-t", "style": "curly", "label_mode": "stars" })),
        ],
    );
    assert!(matches!(gs(&session, "g-ba").spec.series[0].gdo, GraphicalDataObject::BeforeAfterLines { .. }));
    let _ = export_svg_text(&mut session, "g-ba");
}

#[test]
fn beta_g9_cbtemplate_save_instantiate_and_info_constant_hooks() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Assay Template Master" }));
    populate_xy_dose_response(&mut session, "tpl-xy", true, false, 100.0);

    let info_id = ds(&session, "tpl-xy").info_sheet_id.as_ref().unwrap().as_str().to_owned();
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
    run_cmds(
        &mut session,
        &[
            ("info.setConstant", json!({ "sheet_id": &info_id, "name": "TopConstraint", "value": 100.0 })),
            ("info.setConstant", json!({ "sheet_id": &info_id, "name": "AssayBatch", "value": "BATCH-A1" })),
            ("fit.run", json!({ "source_sheet_id": "tpl-xy", "sheet_id": "tpl-fit", "spec": serde_json::to_value(&fit_spec).unwrap() })),
            ("graph.create", json!({ "sheet_id": "tpl-graph", "data_sheet_id": "tpl-xy", "kind": "xy", "title": "Template Graph" })),
            (
                "graph.setAxes",
                json!({ "sheet_id": "tpl-graph", "x_axis": { "title": "Log10 Dose — {{info:AssayBatch}}" }, "y_axis": { "title": "Response (Cap={{info:TopConstraint}})" } }),
            ),
            ("graph.setRegressionOverlay", json!({ "sheet_id": "tpl-graph", "result_sheet_id": "tpl-fit" })),
        ],
    );

    let tpl_b64 = payload_str(&exec_ok(&mut session, "project.saveTemplate", json!({ "return_bytes": true })), "bytes_base64").to_owned();
    let mut run2 = Session::new();
    exec_ok(&mut run2, "project.instantiateTemplate", json!({ "bytes_base64": tpl_b64, "title": "Assay Run B2" }));
    assert!(ds(&run2, "tpl-xy").cell(CellCoord::new(0, 0, 0)).unwrap().is_empty() && rs(&run2, "tpl-fit").status == ResultStatus::Dirty);

    let log_doses = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let x_cells: Vec<Value> = log_doses.iter().enumerate().map(|(r, &lx)| json!({ "row": r, "value": lx })).collect();
    let y_cells: Vec<Value> = log_doses
        .iter()
        .enumerate()
        .flat_map(|(r, &lx)| {
            let y = hill_response(lx, 8.0, 140.0, -6.1, 1.1);
            [json!({ "row": r, "col": 0, "subcol": 0, "value": y }), json!({ "row": r, "col": 0, "subcol": 1, "value": y + 0.5 })]
        })
        .collect();
    run_cmds(
        &mut run2,
        &[
            ("table.setXCells", json!({ "sheet_id": "tpl-xy", "edits": x_cells })),
            ("table.setCells", json!({ "sheet_id": "tpl-xy", "edits": y_cells })),
        ],
    );

    let svg_before = export_svg_text(&mut run2, "tpl-graph");
    assert!(svg_before.contains("BATCH-A1") && svg_before.contains("Cap=100"));
    assert!((scalar_val(rs(&run2, "tpl-fit"), "Top", None) - 100.0).abs() < 1e-6);

    run_cmds(
        &mut run2,
        &[
            ("info.setConstant", json!({ "sheet_id": &info_id, "name": "TopConstraint", "value": 140.0 })),
            ("info.setConstant", json!({ "sheet_id": &info_id, "name": "AssayBatch", "value": "BATCH-B2" })),
        ],
    );
    assert!((scalar_val(rs(&run2, "tpl-fit"), "Top", None) - 140.0).abs() < 1e-6);
    let svg_after = export_svg_text(&mut run2, "tpl-graph");
    assert!(svg_after.contains("BATCH-B2") && svg_after.contains("Cap=140"));
}

#[test]
fn beta_g9_freeze_unfreeze_and_clone_pipeline_for_new_assay_run() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Freeze and ClonePipeline" })),
            ("sheet.createData", json!({ "sheet_id": "up", "title": "Upstream", "archetype": "column", "rows": 4, "cols": 1, "auto_graph": false })),
            ("table.paste", json!({ "sheet_id": "up", "text": "10\n20\n30\n40" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "dn", "title": "Downstream", "archetype": "column", "rows": 4, "cols": 1, "auto_graph": false }),
            ),
            ("table.pasteLink", json!({ "source_sheet_id": "up", "target_sheet_id": "dn" })),
            ("stats.run", json!({ "source_sheet_id": "dn", "sheet_id": "dn-desc", "kind": "descriptive" })),
            ("sheet.freeze", json!({ "sheet_id": "dn" })),
        ],
    );
    assert!((scalar_val(rs(&session, "dn-desc"), "Mean", Some(0)) - 25.0).abs() < 1e-9 && ds(&session, "dn").frozen);

    set_cell(&mut session, "up", 0, 0, 0, 410.0);
    assert_data_cell_number(&session, "dn", 0, 0, 10.0, 1e-9);
    assert!((scalar_val(rs(&session, "dn-desc"), "Mean", Some(0)) - 25.0).abs() < 1e-9);

    exec_ok(&mut session, "sheet.unfreeze", json!({ "sheet_id": "dn" }));
    assert!(!ds(&session, "dn").frozen);
    assert_data_cell_number(&session, "dn", 0, 0, 410.0, 1e-9);
    assert!((scalar_val(rs(&session, "dn-desc"), "Mean", Some(0)) - 125.0).abs() < 1e-9);

    let log_doses = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let x_cells: Vec<Value> = log_doses.iter().enumerate().map(|(r, &lx)| json!({ "row": r, "value": lx })).collect();
    let y_cells: Vec<Value> = log_doses
        .iter()
        .enumerate()
        .map(|(r, &lx)| json!({ "row": r, "col": 0, "subcol": 0, "value": hill_response(lx, 5.0, 100.0, -6.2, 1.0) }))
        .collect();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "pipe-xy", "title": "Run 1", "archetype": "xy", "rows": 8, "cols": 1, "replicates": 1, "auto_info": true, "auto_graph": false }),
            ),
            ("table.setXCells", json!({ "sheet_id": "pipe-xy", "edits": x_cells })),
            ("table.setCells", json!({ "sheet_id": "pipe-xy", "edits": y_cells })),
            ("fit.run", json!({ "source_sheet_id": "pipe-xy", "sheet_id": "pipe-fit", "kind": "four_parameter_logistic" })),
            ("graph.create", json!({ "sheet_id": "pipe-graph", "data_sheet_id": "pipe-xy", "kind": "xy", "title": "Pipeline Graph" })),
            ("graph.setRegressionOverlay", json!({ "sheet_id": "pipe-graph", "result_sheet_id": "pipe-fit" })),
            ("layout.create", json!({ "sheet_id": "pipe-layout", "title": "Pipeline Figure" })),
            ("layout.addPanel", json!({ "sheet_id": "pipe-layout", "graph_sheet_id": "pipe-graph" })),
        ],
    );

    let orig_ec50 = scalar_val(rs(&session, "pipe-fit"), "LogEC50", Some(0));
    let clone_out = exec_ok(
        &mut session,
        "sheet.clonePipeline",
        json!({ "sheet_id": "pipe-xy", "new_sheet_id": "run2-xy", "title": "Run 2", "keep_x_values": true, "clear_y_values": true }),
    );
    let cloned_data_id = payload_str(&clone_out, "sheet_id").to_owned();
    let cloned_res_id = clone_out.payload["result_sheet_ids"][0].as_str().unwrap().to_owned();
    let cloned_graph_id = clone_out.payload["graph_sheet_ids"][0].as_str().unwrap().to_owned();
    let cloned_layout_id = clone_out.payload["layout_sheet_ids"][0].as_str().unwrap().to_owned();

    assert_eq!(x_num(&session, &cloned_data_id, 0), Some(-9.0));
    assert!(ds(&session, &cloned_data_id).cell(CellCoord::new(0, 0, 0)).unwrap().is_empty());
    assert_eq!(rs(&session, &cloned_res_id).status, ResultStatus::Dirty);
    assert_eq!(gs(&session, &cloned_graph_id).spec.primary_source_sheet_id.as_str(), cloned_data_id);
    assert!(!ls(&session, &cloned_layout_id).spec.items.is_empty());

    let run2_y: Vec<Value> = log_doses
        .iter()
        .enumerate()
        .map(|(r, &lx)| json!({ "row": r, "col": 0, "subcol": 0, "value": hill_response(lx, 5.0, 100.0, -5.1, 1.0) }))
        .collect();
    run_cmds(&mut session, &[("table.setCells", json!({ "sheet_id": &cloned_data_id, "edits": run2_y })), ("project.recalculate", json!({}))]);
    assert!((scalar_val(rs(&session, &cloned_res_id), "LogEC50", Some(0)) + 5.1).abs() < 0.12);
    assert!((scalar_val(rs(&session, "pipe-fit"), "LogEC50", Some(0)) - orig_ec50).abs() < 1e-12);
}

#[test]
fn beta_g7_g8_multivariate_calculated_vars_cox_logistic_and_bubble_scatter() {
    let mut session = Session::new();
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
    let cells: Vec<Value> = rows_data
        .iter()
        .enumerate()
        .flat_map(|(r, &(t, ev, conc, age, bio, pid))| {
            [
                json!({ "row": r, "col": 0, "value": t }),
                json!({ "row": r, "col": 1, "value": ev }),
                json!({ "row": r, "col": 2, "value": conc }),
                json!({ "row": r, "col": 3, "value": age }),
                json!({ "row": r, "col": 4, "value": bio }),
                json!({ "row": r, "col": 5, "value": pid }),
            ]
        })
        .collect();
    let glm_spec = AnalysisSpec::new(
        "logistic-res",
        "mv-cohort",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 1,
            predictor_columns: vec![6, 7],
            reference_levels: Vec::new(),
            compute_vif: true,
            ..Default::default()
        }),
    )
    .unwrap();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Multivariate Cohort Study" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "mv-cohort", "title": "Patient Biomarkers", "archetype": "multiple_variables", "rows": 16, "cols": 6, "variables": [{ "title": "Time", "role": "continuous" }, { "title": "Event", "role": "categorical" }, { "title": "ConcRaw", "role": "continuous" }, { "title": "Age", "role": "continuous" }, { "title": "Biomarker", "role": "continuous" }, { "title": "PatientId", "role": "label" }], "auto_graph": false }),
            ),
            ("table.setCells", json!({ "sheet_id": "mv-cohort", "edits": cells })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-cohort", "title": "LogConc", "formula": "log10(ConcRaw)" })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-cohort", "title": "CenteredAge", "formula": "(Age - 50) / 10" })),
            ("table.setVariableRole", json!({ "sheet_id": "mv-cohort", "col": "Event", "role": "continuous" })),
            (
                "stats.run",
                json!({ "source_sheet_id": "mv-cohort", "sheet_id": "cox-res", "kind": "cox_ph", "time_column": 0, "status_column": 1, "predictor_columns": [6, 7] }),
            ),
            ("stats.run", json!({ "source_sheet_id": "mv-cohort", "sheet_id": "logistic-res", "spec": serde_json::to_value(&glm_spec).unwrap() })),
            (
                "graph.create",
                json!({ "sheet_id": "g-bubble", "data_sheet_id": "mv-cohort", "kind": "multiple_variables", "title": "5-Channel Bubble Scatter" }),
            ),
            (
                "graph.setSeries",
                json!({ "sheet_id": "g-bubble", "multivariate": { "x_column": 6, "y_column": 4, "color": { "column": 3, "categorical": false }, "size": { "column": 2, "min_radius_pt": 3.0, "max_radius_pt": 13.0 }, "label_column": 5, "show_convex_hulls": false } }),
            ),
        ],
    );
    assert_data_cell_number(&session, "mv-cohort", 13, 6, 2.0, 1e-9);
    assert_data_cell_number(&session, "mv-cohort", 13, 7, 1.8, 1e-9);
    let TableArchetype::MultipleVariables(mv) = &ds(&session, "mv-cohort").archetype else { panic!("expected MV") };
    assert_eq!(mv.variables[1].role, VariableRole::Continuous);

    let cox = rs(&session, "cox-res");
    assert_eq!(cox.status, ResultStatus::UpToDate);
    assert!(!find_tab(cox, ResultTabKind::ParametersAndConfidenceIntervals).rows.is_empty());
    assert!(cox.checklist.iter().any(|c| c.assumption.contains("Proportional") || c.assumption.contains("Cox")));
    assert_eq!(rs(&session, "logistic-res").status, ResultStatus::UpToDate);
    assert!(!find_tab(rs(&session, "logistic-res"), ResultTabKind::ParametersAndConfidenceIntervals).rows.is_empty());

    let bubble_svg = export_svg_text(&mut session, "g-bubble");
    assert!(bubble_svg.contains("P01") && bubble_svg.contains("P16"));
}

#[test]
fn beta_g11_multipanel_layout_aligned_origins_badges_and_publication_exports() {
    let mut session = Session::new();
    exec_ok(&mut session, "project.new", json!({ "title": "Publication Layout Suite" }));
    populate_xy_dose_response(&mut session, "lay-xy", true, false, 100.0);
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-a", "data_sheet_id": "lay-xy", "kind": "xy", "title": "Potency" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "lay-col", "title": "Groups", "archetype": "column", "rows": 4, "cols": 2, "auto_graph": false }),
            ),
            ("table.paste", json!({ "sheet_id": "lay-col", "text": "10\t25\n12\t28\n11\t26\n9\t24" })),
            ("graph.create", json!({ "sheet_id": "g-b", "data_sheet_id": "lay-col", "kind": "column", "title": "Cohort Mean" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "lay-ba", "title": "Matched", "archetype": "column", "design_hint": "paired", "rows": 4, "cols": 2, "auto_graph": false }),
            ),
            ("table.paste", json!({ "sheet_id": "lay-ba", "text": "5\t11\n6\t13\n7\t14\n5.5\t12" })),
            ("graph.create", json!({ "sheet_id": "g-c", "data_sheet_id": "lay-ba", "kind": "column", "title": "Paired Shift" })),
            (
                "layout.create",
                json!({ "sheet_id": "fig-1", "title": "Figure 1", "width": 180.0, "height": 140.0, "unit": "mm", "grid_template": [2, 2], "align_plot_origins": true, "panel_badges": "uppercase" }),
            ),
            ("layout.addPanel", json!({ "sheet_id": "fig-1", "graph_sheet_id": "g-a", "row": 0, "col": 0 })),
            ("layout.addPanel", json!({ "sheet_id": "fig-1", "graph_sheet_id": "g-b", "row": 0, "col": 1 })),
            ("layout.addPanel", json!({ "sheet_id": "fig-1", "graph_sheet_id": "g-c", "row": 1, "col": 0 })),
        ],
    );
    assert_eq!(
        exec_ok(&mut session, "layout.alignOrigins", json!({ "sheet_id": "fig-1", "align": true, "normalize_bounds": true })).payload["aligned_panels"],
        3
    );

    let layout_svg = export_svg_text(&mut session, "fig-1");
    assert!(layout_svg.contains(">A<") && layout_svg.contains(">B<") && layout_svg.contains(">C<"));
    let pdf_text = String::from_utf8_lossy(&export_pdf_bytes(&mut session, "fig-1", "cmyk", false)).into_owned();
    assert!(pdf_text.contains(" k\n") || pdf_text.contains(" K\n"));
    let (_png600, w_px, h_px) = export_png_bytes(&mut session, "fig-1", 600);
    assert!(w_px >= 4000 && h_px >= 3000);
    let tiff_out = exec_ok(&mut session, "export.tiff", json!({ "sheet_id": "fig-1", "dpi": 300.0, "compression": "lzw" }));
    assert_eq!(tiff_out.payload["compression"], "lzw");
    let tiff_bytes = decode_base64(payload_str(&tiff_out, "base64"));
    assert!((tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*")) && tiff_bytes.len() > 200);
}

#[test]
fn beta_g10_pzfx_and_prism_cleanroom_imports_reanalysis_and_exports() {
    // Sub-journey A: Clean-room .pzfx XML
    let mut pzfx = Session::new();
    assert!(exec_ok(&mut pzfx, "table.importPzfx", json!({ "xml": build_cleanroom_pzfx_xml() })).payload["sheet_count"].as_u64().unwrap() >= 2);
    let data_id = pzfx.document().data_sheets[0].id.as_str().to_owned();
    let info_id = pzfx.document().info_sheets[0].id.as_str().to_owned();
    assert_eq!(info(&pzfx, &info_id).get_constant("Protocol").unwrap().value.display_text(), "ELISA-v3");
    assert!(ds(&pzfx, &data_id).cell(CellCoord::new(4, 0, 0)).unwrap().excluded);

    exec_ok(
        &mut pzfx,
        "stats.run",
        json!({ "source_sheet_id": &data_id, "sheet_id": "pzfx-anova", "kind": "one_way_anova", "multiplicity": "tukey" }),
    );
    let f_before = scalar_val(rs(&pzfx, "pzfx-anova"), "F", None);
    assert!(f_before > 100.0);
    set_cell(&mut pzfx, &data_id, 0, 2, 0, 65.0);
    assert!((scalar_val(rs(&pzfx, "pzfx-anova"), "F", None) - f_before).abs() > 1.0);
    exec_ok(&mut pzfx, "graph.create", json!({ "sheet_id": "g-pzfx", "data_sheet_id": &data_id, "kind": "column", "title": "Imported XML Cohort" }));
    let _ = (export_svg_text(&mut pzfx, "g-pzfx"), export_pdf_bytes(&mut pzfx, "g-pzfx", "srgb", true));

    // Sub-journey B: Clean-room .prism ZIP container
    let mut zip_s = Session::new();
    assert!(
        exec_ok(&mut zip_s, "table.importPrism", json!({ "bytes_base64": encode_base64(&build_cleanroom_prism_zip_bytes()) })).payload["sheet_count"]
            .as_u64()
            .unwrap()
            >= 2
    );
    let zip_data_id = zip_s.document().data_sheets[0].id.as_str().to_owned();
    assert_eq!(zip_s.document().info_sheets[0].get_constant("AssayCode").unwrap().value.display_text(), "ZIP-901");

    exec_ok(&mut zip_s, "fit.run", json!({ "source_sheet_id": &zip_data_id, "sheet_id": "zip-fit", "kind": "four_parameter_logistic" }));
    let ec50_before = scalar_val(rs(&zip_s, "zip-fit"), "LogEC50", Some(0));
    assert!((ec50_before + 6.05).abs() < 0.3);
    set_cell(&mut zip_s, &zip_data_id, 3, 0, 0, 12.0);
    assert!((scalar_val(rs(&zip_s, "zip-fit"), "LogEC50", Some(0)) - ec50_before).abs() > 0.05);

    run_cmds(
        &mut zip_s,
        &[
            ("graph.create", json!({ "sheet_id": "g-zip", "data_sheet_id": &zip_data_id, "kind": "xy", "title": "Imported Archive Fit" })),
            ("graph.setRegressionOverlay", json!({ "sheet_id": "g-zip", "result_sheet_id": "zip-fit" })),
        ],
    );
    let _ = (export_svg_text(&mut zip_s, "g-zip"), export_pdf_bytes(&mut zip_s, "g-zip", "srgb", true));
}
