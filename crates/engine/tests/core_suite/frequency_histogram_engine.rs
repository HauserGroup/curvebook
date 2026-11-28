#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! End-to-end engine command, reactive DAG, methods-report, SVG/PDF/PNG export, and archive
//! round-trip tests for standalone Frequency Distribution (`AnalysisKind::FrequencyDistribution`)
//! and Histogram (`GraphKind::Histogram`).

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, FrequencyBinningRule, FrequencyOutputMode, GraphKind,
    HistogramBinAlignment, HistogramDisplayStyle, HistogramMultiSeriesMode, ProjectDocument, ResultStatus, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_format::{open_project_bytes, save_project_bytes};
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_render::{RenderTargetSpec, encode_png, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use serde_json::json;

fn make_session_with_column_data() -> Session {
    let mut doc = ProjectDocument::new("Frequency Histogram Study").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::default(), 2, 10).unwrap();
    col.data_sets[0].title = String::from("Baseline");
    col.data_sets[1].title = String::from("Post-dose");
    let mut ds = DataSheet::new("data-1", "Assay Measurements", TableArchetype::Column(col)).unwrap();
    let a = [1.0, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 6.0, 8.0];
    let b = [3.0, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 8.0, 9.5];
    for r in 0..a.len() {
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(a[r])).unwrap();
        ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(b[r])).unwrap();
    }
    doc.add_data_sheet(ds).unwrap();
    Session::from_document(doc)
}

#[test]
fn frequency_distribution_and_histogram_end_to_end_engine_workflow() {
    let mut session = make_session_with_column_data();

    // 1. Run standalone Frequency Distribution via stats.run with shorthand & auto-histogram creation.
    let out = session
        .execute(
            "stats.run",
            json!({
                "source_sheet_id": "data-1",
                "sheet_id": "res-freq",
                "kind": "frequency_distribution",
                "bin_width": 2.0,
                "first_bin_center": 1.0,
                "last_bin_center": 9.0,
                "output_mode": "percentages",
                "overlay_gaussian": true
            }),
        )
        .expect("stats.run frequency_distribution");

    let hist_graph_id = out.payload["histogram_graph_sheet_id"].as_str().expect("auto-created histogram graph id").to_owned();
    let res = session.document().result_sheet(&SheetId::new("res-freq")).expect("result sheet present");
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert!(matches!(
        res.analysis.kind,
        AnalysisKind::FrequencyDistribution(ref fd)
            if fd.binning_rule == FrequencyBinningRule::ExplicitWidth { width: 2.0 }
                && fd.output_mode == FrequencyOutputMode::Percentages
                && fd.overlay_gaussian
    ));
    let hist_graph = session.document().graph_sheet(&SheetId::new(&hist_graph_id)).expect("auto histogram graph").clone();
    assert_eq!(hist_graph.spec.kind, GraphKind::Histogram);
    assert!(hist_graph.spec.histogram.show_gaussian_overlay);
    assert_eq!(hist_graph.spec.primary_source_sheet_id.as_str(), "res-freq");

    // 2. Methods report cites binning rules cleanly in PlainText, Markdown, and LaTeX.
    let freq_id = SheetId::new("res-freq");
    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        let report = generate_methods_report(session.document(), &freq_id, fmt).expect("methods report");
        assert!(report.contains("explicit uniform bin width of 2"), "missing bin width narrative in {fmt:?}");
        assert!(report.contains("Gaussian density curve"), "missing Gaussian overlay note in {fmt:?}");
    }

    // 3. Upstream edit reactively recalculates FrequencyDistribution through the DAG.
    let mean_before = session
        .document()
        .result_sheet(&SheetId::new("res-freq"))
        .unwrap()
        .scalars
        .iter()
        .find(|s| s.key == "Mean" && s.data_set_index == Some(0))
        .expect("Mean scalar before edit")
        .value;
    session.execute("table.setCell", json!({ "sheet_id": "data-1", "row": 0, "col": 0, "subcol": 0, "value": 1.5 })).expect("table.setCell");
    let res_after = session.document().result_sheet(&SheetId::new("res-freq")).unwrap();
    assert_eq!(res_after.status, ResultStatus::UpToDate);
    let mean_after = res_after.scalars.iter().find(|s| s.key == "Mean" && s.data_set_index == Some(0)).expect("Mean scalar after edit").value;
    assert!((mean_after - (mean_before + 0.05)).abs() < 1e-12, "expected reactive DAG mean shift +0.05: before={mean_before}, after={mean_after}");
    session.execute("analyze.run", json!({})).expect("analyze.run");
    assert_eq!(session.document().result_sheet(&SheetId::new("res-freq")).unwrap().status, ResultStatus::UpToDate);

    // 4. Update parameters in place via analysis.updateParams (switch to Scott + cumulative_frequency).
    session
        .execute(
            "analysis.updateParams",
            json!({
                "sheet_id": "res-freq",
                "binning_rule": "scott",
                "first_bin_center": null,
                "last_bin_center": null,
                "output_mode": "cumulative_frequency"
            }),
        )
        .expect("analysis.updateParams");
    let updated_report = generate_methods_report(session.document(), &freq_id, ReportFormat::PlainText).expect("updated methods report");
    assert!(updated_report.contains("Scott (1979)"));
    assert!(updated_report.contains("Gaussian cumulative distribution curve"));

    // 5. graph.create targeting a FrequencyDistribution ResultSheet auto-selects GraphKind::Histogram.
    let g_out = session.execute("graph.create", json!({ "data_sheet_id": "res-freq", "sheet_id": "g-auto" })).expect("graph.create auto histogram");
    assert_eq!(g_out.payload["kind"], json!(GraphKind::Histogram));

    // 6. graph.setKind (Change Graph Type) + graph.setSpec (histogram display options) on raw DataSheet.
    session.execute("graph.create", json!({ "data_sheet_id": "data-1", "sheet_id": "g-raw" })).expect("graph.create raw");
    session
        .execute("graph.setKind", json!({ "sheet_id": "g-raw", "kind": "histogram", "option_id": "hist.cumulative" }))
        .expect("graph.setKind hist.cumulative");
    session
        .execute(
            "graph.setSpec",
            json!({
                "sheet_id": "g-raw",
                "histogram": {
                    "display_style": "step_outline",
                    "bin_alignment": "bin_edges",
                    "multi_series_mode": "side_by_side",
                    "show_gaussian_overlay": true,
                    "bar_alpha": 0.65
                }
            }),
        )
        .expect("graph.setSpec histogram");
    let raw_g = session.document().graph_sheet(&SheetId::new("g-raw")).unwrap().clone();
    assert_eq!(raw_g.spec.kind, GraphKind::Histogram);
    assert_eq!(raw_g.spec.histogram.display_style, HistogramDisplayStyle::StepOutline);
    assert_eq!(raw_g.spec.histogram.bin_alignment, HistogramBinAlignment::BinEdges);
    assert_eq!(raw_g.spec.histogram.multi_series_mode, HistogramMultiSeriesMode::SideBySide);
    assert!(raw_g.spec.histogram.cumulative);
    assert!(raw_g.spec.histogram.show_gaussian_overlay);

    // 7. Export SVG, PDF, and PNG of the Histogram graph to confirm full export pipeline compatibility.
    let hist_scene = curvebook_chart::compile_graph(&hist_graph, session.document()).expect("compile histogram scene");
    let svg = render_svg(&hist_scene, &SvgExportOptions::default()).expect("svg export");
    assert!(svg.contains("<svg"));
    let pdf = render_pdf(&hist_scene, &PdfExportOptions::default()).expect("pdf export");
    assert!(pdf.starts_with(b"%PDF"));
    let img = render_scene(&hist_scene, RenderTargetSpec { width_px: 320, height_px: 240, dpi: 150 }).expect("rasterize histogram");
    let png = encode_png(&img).expect("png export");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

    // 8. Archive (.curvebook) save/reload round-trip preserves FrequencyDistributionSpec and HistogramSpec.
    let bytes = save_project_bytes(session.document()).expect("save archive");
    let loaded = open_project_bytes(&bytes).expect("load archive");
    let loaded_g = loaded.graph_sheet(&SheetId::new("g-raw")).expect("reloaded histogram graph");
    assert_eq!(loaded_g.spec.kind, GraphKind::Histogram);
    assert_eq!(loaded_g.spec.histogram, raw_g.spec.histogram);
}

#[test]
fn frequency_distribution_rejects_invalid_parameters() {
    let mut session = make_session_with_column_data();
    let bad_width = session.execute("stats.run", json!({ "source_sheet_id": "data-1", "kind": "frequency_distribution", "bin_width": -1.5 }));
    assert!(bad_width.is_err());

    let bad_bounds = session.execute(
        "stats.run",
        json!({
            "source_sheet_id": "data-1",
            "kind": "frequency_distribution",
            "bin_width": 1.0,
            "first_bin_center": 10.0,
            "last_bin_center": 2.0
        }),
    );
    assert!(bad_bounds.is_err());
}
