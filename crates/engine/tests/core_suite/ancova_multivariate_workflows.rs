//! End-to-end engine workflows for general ANCOVA, Holm-Bonferroni & Duncan multiplicity,
//! Lineweaver-Burk double-reciprocal transforms, CurveCalculus, Kendall's tau-b,
//! standalone Dendrograms, and PCA Biplots.

use curvebook_doc::{AnalysisKind, Cell, ChecklistStatus, DiagnosticPlotKind, GraphKind, MultiplicityMethod, SheetId, VariableRole};
use serde_json::json;

use super::common::{col_sheet, mv_sheet, session_with, xy_sheet};

#[test]
fn ancova_mv_xy_multiplicity_and_methods_report_e2e() -> Result<(), Box<dyn std::error::Error>> {
    // Populate Response (0), Treatment (1), BaselineX1 (2), CovariateX2 (3) matching the R 4.5.2 oracle dataset.
    let g_labels = ["Ctrl", "Ctrl", "Ctrl", "Ctrl", "Ctrl", "Low", "Low", "Low", "Low", "High", "High", "High", "High", "High"];
    let x1 = [1.0, 2.5, 4.0, 5.5, 7.0, 1.5, 3.0, 5.0, 6.5, 2.0, 3.5, 4.5, 6.0, 8.0];
    let x2 = [5.0, 3.0, 6.0, 2.0, 8.0, 4.0, 7.0, 2.0, 5.0, 6.0, 3.0, 7.0, 4.0, 9.0];
    let y_base = [10.2, 13.1, 16.5, 19.0, 22.4, 14.8, 17.5, 22.1, 24.9, 19.2, 22.8, 24.6, 28.0, 31.9];
    let eps = [0.1, -0.2, 0.3, -0.1, 0.2, -0.3, 0.4, -0.2, 0.1, 0.2, -0.1, 0.3, -0.4, 0.1];

    let y_cells: Vec<Cell> = (0..14).map(|r| Cell::number(y_base[r] + 0.7 * x2[r] + eps[r])).collect();
    let g_cells: Vec<Cell> = g_labels.iter().map(|g| Cell::text(*g)).collect();
    let x1_cells: Vec<Cell> = x1.iter().map(|&v| Cell::number(v)).collect();
    let x2_cells: Vec<Cell> = x2.iter().map(|&v| Cell::number(v)).collect();

    let ancova_ds = mv_sheet(
        "ancova-study",
        "ANCOVA Study",
        &[],
        &[
            ("Response", Some(VariableRole::Continuous), &y_cells),
            ("Treatment", Some(VariableRole::Categorical), &g_cells),
            ("BaselineX1", Some(VariableRole::Continuous), &x1_cells),
            ("CovariateX2", Some(VariableRole::Continuous), &x2_cells),
        ],
    );
    let col_ds = col_sheet(
        "groups-col",
        "Groups",
        &[
            ("Control", &[10.0, 10.4, 9.8, 10.2, 10.1]),
            ("Low", &[10.8, 11.1, 10.6, 10.9, 11.0]),
            ("Mid", &[11.9, 12.2, 11.7, 12.0, 12.1]),
            ("High", &[13.8, 14.2, 13.5, 14.0, 13.9]),
        ],
    );

    let mut sess = session_with("ANCOVA E2E", vec![ancova_ds, col_ds]);

    // 1. 2-covariate ANCOVA with Holm-Bonferroni post-hoc
    let run_out = sess.execute(
        "stats.run",
        json!({
            "source_sheet_id": "ancova-study",
            "kind": "ancova",
            "response_column": 0,
            "group_column": 1,
            "covariate_columns": [2, 3],
            "ss_type": "TypeIII",
            "test_homogeneity_of_slopes": true,
            "multiplicity": "holm_bonferroni"
        }),
    )?;
    let res_id = run_out.payload["sheet_id"].as_str().ok_or("missing ancova sheet_id")?.to_owned();
    let rs = sess.document().result_sheet(&SheetId::new(res_id.as_str())).ok_or("missing ancova result")?;
    assert!(rs.tabs.iter().any(|t| t.id == "ancova-table"));
    assert!(rs.tabs.iter().any(|t| t.id == "ancova-adjusted-means"));
    assert!(rs.tabs.iter().any(|t| t.id == "ancova-covariate-slopes"));
    assert!(rs.tabs.iter().any(|t| t.id == "ancova-homogeneity"));
    assert!(rs.tabs.iter().any(|t| t.id == "ancova-posthoc"));
    assert_eq!(rs.pairwise_comparisons.len(), 3);
    assert!((rs.lookup_scalar("Group F", None).ok_or("Group F")? - 1_129.296_00).abs() < 1e-2);

    // 2. Switch to Duncan post-hoc via analysis.updateParams and verify methods report across formats
    sess.execute("analysis.updateParams", json!({ "sheet_id": res_id, "multiplicity": "duncan" }))?;
    let rs_duncan = sess.document().result_sheet(&SheetId::new(res_id.as_str())).ok_or("missing duncan result")?;
    assert!(matches!(
        &rs_duncan.analysis.kind,
        AnalysisKind::Ancova(spec) if spec.multiplicity.method == MultiplicityMethod::Duncan
    ));
    for fmt in ["plain", "markdown", "latex"] {
        let rep = sess.execute("result.generateReport", json!({ "result_sheet_id": res_id, "format": fmt }))?;
        let text = rep.payload["report"].as_str().ok_or("missing report")?;
        assert!(text.contains("analysis of covariance") || text.contains("ANCOVA"), "{fmt}: {text}");
        assert!(text.contains("Duncan"), "{fmt}: {text}");
        assert!(text.contains("Maxwell & Delaney") || text.contains(r"Maxwell \& Delaney"), "{fmt}: {text}");
    }

    // 3. One-way ANOVA with Holm-Bonferroni and Duncan multiplicity shorthands
    let holm_out =
        sess.execute("stats.run", json!({ "source_sheet_id": "groups-col", "kind": "one_way_anova", "multiplicity": "holm", "control_column": 0 }))?;
    let holm_id = holm_out.payload["sheet_id"].as_str().ok_or("missing holm sheet_id")?;
    let holm_rs = sess.document().result_sheet(&SheetId::new(holm_id)).ok_or("missing holm rs")?;
    assert_eq!(holm_rs.pairwise_comparisons.len(), 3);
    let holm_rep = sess.execute("result.generateReport", json!({ "result_sheet_id": holm_id, "format": "plain" }))?;
    let holm_text = holm_rep.payload["report"].as_str().ok_or("rep")?;
    assert!(holm_text.contains("Holm–Bonferroni") || holm_text.contains("Holm-Bonferroni"), "{holm_text}");

    let dunc_out = sess.execute("stats.run", json!({ "source_sheet_id": "groups-col", "kind": "one_way_anova", "multiplicity": "duncan" }))?;
    let dunc_id = dunc_out.payload["sheet_id"].as_str().ok_or("missing dunc sheet_id")?;
    let dunc_rs = sess.document().result_sheet(&SheetId::new(dunc_id)).ok_or("missing dunc rs")?;
    assert_eq!(dunc_rs.pairwise_comparisons.len(), 6);
    Ok(())
}

#[test]
fn lineweaver_burk_curve_calculus_kendall_dendrogram_and_biplot_e2e() -> Result<(), Box<dyn std::error::Error>> {
    let xs = [0.0_f64, 1.0, 2.0, 4.0, 8.0];
    let ys: Vec<f64> = xs.iter().map(|&s| if s == 0.0 { 0.0 } else { 100.0 * s / (2.0 + s) }).collect();
    let xy_ds = xy_sheet("mm-xy", "Michaelis-Menten", "[Substrate] (uM)", &xs, &[("Velocity", &ys)]);

    let mv_rows = [
        ("Ctrl", 1.0, 2.0, 8.0),
        ("Ctrl", 2.0, 2.0, 7.5),
        ("Ctrl", 2.0, 3.0, 7.0),
        ("Ctrl", 3.0, 5.0, 6.2),
        ("Drug", 4.0, 5.0, 3.5),
        ("Drug", 5.0, 6.0, 2.8),
        ("Drug", 6.0, 7.0, 2.1),
        ("Drug", 7.0, 8.0, 1.5),
    ];
    let g_col: Vec<Cell> = mv_rows.iter().map(|(g, _, _, _)| Cell::text(*g)).collect();
    let a_col: Vec<Cell> = mv_rows.iter().map(|(_, a, _, _)| Cell::number(*a)).collect();
    let b_col: Vec<Cell> = mv_rows.iter().map(|(_, _, b, _)| Cell::number(*b)).collect();
    let c_col: Vec<Cell> = mv_rows.iter().map(|(_, _, _, c)| Cell::number(*c)).collect();
    let mv_ds = mv_sheet(
        "mv-panel",
        "MV Panel",
        &[],
        &[
            ("Group", Some(VariableRole::Categorical), &g_col),
            ("VarA", Some(VariableRole::Continuous), &a_col),
            ("VarB", Some(VariableRole::Continuous), &b_col),
            ("VarC", Some(VariableRole::Continuous), &c_col),
        ],
    );

    let mut sess = session_with("Tier 2-3 Workflows", vec![xy_ds, mv_ds]);

    // 1. Lineweaver-Burk double-reciprocal transform & XY graph
    let lb_out = sess.execute(
        "transform.run",
        json!({
            "source_sheet_id": "mm-xy",
            "kind": "math",
            "pharmacology_preset": "LineweaverBurk",
            "create_graph": true
        }),
    )?;
    let lb_id = lb_out.payload["sheet_id"].as_str().ok_or("missing lb_id")?;
    let lb_rs = sess.document().result_sheet(&SheetId::new(lb_id)).ok_or("missing lb rs")?;
    assert!((lb_rs.lookup_scalar("Apparent Vmax", Some(0)).ok_or("vmax")? - 100.0).abs() < 1e-10);
    assert!((lb_rs.lookup_scalar("Apparent Km", Some(0)).ok_or("km")? - 2.0).abs() < 1e-10);
    assert!(lb_rs.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("distortion")));
    let lb_graph_id = lb_out.payload["graph_sheet_id"].as_str().ok_or("missing lb graph")?;
    let lb_svg = sess.execute("graph.exportSvg", json!({ "sheet_id": lb_graph_id }))?;
    assert!(lb_svg.payload["svg"].as_str().ok_or("svg")?.contains("<svg"));

    // 2. CurveCalculus (Savitzky-Golay smoothing) with auto-graphing
    let cc_out = sess.execute(
        "transform.run",
        json!({
            "source_sheet_id": "mm-xy",
            "kind": "curve_calculus",
            "operation": { "SmoothSavitzkyGolay": { "window": 5, "order": 2 } },
            "create_graph": true
        }),
    )?;
    let cc_id = cc_out.payload["sheet_id"].as_str().ok_or("missing cc_id")?;
    assert!(sess.document().result_sheet(&SheetId::new(cc_id)).and_then(|r| r.analyzable_data.as_ref()).is_some());
    assert!(cc_out.payload["graph_sheet_id"].as_str().is_some());

    // 3a. Kendall's tau-b correlation matrix
    let kendall_out = sess.execute("stats.run", json!({ "source_sheet_id": "mv-panel", "kind": "kendall", "selected_data_sets": [1, 2, 3] }))?;
    let kendall_id = kendall_out.payload["sheet_id"].as_str().ok_or("missing kendall_id")?;
    let kendall_rep = sess.execute("result.generateReport", json!({ "result_sheet_id": kendall_id, "format": "plain" }))?;
    assert!(kendall_rep.payload["report"].as_str().ok_or("kendall rep")?.contains("Kendall"));

    // 3b. PCA + standalone PCA Biplot SVG export
    let pca_out = sess.execute("stats.run", json!({ "source_sheet_id": "mv-panel", "kind": "pca", "selected_data_sets": [1, 2, 3] }))?;
    let pca_id = pca_out.payload["sheet_id"].as_str().ok_or("missing pca_id")?;
    let biplot_g = sess.execute(
        "graph.create",
        json!({ "data_sheet_id": pca_id, "kind": GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot), "initialized": true }),
    )?;
    let biplot_gid = biplot_g.payload["sheet_id"].as_str().ok_or("missing biplot_gid")?;
    let biplot_svg = sess.execute("graph.exportSvg", json!({ "sheet_id": biplot_gid }))?;
    let biplot_svg_str = biplot_svg.payload["svg"].as_str().ok_or("biplot svg")?;
    assert!(biplot_svg_str.contains("<svg") && biplot_svg_str.contains("VarA"));

    // 3c. Hierarchical Clustering + standalone Dendrogram SVG export
    let cl_out = sess
        .execute("stats.run", json!({ "source_sheet_id": "mv-panel", "kind": "clustering", "selected_data_sets": [1, 2, 3], "cut_height": 2.0 }))?;
    let cl_id = cl_out.payload["sheet_id"].as_str().ok_or("missing cl_id")?;
    let dendro_g = sess.execute("graph.create", json!({ "data_sheet_id": cl_id, "kind": "dendrogram", "initialized": true }))?;
    let dendro_gid = dendro_g.payload["sheet_id"].as_str().ok_or("missing dendro_gid")?;
    // Also verify graph.setKind with the UI option_id alias "diag.dendrogram"
    sess.execute("graph.setKind", json!({ "sheet_id": dendro_gid, "kind": "dendrogram", "option_id": "diag.dendrogram" }))?;
    let dendro_svg = sess.execute("graph.exportSvg", json!({ "sheet_id": dendro_gid }))?;
    assert!(dendro_svg.payload["svg"].as_str().ok_or("dendro svg")?.contains("<svg"));
    Ok(())
}
