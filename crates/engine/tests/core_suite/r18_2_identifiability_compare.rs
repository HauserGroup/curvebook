//! Engine end-to-end integration tests for R18.2 (§2.5 Identifiability, Model-Comparison,
//! Output & Quality-Flag Extensions, linked parameter summary graphs, and antilog transform chaining).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{Cell, CellCoord, DataSheet, GraphKind, ProjectDocument, ResultCell, ResultStatus, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

#[test]
fn r18_2_engine_summary_graph_flags_and_methods_report() {
    let mut doc = ProjectDocument::new("R18.2 Engine Test").unwrap();
    let t_kin = [0.0, 1.0, 2.0, 4.0, 8.0, 12.0, 20.0];
    let e_kin = [0.1, -0.15, 0.08, -0.12, 0.09, -0.05, 0.05];
    let concs = [1.0, 2.0, 4.0];

    let mut xy = XyTable::new(SubcolumnMode::Single, 3, t_kin.len()).unwrap();
    for (c, &conc) in concs.iter().enumerate() {
        xy.data_sets[c].title = format!("{conc}");
        let k_true = 0.10 + 0.08 * conc;
        for (r, (&t, &e)) in t_kin.iter().zip(e_kin.iter()).enumerate() {
            xy.set_x_cell(r, Cell::number(t)).unwrap();
            xy.data_sets[c].cells[r][0] = Cell::number(5.0 + 95.0 * (1.0 - (-k_true * t).exp()) + e);
        }
    }
    doc.add_data_sheet(DataSheet::new("xy-kin", "Kinetics", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    let out = session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-kin",
                "source_sheet_id": "xy-kin",
                "kind": "one_phase_association",
                "profile_likelihood_ci": false,
                "identifiability_mode": "IdentifyUnstableParameters",
                "summary_parameter": "K",
                "create_summary_graph": true,
                "output_significant_digits": 4,
                "fit_flag_rules": {
                    "min_r_squared": 0.90,
                    "min_observations": 5
                }
            }),
        )
        .unwrap();

    let summary_graph_id =
        out.payload.get("summary_graph_sheet_id").and_then(|v| v.as_str()).expect("create_summary_graph must emit summary_graph_sheet_id");
    let graph = session.document().graph_sheet(&summary_graph_id.into()).expect("linked secondary graph must exist");
    assert_eq!(graph.spec.kind, GraphKind::Xy);
    assert_eq!(graph.spec.primary_source_sheet_id.as_str(), "fit-kin");

    let rs = session.document().result_sheet(&"fit-kin".into()).unwrap();
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert!(rs.tabs.iter().any(|t| t.id == "nonlinear_parameter_summary"));
    let gof = rs.tabs.iter().find(|t| t.id == "nonlinear_gof").unwrap();
    let flag_idx = gof.row_labels.iter().position(|l| l == "Fit quality flag").unwrap();
    assert_eq!(gof.rows[flag_idx][0], ResultCell::Text("Pass".to_owned()));

    let md = generate_methods_report(session.document(), &"fit-kin".into(), ReportFormat::Markdown).unwrap();
    assert!(md.contains("greedy Hessian"), "methods report should describe identifiability mode: {md}");
    assert!(md.contains("Parameter summary table emitted for `K`"), "methods report should mention summary_parameter: {md}");
    assert!(md.contains("4 significant digits"), "methods report should mention output_significant_digits: {md}");
}

#[test]
fn r18_2_engine_dual_interpolation_chains_into_antilog_transform() {
    let mut doc = ProjectDocument::new("R18.2 Antilog Chain").unwrap();
    let xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 6).unwrap();
    let mut sheet = DataSheet::new("xy-std", "Standard Curve", TableArchetype::Xy(xy)).unwrap();
    for r in 0..5 {
        let x = r as f64;
        sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(3.0 + 2.0 * x - 0.25)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 1), Cell::number(3.0 + 2.0 * x + 0.25)).unwrap();
    }
    sheet.set_cell(CellCoord::new(5, 0, 0), Cell::number(6.0)).unwrap();
    sheet.set_cell(CellCoord::new(5, 0, 1), Cell::number(8.0)).unwrap();
    doc.add_data_sheet(sheet).unwrap();

    let mut session = Session::from_document(doc);
    session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-std",
                "source_sheet_id": "xy-std",
                "kind": "straight_line",
                "profile_likelihood_ci": false,
                "interpolate_unknowns": true
            }),
        )
        .unwrap();

    // Chain Transform (10^X antilog) directly onto the ResultSheet's analyzable_data!
    session
        .execute(
            "transform.run",
            json!({
                "sheet_id": "antilog-std",
                "source_sheet_id": "fit-std",
                "kind": "math",
                "x_transform": "Exp10"
            }),
        )
        .unwrap();

    let transformed_rs = session.document().result_sheet(&"antilog-std".into()).unwrap();
    assert_eq!(transformed_rs.status, ResultStatus::UpToDate);
    let analyzable = transformed_rs.analyzable_data.as_ref().expect("transform output analyzable_data");
    let TableArchetype::Xy(out_xy) = &analyzable.archetype else {
        panic!("expected XyTable output from antilog transform");
    };
    // Row 0 = Mean X (10^2 = 100.0, exact Fieller band-inversion CI = [79.033793, 126.528155])
    let est = out_xy.x_cells[0][0].active_number().unwrap();
    let lo = out_xy.x_cells[0][1].active_number().unwrap();
    let hi = out_xy.x_cells[0][2].active_number().unwrap();
    assert!((est - 100.0).abs() < 1e-6, "10^X estimate mismatch: {est}");
    assert!((lo - 79.033_793_211_633).abs() < 5e-2, "10^CI_lo mismatch: {lo}");
    assert!((hi - 126.528_154_522_743).abs() < 5e-2, "10^CI_hi mismatch: {hi}");
}
