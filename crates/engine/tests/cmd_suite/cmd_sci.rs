//! End-to-end integration tests for `cmd_sci`, `cmd::registry()`, and `compile_sheet_scene`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BreakOrnament, Cell, ColumnDesignHint, ColumnTable, DataSheet, FramePreset, GraphKind, MultipleVariablesTable,
    NonlinearFitSpec, ProjectDocument, ResultStatus, ResultTabKind, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::{Session, cmd, cmd::helpers::compile_sheet_scene};
use serde_json::{Value, json};

use super::common::{ok_cmd, unique_temp_path};

fn sample_4pl_xy_sheet(id: &str) -> DataSheet {
    // True curve: Y = 10 + 90 / (1 + 10^((-6.5 - X) * 1.1))
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut table = XyTable::new(SubcolumnMode::RawReplicates(2), 1, xs.len()).unwrap();
    table.x_title = String::from("Log10 Concentration (M)");
    if let Some(ds0) = table.data_sets.first_mut() {
        ds0.title = String::from("Compound A");
        for ((&x, x_row), row) in xs.iter().zip(table.x_cells.iter_mut()).zip(ds0.cells.iter_mut()) {
            if let Some(x_cell) = x_row.first_mut() {
                *x_cell = Cell::number(x);
            }
            let y_true = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.1));
            if let [c0, c1] = row.as_mut_slice() {
                *c0 = Cell::number(y_true - 0.8);
                *c1 = Cell::number(y_true + 0.8);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Dose Response", TableArchetype::Xy(table)).unwrap()
}

fn sample_anova_column_sheet(id: &str, offset: f64) -> DataSheet {
    let groups =
        [("Vehicle", [10.2_f64, 11.0, 9.8, 10.5, 10.1]), ("Low Dose", [14.5, 15.2, 13.9, 14.8, 15.1]), ("High Dose", [22.4, 23.1, 21.8, 22.9, 23.0])];
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, groups.len(), 5).unwrap();
    for (ds, (name, vals)) in table.data_sets.iter_mut().zip(groups.iter()) {
        ds.title = (*name).to_owned();
        for (row, &v) in ds.cells.iter_mut().zip(vals.iter()) {
            if let Some(cell) = row.first_mut() {
                *cell = Cell::number(v + offset);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Treatment Response", TableArchetype::Column(table)).unwrap()
}

fn sample_survival_sheet(id: &str) -> DataSheet {
    let rows = [
        (4.0_f64, Some(1.0_f64), None),
        (7.0, Some(1.0), None),
        (10.0, Some(0.0), None),
        (13.0, Some(1.0), None),
        (18.0, Some(1.0), None),
        (6.0, None, Some(1.0)),
        (11.0, None, Some(0.0)),
        (16.0, None, Some(1.0)),
        (22.0, None, Some(0.0)),
        (28.0, None, Some(1.0)),
    ];
    let mut table = SurvivalTable::new(2, rows.len()).unwrap();
    if let [ch0, ch1] = table.cohorts.as_mut_slice() {
        ch0.title = String::from("Control");
        ch1.title = String::from("Treated");
        for ((&(t, c0, c1), time_cell), (row0, row1)) in rows.iter().zip(table.times.iter_mut()).zip(ch0.cells.iter_mut().zip(ch1.cells.iter_mut())) {
            *time_cell = Cell::number(t);
            if let Some(ev) = c0
                && let Some(cell) = row0.first_mut()
            {
                *cell = Cell::number(ev);
            }
            if let Some(ev) = c1
                && let Some(cell) = row1.first_mut()
            {
                *cell = Cell::number(ev);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Survival Cohort", TableArchetype::Survival(table)).unwrap()
}

fn sample_multivariate_sheet(id: &str) -> DataSheet {
    let rows = [
        ("S1", 1.2_f64, 10.5_f64, 24.0_f64, 2.0_f64, 0.0_f64, "Ctrl"),
        ("S2", 2.1, 14.2, 31.0, 4.0, 0.0, "Ctrl"),
        ("S3", 3.4, 18.8, 39.0, 7.0, 1.0, "Treat"),
        ("S4", 4.5, 22.1, 46.0, 10.0, 1.0, "Treat"),
        ("S5", 5.8, 27.9, 54.0, 13.0, 1.0, "Treat"),
        ("S6", 6.9, 31.3, 62.0, 16.0, 0.0, "Ctrl"),
    ];
    let mut table = MultipleVariablesTable::new(6, rows.len()).unwrap();
    let titles = ["Marker X", "Marker Y", "Marker Z", "Weight", "Status", "Group"];
    for (var, title) in table.variables.iter_mut().zip(titles.iter()) {
        var.title = (*title).to_owned();
    }
    if let [v0, v1, v2, v3, v4, v5] = table.variables.as_mut_slice() {
        v5.role = VariableRole::Categorical;
        v5.role_inferred = false;

        for (r, &(label, x, y, z, w, st, grp)) in rows.iter().enumerate() {
            if let Some(rt) = table.row_titles.get_mut(r) {
                *rt = label.to_owned();
            }
            if let Some(c) = v0.cells.get_mut(r) {
                *c = Cell::number(x);
            }
            if let Some(c) = v1.cells.get_mut(r) {
                *c = Cell::number(y);
            }
            if let Some(c) = v2.cells.get_mut(r) {
                *c = Cell::number(z);
            }
            if let Some(c) = v3.cells.get_mut(r) {
                *c = Cell::number(w);
            }
            if let Some(c) = v4.cells.get_mut(r) {
                *c = Cell::number(st);
            }
            if let Some(c) = v5.cells.get_mut(r) {
                *c = Cell::text(grp);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Multivariate Assay", TableArchetype::MultipleVariables(table)).unwrap()
}

#[test]
fn test_4pl_fit_graph_overlay_styling_and_all_exports() {
    let mut doc = ProjectDocument::new("4PL Pharmacology").unwrap();
    doc.add_data_sheet(sample_4pl_xy_sheet("data-4pl")).unwrap();
    let mut session = Session::from_document(doc);

    // 1. Run 4PL nonlinear regression via shorthand dict.
    let fit_out = ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-4pl",
            "source_sheet_id": "data-4pl",
            "kind": "four_parameter_logistic",
            "confidence_bands": true,
            "title": "Compound A 4PL"
        }),
    );
    assert!(fit_out.mutated);
    let fit_res = session.document().result_sheet(&SheetId::new("fit-4pl")).unwrap();
    assert_eq!(fit_res.status, ResultStatus::UpToDate);
    assert!(!fit_res.fitted_curves.is_empty());

    // Verify full typed AnalysisSpec JSON input path works too.
    let typed_spec =
        AnalysisSpec::new("fit-typed", "data-4pl", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": typed_spec }));
    assert!(session.document().result_sheet(&SheetId::new("fit-typed")).is_some());

    // 2. Export analysis results to JSON and tidy/tab CSV.
    let json_path = unique_temp_path("fit_4pl", "json");
    let csv_path = unique_temp_path("fit_4pl", "csv");
    let exp_json = ok_cmd(&mut session, "analysis.exportJson", json!({ "sheet_id": "fit-4pl", "path": json_path.to_string_lossy() }));
    assert!(!exp_json.mutated);
    let parsed_json: Value = serde_json::from_str(exp_json.payload["json"].as_str().unwrap()).unwrap();
    assert_eq!(parsed_json["id"], "fit-4pl");
    assert!(std::fs::read_to_string(&json_path).unwrap().contains("log_agonist_vs_response_4pl"));

    let exp_csv = ok_cmd(&mut session, "analysis.exportCsv", json!({ "sheet_id": "fit-4pl", "path": csv_path.to_string_lossy() }));
    assert!(exp_csv.payload["csv"].as_str().unwrap().contains("fit-4pl"));
    assert!(std::fs::metadata(&csv_path).unwrap().len() > 20);

    // 3. Create uninitialized graph -> initialize -> configure axes (including 2 discontinuous Y segments + tick format), series & overlay & annotations.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-4pl", "data_sheet_id": "data-4pl", "initialized": false }));
    assert!(!session.document().graph_sheet(&SheetId::new("g-4pl")).unwrap().initialized);

    ok_cmd(&mut session, "graph.initialize", json!({ "sheet_id": "g-4pl", "kind": "xy", "title": "Dose-Response Curve" }));
    assert!(session.document().graph_sheet(&SheetId::new("g-4pl")).unwrap().initialized);

    ok_cmd(
        &mut session,
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
                "segments": [
                    { "min": 0.0, "max": 40.0, "scale": "linear", "length_share": 0.4 },
                    { "min": 50.0, "max": 110.0, "scale": "linear", "length_share": 0.6 }
                ]
            }
        }),
    );
    assert_eq!(session.document().graph_sheet(&SheetId::new("g-4pl")).unwrap().spec.y_axis.segments.len(), 2);

    // Reject 4 segments cleanly (must allow 1..=3 segments).
    assert!(
        session
            .execute(
                "graph.setAxes",
                json!({
                    "sheet_id": "g-4pl",
                    "axis": "y",
                    "segments": [{}, {}, {}, {}]
                })
            )
            .is_err()
    );

    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g-4pl",
            "series_index": 0,
            "legend_label": "Compound A (n=2)",
            "color": "#1F77B4",
            "marker": { "shape": "diamond", "fill_mode": "open", "size_pt": 6.0 },
            "y_error_bars": { "metric": "sem", "direction": "both", "cap_width_pt": 5.0 },
            "connecting_line": { "kind": "none" }
        }),
    );

    ok_cmd(
        &mut session,
        "graph.setRegressionOverlay",
        json!({
            "sheet_id": "g-4pl",
            "result_sheet_id": "fit-4pl",
            "series_index": 0,
            "sample_segments": 300,
            "show_confidence_band": true,
            "shaded_ribbon": true
        }),
    );

    ok_cmd(
        &mut session,
        "graph.addAnnotation",
        json!({ "sheet_id": "g-4pl", "kind": "text", "text": "LogEC50 = {{scalar:fit-4pl:LogEC50:none}}", "x_pt": 180.0, "y_pt": 60.0 }),
    );
    ok_cmd(
        &mut session,
        "graph.addAnnotation",
        json!({ "sheet_id": "g-4pl", "kind": "table", "result_sheet_id": "fit-4pl", "bounds_pt": [220.0, 150.0, 140.0, 60.0] }),
    );

    // Apply Style From preserving target data bindings.
    let mut target_doc = session.document().clone();
    target_doc.add_data_sheet(sample_4pl_xy_sheet("data-4pl-2")).unwrap();
    session = Session::from_document(target_doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-4pl-2", "data_sheet_id": "data-4pl-2", "kind": "xy" }));
    ok_cmd(&mut session, "graph.applyStyleFrom", json!({ "sheet_id": "g-4pl-2", "donor_sheet_id": "g-4pl" }));
    let g2 = session.document().graph_sheet(&SheetId::new("g-4pl-2")).unwrap();
    assert_eq!(g2.spec.primary_source_sheet_id.as_str(), "data-4pl-2");
    assert_eq!(g2.spec.series[0].source_sheet_id.as_str(), "data-4pl-2");
    assert_eq!(g2.spec.frame_preset, FramePreset::OpenL);
    assert_eq!(g2.spec.y_axis.break_ornament, BreakOrnament::DoubleSlash);
    assert_eq!(g2.spec.y_axis.segments.len(), 1);

    // 4. Compile scene and export SVG, PDF, PNG, TIFF (both payload & file output).
    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-4pl")).unwrap();
    assert!(scene.width_pt > 0.0 && scene.height_pt > 0.0);

    let svg_path = unique_temp_path("g_4pl", "svg");
    let pdf_path = unique_temp_path("g_4pl", "pdf");
    let png_path = unique_temp_path("g_4pl", "png");
    let tiff_path = unique_temp_path("g_4pl", "tiff");

    let svg_out = ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "g-4pl", "path": svg_path.to_string_lossy() }));
    let svg_str = svg_out.payload["svg"].as_str().unwrap();
    assert!(svg_str.contains("<svg") && curvebook_svg::validate_svg_structure(svg_str).is_ok());
    assert!(std::fs::read_to_string(&svg_path).unwrap().contains("<svg"));

    let pdf_out = ok_cmd(&mut session, "export.pdf", json!({ "sheet_id": "g-4pl", "color_space": "cmyk", "path": pdf_path.to_string_lossy() }));
    assert!(pdf_out.payload["byte_len"].as_u64().unwrap() > 100);
    assert!(pdf_out.payload["base64"].as_str().unwrap().len() > 100);
    let pdf_bytes = std::fs::read(&pdf_path).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    let png_out = ok_cmd(&mut session, "export.png", json!({ "sheet_id": "g-4pl", "dpi": 150, "path": png_path.to_string_lossy() }));
    assert!(png_out.payload["byte_len"].as_u64().unwrap() > 100);
    let png_bytes = std::fs::read(&png_path).unwrap();
    assert!(png_bytes.starts_with(&[0x89, b'P', b'N', b'G']));

    let tiff_out =
        ok_cmd(&mut session, "export.tiff", json!({ "sheet_id": "g-4pl", "dpi": 150, "compression": "lzw", "path": tiff_path.to_string_lossy() }));
    assert!(tiff_out.payload["byte_len"].as_u64().unwrap() > 100);
    let tiff_bytes = std::fs::read(&tiff_path).unwrap();
    assert!(tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*"));
}

#[test]
fn test_stats_anova_auto_brackets_update_params_and_retarget() {
    let mut doc = ProjectDocument::new("ANOVA Study").unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("col-1", 0.0)).unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("col-2", 5.0)).unwrap();
    doc.add_data_sheet(sample_survival_sheet("surv-incompatible")).unwrap();
    let mut session = Session::from_document(doc);

    // Run One-Way ANOVA with Tukey multiplicity.
    let anova_out = ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "stat-anova",
            "source_sheet_id": "col-1",
            "kind": "one_way_anova",
            "multiplicity": "tukey"
        }),
    );
    assert!(anova_out.payload["pairwise_comparisons_count"].as_u64().unwrap() >= 3);

    // Create column graph & auto-hook pairwise brackets.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column" }));
    let br_out = ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-col",
            "result_sheet_id": "stat-anova",
            "significant_only": true,
            "style": "square",
            "label_mode": "stars"
        }),
    );
    assert!(br_out.payload["added"].as_u64().unwrap() >= 1);

    // Also add an explicit manual bracket referencing group labels.
    ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-col",
            "result_sheet_id": "stat-anova",
            "group_a": "Vehicle",
            "group_b": "High Dose",
            "style": "curly",
            "label_mode": "numeric"
        }),
    );

    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-col")).unwrap();
    assert!(scene.width_pt > 0.0);

    // Update parameters (switch ANOVA multiplicity & method -> Kruskal-Wallis + Dunn).
    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "stat-anova", "kind": "kruskal_wallis", "multiplicity": "dunn", "confidence_percent": 99.0 }),
    );
    assert!(matches!(session.document().result_sheet(&SheetId::new("stat-anova")).unwrap().analysis.kind, AnalysisKind::OneWayAnova(_)));

    // Retarget to compatible Column table succeeds; incompatible Survival table rolls back cleanly.
    ok_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "stat-anova", "source_sheet_id": "col-2" }));
    assert_eq!(session.document().result_sheet(&SheetId::new("stat-anova")).unwrap().analysis.source_sheet_id.as_str(), "col-2");

    assert!(session.execute("analysis.retargetData", json!({ "sheet_id": "stat-anova", "source_sheet_id": "surv-incompatible" })).is_err());
    assert_eq!(session.document().result_sheet(&SheetId::new("stat-anova")).unwrap().analysis.source_sheet_id.as_str(), "col-2");
}

#[test]
fn test_survival_km_and_cox_with_number_at_risk_graph() {
    let mut doc = ProjectDocument::new("Survival & Cox Study").unwrap();
    doc.add_data_sheet(sample_survival_sheet("surv-1")).unwrap();
    doc.add_data_sheet(sample_multivariate_sheet("mv-cox")).unwrap();
    let mut session = Session::from_document(doc);

    // Run Kaplan-Meier.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "stat-km", "source_sheet_id": "surv-1", "kind": "kaplan_meier" }));
    let km = session.document().result_sheet(&SheetId::new("stat-km")).unwrap();
    assert!(!km.fitted_curves.is_empty());
    assert!(km.tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk));

    // Run Cox PH on MultipleVariables table.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "stat-cox",
            "source_sheet_id": "mv-cox",
            "kind": "cox_ph",
            "time_column": 3,
            "status_column": 4,
            "predictor_columns": [0, 1]
        }),
    );
    assert_eq!(session.document().result_sheet(&SheetId::new("stat-cox")).unwrap().status, ResultStatus::UpToDate);

    // Build Survival graph with Number-at-Risk table enabled.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-surv", "data_sheet_id": "surv-1", "kind": "survival" }));
    ok_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g-surv", "survival_number_at_risk": true }));
    assert!(session.document().graph_sheet(&SheetId::new("g-surv")).unwrap().spec.survival_number_at_risk);

    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-surv")).unwrap();
    assert!(scene.width_pt > 0.0 && !scene.primitives.is_empty());
    let svg = ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "g-surv" }));
    assert!(svg.payload["svg"].as_str().unwrap().contains("<svg"));
}

#[test]
fn test_multivariate_pca_bubble_and_transform() {
    let mut doc = ProjectDocument::new("Multivariate Bubble").unwrap();
    doc.add_data_sheet(sample_multivariate_sheet("mv-1")).unwrap();
    doc.add_data_sheet(sample_4pl_xy_sheet("xy-1")).unwrap();
    let mut session = Session::from_document(doc);

    // Run transform + PCA.
    ok_cmd(&mut session, "transform.run", json!({ "sheet_id": "tr-log", "source_sheet_id": "xy-1", "kind": "math", "y_transform": "log10" }));
    assert_eq!(session.document().result_sheet(&SheetId::new("tr-log")).unwrap().status, ResultStatus::UpToDate);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "stat-pca", "source_sheet_id": "mv-1", "kind": "pca", "standardize": true }));
    assert_eq!(session.document().result_sheet(&SheetId::new("stat-pca")).unwrap().status, ResultStatus::UpToDate);

    // Configure 5-channel Multivariate Bubble plot via graph.create + graph.setKind + graph.setSeries.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-mv", "data_sheet_id": "mv-1" }));
    ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": "g-mv", "kind": "multiple_variables" }));
    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g-mv",
            "multivariate": {
                "x_column": 0,
                "y_column": 1,
                "color": { "column": 5, "categorical": true },
                "size": { "column": 2, "min_radius_pt": 4.0, "max_radius_pt": 14.0 },
                "label_column": 5,
                "show_convex_hulls": true
            }
        }),
    );

    let g_mv = session.document().graph_sheet(&SheetId::new("g-mv")).unwrap();
    assert_eq!(g_mv.spec.kind, GraphKind::MultipleVariables);
    assert!(g_mv.spec.multivariate.is_some());

    let scene = compile_sheet_scene(session.document(), &SheetId::new("g-mv")).unwrap();
    assert!(scene.width_pt > 0.0 && !scene.primitives.is_empty());
}

#[test]
fn test_aligned_layout_scene_compilation_and_exports() {
    let mut doc = ProjectDocument::new("Publication Figure").unwrap();
    doc.add_data_sheet(sample_4pl_xy_sheet("xy-1")).unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("col-1", 0.0)).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "stat-1", "source_sheet_id": "col-1", "kind": "one_way_anova" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column" }));

    ok_cmd(
        &mut session,
        "layout.create",
        json!({
            "sheet_id": "layout-fig1",
            "title": "Figure 1",
            "preset": "double_column",
            "grid_template": [2, 1],
            "panel_badges": "uppercase"
        }),
    );

    // Add two skewed vertical graph panels + table + callout, then align origins.
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "layout-fig1", "kind": "graph", "graph_sheet_id": "g-xy", "bounds_pt": [20.0, 20.0, 220.0, 140.0] }),
    );
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "layout-fig1", "kind": "graph", "graph_sheet_id": "g-col", "bounds_pt": [32.0, 180.0, 220.0, 140.0] }),
    );
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "layout-fig1", "kind": "table", "source_sheet_id": "stat-1", "bounds_pt": [265.0, 20.0, 210.0, 140.0] }),
    );
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "layout-fig1", "kind": "text", "markdown": "**Panel summary**", "bounds_pt": [265.0, 180.0, 200.0, 30.0] }),
    );

    let align_out = ok_cmd(&mut session, "layout.alignOrigins", json!({ "sheet_id": "layout-fig1" }));
    assert_eq!(align_out.payload["align_plot_origins"], true);
    assert_eq!(align_out.payload["panels"][0]["x"], align_out.payload["panels"][1]["x"]);

    ok_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "layout-fig1", "orientation": "landscape" }));

    let scene = compile_sheet_scene(session.document(), &SheetId::new("layout-fig1")).unwrap();
    assert!(scene.width_pt > scene.height_pt);
    assert!(scene.primitives.len() >= 4);

    let svg = ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "layout-fig1" }));
    assert!(curvebook_svg::validate_svg_structure(svg.payload["svg"].as_str().unwrap()).is_ok());

    let pdf = ok_cmd(&mut session, "export.pdf", json!({ "sheet_id": "layout-fig1" }));
    assert!(pdf.payload["byte_len"].as_u64().unwrap() > 100);

    let png = ok_cmd(&mut session, "export.png", json!({ "sheet_id": "layout-fig1", "dpi": 150 }));
    assert!(png.payload["byte_len"].as_u64().unwrap() > 100);

    let tiff = ok_cmd(&mut session, "export.tiff", json!({ "sheet_id": "layout-fig1", "dpi": 150, "compression": "deflate" }));
    assert!(tiff.payload["byte_len"].as_u64().unwrap() > 100);
}

#[test]
fn test_registry_completeness_and_undo_redo_of_sci_commands() {
    use std::collections::BTreeSet;

    let reg = cmd::registry();
    let mut seen = BTreeSet::new();
    for spec in reg.specs() {
        assert!(!spec.id.is_empty(), "empty command id");
        assert!(!spec.label.is_empty(), "empty command label for {}", spec.id);
        assert!(!spec.menu_path.is_empty(), "empty command menu_path for {}", spec.id);
        assert!(seen.insert(spec.id), "duplicate command id `{}`", spec.id);
    }

    let required_ids = [
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
    ];
    for id in required_ids {
        assert!(reg.get(id).is_some(), "missing registry entry for `{id}`");
    }

    // Verify every command in cmd::registry() is recognized by Session (none returns unknown command).
    let mut session = Session::new();
    for spec in reg.specs() {
        if matches!(spec.id, "file.new" | "project.new") {
            continue;
        }
        if let Err(err) = session.execute(spec.id, json!({})) {
            assert!(!err.to_string().contains("unknown engine command"), "registered command `{}` is not wired in dispatcher: {err}", spec.id);
        }
    }

    // Verify undo/redo around graph + analysis commands.
    let mut doc = ProjectDocument::new("Undo Test").unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("c1", 0.0)).unwrap();
    let mut s2 = Session::from_document(doc);
    ok_cmd(&mut s2, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "c1", "kind": "column" }));
    ok_cmd(&mut s2, "graph.setAxes", json!({ "sheet_id": "g1", "axis": "y", "title": "Updated Axis" }));
    assert_eq!(s2.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.y_axis.title, "Updated Axis");

    ok_cmd(&mut s2, "edit.undo", json!({}));
    assert_ne!(s2.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.y_axis.title, "Updated Axis");

    ok_cmd(&mut s2, "edit.redo", json!({}));
    assert_eq!(s2.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.y_axis.title, "Updated Axis");
}

#[test]
fn test_malformed_and_non_ascii_color_strings_rejected_without_panic() {
    let mut doc = ProjectDocument::new("Color Validation").unwrap();
    doc.add_data_sheet(sample_anova_column_sheet("c1", 0.0)).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "c1", "kind": "column" }));

    for bad_color in ["#a€bb", "#a€bbcc", "#€€", "#12345", "#zzzzzz", ""] {
        let err = session
            .execute("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "color": bad_color }))
            .expect_err("malformed color must return Err rather than panic");
        assert!(err.to_string().contains("invalid hex"), "unexpected error for `{bad_color}`: {err}");
    }

    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "color": "#1F77B4CC" }));
}
