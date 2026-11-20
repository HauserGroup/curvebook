use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AdditionalTick, AnalysisKind, AnalysisSpec, AxisThreshold, BracketLabelMode, BracketStyle, CellCoord, ColorChannelMapping, ColumnDesignHint,
    ColumnTable, ContingencySpec, DataSheet, DescriptiveSpec, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FactorialAnovaSpec, FitMethod,
    FloatingNote, GraphAnnotation, GraphSheet, GraphicalDataObject, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LinkSource, MathTransformPreset, MultipleRowTestsSpec, MultiplicitySpec,
    MultivariateGraphEncoding, MultivariatePlotMode, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OutlierSpec,
    PairwiseBracketSpec, ParameterRule, ParameterScope, PlacementMode, PointStyleOverride, ProjectDocument, RectPt, ResultSheet, SeriesSpec, SheetId,
    SimulationNoise, SizeChannelMapping, SizePt, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, TwoGroupMethod,
    TwoGroupSpec, WeightingScheme, XyTable,
};
use curvebook_format::{
    FormatError, ImportBaselineAdjustment, apply_csv_import_link, open_project_bytes, open_project_unpacked_dir, save_project_bytes,
    save_project_unpacked_dir, save_template_bytes, validate_project_document,
};
use curvebook_palette::{ColorRamp, ColorStop, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, FormatError> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-s4b-fmt-{label}-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn build_rich_valid_project() -> Result<ProjectDocument, FormatError> {
    let mut doc = ProjectDocument::new("Rich S06-06 Assay")?;

    let info = InfoSheet::new_preseeded("info-1", "Info")?;
    doc.add_info_sheet(info)?;

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    grp.row_numeric_spacing = Some(vec![Some(1.0), Some(2.0)]);
    let mut ds = DataSheet::new("data-1", "Grouped", TableArchetype::Grouped(grp))?;
    ds.floating_notes.push(FloatingNote::new("dn-1", RectPt::new(5.0, 5.0, 60.0, 30.0), "Note"));
    doc.add_data_sheet(ds.clone())?;

    let mut fit = NonlinearFitSpec::new("four_pl");
    fit.parameter_rules.push(ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::Constant(100.0),
        initial_override: Some(95.0),
        lower_bound: Some(0.0),
        upper_bound: Some(200.0),
    });
    let spec = AnalysisSpec::new("res-1", "data-1", AnalysisKind::NonlinearRegression(fit))?;
    let mut res = ResultSheet::new(spec)?;
    res.analyzable_data = Some(ds.clone());
    doc.add_result_sheet(res)?;

    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &ds)?;
    gsheet.spec.canvas_size_pt = SizePt::new(320.0, 240.0);
    gsheet.spec.x_axis.thresholds.push(AxisThreshold {
        value: 5.0,
        shade_to: Some(10.0),
        color: Rgba8::BLACK,
        dash_style: LineDashStyle::Solid,
        stroke_width_pt: 1.0,
        label: None,
    });
    gsheet.spec.x_axis.additional_ticks.push(AdditionalTick { value: 2.5, label: None, draw_gridline: false });
    gsheet.spec.series.push(SeriesSpec {
        source_sheet_id: SheetId::new("data-1"),
        data_set_index: 0,
        legend_label: None,
        bind_to_right_y2: false,
        gdo: GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle {
                shape: MarkerShape::Circle,
                size_pt: 5.0,
                stroke_width_pt: 1.0,
                fill_color: Rgba8::BLACK,
                stroke_color: Rgba8::BLACK,
                fill_mode: MarkerFillMode::Filled,
            }),
            line: Some(curvebook_doc::ConnectingLineStyle {
                kind: curvebook_doc::LineConnectionKind::PointToPoint,
                dash_style: LineDashStyle::Solid,
                stroke_width_pt: 1.5,
                color: Rgba8::BLACK,
            }),
            area_fill: Some(curvebook_doc::AreaFillSpec { baseline_y: 0.0, fill_color: Rgba8::BLACK }),
            replicate_lines: curvebook_doc::ReplicateLineMode::default(),
        },
        y_error_bars: Some(ErrorBarSpec {
            metric: ErrorBarMetric::Sd,
            direction: ErrorBarDirection::Both,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.0,
            cap_width_pt: 4.0,
            color: None,
        }),
        x_error_bars: None,
        regression_overlay: None,
        offset: curvebook_doc::SeriesOffsetSpec::default(),
        bar_width_override_pt: None,
        show_raw_data: true,
    });
    gsheet.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-1"),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 9.0,
        bracket_step_pt: 12.0,
    });
    gsheet.spec.annotations.push(GraphAnnotation::LineOrArrow {
        start_pt: (1.0, 2.0),
        end_pt: (3.0, 4.0),
        arrowhead: true,
        dash: LineDashStyle::Solid,
        color: Rgba8::BLACK,
    });
    gsheet.spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 0,
        size_pt: Some(6.0),
        border_width_pt: Some(1.0),
        ..PointStyleOverride::default()
    });
    gsheet.spec.legend.position_pt = Some((20.0, 20.0));
    gsheet.floating_notes.push(FloatingNote::new("gn-1", RectPt::new(10.0, 10.0, 50.0, 25.0), "Graph Note"));
    doc.add_graph_sheet(gsheet)?;

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("item-g"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 90.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: String::from("item-t"),
        markdown: String::from("Panel A"),
        bounds_pt: RectPt::new(10.0, 110.0, 80.0, 20.0),
        font_size_pt: 10.0,
    });
    let mut lsheet = LayoutSheet::new("layout-1", "Fig 1", lspec)?;
    lsheet.floating_notes.push(FloatingNote::new("ln-1", RectPt::new(2.0, 2.0, 30.0, 15.0), "Layout Note"));
    doc.add_layout_sheet(lsheet)?;

    Ok(doc)
}

fn assert_save_paths_reject_nonfinite(label: &str, doc: &ProjectDocument) -> Result<(), FormatError> {
    assert!(
        matches!(validate_project_document(doc), Err(FormatError::NonFiniteFloat { .. })),
        "{label}: validate_project_document must reject with NonFiniteFloat"
    );
    assert!(
        matches!(save_project_bytes(doc), Err(FormatError::NonFiniteFloat { .. })),
        "{label}: save_project_bytes must refuse to serialize NonFiniteFloat"
    );
    assert!(
        matches!(save_template_bytes(doc), Err(FormatError::NonFiniteFloat { .. })),
        "{label}: save_template_bytes must refuse to serialize NonFiniteFloat"
    );
    let tmp = TempDirGuard::new("reject")?;
    assert!(
        matches!(save_project_unpacked_dir(doc, tmp.path()), Err(FormatError::NonFiniteFloat { .. })),
        "{label}: save_project_unpacked_dir must refuse to serialize NonFiniteFloat"
    );
    Ok(())
}

#[test]
fn r1_s06_06_valid_document_round_trips_archive_and_unpacked_dir() -> Result<(), FormatError> {
    let doc = build_rich_valid_project()?;
    let bytes = save_project_bytes(&doc)?;
    let decoded = open_project_bytes(&bytes)?;
    assert_eq!(decoded, doc);

    let tmp = TempDirGuard::new("roundtrip")?;
    save_project_unpacked_dir(&doc, tmp.path())?;
    let reopened = open_project_unpacked_dir(tmp.path())?;
    assert_eq!(reopened, doc);
    Ok(())
}

#[test]
fn r1_s06_06_nonfinite_floats_rejected_across_graphs_layouts_notes_and_analyses() -> Result<(), FormatError> {
    let base = build_rich_valid_project()?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.y_axis.stroke_width_pt = f64::NAN;
    assert_save_paths_reject_nonfinite("graph.y_axis.stroke_width_pt", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.x_axis.segments[0].max = Some(f64::INFINITY);
    assert_save_paths_reject_nonfinite("graph.x_axis.segments[0].max", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.x_axis.thresholds[0].shade_to = Some(f64::NEG_INFINITY);
    assert_save_paths_reject_nonfinite("graph.x_axis.thresholds[0].shade_to", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.x_axis.additional_ticks[0].value = f64::NAN;
    assert_save_paths_reject_nonfinite("graph.x_axis.additional_ticks[0].value", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.canvas_size_pt.height = f64::INFINITY;
    assert_save_paths_reject_nonfinite("graph.canvas_size_pt.height", &d)?;

    let mut d = base.clone();
    if let Some(series) = d.graph_sheets[0].spec.series.last_mut()
        && let GraphicalDataObject::ScatterAndLine { area_fill: Some(ref mut af), .. } = series.gdo
    {
        af.baseline_y = f64::NAN;
    }
    assert_save_paths_reject_nonfinite("graph.series.area_fill.baseline_y", &d)?;

    let mut d = base.clone();
    if let Some(series) = d.graph_sheets[0].spec.series.last_mut()
        && let Some(ref mut eb) = series.y_error_bars
    {
        eb.cap_width_pt = f64::INFINITY;
    }
    assert_save_paths_reject_nonfinite("graph.series.y_error_bars.cap_width_pt", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Continuous {
            column: 2,
            ramp: ColorRamp::Custom { stops: vec![ColorStop { position: f64::NAN, color: Rgba8::BLACK }] },
            domain_clamp: Some((0.0, f64::INFINITY)),
        },
        size: SizeChannelMapping::Variable { column: 3, min_radius_pt: 2.0, max_radius_pt: 8.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    assert_save_paths_reject_nonfinite("graph.multivariate.domain_clamp_or_ramp", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.comparisons[0].bracket_step_pt = f64::NAN;
    assert_save_paths_reject_nonfinite("graph.comparisons.bracket_step_pt", &d)?;

    let mut d = base.clone();
    if let GraphAnnotation::LineOrArrow { ref mut end_pt, .. } = d.graph_sheets[0].spec.annotations[0] {
        end_pt.0 = f64::INFINITY;
    }
    assert_save_paths_reject_nonfinite("graph.annotations.end_pt", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.point_overrides[0].size_pt = Some(f64::NAN);
    assert_save_paths_reject_nonfinite("graph.point_overrides.size_pt", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].spec.legend.position_pt = Some((f64::NEG_INFINITY, 10.0));
    assert_save_paths_reject_nonfinite("graph.legend.position_pt", &d)?;

    let mut d = base.clone();
    d.graph_sheets[0].floating_notes[0].bounds_pt.x = f64::NAN;
    assert_save_paths_reject_nonfinite("graph.floating_notes", &d)?;

    let mut d = base.clone();
    d.layout_sheets[0].spec.width = f64::NAN;
    assert_save_paths_reject_nonfinite("layout.spec.width", &d)?;

    let mut d = base.clone();
    if let LayoutItem::LiveGraph { ref mut scale_percent, .. } = d.layout_sheets[0].spec.items[0] {
        *scale_percent = f64::INFINITY;
    }
    assert_save_paths_reject_nonfinite("layout.items.live_graph.scale_percent", &d)?;

    let mut d = base.clone();
    if let LayoutItem::TextCallout { ref mut font_size_pt, .. } = d.layout_sheets[0].spec.items[1] {
        *font_size_pt = f64::NEG_INFINITY;
    }
    assert_save_paths_reject_nonfinite("layout.items.text_callout.font_size_pt", &d)?;

    let mut d = base.clone();
    d.layout_sheets[0].floating_notes[0].bounds_pt.height = f64::NAN;
    assert_save_paths_reject_nonfinite("layout.floating_notes", &d)?;

    let mut d = base.clone();
    if let TableArchetype::Grouped(ref mut g) = d.data_sheets[0].archetype {
        g.row_numeric_spacing = Some(vec![Some(1.0), Some(f64::NAN)]);
    }
    assert_save_paths_reject_nonfinite("data.grouped.row_numeric_spacing", &d)?;

    let mut d = base.clone();
    d.data_sheets[0].floating_notes[0].bounds_pt.width = f64::INFINITY;
    assert_save_paths_reject_nonfinite("data.floating_notes", &d)?;

    let mut d = base.clone();
    d.info_sheets[0].floating_notes.push(FloatingNote::new("in", RectPt { x: f64::NAN, y: 0.0, width: 10.0, height: 10.0 }, "bad"));
    assert_save_paths_reject_nonfinite("info.floating_notes", &d)?;

    let mut d = base.clone();
    d.result_sheets[0].floating_notes.push(FloatingNote::new("rn", RectPt { x: 0.0, y: 0.0, width: f64::NAN, height: 10.0 }, "bad"));
    assert_save_paths_reject_nonfinite("result.floating_notes", &d)?;

    let mut d = base.clone();
    if let Some(ref mut arch_ds) = d.result_sheets[0].analyzable_data
        && let TableArchetype::Grouped(ref mut g) = arch_ds.archetype
    {
        g.row_numeric_spacing = Some(vec![Some(f64::INFINITY), Some(2.0)]);
    }
    assert_save_paths_reject_nonfinite("result.analyzable_data.row_numeric_spacing", &d)?;

    let bad_kinds: Vec<(&str, AnalysisKind)> = vec![
        (
            "transform.linear_scale",
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: Some(MathTransformPreset::LinearScale { slope: f64::NAN, intercept: 0.0 }),
                y_transform: None,
                error_propagation: Default::default(),
            }),
        ),
        (
            "transform.molar",
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: f64::INFINITY }),
                error_propagation: Default::default(),
            }),
        ),
        (
            "transform.normalize",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(f64::NEG_INFINITY),
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: false,
            }),
        ),
        (
            "transform.simulate.param",
            AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
                seed: 1,
                row_count: 5,
                replicates: 1,
                x_min: 0.0,
                x_max: 10.0,
                equation_id: String::from("line"),
                parameters: vec![(String::from("m"), f64::NAN)],
                noise: SimulationNoise::None,
            }),
        ),
        (
            "transform.simulate.noise",
            AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
                seed: 1,
                row_count: 5,
                replicates: 1,
                x_min: 0.0,
                x_max: 10.0,
                equation_id: String::from("line"),
                parameters: vec![],
                noise: SimulationNoise::Gaussian { sd: f64::INFINITY },
            }),
        ),
        ("descriptive", AnalysisKind::DescriptiveStats(DescriptiveSpec { custom_upper_percentile: Some(f64::NAN), ..DescriptiveSpec::default() })),
        ("outlier.grubbs", AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: f64::NAN, iterative: false })),
        ("outlier.rout", AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: f64::INFINITY })),
        (
            "two_group.one_sample",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::OneSampleT { hypothetical: f64::NAN },
                ..TwoGroupSpec::default()
            }),
        ),
        (
            "multi_row.multiplicity",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                multiplicity: MultiplicitySpec { threshold: f64::INFINITY, ..MultiplicitySpec::default() },
                ..MultipleRowTestsSpec::default()
            }),
        ),
        (
            "one_way.multiplicity",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                multiplicity: MultiplicitySpec { threshold: f64::NAN, ..MultiplicitySpec::default() },
                ..OneWayAnovaSpec::default()
            }),
        ),
        (
            "factorial.multiplicity",
            AnalysisKind::FactorialAnova(FactorialAnovaSpec {
                multiplicity: MultiplicitySpec { threshold: f64::NEG_INFINITY, ..MultiplicitySpec::default() },
                ..FactorialAnovaSpec::default()
            }),
        ),
        (
            "nested.multiplicity",
            AnalysisKind::NestedComparison(NestedAnalysisSpec {
                two_group_t_test: false,
                multiplicity: MultiplicitySpec { threshold: f64::NAN, ..MultiplicitySpec::default() },
                ..NestedAnalysisSpec::default()
            }),
        ),
        (
            "contingency.expected",
            AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(vec![0.5, f64::NAN]), ..ContingencySpec::default() }),
        ),
        (
            "survival.multiplicity",
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec { threshold: f64::INFINITY, ..MultiplicitySpec::default() },
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        ),
        (
            "nonlinear.weighting",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                weighting: WeightingScheme::GeneralPowerY(f64::NAN),
                ..NonlinearFitSpec::new("four_pl")
            }),
        ),
        (
            "nonlinear.rout",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                method: FitMethod::RoutOutlierElimination { fdr_q_percent: f64::INFINITY },
                ..NonlinearFitSpec::new("four_pl")
            }),
        ),
        (
            "nonlinear.rule_bound",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                parameter_rules: vec![ParameterRule {
                    name: String::from("EC50"),
                    scope: ParameterScope::Constant(f64::NAN),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                }],
                ..NonlinearFitSpec::new("four_pl")
            }),
        ),
    ];

    for (label, kind) in bad_kinds {
        let mut d = base.clone();
        d.result_sheets[0].analysis.kind = kind;
        assert_save_paths_reject_nonfinite(label, &d)?;
    }
    Ok(())
}

fn import_spec_at(anchor: CellCoord) -> ImportLinkSpec {
    ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("test.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor,
            propagate_title_header: true,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    }
}

#[test]
fn r1_s06_13_apply_headers_to_sheet_respects_subcolumns_across_archetypes() -> Result<(), FormatError> {
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 3, 2)?;
    xy_rep.data_sets[0].title = String::from("Init0");
    xy_rep.data_sets[1].title = String::from("Init1");
    xy_rep.data_sets[2].title = String::from("Init2");
    let mut xy_rep_sheet = DataSheet::new("xy-rep", "XY Rep", TableArchetype::Xy(xy_rep))?;

    let csv_rep2 = "Ctrl_R1,Ctrl_R2,Treat_R1,Treat_R2\n1,2,3,4\n5,6,7,8\n";
    apply_csv_import_link(&mut xy_rep_sheet, csv_rep2, &import_spec_at(CellCoord::new(0, 0, 0)), true, ImportBaselineAdjustment::None, "l1", "rep2")?;
    let TableArchetype::Xy(xrep) = &xy_rep_sheet.archetype else {
        panic!("expected XY");
    };
    assert_eq!(xrep.data_sets[0].title, "Ctrl_R1");
    assert_eq!(xrep.data_sets[1].title, "Treat_R1");
    assert_eq!(xrep.data_sets[2].title, "Init2");
    assert_eq!(xy_rep_sheet.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(1.0));
    assert_eq!(xy_rep_sheet.cell(CellCoord::new(0, 0, 1)).and_then(|c| c.active_number()), Some(2.0));
    assert_eq!(xy_rep_sheet.cell(CellCoord::new(0, 1, 0)).and_then(|c| c.active_number()), Some(3.0));
    assert_eq!(xy_rep_sheet.cell(CellCoord::new(0, 1, 1)).and_then(|c| c.active_number()), Some(4.0));

    let mut grp_off = GroupedTable::new(SubcolumnMode::RawReplicates(2), 3, 1)?;
    grp_off.data_sets[0].title = String::from("Keep0");
    grp_off.data_sets[1].title = String::from("Keep1");
    grp_off.data_sets[2].title = String::from("Keep2");
    let mut grp_off_sheet = DataSheet::new("grp-off", "Grouped2", TableArchetype::Grouped(grp_off))?;
    let csv_offset = "Sub1Of0,NewDS1,Sub1Of1,NewDS2\n10,20,30,40\n";
    apply_csv_import_link(
        &mut grp_off_sheet,
        csv_offset,
        &import_spec_at(CellCoord::new(0, 0, 1)),
        true,
        ImportBaselineAdjustment::None,
        "l2",
        "offset subcol",
    )?;
    let TableArchetype::Grouped(g2) = &grp_off_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(g2.data_sets[0].title, "Keep0");
    assert_eq!(g2.data_sets[1].title, "NewDS1");
    assert_eq!(g2.data_sets[2].title, "NewDS2");
    assert_eq!(grp_off_sheet.cell(CellCoord::new(0, 0, 1)).and_then(|c| c.active_number()), Some(10.0));
    assert_eq!(grp_off_sheet.cell(CellCoord::new(0, 1, 0)).and_then(|c| c.active_number()), Some(20.0));
    assert_eq!(grp_off_sheet.cell(CellCoord::new(0, 1, 1)).and_then(|c| c.active_number()), Some(30.0));
    assert_eq!(grp_off_sheet.cell(CellCoord::new(0, 2, 0)).and_then(|c| c.active_number()), Some(40.0));

    let mean_sd_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 2, 1)?;
    let mut mean_sd_sheet = DataSheet::new("col-msd", "PreSummarized", TableArchetype::Column(mean_sd_tbl))?;
    apply_csv_import_link(
        &mut mean_sd_sheet,
        "GroupA,SD_ignored,N_ignored,GroupB,SD_b,N_b\n10,1.5,4,20,2.5,5\n",
        &import_spec_at(CellCoord::new(0, 0, 0)),
        true,
        ImportBaselineAdjustment::None,
        "l3",
        "mean sd n",
    )?;
    let TableArchetype::Column(cmsd) = &mean_sd_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(cmsd.data_sets[0].title, "GroupA");
    assert_eq!(cmsd.data_sets[1].title, "GroupB");

    let grp_tbl = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 1)?;
    let mut grp_sheet = DataSheet::new("grp-1", "Grouped", TableArchetype::Grouped(grp_tbl))?;
    apply_csv_import_link(
        &mut grp_sheet,
        "Vehicle_1,Vehicle_2,Vehicle_3,Drug_1,Drug_2,Drug_3\n1,2,3,4,5,6\n",
        &import_spec_at(CellCoord::new(0, 0, 0)),
        true,
        ImportBaselineAdjustment::None,
        "l4",
        "grouped",
    )?;
    let TableArchetype::Grouped(gt) = &grp_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(gt.data_sets[0].title, "Vehicle_1");
    assert_eq!(gt.data_sets[1].title, "Drug_1");

    let xy_tbl = XyTable::new(SubcolumnMode::MeanSemN, 2, 1)?;
    let mut xy_sheet = DataSheet::new("xy-1", "XY", TableArchetype::Xy(xy_tbl))?;
    apply_csv_import_link(
        &mut xy_sheet,
        "DoseA,SEM_A,N_A,DoseB,SEM_B,N_B\n10,1,3,20,2,4\n",
        &import_spec_at(CellCoord::new(0, 0, 0)),
        true,
        ImportBaselineAdjustment::None,
        "l5",
        "xy mean sem n",
    )?;
    let TableArchetype::Xy(xt) = &xy_sheet.archetype else {
        panic!("expected XY");
    };
    assert_eq!(xt.data_sets[0].title, "DoseA");
    assert_eq!(xt.data_sets[1].title, "DoseB");
    Ok(())
}
