//! Integration parity tests for `AxisSpec::additional_ticks` across all 7 scales & broken
//! axis segments, plus Diagnostic plots (`NormalQqPlot`, `Volcano`, `BlandAltman`, `ResidualPlot`)
//! consuming live `ResultSheet` / `ResultTabKind` outputs.

use curvebook_chart::compile_graph;
use curvebook_chart::scales::{AxisOrientation, ResolvedAxis, compute_additional_ticks};
use curvebook_chart::scene::{ChartScene, PathCmd, ScenePrimitive};
use curvebook_doc::{
    AdditionalTick, AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, AxisThreshold, BlandAltmanSpec, Cell, ColumnDesignHint,
    ColumnTable, DataSheet, DescriptiveSpec, DiagnosticPlotKind, GraphKind, GraphSheet, GroupedTable, LogitDomain, MultipleRowTestsSpec,
    MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SizePt,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::execute_analysis as execute_fit_analysis;
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_stats::column::run_descriptive_analysis;
use curvebook_stats::two_group::{run_bland_altman_analysis, run_multiple_row_t_tests_analysis};
use curvebook_text::LocaleNumberPolicy;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type LineSeg = ((f64, f64), (f64, f64), LineDashStyle);
type ScaleCase = (AxisScale, Vec<(f64, f64)>, Vec<AdditionalTick>, Vec<&'static str>, Vec<&'static str>);

fn diag_graph_for_result(id: &str, ds: &DataSheet, res_id: SheetId, kind: DiagnosticPlotKind) -> TestResult<GraphSheet> {
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new(id), ds)?;
    gs.spec.primary_source_sheet_id = res_id;
    gs.spec.kind = GraphKind::Diagnostic(kind);
    gs.spec.x_axis.title = String::from("X");
    gs.spec.y_axis.title = String::from("Y");
    Ok(gs)
}

fn line_primitives(scene: &ChartScene) -> Vec<LineSeg> {
    let mut out = Vec::new();
    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, stroke: Some(st), .. } = prim
            && path.commands.len() == 2
            && let (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) = (path.commands.first(), path.commands.get(1))
        {
            out.push(((a.x, a.y), (b.x, b.y), st.dash));
        }
    }
    out
}

fn plain_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn filled_circle_centers(scene: &ChartScene) -> Vec<(f64, f64)> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } if path.commands.len() == 6 => {
                if let (Some(PathCmd::MoveTo(top)), Some(PathCmd::CubicTo { to: bottom, .. })) = (path.commands.first(), path.commands.get(2)) {
                    Some((top.x, 0.5 * (top.y + bottom.y)))
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn build_minimal_doc_graph() -> TestResult<(ProjectDocument, GraphSheet)> {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for (r, (x, y)) in [(0.1, 10.0), (1.0, 30.0), (100.0, 60.0), (5000.0, 90.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new(SheetId::new("xy-1"), "Dose Response", TableArchetype::Xy(xy))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    let gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds)?;
    Ok((doc, gs))
}

#[test]
fn additional_ticks_map_across_all_scales_broken_segments_and_gridlines() -> TestResult {
    let policy = LocaleNumberPolicy::publication();
    let (doc, mut gs) = build_minimal_doc_graph()?;
    gs.spec.canvas_size_pt = SizePt::new(400.0, 280.0);

    // 1. Broken 3-segment Linear X axis: [0..=10], [50..=100], [500..=1000]
    let mut x_lin_broken = AxisSpec::linear("X Broken");
    x_lin_broken.segments = vec![
        AxisSegment { min: Some(0.0), max: Some(10.0), scale: AxisScale::Linear, length_share: 1.0, ..AxisSegment::default() },
        AxisSegment { min: Some(50.0), max: Some(100.0), scale: AxisScale::Linear, length_share: 1.0, ..AxisSegment::default() },
        AxisSegment { min: Some(500.0), max: Some(1000.0), scale: AxisScale::Linear, length_share: 1.0, ..AxisSegment::default() },
    ];
    x_lin_broken.additional_ticks = vec![
        AdditionalTick { value: -5.0, label: Some(String::from("Before")), draw_gridline: true },
        AdditionalTick { value: 5.0, label: Some(String::from("Cut_{1}")), draw_gridline: false },
        AdditionalTick { value: 25.0, label: Some(String::from("InGap1")), draw_gridline: true },
        AdditionalTick { value: 75.0, label: None, draw_gridline: true },
        AdditionalTick { value: 250.0, label: Some(String::from("InGap2")), draw_gridline: true },
        AdditionalTick { value: 800.0, label: Some(String::from("High")), draw_gridline: false },
        AdditionalTick { value: 1500.0, label: Some(String::from("After")), draw_gridline: true },
        AdditionalTick { value: f64::NAN, label: Some(String::from("NaN")), draw_gridline: true },
        AdditionalTick { value: f64::INFINITY, label: Some(String::from("Inf")), draw_gridline: true },
    ];
    gs.spec.x_axis = x_lin_broken.clone();

    // Also make Y broken (2 segments) so full plot-span gridlines respect perpendicular break gaps.
    gs.spec.y_axis = AxisSpec::linear("Y Broken");
    gs.spec.y_axis.segments = vec![
        AxisSegment { min: Some(0.0), max: Some(40.0), scale: AxisScale::Linear, length_share: 1.0, ..AxisSegment::default() },
        AxisSegment { min: Some(60.0), max: Some(100.0), scale: AxisScale::Linear, length_share: 1.0, ..AxisSegment::default() },
    ];

    let s_chrome = compile_graph(&gs, &doc)?;
    let mut probe_warnings = Vec::new();
    let x_axis = ResolvedAxis::resolve(&gs.spec.x_axis, AxisOrientation::Horizontal, s_chrome.plot_rect_pt, None, &mut probe_warnings);
    let resolved_x = compute_additional_ticks(&x_axis, &gs.spec.x_axis.additional_ticks, gs.spec.x_axis.tick_format, &policy);
    assert_eq!(resolved_x.len(), 3, "only 5, 75, 800 lie inside active segments; got {resolved_x:?}");
    assert_eq!(resolved_x[0].segment_index, 0);
    assert_eq!(resolved_x[0].label.as_ref().map(|r| r.plain_text()), Some(String::from("Cut1")));
    assert_eq!(resolved_x[1].segment_index, 1);
    assert_eq!(resolved_x[1].label.as_ref().map(|r| r.plain_text()), Some(String::from("75")));
    assert_eq!(resolved_x[2].segment_index, 2);
    assert_eq!(resolved_x[2].label.as_ref().map(|r| r.plain_text()), Some(String::from("High")));
    assert!(resolved_x[0].coord_pt < resolved_x[1].coord_pt && resolved_x[1].coord_pt < resolved_x[2].coord_pt);

    let texts = plain_texts(&s_chrome);
    assert!(texts.contains(&String::from("Cut1")));
    assert!(texts.contains(&String::from("75")));
    assert!(texts.contains(&String::from("High")));
    for forbidden in ["Before", "InGap1", "InGap2", "After", "NaN", "Inf"] {
        assert!(!texts.contains(&String::from(forbidden)), "out-of-domain tick '{forbidden}' must be omitted");
    }
    let grid_x_75 = resolved_x[1].coord_pt;
    let lines = line_primitives(&s_chrome);
    let vertical_grid_at_75: Vec<_> = lines
        .iter()
        .filter(|(a, b, dash)| *dash == LineDashStyle::Dotted && (a.0 - grid_x_75).abs() < 1e-3 && (b.0 - grid_x_75).abs() < 1e-3)
        .collect();
    assert_eq!(vertical_grid_at_75.len(), gs.spec.y_axis.segments.len(), "draw_gridline=true emits 1 dotted line per active perpendicular Y segment");

    // 2. Broken Log10, Log2, Ln, Logit(Proportion & Percent), Probit, Reciprocal scales
    let scale_cases: Vec<ScaleCase> = vec![
        (
            AxisScale::Log10,
            vec![(0.1, 10.0), (100.0, 10_000.0)],
            vec![
                AdditionalTick { value: 0.0, label: Some(String::from("ZeroLog")), draw_gridline: true },
                AdditionalTick { value: -2.0, label: Some(String::from("NegLog")), draw_gridline: true },
                AdditionalTick { value: 50.0, label: Some(String::from("GapLog")), draw_gridline: true },
                AdditionalTick { value: 3.0, label: None, draw_gridline: true },
                AdditionalTick { value: 500.0, label: Some(String::from("IC90")), draw_gridline: false },
            ],
            vec!["3", "IC90"],
            vec!["ZeroLog", "NegLog", "GapLog"],
        ),
        (
            AxisScale::Log2,
            vec![(1.0, 64.0)],
            vec![
                AdditionalTick { value: 0.0, label: Some(String::from("BadLog2")), draw_gridline: true },
                AdditionalTick { value: 6.0, label: None, draw_gridline: true },
            ],
            vec!["6"],
            vec!["BadLog2"],
        ),
        (
            AxisScale::Ln,
            vec![(1.0, 50.0)],
            vec![
                AdditionalTick { value: -1.0, label: Some(String::from("BadLn")), draw_gridline: false },
                AdditionalTick { value: 12.0, label: Some(String::from("eFold")), draw_gridline: true },
            ],
            vec!["eFold"],
            vec!["BadLn"],
        ),
        (
            AxisScale::Logit(LogitDomain::Proportion),
            vec![(0.05, 0.95)],
            vec![
                AdditionalTick { value: 0.0, label: Some(String::from("ZeroProp")), draw_gridline: false },
                AdditionalTick { value: 1.0, label: Some(String::from("OneProp")), draw_gridline: false },
                AdditionalTick { value: 0.33, label: None, draw_gridline: true },
            ],
            vec!["0.33"],
            vec!["ZeroProp", "OneProp"],
        ),
        (
            AxisScale::Logit(LogitDomain::Percent),
            vec![(5.0, 95.0)],
            vec![
                AdditionalTick { value: 100.0, label: Some(String::from("Pct100")), draw_gridline: false },
                AdditionalTick { value: 37.5, label: None, draw_gridline: true },
            ],
            vec!["37.5"],
            vec!["Pct100"],
        ),
        (
            AxisScale::Probit,
            vec![(5.0, 95.0)],
            vec![
                AdditionalTick { value: 0.0, label: Some(String::from("Probit0")), draw_gridline: false },
                AdditionalTick { value: 50.0, label: Some(String::from("ED50")), draw_gridline: true },
            ],
            vec!["ED50"],
            vec!["Probit0"],
        ),
        (
            AxisScale::Reciprocal,
            vec![(0.5, 10.0)],
            vec![
                AdditionalTick { value: 0.0, label: Some(String::from("RecipZero")), draw_gridline: true },
                AdditionalTick { value: 2.5, label: None, draw_gridline: true },
            ],
            vec!["2.5"],
            vec!["RecipZero"],
        ),
    ];

    for (scale, segs, add_ticks, expected, omitted) in scale_cases {
        let mut case_gs = gs.clone();
        case_gs.spec.canvas_size_pt = SizePt::new(360.0, 260.0);
        case_gs.spec.x_axis.segments = segs
            .into_iter()
            .map(|(lo, hi)| AxisSegment { min: Some(lo), max: Some(hi), scale, length_share: 1.0, ..AxisSegment::default() })
            .collect();
        case_gs.spec.x_axis.additional_ticks = add_ticks;
        case_gs.spec.y_axis = AxisSpec::linear("Y");
        case_gs.spec.y_axis.segments[0].min = Some(0.0);
        case_gs.spec.y_axis.segments[0].max = Some(100.0);

        let sc = compile_graph(&case_gs, &doc)?;
        let sc_texts = plain_texts(&sc);
        for lbl in expected {
            assert!(sc_texts.contains(&String::from(lbl)), "expected additional tick '{lbl}' on scale {scale:?}; got {sc_texts:?}");
        }
        for bad in omitted {
            assert!(!sc_texts.contains(&String::from(bad)), "unexpected out-of-domain tick '{bad}' on scale {scale:?}");
        }
        let has_grid = sc.primitives.iter().any(
            |p| matches!(p, ScenePrimitive::Path { path, stroke: Some(st), .. } if st.dash == LineDashStyle::Dotted && path.commands.len() == 2),
        );
        assert!(has_grid, "expected draw_gridline=true stroke for scale {scale:?}");
    }

    // 3. Full `compile_graph` end-to-end on XY graph with broken Log10 X axis + custom additional ticks.
    let mut xy_gs = gs;
    xy_gs.spec.x_axis.title = String::from("Concentration");
    xy_gs.spec.x_axis.segments = vec![
        AxisSegment { min: Some(0.1), max: Some(10.0), scale: AxisScale::Log10, length_share: 1.0, ..AxisSegment::default() },
        AxisSegment { min: Some(100.0), max: Some(10_000.0), scale: AxisScale::Log10, length_share: 1.0, ..AxisSegment::default() },
    ];
    xy_gs.spec.x_axis.additional_ticks = vec![
        AdditionalTick { value: 2.5, label: Some(String::from("EC_{50}")), draw_gridline: true },
        AdditionalTick { value: 45.0, label: Some(String::from("InLogGap")), draw_gridline: true },
        AdditionalTick { value: 500.0, label: None, draw_gridline: false },
    ];
    let compiled_xy = compile_graph(&xy_gs, &doc)?;
    let xy_texts = plain_texts(&compiled_xy);
    assert!(xy_texts.contains(&String::from("EC50")));
    assert!(xy_texts.contains(&String::from("500")));
    assert!(!xy_texts.contains(&String::from("InLogGap")));
    let r_xy = compiled_xy.plot_rect_pt;
    let has_full_v_grid = line_primitives(&compiled_xy)
        .iter()
        .any(|(a, b, dash)| *dash == LineDashStyle::Dotted && (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() > 0.25 * r_xy.height);
    assert!(has_full_v_grid, "compile_graph should emit full-span additional tick gridline on broken Log10 XY");
    Ok(())
}

#[test]
fn normal_qq_plot_consumes_normal_qq_tab_and_residuals_tab() -> TestResult {
    // 1. Live DescriptiveStats ResultSheet (`ResultTabKind::NormalQq`) with two datasets.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 8)?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Treated");
    for (r, (v0, v1)) in
        [(10.2, 50.5), (11.1, 52.1), (9.8, 49.0), (10.6, 53.4), (10.9, 51.8), (11.4, 54.0), (10.0, 50.1), (10.4, 52.7)].into_iter().enumerate()
    {
        col.data_sets[0].cells[r][0] = Cell::number(v0);
        col.data_sets[1].cells[r][0] = Cell::number(v1);
    }
    let ds = DataSheet::new(SheetId::new("col-qq"), "Groups", TableArchetype::Column(col))?;
    let desc_spec = DescriptiveSpec::default();
    let mut an_spec = AnalysisSpec::new(SheetId::new("desc-res"), ds.id.clone(), AnalysisKind::DescriptiveStats(desc_spec.clone()))?;
    an_spec.selected_data_sets = vec![0, 1];
    let desc_res = run_descriptive_analysis(&ds, &an_spec, &desc_spec)?;
    assert!(desc_res.tabs.iter().any(|t| t.kind == ResultTabKind::NormalQq));

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(desc_res.clone())?;

    // Graph pointing directly at the ResultSheet, selecting Treated (dataset index 1).
    let mut qq_direct = diag_graph_for_result("g-qq-desc", &ds, desc_res.id, DiagnosticPlotKind::NormalQqPlot)?;
    qq_direct.spec.series = vec![qq_direct.spec.series[1].clone()];
    let scene_treated = compile_graph(&qq_direct, &doc)?;
    assert!(scene_treated.warnings.is_empty(), "warnings={:?}", scene_treated.warnings);
    assert_eq!(filled_circle_centers(&scene_treated).len(), 8);
    assert!(line_primitives(&scene_treated).iter().any(|(_, _, d)| *d == LineDashStyle::Dashed), "expected dashed quartile Q-Q reference line");
    let y_texts_treated = plain_texts(&scene_treated);
    assert!(
        y_texts_treated.iter().any(|t| t == "50" || t == "52" || t == "54"),
        "Dataset 1 Q-Q should plot Treated values (~50..54), got {y_texts_treated:?}"
    );

    // Selecting Dataset 0 should plot Vehicle (~10..11), NOT Treated (~50).
    let mut qq_vehicle = diag_graph_for_result("g-qq-veh", &ds, qq_direct.spec.primary_source_sheet_id.clone(), DiagnosticPlotKind::NormalQqPlot)?;
    qq_vehicle.spec.series.truncate(1);
    let scene_vehicle = compile_graph(&qq_vehicle, &doc)?;
    let y_texts_vehicle = plain_texts(&scene_vehicle);
    assert!(
        y_texts_vehicle.iter().any(|t| t == "10" || t == "11" || t == "9.5"),
        "Dataset 0 Q-Q should plot Vehicle values (~10..11), got {y_texts_vehicle:?}"
    );

    // 2. Multi-column Residuals tab (X, Y, Predicted Y, Residual) -> Q-Q plots Residuals (~0), NOT Y (~1000..5000).
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6)?;
    for r in 0..6 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(1000.0 * ((r + 1) as f64));
    }
    let fit_ds = DataSheet::new(SheetId::new("fit-ds"), "Fit", TableArchetype::Xy(xy))?;
    let nl_spec = AnalysisSpec::new(
        SheetId::new("fit-res"),
        fit_ds.id.clone(),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("four_pl_log_agonist")),
    )?;
    let mut fit_res = ResultSheet::new(nl_spec)?;
    fit_res.tabs.push(ResultTab {
        id: String::from("nl-residuals"),
        title: String::from("Residuals"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![
            String::from("X"),
            String::from("Y"),
            String::from("Predicted Y"),
            String::from("Residual"),
            String::from("Normal Q-Q Expected"),
        ],
        row_labels: (1..=6).map(|i| format!("Row {i}")).collect(),
        rows: vec![
            vec![Cell::number(1.0), Cell::number(1000.0), Cell::number(1000.2), Cell::number(-0.2), Cell::number(-1.38)],
            vec![Cell::number(2.0), Cell::number(2000.0), Cell::number(1999.6), Cell::number(0.4), Cell::number(-0.64)],
            vec![Cell::number(3.0), Cell::number(3000.0), Cell::number(3000.3), Cell::number(-0.3), Cell::number(-0.21)],
            vec![Cell::number(4.0), Cell::number(4000.0), Cell::number(3999.5), Cell::number(0.5), Cell::number(0.21)],
            vec![Cell::number(5.0), Cell::number(5000.0), Cell::number(5000.1), Cell::number(-0.1), Cell::number(0.64)],
            vec![Cell::number(6.0), Cell::number(6000.0), Cell::number(6000.25), Cell::number(-0.25), Cell::number(1.38)],
        ]
        .into_iter()
        .map(|row| row.into_iter().map(|c| ResultCell::Number { value: c.active_number().unwrap_or(0.0), prefix: None, units: None }).collect())
        .collect(),
        footnotes: Vec::new(),
    });
    let mut fit_doc = ProjectDocument::default();
    let mut qq_resid = diag_graph_for_result("g-qq-fit", &fit_ds, fit_res.id.clone(), DiagnosticPlotKind::NormalQqPlot)?;
    fit_doc.add_data_sheet(fit_ds)?;
    fit_doc.add_result_sheet(fit_res)?;
    qq_resid.spec.y_axis.title = String::from("Residual");
    let qq_resid_scene = compile_graph(&qq_resid, &fit_doc)?;
    assert_eq!(filled_circle_centers(&qq_resid_scene).len(), 6);
    let resid_texts = plain_texts(&qq_resid_scene);
    assert!(!resid_texts.iter().any(|t| t == "2000" || t == "4000" || t == "6000"), "must plot residual column, not Y: {resid_texts:?}");
    Ok(())
}

#[test]
fn volcano_plot_consumes_multiple_row_t_tests_result_sheet() -> TestResult {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 5)?;
    grp.data_sets[0].title = String::from("Treated");
    grp.data_sets[1].title = String::from("Control");
    let genes = ["TP53", "ACTB", "EGFR", "BRCA1", "GAPDH"];
    let pairs = [
        ([12.0, 12.2, 11.9], [8.5, 8.6, 8.4]),
        ([10.1, 10.0, 9.9], [10.0, 10.1, 9.8]),
        ([5.1, 5.2, 5.0], [9.5, 9.4, 9.6]),
        ([14.2, 14.1, 14.3], [11.0, 11.1, 10.9]),
        ([7.9, 8.1, 8.0], [7.8, 8.0, 8.1]),
    ];
    for (r, (gene, (ga, gb))) in genes.into_iter().zip(pairs).enumerate() {
        grp.row_titles[r] = String::from(gene);
        for rep in 0..3 {
            grp.data_sets[0].cells[r][rep] = Cell::number(ga[rep]);
            grp.data_sets[1].cells[r][rep] = Cell::number(gb[rep]);
        }
    }
    let ds = DataSheet::new(SheetId::new("mr-ds"), "Transcriptomics", TableArchetype::Grouped(grp))?;
    let mr_spec = MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: false,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
    };
    let an_spec = AnalysisSpec::new(SheetId::new("mr-res"), ds.id.clone(), AnalysisKind::MultipleRowTTests(mr_spec))?;
    let mut mr_res = run_multiple_row_t_tests_analysis(&ds, &an_spec, &mr_spec)?;
    assert_eq!(mr_res.pairwise_comparisons.len(), 5);

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(mr_res.clone())?;

    // 1. Primary source = ResultSheet (uses pairwise_comparisons directly)
    let mut gs_res = diag_graph_for_result("g-vol-res", &ds, mr_res.id.clone(), DiagnosticPlotKind::Volcano)?;
    gs_res.spec.x_axis.thresholds = vec![
        AxisThreshold { value: -1.0, shade_to: None, color: Rgba8::BLACK, dash_style: LineDashStyle::Dashed, stroke_width_pt: 0.8, label: None },
        AxisThreshold { value: 1.0, shade_to: None, color: Rgba8::BLACK, dash_style: LineDashStyle::Dashed, stroke_width_pt: 0.8, label: None },
    ];
    let sc_res = compile_graph(&gs_res, &doc)?;
    assert!(sc_res.warnings.is_empty());
    assert_eq!(filled_circle_centers(&sc_res).len(), 5);
    let texts = plain_texts(&sc_res);
    assert!(texts.contains(&String::from("TP53")), "significant row label missing: {texts:?}");
    assert!(texts.contains(&String::from("EGFR")), "significant row label missing: {texts:?}");
    assert!(line_primitives(&sc_res).iter().any(|(_, _, d)| *d == LineDashStyle::Dashed));

    // 2. Fallback to ResultTabKind::MultipleComparisons rows when pairwise_comparisons is empty.
    mr_res.pairwise_comparisons.clear();
    let mut doc_tab = ProjectDocument::default();
    doc_tab.add_data_sheet(ds)?;
    doc_tab.add_result_sheet(mr_res)?;
    gs_res.spec.primary_source_sheet_id = SheetId::new("mr-res");
    let sc_tab = compile_graph(&gs_res, &doc_tab)?;
    assert_eq!(filled_circle_centers(&sc_tab).len(), 5);
    assert!(plain_texts(&sc_tab).contains(&String::from("BRCA1")));
    Ok(())
}

#[test]
fn bland_altman_plot_consumes_bland_altman_result_sheet_absolute_and_percent() -> TestResult {
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 6)?;
    col.data_sets[0].title = String::from("Ref");
    col.data_sets[1].title = String::from("Device A");
    col.data_sets[2].title = String::from("Device B");
    for (r, (v0, v1, v2)) in [(10.0, 10.4, 9.8), (20.0, 20.6, 19.5), (30.0, 30.2, 29.7), (40.0, 41.0, 39.2), (50.0, 50.8, 49.4), (60.0, 61.1, 59.1)]
        .into_iter()
        .enumerate()
    {
        col.data_sets[0].cells[r][0] = Cell::number(v0);
        col.data_sets[1].cells[r][0] = Cell::number(v1);
        col.data_sets[2].cells[r][0] = Cell::number(v2);
    }
    let ds = DataSheet::new(SheetId::new("ba-ds"), "Methods", TableArchetype::Column(col))?;

    // Absolute difference on cols (0, 1)
    let ba_abs = BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, ..BlandAltmanSpec::default() };
    let an_abs = AnalysisSpec::new(SheetId::new("ba-abs-res"), ds.id.clone(), AnalysisKind::BlandAltman(ba_abs))?;
    let res_abs = run_bland_altman_analysis(&ds, &an_abs, &ba_abs)?;

    // Percent difference on cols (1, 2)
    let ba_pct = BlandAltmanSpec { column_a: 1, column_b: 2, percent_difference: true, ..BlandAltmanSpec::default() };
    let an_pct = AnalysisSpec::new(SheetId::new("ba-pct-res"), ds.id.clone(), AnalysisKind::BlandAltman(ba_pct))?;
    let res_pct = run_bland_altman_analysis(&ds, &an_pct, &ba_pct)?;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res_abs.clone())?;
    doc.add_result_sheet(res_pct.clone())?;

    let gs_abs = diag_graph_for_result("g-ba-abs", &ds, res_abs.id, DiagnosticPlotKind::BlandAltman)?;
    let sc_abs = compile_graph(&gs_abs, &doc)?;
    assert_eq!(filled_circle_centers(&sc_abs).len(), 6);
    let abs_texts = plain_texts(&sc_abs);
    assert!(abs_texts.iter().any(|t| t.starts_with("Bias:")));
    assert!(abs_texts.iter().any(|t| t.starts_with("+1.96 SD:")));
    assert!(abs_texts.iter().any(|t| t.starts_with("-1.96 SD:")));
    let dashed_loa_count = line_primitives(&sc_abs).iter().filter(|(_, _, d)| *d == LineDashStyle::Dashed).count();
    assert!(dashed_loa_count >= 2, "expected +/- 1.96 SD dashed LoA threshold lines");

    let gs_pct = diag_graph_for_result("g-ba-pct", &ds, res_pct.id, DiagnosticPlotKind::BlandAltman)?;
    let sc_pct = compile_graph(&gs_pct, &doc)?;
    assert_eq!(filled_circle_centers(&sc_pct).len(), 6);
    let pct_texts = plain_texts(&sc_pct);
    assert!(pct_texts.contains(&String::from("Percent Difference (%)")));
    assert!(pct_texts.iter().any(|t| t.starts_with("Bias:")));
    Ok(())
}

#[test]
fn residual_plot_consumes_nonlinear_regression_residuals_tab() -> TestResult {
    // 1. Live `NonlinearRegression` execution producing ResultSheet (+ structured multi-column Residuals tab).
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 7)?;
    let log_doses = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0];
    let obs_y = [10.2, 15.1, 34.8, 55.4, 78.9, 93.2, 99.6];
    for (r, (x, y)) in log_doses.into_iter().zip(obs_y).enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new(SheetId::new("nl-ds"), "Agonist Curve", TableArchetype::Xy(xy))?;
    let mut nl_spec = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_spec.emit_residuals_tab = true;
    let an_spec = AnalysisSpec::new(SheetId::new("nl-res"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl_spec))?;
    let fit_res = execute_fit_analysis(&an_spec, &ds, &Default::default())?;
    assert!(
        fit_res.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals),
        "NonlinearRegression with emit_residuals_tab=true must emit Residuals tab"
    );

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(fit_res.clone())?;

    // Check both linking via DataSheet and linking directly to the NonlinearRegression ResultSheet.
    for src in [ds.id.clone(), fit_res.id.clone()] {
        let gs = diag_graph_for_result("g-nl-res", &ds, src, DiagnosticPlotKind::ResidualPlot)?;
        let sc = compile_graph(&gs, &doc)?;
        assert!(sc.warnings.is_empty(), "warnings={:?}", sc.warnings);
        assert_eq!(filled_circle_centers(&sc).len(), 7);
        assert!(line_primitives(&sc).iter().any(|(_, _, d)| *d == LineDashStyle::Dashed), "expected zero baseline");
        let y_tick_texts: Vec<String> = sc
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) if t.anchor_pt.x < sc.plot_rect_pt.x && t.rotation_deg.abs() < 1.0 => Some(t.rich.plain_text()),
                _ => None,
            })
            .collect();
        assert!(y_tick_texts.contains(&String::from("0")), "Y-axis should include 0 residual tick: {y_tick_texts:?}");
        assert!(
            !y_tick_texts.contains(&String::from("90")) && !y_tick_texts.contains(&String::from("100")),
            "Y-axis should show small residuals near 0, not raw Y: {y_tick_texts:?}"
        );
    }
    // Also verify NormalQqPlot seamlessly consumes NonlinearRegression's ResultTabKind::Residuals tab.
    let qq_nl = diag_graph_for_result("g-nl-qq", &ds, fit_res.id, DiagnosticPlotKind::NormalQqPlot)?;
    let qq_sc = compile_graph(&qq_nl, &doc)?;
    assert_eq!(filled_circle_centers(&qq_sc).len(), 7);
    Ok(())
}
