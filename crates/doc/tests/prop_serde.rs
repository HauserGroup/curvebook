#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, CiBound,
    ColorChannelMapping, ColumnDesignHint, ColumnTable, ConfidenceLevel, ConstantValue, ContingencyMethod, ContingencySpec, ContingencyTable,
    CurveSamplePoint, DataSheet, DescriptiveSpec, DiagnosticPlotKind, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FactorialAnovaSpec, FitMethod,
    FittedCurveOverlay, FloatingNote, GlmFamily, GlmSpec, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, GroupedArrangement,
    GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet,
    LayoutSpec, LegendSpec, LevelOrderKind, LevelOrderSpec, LinearRegressionSpec, LinkSource, LinkedDataRegion, LogitDomain, MathTransformPreset,
    ModelComparisonSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode,
    MultivariateSpec, NestedAnalysisSpec, NestedTable, NonlinearFitSpec, NormalizeAnchor, NoteId, OneWayAnovaSpec, OneWayMethod, OutlierSpec,
    PValueFormat, PairwiseBracketSpec, PairwiseComparisonRecord, PanelBadgeStyle, ParameterRule, ParameterScope, PartsOfWholeLayout,
    PartsOfWholeTable, PhysicalUnit, PlacementMode, ProjectDocument, RectPt, RegressionOverlaySpec, RepeatedMeasuresDesign, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, RocCiMethod, RocSpec, SequenceProgression, SheetCellRange, SheetId,
    SimulationNoise, SizeChannelMapping, SizePt, SubcolumnMode, SumOfSquaresType, SummaryMeasure, SurvivalAnalysisSpec, SurvivalTable,
    TableArchetype, TickLabelFormat, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, WeightingScheme, WhiskerMethod, XColumnMode,
    XyTable,
};
use lumencraft_palette::{
    CategoricalPalette, Cmyk, ColorRamp, ColorStop, ColormapId, CvdKind, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8,
};
use proptest::prelude::*;

fn assert_json_roundtrip<T>(value: &T)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value);
}

fn arb_finite_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        (-1e12_f64..1e12_f64),
        Just(0.0_f64),
        Just(-0.0_f64),
        Just(1e-300_f64),
        Just(-1e-300_f64),
        Just(f64::MIN_POSITIVE),
        Just(-f64::MIN_POSITIVE),
        Just(-1.234_567_890_123_456_7e12_f64),
        Just(core::f64::consts::PI),
    ]
}

fn arb_cell() -> impl Strategy<Value = Cell> {
    let val_strat =
        prop_oneof![Just(CellValue::Empty), arb_finite_f64().prop_map(CellValue::Number), "[a-zA-Z0-9 _.-]{0,16}".prop_map(CellValue::Text),];
    (val_strat, any::<bool>()).prop_map(|(value, excluded)| Cell { value, excluded })
}

fn arb_subcolumn_mode() -> impl Strategy<Value = SubcolumnMode> {
    prop_oneof![
        Just(SubcolumnMode::Single),
        (1usize..=8usize).prop_map(SubcolumnMode::RawReplicates),
        Just(SubcolumnMode::MeanSdN),
        Just(SubcolumnMode::MeanSemN),
        Just(SubcolumnMode::MeanCvN),
        Just(SubcolumnMode::MeanLowHigh),
        Just(SubcolumnMode::UpperLowerLimits),
        Just(SubcolumnMode::MedianIqr),
    ]
}

fn arb_rgba8() -> impl Strategy<Value = Rgba8> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(r, g, b, a)| Rgba8::rgba(r, g, b, a))
}

fn arb_axis_scale() -> impl Strategy<Value = AxisScale> {
    prop_oneof![
        Just(AxisScale::Linear),
        Just(AxisScale::Log10),
        Just(AxisScale::Log2),
        Just(AxisScale::Ln),
        Just(AxisScale::Logit(LogitDomain::Percent)),
        Just(AxisScale::Logit(LogitDomain::Proportion)),
        Just(AxisScale::Probit),
        Just(AxisScale::Reciprocal),
    ]
}

fn arb_pvalue_format() -> impl Strategy<Value = PValueFormat> {
    prop_oneof![
        Just(PValueFormat::FourDecimal),
        Just(PValueFormat::Apa),
        Just(PValueFormat::Nejm),
        (1u8..=15u8, prop::option::of(1e-12_f64..1.0_f64))
            .prop_map(|(decimals, scientific_below)| PValueFormat::new_custom(decimals, scientific_below)),
    ]
}

fn arb_simulation_noise() -> impl Strategy<Value = SimulationNoise> {
    prop_oneof![
        Just(SimulationNoise::None),
        (0.0_f64..100.0_f64).prop_map(|sd| SimulationNoise::Gaussian { sd }),
        (0.0_f64..50.0_f64).prop_map(|cv_percent| SimulationNoise::RelativePercent { cv_percent }),
        Just(SimulationNoise::Poisson),
    ]
}

fn arb_simulate_spec() -> impl Strategy<Value = TransformAnalysisSpec> {
    (
        any::<u64>(),
        1usize..=200usize,
        1usize..=16usize,
        -1000.0_f64..0.0_f64,
        0.01_f64..1000.0_f64,
        "[a-z0-9_]{3,20}",
        prop::collection::vec(("[a-zA-Z0-9_]{1,12}", arb_finite_f64()), 0..=6),
        arb_simulation_noise(),
    )
        .prop_map(|(seed, row_count, replicates, x_min, x_max, equation_id, parameters, noise)| TransformAnalysisSpec::Simulate {
            seed,
            row_count,
            replicates,
            x_min,
            x_max,
            equation_id,
            parameters,
            noise,
        })
}

#[test]
fn extreme_finite_floats_round_trip_bit_exact() {
    let extremes = [1e-300_f64, -1e-300_f64, f64::MIN_POSITIVE, -f64::MIN_POSITIVE, -1.234_567_890_123_456_7e12_f64, 0.0_f64];
    for &x in &extremes {
        let cell = Cell::number(x);
        let json = serde_json::to_string(&cell).unwrap();
        let back: Cell = serde_json::from_str(&json).unwrap();
        let CellValue::Number(y) = back.value else {
            panic!("expected Number cell");
        };
        assert_eq!(x.to_bits(), y.to_bits(), "bit mismatch for {x:e}: json={json}");
    }
}

#[test]
fn invalid_confidence_levels_rejected_on_json_deserialize() {
    for raw in ["0.0", "-5.0", "100.0", "100.001", "0.0099", "null", "\"95.0\""] {
        let parsed: Result<ConfidenceLevel, _> = serde_json::from_str(raw);
        assert!(parsed.is_err(), "expected {raw} to be rejected, got {parsed:?}");
    }
    for valid in ["0.01", "50.0", "95.0", "99.99"] {
        let level: ConfidenceLevel = serde_json::from_str(valid).unwrap();
        assert_json_roundtrip(&level);
    }
}

#[test]
fn full_project_document_covers_all_eight_archetypes_specs_results_graphs_layouts_and_palette() {
    let mut proj = ProjectDocument::new("Comprehensive Schema Project").unwrap();

    // InfoSheet
    let mut info = InfoSheet::new_preseeded("info-1", "Lab Notes").unwrap();
    info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM"))).unwrap();
    info.set_constant("Operator", ConstantValue::Text(String::from("Dr. Ada")), None).unwrap();
    info.set_constant("RunDate", ConstantValue::Date(String::from("2026-11-13")), None).unwrap();
    info.notes_markdown = String::from("# Assay\nAll controls verified.");
    info.color_tag = Some(Rgba8::rgb(0x00, 0x72, 0xB2));
    info.floating_notes.push(FloatingNote::new("note-info", RectPt::new(10.0, 20.0, 150.0, 80.0), "Pinned note"));
    proj.add_info_sheet(info).unwrap();

    // 1. XY
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::RawReplicates(3), 2, 3).unwrap();
    xy.x_cells[0] = vec![Cell::number(1e-9), Cell::number(1e-10), Cell::number(3.0)];
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::excluded_number(99.9), Cell::number(12.0)];
    let mut xy_sheet = DataSheet::new("ds-xy", "Dose Response", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.info_sheet_id = Some(SheetId::new("info-1"));
    xy_sheet
        .add_linked_region(LinkedDataRegion {
            id: String::from("link-ext"),
            spec: ImportLinkSpec {
                source: LinkSource::ExternalDelimitedFile { path: String::from("data/assay.csv"), delimiter: ',', decimal_separator: '.' },
                filter: ImportFilterSpec {
                    row_window: Some((0, 10)),
                    col_window: Some((0, 3)),
                    decimate_every_k: Some(2),
                    unstack_group_column: None,
                    parse_asterisk_exclusions: true,
                },
                placement: ImportPlacementSpec {
                    mode: PlacementMode::Overwrite,
                    anchor: CellCoord::new(0, 0, 0),
                    propagate_title_header: true,
                    skip_contiguous_blanks: false,
                    transpose: false,
                },
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 2)),
            provenance_summary: String::from("CSV import"),
        })
        .unwrap();
    proj.add_data_sheet(xy_sheet.clone()).unwrap();

    // 2. Column
    let mut col_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanCvN, 2, 2).unwrap();
    col_tbl.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(5.0), Cell::number(6.0)];
    proj.add_data_sheet(DataSheet::new("ds-col", "Column", TableArchetype::Column(col_tbl)).unwrap()).unwrap();

    // 3. Grouped
    let grp_tbl = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 2, 2).unwrap();
    proj.add_data_sheet(DataSheet::new("ds-grp", "Grouped", TableArchetype::Grouped(grp_tbl)).unwrap()).unwrap();

    // 4. Contingency
    let mut cont_tbl = ContingencyTable::new(2, 2).unwrap();
    cont_tbl.data_sets[0].cells[0][0] = Cell::number(18.0);
    proj.add_data_sheet(DataSheet::new("ds-cont", "Contingency", TableArchetype::Contingency(cont_tbl)).unwrap()).unwrap();

    // 5. Survival
    let mut surv_tbl = SurvivalTable::new(2, 2).unwrap();
    surv_tbl.times = vec![Cell::number(4.5), Cell::number(12.0)];
    proj.add_data_sheet(DataSheet::new("ds-surv", "Survival", TableArchetype::Survival(surv_tbl)).unwrap()).unwrap();

    // 6. Parts of Whole
    let pow_tbl = PartsOfWholeTable::new(1, 3).unwrap();
    proj.add_data_sheet(DataSheet::new("ds-pow", "Parts", TableArchetype::PartsOfWhole(pow_tbl)).unwrap()).unwrap();

    // 7. Multiple Variables
    let mut mv_tbl = MultipleVariablesTable::new(2, 2).unwrap();
    mv_tbl.variables[0].lock_role(VariableRole::Categorical);
    mv_tbl.variables[0].level_order = LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("Control")]), reversed: true };
    mv_tbl.variables[1].set_calculated_formula("= [A] * 2");
    proj.add_data_sheet(DataSheet::new("ds-mv", "Multivariate", TableArchetype::MultipleVariables(mv_tbl)).unwrap()).unwrap();

    // 8. Nested
    let nest_tbl = NestedTable::new(2, 3, 4).unwrap();
    proj.add_data_sheet(DataSheet::new("ds-nest", "Nested", TableArchetype::Nested(nest_tbl)).unwrap()).unwrap();

    // Analyses & ResultSheet
    let mut sim_spec = AnalysisSpec::new(
        "res-sim",
        "ds-xy",
        AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed: 9001,
            row_count: 12,
            replicates: 4,
            x_min: -10.0,
            x_max: -4.0,
            equation_id: String::from("log_agonist_vs_response_variable_slope"),
            parameters: vec![
                (String::from("Bottom"), 0.0),
                (String::from("Top"), 100.0),
                (String::from("LogEC50"), -7.0),
                (String::from("HillSlope"), 1.0),
            ],
            noise: SimulationNoise::Gaussian { sd: 2.5 },
        }),
    )
    .unwrap();
    sim_spec.confidence_level = ConfidenceLevel::new(99.5).unwrap();
    sim_spec.p_value_format = PValueFormat::new_custom(3, Some(1e-4));

    let mut res_sheet = ResultSheet::new(sim_spec).unwrap();
    res_sheet.status = ResultStatus::Failed { code: String::from("E_TEST"), message: String::from("Synthetic diagnostic") };
    res_sheet.tabs.push(ResultTab {
        id: String::from("tab-1"),
        title: String::from("Summary"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![String::from("Parameter"), String::from("Estimate")],
        row_labels: vec![String::from("LogEC50")],
        rows: vec![vec![
            ResultCell::ConfidenceInterval { lower: CiBound::Finite(-7.2), upper: CiBound::Open },
            ResultCell::PValue { raw: 2.41e-9, formatted: String::from("2.41e-9"), stars: String::from("****") },
        ]],
        footnotes: vec![String::from("Profile likelihood CI")],
    });
    res_sheet.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 1e-7, units: Some(String::from("M")) });
    res_sheet.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("Ctrl"),
        label_b: String::from("Drug"),
        estimate_diff: Some(-12.4),
        raw_p_or_q: 0.0012,
        is_fdr_q: true,
        significant: true,
    });
    res_sheet.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("four_pl"),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint {
            x: -7.0,
            y: 50.0,
            ci_lower: Some(45.0),
            ci_upper: Some(55.0),
            pred_lower: Some(40.0),
            pred_upper: Some(60.0),
        }],
    });
    res_sheet.checklist.push(ChecklistItem {
        assumption: String::from("Normality of residuals"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("W = 0.98")),
        explanation: String::from("No evidence of non-normality."),
    });
    proj.add_result_sheet(res_sheet).unwrap();

    // GraphSheet covering palette, axis scales, GDOs, multivariate, brackets, annotations
    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap();
    gsheet.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Volcano);
    gsheet.spec.palette = PaletteId::PaulTolVibrant;
    gsheet.spec.x_axis = AxisSpec {
        segments: vec![AxisSegment {
            min: Some(0.01),
            max: Some(99.99),
            scale: AxisScale::Logit(LogitDomain::Percent),
            length_share: 1.0,
            major_tick_interval: Some(10.0),
            minor_ticks_per_interval: 2,
        }],
        tick_format: TickLabelFormat::PowerOfTenSuperscript,
        ..AxisSpec::linear("Logit %")
    };
    gsheet.spec.grouped_arrangement = GroupedArrangement::Separated;
    gsheet.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle {
            shape: MarkerShape::Star,
            fill_mode: MarkerFillMode::SemiFilledTop,
            size_pt: 5.5,
            fill_color: Rgba8::rgb(10, 20, 30),
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
        }),
        line: None,
        area_fill: None,
    };
    gsheet.spec.series[0].y_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::ConfidenceInterval95,
        direction: ErrorBarDirection::UpperOnly,
        dash_style: LineDashStyle::DashDotDot,
        stroke_width_pt: 1.25,
        cap_width_pt: 4.5,
        color: Some(Rgba8::BLACK),
    });
    gsheet.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-sim"),
        sample_segments: 512,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    gsheet.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::BoxAndWhisker {
                whiskers: WhiskerMethod::Tukey1_5Iqr,
                fill_color: Rgba8::WHITE,
                stroke_color: Rgba8::BLACK,
                show_outliers: true,
                show_all_points: false,
            },
            y_error_bars: None,
        },
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Continuous {
            column: 1,
            ramp: ColorRamp::Custom {
                stops: vec![ColorStop { position: 0.0, color: Rgba8::BLACK }, ColorStop { position: 1.0, color: Rgba8::WHITE }],
            },
            domain_clamp: Some((0.0, 1.0)),
        },
        size: SizeChannelMapping::Variable { column: 1, min_radius_pt: 2.0, max_radius_pt: 8.0 },
        label_column: Some(0),
        x_level_order_override: None,
        show_convex_hulls: true,
        show_confidence_ellipses: false,
    });
    gsheet.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-sim"),
        group_a: 0,
        group_b: 1,
        style: lumencraft_doc::BracketStyle::CurlyBrace,
        label_mode: lumencraft_doc::BracketLabelMode::SummaryStars,
        hide_if_non_significant: true,
        stroke_width_pt: 1.0,
        font_size_pt: 9.0,
        bracket_step_pt: 12.0,
    });
    gsheet.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 90.0, 30.0),
        markdown: String::from("EC50 = {{result:res-sim:EC50}}"),
        bordered: true,
    });
    gsheet.spec.legend = LegendSpec { visible: true, position_pt: Some((220.0, 30.0)), bordered: true };
    proj.add_graph_sheet(gsheet).unwrap();

    // LayoutSheet
    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    lspec.unit = PhysicalUnit::Millimeters;
    lspec.panel_badges = PanelBadgeStyle::LowerCaseLetters;
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("g-1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(10.0, 10.0, 200.0, 160.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    lspec.items.push(LayoutItem::LiveTable {
        id: String::from("t-1"),
        source_sheet_id: SheetId::new("res-sim"),
        tab_id: Some(String::from("tab-1")),
        bounds_pt: RectPt::new(220.0, 10.0, 180.0, 120.0),
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: String::from("c-1"),
        markdown: String::from("{{info:HotNM}}"),
        bounds_pt: RectPt::new(10.0, 180.0, 200.0, 40.0),
        font_size_pt: 10.0,
    });
    proj.add_layout_sheet(LayoutSheet::new("layout-1", "Main Figure", lspec).unwrap()).unwrap();

    assert_json_roundtrip(&proj);

    // Palette standalone types
    assert_json_roundtrip(&Rgba8::BLACK.to_linear());
    assert_json_roundtrip(&Rgba8::BLACK.to_oklab());
    assert_json_roundtrip(&Rgba8::BLACK.to_oklch());
    assert_json_roundtrip(&Cmyk::from_srgb(Rgba8::rgb(12, 34, 56)));
    assert_json_roundtrip(&CategoricalPalette::custom("MyPalette", vec![Rgba8::WHITE]));
    assert_json_roundtrip(&ColormapId::Cividis);
    assert_json_roundtrip(&CvdKind::Tritanopia);
    assert_json_roundtrip(&SummaryMeasure::GeometricMean);
    assert_json_roundtrip(&PartsOfWholeLayout::Waffle10x10);
    assert_json_roundtrip(&SizePt::default());
    assert_json_roundtrip(&NoteId::new("note-1"));

    // Exercise additional AnalysisKind variants to ensure every nested enum serializes cleanly.
    let extra_analyses = [
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-9 }),
            y_transform: Some(MathTransformPreset::LinearScale { slope: 2.0, intercept: -1.0 }),
            error_propagation: lumencraft_doc::ErrorPropagationMode::FirstOrderDeltaMethod,
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Max") }),
            output_percent: true,
            per_data_set: true,
        }),
        AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 1.0 }, ..TwoGroupSpec::default() }),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 0 },
                threshold: 0.05,
                report_confidence_intervals: true,
            },
        }),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            ss_type: SumOfSquaresType::TypeIII,
            repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
            ..FactorialAnovaSpec::default()
        }),
        AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, multiplicity: MultiplicitySpec::default() }),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FisherExact, ..ContingencySpec::default() }),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
        }),
        AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: vec![(1, String::from("Baseline"))],
            compute_vif: true,
        }),
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(123) }),
        AnalysisKind::LinearRegression(LinearRegressionSpec::default()),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
            weighting: WeightingScheme::GeneralPowerY(2.0),
            parameter_rules: vec![ParameterRule {
                name: String::from("Top"),
                scope: ParameterScope::Constant(100.0),
                initial_override: Some(90.0),
                lower_bound: Some(0.0),
                upper_bound: Some(200.0),
            }],
            comparison: Some(ModelComparisonSpec::Aicc { alternative_equation_id: String::from("three_pl") }),
            ..NonlinearFitSpec::new("four_pl")
        }),
    ];
    for kind in extra_analyses {
        assert_json_roundtrip(&kind);
    }
}

proptest! {
    #[test]
    fn prop_cell_and_xy_table_round_trip(
        cells in prop::collection::vec(arb_cell(), 1..=16),
        submode in arb_subcolumn_mode(),
        progression in prop_oneof![Just(SequenceProgression::Arithmetic), Just(SequenceProgression::Geometric)],
        start in arb_finite_f64(),
        step in arb_finite_f64(),
    ) {
        for cell in &cells {
            assert_json_roundtrip(cell);
        }

        let mut xy = XyTable::new(submode, 1, 2).unwrap();
        xy.x_mode = XColumnMode::GeneratedSequence { start, step, progression };
        let subs = submode.subcolumn_count();
        for (i, cell) in cells.iter().take(subs).enumerate() {
            xy.data_sets[0].cells[0][i] = cell.clone();
        }
        assert_json_roundtrip(&xy);
    }

    #[test]
    fn prop_confidence_level_pvalue_format_and_axis_scale_round_trip(
        pct in 0.01_f64..=99.99_f64,
        pfmt in arb_pvalue_format(),
        scale in arb_axis_scale(),
        rgba in arb_rgba8(),
    ) {
        let level = ConfidenceLevel::from_percent(pct).unwrap();
        assert_json_roundtrip(&level);
        assert_json_roundtrip(&pfmt);
        assert_json_roundtrip(&scale);
        assert_json_roundtrip(&rgba);
    }

    #[test]
    fn prop_simulate_analysis_spec_round_trip(
        sim in arb_simulate_spec(),
        pct in 0.01_f64..=99.99_f64,
        pfmt in arb_pvalue_format(),
        sheet_id in "[a-z][a-z0-9-]{1,12}",
        source_id in "[a-z][a-z0-9-]{1,12}",
    ) {
        let mut spec = AnalysisSpec::new(sheet_id, source_id, AnalysisKind::Transform(sim)).unwrap();
        spec.confidence_level = ConfidenceLevel::from_percent(pct).unwrap();
        spec.p_value_format = pfmt;
        assert_json_roundtrip(&spec);
    }
}
