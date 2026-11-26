//! End-to-end integration tests for `cmd_sci`, `cmd::registry()`, and `compile_sheet_scene`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BreakOrnament, Cell, DataSheet, DemingErrorSpec, FramePreset, GraphKind, NonlinearFitSpec,
    PointStyleOverride, ProjectDocument, ResultStatus, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::{Session, cmd, cmd::helpers::compile_sheet_scene};
use serde_json::{Value, json};

use super::common::{
    TempDir, assert_bad_input, assert_bad_inputs, col_sheet, err_cmd, gs, mv_sheet, ok_cmd, rs, run_cmds, scalar, session_with, survival_sheet,
    xy_4pl_sheet, xy_sheet,
};

fn anova_col(id: &str, offset: f64) -> DataSheet {
    let v: Vec<f64> = [10.2, 11.0, 9.8, 10.5, 10.1].iter().map(|x| x + offset).collect();
    let lo: Vec<f64> = [14.5, 15.2, 13.9, 14.8, 15.1].iter().map(|x| x + offset).collect();
    let hi: Vec<f64> = [22.4, 23.1, 21.8, 22.9, 23.0].iter().map(|x| x + offset).collect();
    col_sheet(id, "Treatment Response", &[("Vehicle", &v), ("Low Dose", &lo), ("High Dose", &hi)])
}

fn sample_survival_sheet(id: &str) -> DataSheet {
    survival_sheet(
        id,
        "Survival Cohort",
        &["Control", "Treated"],
        &[
            (4.0, &[Some(1.0), None]),
            (7.0, &[Some(1.0), None]),
            (10.0, &[Some(0.0), None]),
            (13.0, &[Some(1.0), None]),
            (18.0, &[Some(1.0), None]),
            (6.0, &[None, Some(1.0)]),
            (11.0, &[None, Some(0.0)]),
            (16.0, &[None, Some(1.0)]),
            (22.0, &[None, Some(0.0)]),
            (28.0, &[None, Some(1.0)]),
        ],
    )
}

fn sample_multivariate_sheet(id: &str) -> DataSheet {
    mv_sheet(
        id,
        "Multivariate Assay",
        &["S1", "S2", "S3", "S4", "S5", "S6"],
        &[
            ("Marker X", None, &[Cell::number(1.2), Cell::number(2.1), Cell::number(3.4), Cell::number(4.5), Cell::number(5.8), Cell::number(6.9)]),
            (
                "Marker Y",
                None,
                &[Cell::number(10.5), Cell::number(14.2), Cell::number(18.8), Cell::number(22.1), Cell::number(27.9), Cell::number(31.3)],
            ),
            (
                "Marker Z",
                None,
                &[Cell::number(24.0), Cell::number(31.0), Cell::number(39.0), Cell::number(46.0), Cell::number(54.0), Cell::number(62.0)],
            ),
            ("Weight", None, &[Cell::number(2.0), Cell::number(4.0), Cell::number(7.0), Cell::number(10.0), Cell::number(13.0), Cell::number(16.0)]),
            ("Status", None, &[Cell::number(0.0), Cell::number(0.0), Cell::number(1.0), Cell::number(1.0), Cell::number(1.0), Cell::number(0.0)]),
            (
                "Group",
                Some(VariableRole::Categorical),
                &[Cell::text("Ctrl"), Cell::text("Ctrl"), Cell::text("Treat"), Cell::text("Treat"), Cell::text("Treat"), Cell::text("Ctrl")],
            ),
        ],
    )
}

#[test]
fn test_4pl_fit_graph_overlay_styling_and_all_exports() {
    let dir = TempDir::new("cmd-sci-4pl");
    let mut session = session_with("4PL Pharmacology", vec![xy_4pl_sheet("data-4pl")]);

    // 1. Run 4PL nonlinear regression via shorthand dict and full typed AnalysisSpec JSON.
    let fit_out = ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-4pl", "source_sheet_id": "data-4pl", "kind": "four_parameter_logistic", "confidence_bands": true, "title": "Compound A 4PL" }),
    );
    assert!(fit_out.mutated);
    assert_eq!(rs(&session, "fit-4pl").status, ResultStatus::UpToDate);
    assert!(!rs(&session, "fit-4pl").fitted_curves.is_empty());

    let typed_spec =
        AnalysisSpec::new("fit-typed", "data-4pl", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": typed_spec }));
    assert!(session.document().result_sheet(&SheetId::new("fit-typed")).is_some());

    // 2. Export analysis results to JSON and tidy/tab CSV.
    let (json_path, csv_path) = (dir.child("fit_4pl.json"), dir.child("fit_4pl.csv"));
    let exp_json = ok_cmd(&mut session, "analysis.exportJson", json!({ "sheet_id": "fit-4pl", "path": json_path.to_string_lossy() }));
    assert!(!exp_json.mutated);
    let parsed_json: Value = serde_json::from_str(exp_json.payload["json"].as_str().unwrap()).unwrap();
    assert_eq!(parsed_json["id"], "fit-4pl");
    assert!(std::fs::read_to_string(&json_path).unwrap().contains("log_agonist_vs_response_4pl"));

    let exp_csv = ok_cmd(&mut session, "analysis.exportCsv", json!({ "sheet_id": "fit-4pl", "path": csv_path.to_string_lossy() }));
    assert!(exp_csv.payload["csv"].as_str().unwrap().contains("fit-4pl"));
    assert!(std::fs::metadata(&csv_path).unwrap().len() > 20);

    // 3. Create uninitialized graph -> initialize -> configure axes, series, overlay, annotations.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-4pl", "data_sheet_id": "data-4pl", "initialized": false }));
    assert!(!gs(&session, "g-4pl").initialized);

    run_cmds(
        &mut session,
        &[
            ("graph.initialize", json!({ "sheet_id": "g-4pl", "kind": "xy", "title": "Dose-Response Curve" })),
            (
                "graph.setAxes",
                json!({
                    "sheet_id": "g-4pl",
                    "frame_preset": "open_l",
                    "x_axis": { "title": "Log10 [Drug] (M)", "min": -10.0, "max": -3.5, "scale": "linear", "tick_format": "decimal" },
                    "y_axis": {
                        "title": "% Normalized Response",
                        "break_ornament": "double_slash",
                        "gap_width_pt": 8.0,
                        "tick_format": "decimal",
                        "segments": [{ "min": 0.0, "max": 40.0, "scale": "linear", "length_share": 0.4 }, { "min": 50.0, "max": 110.0, "scale": "linear", "length_share": 0.6 }]
                    }
                }),
            ),
        ],
    );
    assert!(gs(&session, "g-4pl").initialized);
    assert_eq!(gs(&session, "g-4pl").spec.y_axis.segments.len(), 2);
    let _ = err_cmd(&mut session, "graph.setAxes", json!({ "sheet_id": "g-4pl", "axis": "y", "segments": [{}, {}, {}, {}] }));

    run_cmds(
        &mut session,
        &[
            (
                "graph.setSeries",
                json!({ "sheet_id": "g-4pl", "series_index": 0, "legend_label": "Compound A (n=2)", "color": "#1F77B4", "marker": { "shape": "diamond", "fill_mode": "open", "size_pt": 6.0 }, "y_error_bars": { "metric": "sem", "direction": "both", "cap_width_pt": 5.0 }, "connecting_line": { "kind": "none" } }),
            ),
            (
                "graph.setRegressionOverlay",
                json!({ "sheet_id": "g-4pl", "result_sheet_id": "fit-4pl", "series_index": 0, "sample_segments": 300, "show_confidence_band": true, "shaded_ribbon": true }),
            ),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g-4pl", "kind": "text", "text": "LogEC50 = {{scalar:fit-4pl:LogEC50:none}}", "x_pt": 180.0, "y_pt": 60.0 }),
            ),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g-4pl", "kind": "table", "result_sheet_id": "fit-4pl", "bounds_pt": [220.0, 150.0, 140.0, 60.0] }),
            ),
        ],
    );

    // Apply Style From preserving target data bindings.
    let mut target_doc = session.document().clone();
    target_doc.add_data_sheet(xy_4pl_sheet("data-4pl-2")).unwrap();
    session = Session::from_document(target_doc);
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-4pl-2", "data_sheet_id": "data-4pl-2", "kind": "xy" })),
            ("graph.applyStyleFrom", json!({ "sheet_id": "g-4pl-2", "donor_sheet_id": "g-4pl" })),
        ],
    );
    let g2 = gs(&session, "g-4pl-2");
    assert_eq!(g2.spec.primary_source_sheet_id.as_str(), "data-4pl-2");
    assert_eq!(g2.spec.series[0].source_sheet_id.as_str(), "data-4pl-2");
    assert_eq!(g2.spec.frame_preset, FramePreset::OpenL);
    assert_eq!(g2.spec.y_axis.break_ornament, BreakOrnament::DoubleSlash);
    assert_eq!(g2.spec.y_axis.segments.len(), 1);

    // 4. Compile scene and export SVG, PDF, PNG, TIFF (both payload & file output).
    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-4pl")).unwrap();
    assert!(scene.width_pt > 0.0 && scene.height_pt > 0.0);

    let (svg_path, pdf_path, png_path, tiff_path) = (dir.child("g.svg"), dir.child("g.pdf"), dir.child("g.png"), dir.child("g.tiff"));
    let svg_out = ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "g-4pl", "path": svg_path.to_string_lossy() }));
    let svg_str = svg_out.payload["svg"].as_str().unwrap();
    assert!(svg_str.contains("<svg") && curvebook_svg::validate_svg_structure(svg_str).is_ok());
    assert!(std::fs::read_to_string(&svg_path).unwrap().contains("<svg"));

    let pdf_out = ok_cmd(&mut session, "export.pdf", json!({ "sheet_id": "g-4pl", "color_space": "cmyk", "path": pdf_path.to_string_lossy() }));
    assert!(pdf_out.payload["byte_len"].as_u64().unwrap() > 100 && pdf_out.payload["base64"].as_str().unwrap().len() > 100);
    assert!(std::fs::read(&pdf_path).unwrap().starts_with(b"%PDF-"));
    // AUD-02-21: export.pdf grayscale & rejecting both sheet_id and sheet_ids.
    let gray = ok_cmd(&mut session, "export.pdf", json!({ "sheet_id": "g-4pl", "color_space": "grayscale" }));
    assert!(gray.payload["byte_len"].as_u64().unwrap() > 100 && gray.payload["color_space"] == "Grayscale");
    assert_bad_input(&mut session, "export.pdf", json!({ "sheet_id": "g-4pl", "sheet_ids": ["g-4pl"] }));

    let png_out = ok_cmd(&mut session, "export.png", json!({ "sheet_id": "g-4pl", "dpi": 150, "path": png_path.to_string_lossy() }));
    assert!(png_out.payload["byte_len"].as_u64().unwrap() > 100);
    assert!(std::fs::read(&png_path).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));

    let tiff_out =
        ok_cmd(&mut session, "export.tiff", json!({ "sheet_id": "g-4pl", "dpi": 150, "compression": "lzw", "path": tiff_path.to_string_lossy() }));
    assert!(tiff_out.payload["byte_len"].as_u64().unwrap() > 100);
    let tiff_bytes = std::fs::read(&tiff_path).unwrap();
    assert!(tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*"));
}

#[test]
fn test_stats_anova_auto_brackets_update_params_and_retarget() {
    let mut session = session_with("ANOVA Study", vec![anova_col("col-1", 0.0), anova_col("col-2", 5.0), sample_survival_sheet("surv-incompatible")]);

    let anova_out = ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "stat-anova", "source_sheet_id": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }),
    );
    assert!(anova_out.payload["pairwise_comparisons_count"].as_u64().unwrap() >= 3);

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column" }));
    let br_out = ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-col", "result_sheet_id": "stat-anova", "significant_only": true, "style": "square", "label_mode": "stars" }),
    );
    assert!(br_out.payload["added"].as_u64().unwrap() >= 1);

    ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-col", "result_sheet_id": "stat-anova", "group_a": "Vehicle", "group_b": "High Dose", "style": "curly", "label_mode": "numeric" }),
    );
    // AUD-02-14: bracket label_mode rejects blank and preserves case in Custom label.
    assert_bad_input(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-col", "result_sheet_id": "stat-anova", "label_mode": "   " }),
    );
    ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-col", "result_sheet_id": "stat-anova", "group_a": "Vehicle", "group_b": "Low Dose", "label_mode": "P < 0.001 (Bonferroni)" }),
    );
    let custom_br = gs(&session, "g-col").spec.comparisons.iter().find(|b| b.group_a == 0 && b.group_b == 1).expect("Vehicle vs Low Dose bracket");
    assert_eq!(custom_br.label_mode, BracketLabelMode::Custom("P < 0.001 (Bonferroni)".into()));
    assert!(compile_sheet_scene(session.document(), &SheetId::new("g-col")).unwrap().width_pt > 0.0);

    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "stat-anova", "kind": "kruskal_wallis", "multiplicity": "dunn", "confidence_percent": 99.0 }),
    );
    assert!(matches!(rs(&session, "stat-anova").analysis.kind, AnalysisKind::OneWayAnova(_)));

    ok_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "stat-anova", "source_sheet_id": "col-2" }));
    assert_eq!(rs(&session, "stat-anova").analysis.source_sheet_id.as_str(), "col-2");
    let _ = err_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "stat-anova", "source_sheet_id": "surv-incompatible" }));
    assert_eq!(rs(&session, "stat-anova").analysis.source_sheet_id.as_str(), "col-2");
}

#[test]
fn test_survival_km_and_cox_with_number_at_risk_graph() {
    let mut session = session_with("Survival & Cox Study", vec![sample_survival_sheet("surv-1"), sample_multivariate_sheet("mv-cox")]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "stat-km", "source_sheet_id": "surv-1", "kind": "kaplan_meier" })),
            (
                "stats.run",
                json!({ "sheet_id": "stat-cox", "source_sheet_id": "mv-cox", "kind": "cox_ph", "time_column": 3, "status_column": 4, "predictor_columns": [0, 1] }),
            ),
            ("graph.create", json!({ "sheet_id": "g-surv", "data_sheet_id": "surv-1", "kind": "survival" })),
            ("graph.setSpec", json!({ "sheet_id": "g-surv", "survival_number_at_risk": true })),
        ],
    );

    assert!(!rs(&session, "stat-km").fitted_curves.is_empty());
    assert!(rs(&session, "stat-km").tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk));
    assert_eq!(rs(&session, "stat-cox").status, ResultStatus::UpToDate);
    assert!(gs(&session, "g-surv").spec.survival_number_at_risk);

    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-surv")).unwrap();
    assert!(scene.width_pt > 0.0 && !scene.primitives.is_empty());
    assert!(ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "g-surv" })).payload["svg"].as_str().unwrap().contains("<svg"));
}

#[test]
fn test_multivariate_pca_bubble_and_transform() {
    let mut session = session_with("Multivariate Bubble", vec![sample_multivariate_sheet("mv-1"), xy_4pl_sheet("xy-1")]);
    run_cmds(
        &mut session,
        &[
            ("transform.run", json!({ "sheet_id": "tr-log", "source_sheet_id": "xy-1", "kind": "math", "y_transform": "log10" })),
            ("stats.run", json!({ "sheet_id": "stat-pca", "source_sheet_id": "mv-1", "kind": "pca", "standardize": true })),
            ("graph.create", json!({ "sheet_id": "g-mv", "data_sheet_id": "mv-1" })),
            ("graph.setKind", json!({ "sheet_id": "g-mv", "kind": "multiple_variables" })),
            (
                "graph.setSeries",
                json!({ "sheet_id": "g-mv", "multivariate": { "x_column": 0, "y_column": 1, "color": { "column": 5, "categorical": true }, "size": { "column": 2, "min_radius_pt": 4.0, "max_radius_pt": 14.0 }, "label_column": 5, "show_convex_hulls": true } }),
            ),
        ],
    );
    assert_eq!(rs(&session, "tr-log").status, ResultStatus::UpToDate);
    assert_eq!(rs(&session, "stat-pca").status, ResultStatus::UpToDate);
    assert_eq!(gs(&session, "g-mv").spec.kind, GraphKind::MultipleVariables);
    assert!(gs(&session, "g-mv").spec.multivariate.is_some());

    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-mv")).unwrap();
    assert!(scene.width_pt > 0.0 && !scene.primitives.is_empty());
}

#[test]
fn test_aligned_layout_scene_compilation_and_exports() {
    let mut session = session_with("Publication Figure", vec![xy_4pl_sheet("xy-1"), anova_col("col-1", 0.0)]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "stat-1", "source_sheet_id": "col-1", "kind": "one_way_anova" })),
            ("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy" })),
            ("graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column" })),
            (
                "layout.create",
                json!({ "sheet_id": "layout-fig1", "title": "Figure 1", "preset": "double_column", "grid_template": [2, 1], "panel_badges": "uppercase" }),
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "layout-fig1", "kind": "graph", "graph_sheet_id": "g-xy", "bounds_pt": [20.0, 20.0, 220.0, 140.0] }),
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "layout-fig1", "kind": "graph", "graph_sheet_id": "g-col", "bounds_pt": [32.0, 180.0, 220.0, 140.0] }),
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "layout-fig1", "kind": "table", "source_sheet_id": "stat-1", "bounds_pt": [265.0, 20.0, 210.0, 140.0] }),
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "layout-fig1", "kind": "text", "markdown": "**Panel summary**", "bounds_pt": [265.0, 180.0, 200.0, 30.0] }),
            ),
        ],
    );

    let align_out = ok_cmd(&mut session, "layout.alignOrigins", json!({ "sheet_id": "layout-fig1" }));
    assert_eq!(align_out.payload["align_plot_origins"], true);
    assert_eq!(align_out.payload["panels"][0]["x"], align_out.payload["panels"][1]["x"]);

    ok_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "layout-fig1", "orientation": "landscape" }));
    let scene = compile_sheet_scene(session.document(), &SheetId::new("layout-fig1")).unwrap();
    assert!(scene.width_pt > scene.height_pt && scene.primitives.len() >= 4);

    let svg = ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "layout-fig1" }));
    assert!(curvebook_svg::validate_svg_structure(svg.payload["svg"].as_str().unwrap()).is_ok());
    for (id, params) in [
        ("export.pdf", json!({ "sheet_id": "layout-fig1" })),
        ("export.png", json!({ "sheet_id": "layout-fig1", "dpi": 150 })),
        ("export.tiff", json!({ "sheet_id": "layout-fig1", "dpi": 150, "compression": "deflate" })),
    ] {
        assert!(ok_cmd(&mut session, id, params).payload["byte_len"].as_u64().unwrap() > 100);
    }
}

#[test]
fn test_registry_completeness_and_undo_redo_of_sci_commands() {
    let reg = cmd::registry();
    let mut seen = BTreeSet::new();
    for spec in reg.specs() {
        assert!(!spec.id.is_empty() && !spec.label.is_empty() && !spec.menu_path.is_empty());
        assert!(seen.insert(spec.id), "duplicate command id `{}`", spec.id);
    }

    for id in [
        "edit.undo",
        "edit.redo",
        "project.new",
        "sheet.createData",
        "table.setCell",
        "transform.run",
        "stats.run",
        "fit.run",
        "analysis.run",
        "analysis.updateParams",
        "analysis.retargetData",
        "analysis.exportJson",
        "analysis.exportCsv",
        "graph.create",
        "graph.initialize",
        "graph.setKind",
        "graph.setSpec",
        "graph.setAxes",
        "graph.setSeries",
        "graph.setRegressionOverlay",
        "graph.addComparisonBrackets",
        "graph.addAnnotation",
        "graph.applyStyleFrom",
        "layout.create",
        "layout.setSpec",
        "layout.addPanel",
        "layout.alignOrigins",
        "export.svg",
        "export.pdf",
        "export.png",
        "export.tiff",
    ] {
        assert!(reg.get(id).is_some(), "missing registry entry for `{id}`");
    }

    // Verify every command in cmd::registry() is recognized by Session (none returns unknown command).
    let mut session = Session::new();
    for spec in reg.specs() {
        if matches!(spec.id, "file.new" | "project.new") {
            continue;
        }
        if let Err(err) = session.execute(spec.id, json!({})) {
            assert!(!err.to_string().contains("unknown engine command"), "registered command `{}` not wired: {err}", spec.id);
        }
    }

    // Verify undo/redo around graph + analysis commands.
    let mut s2 = session_with("Undo Test", vec![anova_col("c1", 0.0)]);
    run_cmds(
        &mut s2,
        &[
            ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "c1", "kind": "column" })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "axis": "y", "title": "Updated Axis" })),
        ],
    );
    assert_eq!(gs(&s2, "g1").spec.y_axis.title, "Updated Axis");
    ok_cmd(&mut s2, "edit.undo", json!({}));
    assert_ne!(gs(&s2, "g1").spec.y_axis.title, "Updated Axis");
    ok_cmd(&mut s2, "edit.redo", json!({}));
    assert_eq!(gs(&s2, "g1").spec.y_axis.title, "Updated Axis");
}

#[test]
fn test_malformed_and_non_ascii_color_strings_rejected_without_panic() {
    let mut session = session_with("Color Validation", vec![anova_col("c1", 0.0)]);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "c1", "kind": "column" }));

    for bad_color in ["#a€bb", "#a€bbcc", "#€€", "#12345", "#zzzzzz", ""] {
        let err = err_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "color": bad_color }));
        assert!(err.to_string().contains("invalid hex"), "unexpected error for `{bad_color}`: {err}");
    }
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "color": "#1F77B4CC" }));
}

#[test]
fn aud_02_04_10_15_22_24_and_ext_01_fit_graph_validation_and_x_scale_warnings() {
    // AUD-02-04: linear/deming fit.run preserves standard_addition, extend_line_to_x_intercept, and Deming VarianceRatio.
    let mut s_xy = session_with("XY", vec![xy_sheet("xy-1", "Dose", "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Drug A", &[3.1, 4.9, 7.2, 8.8, 11.1])])]);
    run_cmds(
        &mut s_xy,
        &[
            (
                "fit.run",
                json!({ "sheet_id": "lin-std", "source_sheet_id": "xy-1", "kind": "linear", "standard_addition": true, "extend_line_to_x_intercept": true }),
            ),
            ("fit.run", json!({ "sheet_id": "dem-eq", "source_sheet_id": "xy-1", "kind": "deming" })),
            (
                "fit.run",
                json!({ "sheet_id": "dem-ratio", "source_sheet_id": "xy-1", "kind": "deming", "deming_error": { "VarianceRatio": { "lambda": 10.0 } }, "extend_line_to_x_intercept": true }),
            ),
        ],
    );
    let AnalysisKind::LinearRegression(ref lin) = rs(&s_xy, "lin-std").analysis.kind else { panic!("expected LinearRegression") };
    assert!(lin.standard_addition && lin.extend_line_to_x_intercept);
    let AnalysisKind::LinearRegression(ref dem) = rs(&s_xy, "dem-ratio").analysis.kind else { panic!("expected Deming") };
    assert!(dem.deming_error == DemingErrorSpec::VarianceRatio { lambda: 10.0 } && dem.extend_line_to_x_intercept);
    assert!((scalar(&s_xy, "dem-eq", "Slope") - scalar(&s_xy, "dem-ratio", "Slope")).abs() > 1e-6);

    // AUD-02-10: graph title stays synced across create, initialize, setSpec, and explicit inner spec.title.
    for (cmd_id, params, exp_sheet, exp_spec) in [
        ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy-1", "title": "Created Title" }), "Created Title", "Created Title"),
        ("graph.initialize", json!({ "sheet_id": "g1", "kind": "xy", "title": "Initialized Title" }), "Initialized Title", "Initialized Title"),
        ("graph.setSpec", json!({ "sheet_id": "g1", "title": "SetSpec Title" }), "SetSpec Title", "SetSpec Title"),
        (
            "graph.setSpec",
            json!({ "sheet_id": "g1", "title": "Sheet Label", "spec": { "title": "Explicit Inner Spec Title" } }),
            "Sheet Label",
            "Explicit Inner Spec Title",
        ),
    ] {
        ok_cmd(&mut s_xy, cmd_id, params);
        assert_eq!((gs(&s_xy, "g1").title.as_str(), gs(&s_xy, "g1").spec.title.as_str()), (exp_sheet, exp_spec));
    }

    // AUD-02-24: formatSelectedPoints mode="clear" removes stale out-of-bounds point override.
    let mut doc_oob = s_xy.document().clone();
    doc_oob.graph_sheet_mut(&SheetId::new("g1")).unwrap().spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 99,
        subcol: None,
        hidden: true,
        ..Default::default()
    });
    let mut s_oob = Session::from_document(doc_oob);
    let clr = ok_cmd(
        &mut s_oob,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g1", "mode": "clear", "points": [{ "series_index": 0, "point_index": 99 }] }),
    );
    assert!(clr.payload["override_count"] == 0 && gs(&s_oob, "g1").spec.point_overrides.is_empty());

    // AUD-02-15 & AUD-02-22(1,2): misplaced create_graph/spec conflicts, strict graph.addAnnotation & edit.pasteResultTable validation.
    let mut s_v = session_with("Val", vec![col_sheet("col-1", "G", &[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])])]);
    run_cmds(
        &mut s_v,
        &[
            ("stats.run", json!({ "sheet_id": "res-1", "source_sheet_id": "col-1", "kind": "descriptive" })),
            ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "col-1" })),
        ],
    );
    let spec_v = serde_json::to_value(AnalysisSpec::new("res-spec", "col-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
    assert_bad_inputs(
        &mut s_v,
        &[
            ("stats.run", json!({ "source_sheet_id": "col-1", "kind": "descriptive", "create_graph": true })),
            ("transform.run", json!({ "source_sheet_id": "col-1", "kind": "normalize", "graph_sheet_id": "g-99" })),
            ("analysis.run", json!({ "create_graph": true })),
            ("analysis.run", json!({ "sheet_id": "other-id", "spec": spec_v })),
            ("analysis.run", json!({ "source_sheet_id": "other-src", "spec": spec_v })),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g1", "annotation": { "TextBox": { "bounds_pt": { "x": 10.0, "y": 10.0, "width": 80.0, "height": 20.0 }, "markdown": "hi", "bordered": false } }, "kind": "text" }),
            ),
            ("graph.addAnnotation", json!({ "sheet_id": "g1", "kind": "text", "text": "hi", "bordred": true })),
            ("graph.addAnnotation", json!({ "sheet_id": "g1", "kind": "text", "text": "hi", "x_pt": "not_a_number" })),
            ("graph.addAnnotation", json!({ "sheet_id": "g1", "kind": "shape", "shape": "triangle" })),
            ("edit.pasteResultTable", json!({ "source_sheet_id": "res-1", "target_sheet_id": "g1", "style": { "bogus_key": true } })),
        ],
    );

    // EXT-01: fit.run default equation picks agonist_vs_response_4pl vs log_agonist_vs_response_4pl by X scale, surfaces scale/identifiability warnings, and inspects multi-subcolumn X.
    let (raw_xs, raw_ys) = ([0.001, 0.01, 0.1, 1.0, 10.0, 100.0, 1000.0], [2.0, 5.0, 18.0, 50.0, 82.0, 95.0, 99.0]);
    let mut s_raw = session_with("Raw", vec![xy_sheet("xy-1", "D", "X", &raw_xs, &[("A", &raw_ys)])]);
    assert_eq!(
        ok_cmd(&mut s_raw, "fit.run", json!({ "sheet_id": "fit-raw", "source_sheet_id": "xy-1" })).payload["equation_id"],
        "agonist_vs_response_4pl"
    );
    assert!(matches!(&rs(&s_raw, "fit-raw").analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "agonist_vs_response_4pl"));

    let mut s_log = session_with("Log", vec![xy_sheet("xy-1", "D", "X", &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0], &[("A", &raw_ys)])]);
    assert_eq!(
        ok_cmd(&mut s_log, "fit.run", json!({ "sheet_id": "fit-log", "source_sheet_id": "xy-1" })).payload["equation_id"],
        "log_agonist_vs_response_4pl"
    );

    let mut s_mis = session_with(
        "Mis",
        vec![xy_sheet("xy-1", "D", "X", &[1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0], &[("A", &[2.0, 12.0, 48.0, 85.0, 97.0, 99.0])])],
    );
    let mis =
        ok_cmd(&mut s_mis, "fit.run", json!({ "sheet_id": "fit-mis", "source_sheet_id": "xy-1", "equation_id": "log_agonist_vs_response_4pl" }));
    assert!(!mis.payload["scale_warnings"].as_array().unwrap().is_empty() && !mis.warnings.is_empty());
    assert!(mis.payload["ambiguous"].is_boolean() && mis.payload["fit_status"].is_string() && mis.payload["identifiability_warnings"].is_array());

    let mut s_amb =
        session_with("Amb", vec![xy_sheet("xy-1", "D", "X", &[-9.0, -8.5, -8.0, -7.5, -7.0, -6.5], &[("A", &[1.0, 1.3, 2.2, 4.5, 9.8, 21.0])])]);
    let amb = ok_cmd(
        &mut s_amb,
        "fit.run",
        json!({ "sheet_id": "fit-amb", "source_sheet_id": "xy-1", "equation_id": "log_agonist_vs_response_4pl", "identify_unstable_parameters": true }),
    );
    assert!(amb.payload["ambiguous"].as_bool().unwrap() || !amb.payload["identifiability_warnings"].as_array().unwrap().is_empty());

    let mut mx =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(2) }, SubcolumnMode::Single, 1, raw_xs.len())
            .unwrap();
    for (r, (&x, &y)) in raw_xs.iter().zip(&raw_ys).enumerate() {
        (mx.x_cells[r][0], mx.x_cells[r][1], mx.data_sets[0].cells[r][0]) = (Cell::empty(), Cell::number(x), Cell::number(y));
    }
    let mut doc_mx = ProjectDocument::new("Multi-X").unwrap();
    doc_mx.add_data_sheet(DataSheet::new("xy-mx", "Multi-X", TableArchetype::Xy(mx)).unwrap()).unwrap();
    assert_eq!(
        ok_cmd(&mut Session::from_document(doc_mx), "fit.run", json!({ "sheet_id": "fit-mx", "source_sheet_id": "xy-mx" })).payload["equation_id"],
        "agonist_vs_response_4pl"
    );
}
