//! Engine end-to-end integration tests for R18.2 (§2.5 Identifiability, Model-Comparison,
//! Output & Quality-Flag Extensions, linked parameter summary graphs, and antilog transform chaining).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{Cell, CellCoord, DataSheet, GraphKind, ResultCell, ResultStatus, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

use super::common::{gs, ok_cmd, rs, session_with, xy_sheet};

#[test]
fn r18_2_engine_summary_graph_flags_and_methods_report() {
    let t_kin = [0.0, 1.0, 2.0, 4.0, 8.0, 12.0, 20.0];
    let e_kin = [0.1, -0.15, 0.08, -0.12, 0.09, -0.05, 0.05];
    let mk_y = |conc: f64| -> Vec<f64> {
        let k = 0.10 + 0.08 * conc;
        t_kin.iter().zip(e_kin).map(|(&t, e)| 5.0 + 95.0 * (1.0 - (-k * t).exp()) + e).collect()
    };
    let (y1, y2, y4) = (mk_y(1.0), mk_y(2.0), mk_y(4.0));
    let mut session = session_with("R18.2 Engine Test", vec![xy_sheet("xy-kin", "Kinetics", "", &t_kin, &[("1", &y1), ("2", &y2), ("4", &y4)])]);

    let out = ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-kin", "source_sheet_id": "xy-kin", "kind": "one_phase_association", "profile_likelihood_ci": false,
            "identifiability_mode": "IdentifyUnstableParameters", "summary_parameter": "K", "create_summary_graph": true,
            "output_significant_digits": 4, "fit_flag_rules": { "min_r_squared": 0.90, "min_observations": 5 }
        }),
    );

    let summary_graph_id =
        out.payload.get("summary_graph_sheet_id").and_then(|v| v.as_str()).expect("create_summary_graph must emit summary_graph_sheet_id");
    let graph = gs(&session, summary_graph_id);
    assert_eq!((graph.spec.kind, graph.spec.primary_source_sheet_id.as_str()), (GraphKind::Xy, "fit-kin"));

    let fit_rs = rs(&session, "fit-kin");
    assert_eq!(fit_rs.status, ResultStatus::UpToDate);
    assert!(fit_rs.tabs.iter().any(|t| t.id == "nonlinear_parameter_summary"));
    let gof = fit_rs.tabs.iter().find(|t| t.id == "nonlinear_gof").unwrap();
    let flag_idx = gof.row_labels.iter().position(|l| l == "Fit quality flag").unwrap();
    assert_eq!(gof.rows[flag_idx][0], ResultCell::Text("Pass".to_owned()));

    let md = generate_methods_report(session.document(), &"fit-kin".into(), ReportFormat::Markdown).unwrap();
    for needle in ["greedy Hessian", "Parameter summary table emitted for `K`", "4 significant digits"] {
        assert!(md.contains(needle), "methods report missing `{needle}`:\n{md}");
    }

    // AUD-13-15: analysis.updateParams creates summary graph once when enabled and does not duplicate on later edits.
    let mut s_up = session_with(
        "Summary Update",
        vec![xy_sheet(
            "xy-1",
            "Assay",
            "X",
            &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0],
            &[("C1", &[2.0, 5.0, 20.0, 52.0, 82.0, 95.0, 99.0]), ("C2", &[1.0, 3.0, 10.0, 30.0, 68.0, 90.0, 98.0])],
        )],
    );
    let f0 = ok_cmd(
        &mut s_up,
        "fit.run",
        json!({ "sheet_id": "fit-1", "source_sheet_id": "xy-1", "equation_id": "log_agonist_vs_response_4pl", "create_summary_graph": false }),
    );
    assert!(f0.payload.get("summary_graph_sheet_id").is_none() && s_up.document().graph_sheets.is_empty());
    let u1 = ok_cmd(&mut s_up, "analysis.updateParams", json!({ "sheet_id": "fit-1", "summary_parameter": "LogEC50", "create_summary_graph": true }));
    let sg_id = u1.payload["summary_graph_sheet_id"].as_str().expect("summary graph id");
    assert_eq!((s_up.document().graph_sheets.len(), gs(&s_up, sg_id).spec.primary_source_sheet_id.as_str()), (1, "fit-1"));
    let u2 = ok_cmd(&mut s_up, "analysis.updateParams", json!({ "sheet_id": "fit-1", "confidence_percent": 90.0 }));
    assert!(u2.payload.get("summary_graph_sheet_id").is_none() && s_up.document().graph_sheets.len() == 1);
}

#[test]
fn r18_2_engine_dual_interpolation_chains_into_antilog_transform() {
    let mut sheet =
        DataSheet::new("xy-std", "Standard Curve", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 6).unwrap())).unwrap();
    for r in 0..5 {
        let x = r as f64;
        sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(3.0 + 2.0 * x - 0.25)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 1), Cell::number(3.0 + 2.0 * x + 0.25)).unwrap();
    }
    sheet.set_cell(CellCoord::new(5, 0, 0), Cell::number(6.0)).unwrap();
    sheet.set_cell(CellCoord::new(5, 0, 1), Cell::number(8.0)).unwrap();

    let mut session = session_with("R18.2 Antilog Chain", vec![sheet]);
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-std", "source_sheet_id": "xy-std", "kind": "straight_line", "profile_likelihood_ci": false, "interpolate_unknowns": true }),
    );
    ok_cmd(&mut session, "transform.run", json!({ "sheet_id": "antilog-std", "source_sheet_id": "fit-std", "kind": "math", "x_transform": "Exp10" }));

    let transformed_rs = rs(&session, "antilog-std");
    assert_eq!(transformed_rs.status, ResultStatus::UpToDate);
    let TableArchetype::Xy(out_xy) = &transformed_rs.analyzable_data.as_ref().expect("transform output analyzable_data").archetype else {
        panic!("expected XyTable output from antilog transform");
    };
    // Row 0 = Mean X (10^2 = 100.0, exact Fieller band-inversion CI = [79.033793, 126.528155])
    let (est, lo, hi) =
        (out_xy.x_cells[0][0].active_number().unwrap(), out_xy.x_cells[0][1].active_number().unwrap(), out_xy.x_cells[0][2].active_number().unwrap());
    assert!((est - 100.0).abs() < 1e-6, "10^X estimate mismatch: {est}");
    assert!((lo - 79.033_793_211_633).abs() < 5e-2, "10^CI_lo mismatch: {lo}");
    assert!((hi - 126.528_154_522_743).abs() < 5e-2, "10^CI_hi mismatch: {hi}");
}
